# Private AI Agent Money Layer — Architecture

## What this is

An open, composable toolkit that lets a human fund a shielded ZEC budget and
hand an AI agent a **constrained** ability to spend from it — daily limits,
category caps, approval thresholds — without the agent ever holding the real
spending key, and without leaking spend activity on a transparent ledger.

Three pieces:

1. **`rust-signer`** — the only component that touches real Zcash keys.
   A small Rust service wrapping `zcash_client_backend` / `zcash_keys` /
   `zcash_primitives` (Ironwood/NU6.3-only), responsible for: light-client
   sync via Zaino, ZIP 32 child key derivation for sub-accounts, building
   and signing shielded transactions, and exposing viewing-key-scoped
   read access for audit.
2. **`mcp-server`** — the brain. TypeScript, speaks MCP (Model Context
   Protocol) so any agent framework can attach to it as a tool. Owns budget
   state (limits, spend-to-date, category caps, approval thresholds),
   talks to `rust-signer` over a local gRPC/IPC boundary, never sees raw
   key material.
3. **`sdk`** — thin TypeScript client wrapping the MCP tool calls, so
   integrating into an existing agent framework (LangChain, custom loops,
   whatever) is a few lines, not a protocol implementation.

A `dashboard` (minimal, read-only-first) lets the human who funded the
budget see spend history (via the viewing key) and adjust limits.

## Why application-layer budget enforcement, stated plainly

Zcash has no native allowance/session-key primitive — nothing like an
ERC-20 `approve()` or a Solana PDA with programmed spend rules. So
"constrained session key" here means: the real Ironwood spending key
lives only in `rust-signer`, under the human's control. The agent talks
to `mcp-server`, which enforces limits in its own state before ever
asking `rust-signer` to sign anything.

This is an explicit, load-bearing design decision, not a hidden gap:
- **Pro:** buildable in the hackathon timeframe, flexible policy logic
  (category caps, time windows, approval hooks) that would be painful to
  encode in shielded-circuit constraints today
- **Con:** the enforcement boundary is our server, not the protocol —
  if `mcp-server` is compromised, its stated limits can be bypassed
- **Mitigation or future work to name explicitly in the pitch:** a
  hardware-backed or TEE-attested enforcement boundary, or exploring
  whether a future Zcash primitive (post-Tachyon) could encode spend
  policy more natively

Say this out loud to judges before they ask — it reads as rigor, not
weakness.

## Sub-accounts via ZIP 32

Each agent (or each budget) gets a distinct child key derived from the
human's Ironwood viewing key via ZIP 32 — not a separate wallet, not a
separate seed. This gives:
- Distinct receiving/change addresses per agent (harder to link agent
  activity together on-chain)
- One viewing key at the top can still reconcile everything for
  human-side audit
- No new key material to separately back up

## Data flow (one spend)

1. Agent calls an MCP tool, e.g. `spend(category, amount, memo)`
2. `mcp-server` checks: daily limit remaining? category cap remaining?
   above approval threshold (needs human sign-off)?
3. If clear, `mcp-server` calls `rust-signer` over local IPC with the
   sub-account id + amount + destination + memo
4. `rust-signer` builds and signs the Ironwood shielded transaction,
   broadcasts via Zaino, returns txid
5. `mcp-server` records the spend in its own audit log (mirrors what's
   recoverable from the memo + viewing key, for redundancy)
6. Agent gets back a result; human's dashboard can independently verify
   via the viewing key, not just trust the server's log

## Stack

- `rust-signer`: Rust, `zcash_client_backend`, `zcash_keys`,
  `zcash_primitives`, Zaino as light-client backend, Ironwood/NU6.3 only
- `mcp-server`: TypeScript, `@modelcontextprotocol/sdk`, local SQLite for
  budget/audit state, gRPC (or simple JSON-over-stdio for hackathon speed)
  to `rust-signer`
- `sdk`: TypeScript, thin wrapper generating MCP tool calls
- `dashboard`: minimal React, read-only view over viewing-key-scanned
  history first, limit-adjustment UI second if time allows

## 4-week build order (restated against this scaffold)

1. `rust-signer`: Zaino sync + Ironwood balance read + basic signed send
   working standalone, tested via CLI, no MCP yet
2. `mcp-server`: budget/limit state machine + IPC to `rust-signer`,
   MCP tools exposed, testable via any MCP-compatible client
3. `sdk` + one reference agent using it end-to-end; encrypted memo
   audit trail wired up
4. `dashboard` + polish + demo video; buffer for the inevitable slip
