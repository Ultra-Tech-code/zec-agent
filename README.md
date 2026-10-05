# zec-agent-budget

Open, composable infrastructure for giving AI agents constrained shielded
ZEC spending power — daily limits, category caps, approval thresholds —
without ever handing the agent a real spending key, and without leaking
agent activity on a transparent ledger.

Built for the Zcash track at Colosseum's Worlds Fair hackathon.

## Status: scaffold, not yet functional

This repo is a **protocol and architecture scaffold**. The MCP server's tool surface, budget policy engine, and
IPC contract with the signer are real and wired together. The signer
itself (`rust-signer`) is stubbed — every place that needs real
`zcash_client_backend` / `zcash_keys` / Zaino calls is marked with a
`// TODO(zcash):` comment. See `docs/ARCHITECTURE.md` for why the pieces
are split this way and what's a deliberate design tradeoff vs. a
to-be-filled gap.

## Layout

```
mcp-server/   MCP server: budget policy engine + tool surface agents call
rust-signer/  Sidecar holding the real Ironwood spending key (stubbed)
sdk/          Thin TS client + reference agent example
dashboard/    (not yet scaffolded) human-facing budget/audit view
docs/         Architecture writeup
```

## Why this exists (vs. building on AxiomAI/Astrea)

AxiomAI/Astrea (astrea-foundation on GitHub) is a vertically-integrated
private-AI-inference product settled in ZEC — their own model, their own
proxy, billed in ZEC. Their agent-budgets code, if it exists, is not
public. This project is a different layer: generic, open infrastructure
any agent framework can attach to via MCP, regardless of which model or
company it's using. See `docs/ARCHITECTURE.md` for the full rationale.

## Getting started (once rust-signer is real)

```bash
# 1. build the signer
cd rust-signer && cargo build --release

# 2. build and run the MCP server
cd ../mcp-server && npm install && npm run build && npm start

# 3. try the reference agent
cd ../sdk && npm install && npm run build
npx tsx examples/reference-agent.ts
```

## Next steps (see docs/ARCHITECTURE.md for the full plan)

1. Replace every `TODO(zcash)` in `rust-signer/src/main.rs` with real
   Zaino sync + Ironwood balance/send calls; prove it standalone via CLI
   before wiring to the MCP server
2. Wire `derive_subaccount` to real ZIP 32 child derivation
3. Add human-approval flow (currently only queues; nothing resolves it)
4. Dashboard: read-only viewing-key-scanned history view first
