//! Key loading from environment (.env, SEED_PHRASE). This is the only
//! module that should ever hold the real key in memory — keep it out
//! of logs, out of error messages that might get printed, out of
//! anything that crosses the stdio boundary to mcp-server.

use anyhow::Context;
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_protocol::consensus::NetworkType;
use bip0039::{English, Mnemonic};

pub fn load_root_key(network: NetworkType) -> anyhow::Result<UnifiedSpendingKey> {
    let raw = std::env::var("SEED_PHRASE")
        .context("SEED_PHRASE not set — check your .env file is present and loaded")?;

    // The riskiest API guesses as per your instructions!
    let mnemonic = Mnemonic::<English>::from_phrase(raw.trim())
        .context("failed to parse SEED_PHRASE as a valid BIP-39 mnemonic")?;
        
    let seed = mnemonic.to_seed(""); // No passphrase

    let usk = UnifiedSpendingKey::from_seed(&network_params(network), &seed, zip32::AccountId::ZERO)
        .context("failed to derive Unified Spending Key from seed")?;

    Ok(usk)
}

fn network_params(network: NetworkType) -> zcash_protocol::consensus::Network {
    match network {
        NetworkType::Test => zcash_protocol::consensus::Network::TestNetwork,
        NetworkType::Main => zcash_protocol::consensus::Network::MainNetwork,
        NetworkType::Regtest => zcash_protocol::consensus::Network::TestNetwork,
    }
}
