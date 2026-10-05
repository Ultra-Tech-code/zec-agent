//! rust-signer — the only process in this system that touches real Zcash key
//! material.
//!
//! Protocol: one JSON object per line on stdin, one JSON object per line
//! on stdout. Logs go to stderr so they never corrupt the stdout protocol
//! stream that mcp-server's SignerClient is reading.

pub mod rpc;
pub mod dummy_provers;
mod rpc_config;
pub mod keys;
pub mod wallet_db;
pub mod sync;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::sync::Arc;
use tokio::sync::Mutex;
use zcash_protocol::consensus::{NetworkType, Network};
use wallet_db::SignerWalletDb;

#[derive(Deserialize)]
struct Request {
    id: String,
    method: String,
    params: serde_json::Value,
}

#[derive(Serialize)]
struct Response {
    id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    eprintln!("rust-signer starting (Ironwood/NU6.3)");

    dotenvy::dotenv().ok();

    let mut network_type = NetworkType::Main;
    let mut network = Network::MainNetwork;

    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|x| x == "--network") {
        if let Some(net) = args.get(pos + 1) {
            if net == "testnet" {
                network_type = NetworkType::Test;
                network = Network::TestNetwork;
            }
        }
    }

    let usk = match keys::load_root_key(network_type) {
        Ok(k) => {
            eprintln!("successfully derived root USK from seed phrase!");
            k
        }
        Err(e) => {
            eprintln!("startup failed: could not load root key: {:#}", e);
            std::process::exit(1);
        }
    };

    let (mut db, is_new) = match wallet_db::open_or_init(network, &usk) {
        Ok((d, new)) => {
            eprintln!("wallet DB ready (new: {})", new);
            (d, new)
        }
        Err(e) => {
            eprintln!("startup failed: wallet DB error: {:#}", e);
            std::process::exit(1);
        }
    };

    // Run an initial sync to pick up tip birthday and any incoming funds.
    eprintln!("syncing with Zaino at {} ...", sync::ZAINO_GRPC);
    if let Err(e) = sync::run_sync(network, &mut db, Some(&usk), is_new).await {
        // Sync failing is non-fatal on startup — we can still serve requests
        // and the balance will just be stale / zero.
        eprintln!("initial sync warning: {:#}", e);
    } else {
        eprintln!("sync complete");
    }

    // Wrap in Arc<Mutex<>> so the handler can read balance.
    let db = Arc::new(Mutex::new(db));

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let db_clone = Arc::clone(&db);
        let usk_clone = usk.clone();

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle(req, network, db_clone, usk_clone).await,
            Err(e) => Response {
                id: "unknown".to_string(),
                ok: false,
                result: None,
                error: Some(format!("bad request json: {e}")),
            },
        };

        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }

    Ok(())
}

async fn handle(
    req: Request,
    network: Network,
    db: Arc<Mutex<SignerWalletDb>>,
    usk: zcash_client_backend::keys::UnifiedSpendingKey,
) -> Response {
    let result = match req.method.as_str() {
        "get_balance" => get_balance(&req.params, db).await,
        "derive_subaccount" => derive_subaccount(&req.params, network, &usk),
        "send" => send(&req.params, network, db, &usk).await,
        "scan_memos" => scan_memos(&req.params),
        other => Err(anyhow::anyhow!("unknown method: {other}")),
    };

    match result {
        Ok(value) => Response { id: req.id, ok: true, result: Some(value), error: None },
        Err(e) => Response { id: req.id, ok: false, result: None, error: Some(e.to_string()) },
    }
}

async fn get_balance(
    params: &serde_json::Value,
    db: Arc<Mutex<SignerWalletDb>>,
) -> anyhow::Result<serde_json::Value> {
    let sub_account_index = params
        .get("sub_account_index")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("missing sub_account_index"))?;

    let db = db.lock().await;
    let zatoshi = wallet_db::read_balance(&db)?;

    Ok(serde_json::json!({
        "zatoshi": zatoshi.to_string(),
        "sub_account_index": sub_account_index,
    }))
}

fn derive_subaccount(
    params: &serde_json::Value,
    network: Network,
    usk: &zcash_client_backend::keys::UnifiedSpendingKey,
) -> anyhow::Result<serde_json::Value> {
    use zcash_keys::keys::UnifiedAddressRequest;
    
    let ufvk = usk.to_unified_full_viewing_key();
    let req = UnifiedAddressRequest::AllAvailableKeys;
    let (addr, _) = ufvk.default_address(req).map_err(|e| anyhow::anyhow!("failed to derive address: {:?}", e))?;
    
    Ok(serde_json::json!({
        "index": 0,
        "address": addr.encode(&network),
        "stub": false
    }))
}

async fn send(
    params: &serde_json::Value,
    network: Network,
    db: Arc<Mutex<SignerWalletDb>>,
    usk: &zcash_client_backend::keys::UnifiedSpendingKey,
) -> anyhow::Result<serde_json::Value> {
    use std::str::FromStr;
    use zcash_keys::address::Address;
    use zcash_address::ZcashAddress;
    use zcash_protocol::value::Zatoshis;
    use zcash_client_backend::zip321::{TransactionRequest, Payment};
    use zcash_client_backend::data_api::wallet::input_selection::{GreedyInputSelector, SpendPolicy};
    use zcash_client_backend::fees::zip317::{SingleOutputChangeStrategy};
    use zcash_primitives::transaction::fees::zip317::FeeRule;
    use zcash_client_backend::ShieldedPool;
    use zcash_client_backend::fees::DustOutputPolicy;
    use zcash_client_backend::data_api::wallet::{propose_transfer, create_proposed_transactions, ConfirmationsPolicy, SpendingKeys};
    use zcash_client_backend::wallet::OvkPolicy;

    let dest_addr_str = params
        .get("destination_address")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing destination_address"))?;

    let amount_str = params
        .get("amount_zatoshi")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing amount_zatoshi"))?;

    let amount_u64 = u64::from_str(amount_str)
        .map_err(|_| anyhow::anyhow!("invalid amount_zatoshi"))?;

    let recipient = Address::decode(&network, dest_addr_str)
        .ok_or_else(|| anyhow::anyhow!("invalid destination address"))?;

    match &recipient {
        Address::Sapling(_) => {
            anyhow::bail!("this signer only supports Orchard/Ironwood-capable addresses");
        }
        Address::Unified(ua) => {
            if ua.orchard().is_none() && ua.transparent().is_none() {
                anyhow::bail!("this signer only supports Orchard/Ironwood-capable addresses");
            }
        }
        Address::Transparent(_) => {}
        Address::Tex(_) => {
            anyhow::bail!("this signer only supports Orchard/Ironwood-capable addresses");
        }
    }

    let amount = Zatoshis::from_u64(amount_u64)
        .map_err(|_| anyhow::anyhow!("invalid amount"))?;

    let z_addr = ZcashAddress::try_from_encoded(dest_addr_str)
        .map_err(|_| anyhow::anyhow!("invalid destination address for zip321"))?;

    let req = TransactionRequest::new(vec![Payment::without_memo(z_addr, amount)])
        .map_err(|e| anyhow::anyhow!("failed to create tx request: {:?}", e))?;

    let fee_rule = FeeRule::standard();
    let change_strategy = SingleOutputChangeStrategy::new(
        fee_rule,
        None,
        ShieldedPool::Orchard,
        DustOutputPolicy::default(),
    );
    let mut db_lock = db.lock().await;
    let selector = GreedyInputSelector::new();
    let accounts = zcash_client_backend::data_api::WalletRead::get_account_ids(&*db_lock)
        .map_err(|e| anyhow::anyhow!("failed to get accounts: {:?}", e))?;
    let account_id = accounts.first().copied().ok_or_else(|| anyhow::anyhow!("no account found"))?;

    let proposal = propose_transfer::<_, _, _, _, <crate::wallet_db::SignerWalletDb as zcash_client_backend::data_api::WalletCommitmentTrees>::Error>(
        &mut *db_lock,
        &network,
        account_id,
        &selector,
        &change_strategy,
        req,
        ConfirmationsPolicy::new_symmetrical(std::num::NonZeroU32::new(1).unwrap(), false),
        &SpendPolicy::default(),
        None,
        None,
    ).map_err(|e| anyhow::anyhow!("failed to propose transfer: {:?}", e))?;

    let spend_prover = crate::dummy_provers::DummySpendProver;
    let output_prover = crate::dummy_provers::DummyOutputProver;
    
    #[cfg(feature = "transparent-key-import")]
    let keys = SpendingKeys::new(usk.clone(), std::collections::HashMap::new());
    #[cfg(not(feature = "transparent-key-import"))]
    let keys = SpendingKeys::new(usk.clone());

    let txids = create_proposed_transactions::<
        _, _,
        std::convert::Infallible,
        _,
        std::convert::Infallible,
        _
    >(
        &mut *db_lock,
        &network,
        &spend_prover,
        &output_prover,
        &keys,
        OvkPolicy::Sender,
        &proposal,
        None,
    ).map_err(|e| anyhow::anyhow!("failed to create proposed transactions: {:?}", e))?;

    let txid = *txids.first();

    let tx = zcash_client_backend::data_api::WalletRead::get_transaction(&*db_lock, txid)
        .map_err(|e| anyhow::anyhow!("failed to read tx from db: {:?}", e))?
        .ok_or_else(|| anyhow::anyhow!("txid not found in db"))?;

    let mut raw_tx = Vec::new();
    tx.write(&mut raw_tx).map_err(|e| anyhow::anyhow!("failed to serialize tx: {}", e))?;

    let rpc_url = std::env::var("ZEBRA_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:18232".to_string());
    let confirmed_txid = broadcast_tx(&raw_tx, &rpc_url).await
        .map_err(|e| anyhow::anyhow!("failed to broadcast tx: {}", e))?;

    Ok(serde_json::json!({
        "txid": confirmed_txid,
        "stub": false,
        "note": "Transaction successfully broadcasted to network"
    }))
}

/// Broadcast a raw transaction via Zebra's sendrawtransaction RPC.
/// Ready for Phase 3 when the transaction builder is wired.
#[allow(dead_code)]
async fn broadcast_tx(raw_tx: &[u8], rpc_url: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let tx_hex = hex::encode(raw_tx);

    // Read Zebra cookie for basic auth (enable_cookie_auth = true in zebrad.toml)
    let cookie_path = std::env::var("ZEBRA_COOKIE_PATH")
        .unwrap_or_else(|_| "/Users/0xblackadam/Library/Caches/zebra/.cookie".to_string());
    let cookie = std::fs::read_to_string(&cookie_path)
        .with_context(|| format!("Failed to read Zebra cookie from {}", cookie_path))?;
    let cookie = cookie.trim();
    let (user, pass) = cookie.split_once(':')
        .ok_or_else(|| anyhow::anyhow!("Invalid cookie format (expected user:pass)"))?;

    let res: serde_json::Value = client
        .post(rpc_url)
        .basic_auth(user, Some(pass))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "sendrawtransaction",
            "params": [tx_hex],
            "id": 1
        }))
        .send()
        .await
        .context("HTTP request to Zebra failed")?
        .json()
        .await
        .context("Failed to parse Zebra JSON response")?;

    if let Some(err) = res.get("error") {
        if !err.is_null() {
            anyhow::bail!("Zebra RPC error: {}", err);
        }
    }

    res["result"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing txid in sendrawtransaction response: {:?}", res))
}

fn scan_memos(_params: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::json!({ "memos": [], "stub": true }))
}
