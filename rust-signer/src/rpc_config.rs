//! RPC endpoint configuration: local Zebra node vs. remote provider
//! (e.g. QuickNode), auto-detected from the URL, since the two need
//! different auth handling.
//!
//! Local Zebra (default): cookie auth, read from a file on disk, unless
//! the operator explicitly disabled cookie auth in zebrad.toml (common
//! when running lightwalletd/Zaino in front of it, since neither speaks
//! Zebra's cookie scheme).
//!
//! Remote provider (QuickNode, etc.): no cookie file exists locally —
//! auth is an API key, either embedded in the URL path or sent as a
//! header, depending on the provider. Zcash JSON-RPC has no
//! provider-agnostic standard for this, so this struct just distinguishes
//! "local, try cookie auth" from "remote, use whatever key was given."

use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum RpcAuth {
    /// Zebra cookie auth: read `__cookie__:<value>` from the given path.
    /// Only meaningful for local nodes; do not attempt this against a
    /// remote provider.
    Cookie { path: PathBuf },
    /// Bearer/API-key auth, sent however the provider expects
    /// (commonly baked into the URL itself for QuickNode-style
    /// endpoints, sometimes a header — confirm against your specific
    /// provider's docs before assuming one or the other).
    ApiKey { key: String },
    /// Cookie auth explicitly disabled (enable_cookie_auth = false in
    /// zebrad.toml) — the common local setup when Zaino/lightwalletd
    /// sits in front, since neither speaks Zebra's cookie scheme.
    None,
}

#[derive(Debug, Clone)]
pub struct RpcConfig {
    pub network: Network,
    pub rpc_url: String,
    pub auth: RpcAuth,
    pub is_local: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Testnet,
    Mainnet,
}

impl RpcConfig {
    /// Build config from a raw RPC URL, auto-detecting local vs. remote.
    ///
    /// Detection rule: loopback/private-range host => local node =>
    /// attempt cookie auth unless overridden. Anything else (a public
    /// hostname, e.g. QuickNode) => remote => require an explicit API
    /// key, never attempt to read a local cookie file for it.
    pub fn from_url(
        network: Network,
        rpc_url: &str,
        explicit_cookie_path: Option<PathBuf>,
        explicit_api_key: Option<String>,
    ) -> anyhow::Result<Self> {
        let is_local = is_local_host(rpc_url);

        let auth = if is_local {
            match explicit_api_key {
                // Local node behind something that still wants a key
                // (unusual, but don't assume it can't happen).
                Some(key) => RpcAuth::ApiKey { key },
                None => {
                    let cookie_path = explicit_cookie_path.unwrap_or_else(default_cookie_path);
                    if cookie_path.exists() {
                        RpcAuth::Cookie { path: cookie_path }
                    } else {
                        // Most likely: enable_cookie_auth = false was set
                        // locally (typical Zaino/lightwalletd setup).
                        // Don't hard-fail here — let the first RPC call
                        // surface an auth error if this guess is wrong.
                        RpcAuth::None
                    }
                }
            }
        } else {
            match explicit_api_key {
                Some(key) => RpcAuth::ApiKey { key },
                None => anyhow::bail!(
                    "rpc_url '{rpc_url}' looks like a remote endpoint (not localhost/private-range) \
                     but no API key was provided. Remote providers (e.g. QuickNode) need explicit \
                     credentials — set --api-key or the appropriate env var."
                ),
            }
        };

        Ok(RpcConfig { network, rpc_url: rpc_url.to_string(), auth, is_local })
    }
}

fn is_local_host(rpc_url: &str) -> bool {
    // Deliberately simple string checks rather than a full URL parser —
    // this only needs to distinguish "developer's own machine/LAN" from
    // "public internet provider," not handle every edge case.
    let lower = rpc_url.to_lowercase();
    lower.contains("127.0.0.1")
        || lower.contains("localhost")
        || lower.contains("0.0.0.0")
        || lower.contains("://10.")
        || lower.contains("://192.168.")
}

fn default_cookie_path() -> PathBuf {
    // Zebra's default cache dir; override via --cookie-path if the
    // operator customized zebra_state_path / cache_dir.
    dirs_next_home().join(".cache").join("zebra").join(".cookie")
}

fn dirs_next_home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

pub fn read_cookie(path: &PathBuf) -> anyhow::Result<(String, String)> {
    let contents = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read Zebra RPC cookie at {path:?}: {e}"))?;
    let (user, pass) = contents
        .trim()
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("cookie file at {path:?} not in 'user:pass' format"))?;
    Ok((user.to_string(), pass.to_string()))
}