use std::path::PathBuf;
use zcash_client_sqlite::{
    WalletDb,
    util::SystemClock,
    wallet::init::init_wallet_db,
};
use zcash_client_backend::data_api::{WalletWrite, WalletRead};
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_protocol::consensus::Network;
use rand::rngs::OsRng;

pub type SignerWalletDb = WalletDb<rusqlite::Connection, Network, SystemClock, OsRng>;

/// Open (or create) the wallet DB, run migrations, and import the USK
/// as account 0 if no accounts exist yet. Idempotent — safe to call on
/// every startup.
///
/// Returns `(db, is_new)` where `is_new` is true if the account was just created.
/// When `is_new`, the caller must fetch a real TreeState from Zaino and call
/// `import_account_ufvk` with proper frontiers before first sync.
pub fn open_or_init(
    network: Network,
    usk: &UnifiedSpendingKey,
) -> anyhow::Result<(SignerWalletDb, bool)> {
    let db_path = PathBuf::from("wallet.sqlite");

    let mut db = WalletDb::for_path(&db_path, network, SystemClock, OsRng)
        .map_err(|e| anyhow::anyhow!("db open error: {}", e))?;

    init_wallet_db(&mut db, None)
        .map_err(|e| anyhow::anyhow!("db migration error: {:?}", e))?;

    // Only import if no accounts exist yet (idempotent startup).
    let existing = db.get_account_ids()
        .map_err(|e| anyhow::anyhow!("get_account_ids error: {:?}", e))?;

    if existing.is_empty() {
        // Birthday will be set in sync.rs after fetching real tree state from Zaino.
        // We return is_new=true so the caller can do this properly.
        eprintln!("wallet DB: new wallet, birthday will be set from Zaino tree state");
        Ok((db, true))
    } else {
        eprintln!("wallet DB: account already exists, skipping import");
        Ok((db, false))
    }
}

/// Read the real shielded balance for account 0 from the wallet DB.
/// Returns zatoshi as a u64, or 0 if the wallet hasn't synced yet.
pub fn read_balance(db: &SignerWalletDb) -> anyhow::Result<u64> {
    // use zcash_client_backend::data_api::AccountBalance;

    let accounts = db.get_account_ids()
        .map_err(|e| anyhow::anyhow!("get_account_ids: {:?}", e))?;

    let Some(account_id) = accounts.into_iter().next() else {
        return Ok(0);
    };

    let summary = db.get_wallet_summary(
        zcash_client_backend::data_api::wallet::ConfirmationsPolicy::default()
    )
        .map_err(|e| anyhow::anyhow!("get_wallet_summary: {:?}", e))?;

    let Some(summary) = summary else {
        return Ok(0);
    };

    let balance = summary.account_balances()
        .get(&account_id)
        .map(|b| u64::from(b.total()))
        .unwrap_or(0);

    Ok(balance)
}
