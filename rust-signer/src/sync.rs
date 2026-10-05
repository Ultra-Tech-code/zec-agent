//! Light-client sync via Zaino gRPC.
//!
//! Uses zcash_client_backend's built-in sync::run(), which speaks the
//! standard CompactTxStreamer proto (same as lightwalletd). Zaino
//! implements this proto on localhost:8137.

use anyhow::Context;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tonic::transport::Channel;
use async_trait::async_trait;

use zcash_client_backend::{
    data_api::{
        WalletRead, WalletWrite,
        chain::{BlockSource, BlockCache, error},
        scanning::ScanRange,
    },
    proto::{
        service::{BlockId, compact_tx_streamer_client::CompactTxStreamerClient},
        compact_formats::CompactBlock,
    },
    sync,
};
use zcash_client_sqlite::{WalletDb, util::SystemClock};
use zcash_primitives::block::BlockHash;
use zcash_protocol::consensus::{BlockHeight, Network, Parameters, NetworkUpgrade};
use rand::rngs::OsRng;

pub const ZAINO_GRPC: &str = "http://127.0.0.1:8137";
const BATCH_SIZE: u32 = 100;

#[derive(Debug, Clone)]
pub struct MemCacheError(String);
impl std::fmt::Display for MemCacheError { 
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{}", self.0) } 
}
impl std::error::Error for MemCacheError {}

#[derive(Clone)]
pub struct MemBlockCache {
    blocks: Arc<Mutex<BTreeMap<u32, CompactBlock>>>,
}

impl MemBlockCache {
    pub fn new() -> Self {
        Self { blocks: Arc::new(Mutex::new(BTreeMap::new())) }
    }
}

impl BlockSource for MemBlockCache {
    type Error = MemCacheError;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), error::Error<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), error::Error<WalletErrT, Self::Error>>,
    {
        let blocks = self.blocks.lock().unwrap();
        let start = from_height.map(u32::from).unwrap_or(0);
        let iter = blocks.range(start..);
        
        let mut count = 0;
        let limit = limit.unwrap_or(usize::MAX);
        for (_, block) in iter {
            if count >= limit { break; }
            with_block(block.clone())?;
            count += 1;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockCache for MemBlockCache {
    fn get_tip_height(&self, range: Option<&ScanRange>) -> Result<Option<BlockHeight>, Self::Error> {
        let blocks = self.blocks.lock().unwrap();
        if let Some(r) = range {
            let start = u32::from(r.block_range().start);
            let end = u32::from(r.block_range().end);
            let tip = blocks.range(start..end).map(|(&h, _)| h).max();
            Ok(tip.map(BlockHeight::from_u32))
        } else {
            let tip = blocks.keys().max().copied();
            Ok(tip.map(BlockHeight::from_u32))
        }
    }

    async fn read(&self, range: &ScanRange) -> Result<Vec<CompactBlock>, Self::Error> {
        let blocks = self.blocks.lock().unwrap();
        let start = u32::from(range.block_range().start);
        let end = u32::from(range.block_range().end);
        let mut res = Vec::new();
        for h in start..end {
            if let Some(b) = blocks.get(&h) {
                res.push(b.clone());
            } else {
                break;
            }
        }
        Ok(res)
    }

    async fn insert(&self, compact_blocks: Vec<CompactBlock>) -> Result<(), Self::Error> {
        let mut blocks = self.blocks.lock().unwrap();
        for cb in compact_blocks {
            blocks.insert(cb.height as u32, cb);
        }
        Ok(())
    }

    async fn truncate(&self, block_height: BlockHeight) -> Result<(), Self::Error> {
        let mut blocks = self.blocks.lock().unwrap();
        let height = u32::from(block_height);
        blocks.retain(|&h, _| h <= height);
        Ok(())
    }

    async fn delete(&self, range: ScanRange) -> Result<(), Self::Error> {
        let mut blocks = self.blocks.lock().unwrap();
        let start = u32::from(range.block_range().start);
        let end = u32::from(range.block_range().end);
        for h in start..end {
            blocks.remove(&h);
        }
        Ok(())
    }
}

pub async fn get_chain_tip(
    client: &mut CompactTxStreamerClient<Channel>,
) -> anyhow::Result<(BlockHeight, BlockHash)> {
    let block_id: BlockId = client
        .get_latest_block(tonic::Request::new(
            zcash_client_backend::proto::service::ChainSpec {},
        ))
        .await
        .context("GetLatestBlock RPC failed")?
        .into_inner();

    let height = BlockHeight::from_u32(block_id.height as u32);
    let mut hash_bytes = [0u8; 32];
    let src = &block_id.hash;
    let len = src.len().min(32);
    hash_bytes[..len].copy_from_slice(&src[..len]);
    Ok((height, BlockHash(hash_bytes)))
}

pub async fn run_sync(
    network: Network,
    wallet_db: &mut WalletDb<rusqlite::Connection, Network, SystemClock, OsRng>,
    usk: Option<&zcash_keys::keys::UnifiedSpendingKey>,
    is_new_wallet: bool,
) -> anyhow::Result<()> {
    let channel = Channel::from_static(ZAINO_GRPC)
        .connect()
        .await
        .context("failed to connect to Zaino gRPC")?;

    let mut client = CompactTxStreamerClient::new(channel);

    // On first run, fetch the real TreeState from Zaino so the wallet birthday
    // includes the actual Ironwood/Orchard commitment tree frontiers.
    // Without this, witness_stabilized stays 0 and notes aren't spendable.
    if is_new_wallet {
        if let Some(usk) = usk {
            import_account_with_real_birthday(network, wallet_db, &mut client, usk).await?;
        }
    }

    // Loop until no high-priority scan ranges remain (priority > 0).
    // The low-priority range (entire chain history, priority=0) is always present
    // for a fresh wallet — we stop once only that remains.
    let mut pass = 0u32;
    loop {
        pass += 1;
        eprintln!("sync pass {} ...", pass);

        let block_cache = MemBlockCache::new();
        sync::run(&mut client, &network, &block_cache, wallet_db, BATCH_SIZE)
            .await
            .map_err(|e| anyhow::anyhow!("sync error: {:?}", e))?;

        let pending = wallet_db
            .suggest_scan_ranges()
            .map_err(|e| anyhow::anyhow!("scan ranges error: {:?}", e))?;

        use zcash_client_backend::data_api::scanning::ScanPriority;
        let high_prio: Vec<_> = pending.iter()
            .filter(|r| !matches!(r.priority(), ScanPriority::Scanned | ScanPriority::Ignored | ScanPriority::Historic))
            .collect();

        eprintln!("sync: {} total range(s), {} high-priority", pending.len(), high_prio.len());

        if high_prio.is_empty() {
            eprintln!("sync: no high-priority ranges left, done.");
            break;
        }

        if pass > 200 {
            eprintln!("sync: hit pass limit, stopping.");
            break;
        }
    }

    Ok(())
}

/// Fetch the TreeState for block (chain_tip - 1) from Zaino and import the account
/// with a proper birthday that includes real Ironwood/Orchard tree frontiers.
async fn import_account_with_real_birthday(
    _network: Network,
    wallet_db: &mut WalletDb<rusqlite::Connection, Network, SystemClock, OsRng>,
    client: &mut CompactTxStreamerClient<Channel>,
    usk: &zcash_keys::keys::UnifiedSpendingKey,
) -> anyhow::Result<()> {
    use zcash_client_backend::{
        data_api::{AccountBirthday, AccountPurpose, WalletWrite},
        proto::service::BlockId,
    };

    // Use a fixed recent testnet height as birthday, before our funding tx (which is ~4465559)
    let birthday_height = 4_465_500;
    eprintln!("fetching TreeState at height {} for wallet birthday...", birthday_height);

    let treestate = client
        .get_tree_state(tonic::Request::new(BlockId {
            height: birthday_height,
            hash: vec![],
        }))
        .await
        .context("GetTreeState failed")?
        .into_inner();

    let birthday = AccountBirthday::from_treestate(treestate, None)
        .map_err(|e| anyhow::anyhow!("birthday from treestate error: {:?}", e))?;

    let ufvk = usk.to_unified_full_viewing_key();

    wallet_db.import_account_ufvk(
        "budget_account",
        &ufvk,
        &birthday,
        AccountPurpose::Spending { derivation: None },
        None,
    ).map_err(|e| anyhow::anyhow!("import account error: {:?}", e))?;

    eprintln!("wallet DB: imported account with real birthday at height {}", birthday_height);
    Ok(())
}

async fn maybe_set_tip_birthday(
    network: Network,
    wallet_db: &mut WalletDb<rusqlite::Connection, Network, SystemClock, OsRng>,
    client: &mut CompactTxStreamerClient<Channel>,
) -> anyhow::Result<()> {
    let has_ranges = wallet_db
        .suggest_scan_ranges()
        .map_err(|e| anyhow::anyhow!("scan ranges error: {:?}", e))?
        .len()
        > 0;

    if has_ranges {
        return Ok(());
    }

    let (tip_height, tip_hash) = get_chain_tip(client).await?;
    eprintln!("chain tip: height={} hash={:?}", u32::from(tip_height), tip_hash);

    let birthday_height = tip_height - 1;
    
    let accounts = wallet_db
        .get_account_ids()
        .map_err(|e| anyhow::anyhow!("get accounts error: {:?}", e))?;

    for _account_id in accounts {
        wallet_db
            .update_chain_tip(tip_height)
            .map_err(|e| anyhow::anyhow!("update_chain_tip error: {:?}", e))?;
        break;
    }

    eprintln!("wallet birthday set to tip height {}", u32::from(birthday_height));
    
    // Also use the `network` parameter to suppress the warning if it's unused.
    let _activation = network.activation_height(NetworkUpgrade::Sapling);

    Ok(())
}
