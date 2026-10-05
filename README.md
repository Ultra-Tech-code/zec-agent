# zec-agent-budget

Open, composable infrastructure for giving AI agents constrained shielded
ZEC spending power — daily limits, category caps, approval thresholds —
without ever handing the agent a real spending key, and without leaking
agent activity on a transparent ledger.

Built for the Zcash track at Colosseum's Worlds Fair hackathon.

## Status: Functional (Ironwood/NU6)

This repo provides a fully integrated **protocol and architecture scaffold**. The MCP server's tool surface, budget policy engine, and IPC contract with the signer are wired together. The signer (`rust-signer`) is fully functional — using `zcash_client_sqlite` and `zcash_client_backend` to sync with a Zaino lightwalletd server, construct zero-knowledge proofs (Ironwood/Orchard), and broadcast shielded transactions to a local Zebra node. See `docs/ARCHITECTURE.md` for design tradeoffs.

## Layout

```
mcp-server/   MCP server: budget policy engine + tool surface agents call
rust-signer/  Sidecar holding the real Ironwood spending key
sdk/          Thin TS client + reference agent example
dashboard/    (not yet scaffolded) human-facing budget/audit view
docs/         Architecture writeup
```

## Why this exists (vs. building on AxiomAI/Astrea)

AxiomAI/Astrea (astrea-foundation on GitHub) is a vertically-integrated private-AI-inference product settled in ZEC. This project is a different layer: generic, open infrastructure any agent framework can attach to via MCP, regardless of which model or company it's using. See `docs/ARCHITECTURE.md` for the full rationale.

## Getting started

Ensure you have a local Zebra node and Zaino instance running on testnet.

### 1. Verify your Zebra Node RPC
You can verify your Zebra node is responding on the testnet RPC port using this `curl` command (adjust the cookie path if you aren't on macOS):

```bash
curl -s --user "$(cat ~/Library/Caches/zebra/.cookie)" \
  --data-binary '{"jsonrpc":"2.0","id":1,"method":"getinfo","params":[]}' \
  -H 'content-type: application/json' http://127.0.0.1:18232/
```

**Expected Result:**
```json
{"jsonrpc":"2.0","id":1,"result":{"version":6040200,"build":"v6.4.2","subversion":"/Zebra:6.4.2/","protocolversion":170160,"blocks":4466571,"connections":23,"difficulty":4.730764719151816,"testnet":true,"paytxfee":0.0,"relayfee":1e-6,"errors":"getaddrs response hasn't been refreshed in some time","errorstimestamp":1791239517}}
```

### 2. Build and run the project

```bash
# 1. build the signer
cd rust-signer
# Create a .env file with your testnet SEED_PHRASE
cargo build

# 2. build the MCP server
cd ../mcp-server
npm install
npm run build

# 3. try the reference agent end-to-end (MCP -> SDK -> rust-signer -> network)
cd ../sdk
npm install
npm run build
ZEBRA_RPC_URL="http://127.0.0.1:18232" npx tsx examples/reference-agent.ts
```

### 3. Manual Testing via CLI

You can bypass the MCP server and interact directly with the Rust signer sidecar via JSON-over-stdio to query balances, manually build/broadcast a transaction, or check internal state.

**Note:** Ensure you are in the `rust-signer` directory.

**Check Wallet Balance:**
```bash
echo '{"id": "1", "method": "get_balance", "params": {"sub_account_index": 0}}' | cargo run -q --bin rust-signer -- --network testnet
```

**Send a test transaction:**
```bash
# Agent Testnet Address: utest1vpmwkhhk9tfcc5q48xy8nwzwwtxn7fskw97spyqkkat06ze6ffgvjsuf84r6c94u0ejuwxhh99jttrklx4wf2ancwv2uf6h0hx9n9v2vnsy3g5k8za2fh4nspaszteu6djlh2rywz8velsh2xl27enuux3s62cl065zl94kxg05t9st4zv4w4vleq6kvxses79jye6pncrjcsanngc2
# Target Wallet Address: utest1vs5pqe2ylczkyp4ape3pwpsjfuy86kp8mknukcday6a9emrafhuagtcjdvxvc4vstr4fg3sgwanyq393z2y3t9jz0uvhv0vfkry8tcan20lj89922ggv2nrn0zkxjcf34ufahymh7g6wdaqwk6nvka3wexrs73fhtatp6h50lsq4shkr

echo '{"id": "2", "method": "send", "params": {"amount_zatoshi": "50000", "destination_address": "utest1vs5pqe2ylczkyp4ape3pwpsjfuy86kp8mknukcday6a9emrafhuagtcjdvxvc4vstr4fg3sgwanyq393z2y3t9jz0uvhv0vfkry8tcan20lj89922ggv2nrn0zkxjcf34ufahymh7g6wdaqwk6nvka3wexrs73fhtatp6h50lsq4shkr"}}' | ZEBRA_RPC_URL="http://127.0.0.1:18232" cargo run -q --bin rust-signer -- --network testnet
```

**Expected Send Result:**
```json
{"id":"2","ok":true,"result":{"note":"Transaction successfully broadcasted to network","stub":false,"txid":"72bbd29a98d1d8085835abaef2d9fd19b8bddc7519f38f99b56c34379874d15b"}}
```

**Verify local state in SQLite:**
```bash
sqlite3 wallet.sqlite "SELECT count(*) as pending_scan FROM scan_queue;" && sqlite3 wallet.sqlite "SELECT witness_stabilized, count(*) FROM ironwood_received_notes GROUP BY witness_stabilized;"
```

## Next steps

1. Wire `derive_subaccount` to real ZIP 32 child derivation
2. Add human-approval flow (currently only queues; nothing resolves it)
3. Dashboard: read-only viewing-key-scanned history view first
