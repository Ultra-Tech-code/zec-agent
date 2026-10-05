/**
 * Budget policy engine.
 *
 * This is the actual differentiator: Zcash has no native allowance/session
 * primitive, so every constraint here is enforced by this server, in its
 * own state, BEFORE it ever asks rust-signer to sign a real transaction.
 *
 * State lives in SQLite for hackathon simplicity. Swap for something more
 * durable (and consider a TEE-attested enforcement boundary) post-hackathon.
 */

import Database from "better-sqlite3";

export interface BudgetLimits {
  dailyCapZatoshi: bigint;
  categoryCapsZatoshi: Record<string, bigint>;
  approvalThresholdZatoshi: bigint; // spends >= this need human approval
}

export interface SpendRequest {
  agentId: string;
  category: string;
  amountZatoshi: bigint;
  destinationAddress: string;
  memo?: string;
}

export type SpendDecision =
  | { allowed: true; requiresApproval: false }
  | { allowed: true; requiresApproval: true; approvalId: string }
  | { allowed: false; reason: string };

export class BudgetEngine {
  private db: Database.Database;

  constructor(dbPath: string) {
    this.db = new Database(dbPath);
    this.db.pragma("journal_mode = WAL");
    this.migrate();
  }

  private migrate() {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS agents (
        agent_id TEXT PRIMARY KEY,
        sub_account_index INTEGER NOT NULL,
        daily_cap_zatoshi TEXT NOT NULL,
        category_caps_json TEXT NOT NULL DEFAULT '{}',
        approval_threshold_zatoshi TEXT NOT NULL,
        created_at INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS spends (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        agent_id TEXT NOT NULL,
        category TEXT NOT NULL,
        amount_zatoshi TEXT NOT NULL,
        destination TEXT NOT NULL,
        memo TEXT,
        txid TEXT,
        status TEXT NOT NULL, -- pending_approval | sent | rejected | failed
        created_at INTEGER NOT NULL,
        FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
      );

      CREATE TABLE IF NOT EXISTS approvals (
        approval_id TEXT PRIMARY KEY,
        spend_id INTEGER NOT NULL,
        status TEXT NOT NULL DEFAULT 'pending', -- pending | approved | denied
        created_at INTEGER NOT NULL,
        resolved_at INTEGER,
        FOREIGN KEY (spend_id) REFERENCES spends(id)
      );
    `);
  }

  registerAgent(agentId: string, subAccountIndex: number, limits: BudgetLimits) {
    this.db
      .prepare(
        `INSERT INTO agents (agent_id, sub_account_index, daily_cap_zatoshi, category_caps_json, approval_threshold_zatoshi, created_at)
         VALUES (?, ?, ?, ?, ?, ?)`
      )
      .run(
        agentId,
        subAccountIndex,
        limits.dailyCapZatoshi.toString(),
        JSON.stringify(
          Object.fromEntries(
            Object.entries(limits.categoryCapsZatoshi).map(([k, v]) => [k, v.toString()])
          )
        ),
        limits.approvalThresholdZatoshi.toString(),
        Date.now()
      );
  }

  private spentToday(agentId: string): bigint {
    const startOfDay = new Date();
    startOfDay.setHours(0, 0, 0, 0);
    const rows = this.db
      .prepare(
        `SELECT amount_zatoshi FROM spends
         WHERE agent_id = ? AND status = 'sent' AND created_at >= ?`
      )
      .all(agentId, startOfDay.getTime()) as { amount_zatoshi: string }[];
    return rows.reduce((sum, r) => sum + BigInt(r.amount_zatoshi), 0n);
  }

  private spentTodayInCategory(agentId: string, category: string): bigint {
    const startOfDay = new Date();
    startOfDay.setHours(0, 0, 0, 0);
    const rows = this.db
      .prepare(
        `SELECT amount_zatoshi FROM spends
         WHERE agent_id = ? AND category = ? AND status = 'sent' AND created_at >= ?`
      )
      .all(agentId, category, startOfDay.getTime()) as { amount_zatoshi: string }[];
    return rows.reduce((sum, r) => sum + BigInt(r.amount_zatoshi), 0n);
  }

  /**
   * Evaluate a spend request against the agent's limits. Does NOT touch
   * rust-signer or the chain — this is purely the policy decision.
   */
  evaluate(req: SpendRequest): SpendDecision {
    const agent = this.db
      .prepare(`SELECT * FROM agents WHERE agent_id = ?`)
      .get(req.agentId) as
      | {
        daily_cap_zatoshi: string;
        category_caps_json: string;
        approval_threshold_zatoshi: string;
      }
      | undefined;

    if (!agent) {
      return { allowed: false, reason: `Unknown agent: ${req.agentId}` };
    }

    const dailyCap = BigInt(agent.daily_cap_zatoshi);
    const categoryCaps: Record<string, string> = JSON.parse(agent.category_caps_json);
    const approvalThreshold = BigInt(agent.approval_threshold_zatoshi);

    const spentToday = this.spentToday(req.agentId);
    if (spentToday + req.amountZatoshi > dailyCap) {
      return {
        allowed: false,
        reason: `Daily cap exceeded: ${spentToday} + ${req.amountZatoshi} > ${dailyCap}`,
      };
    }

    const categoryCap = categoryCaps[req.category];
    if (categoryCap !== undefined) {
      const spentInCategory = this.spentTodayInCategory(req.agentId, req.category);
      if (spentInCategory + req.amountZatoshi > BigInt(categoryCap)) {
        return {
          allowed: false,
          reason: `Category cap exceeded for "${req.category}": ${spentInCategory} + ${req.amountZatoshi} > ${categoryCap}`,
        };
      }
    }

    if (req.amountZatoshi >= approvalThreshold) {
      const approvalId = crypto.randomUUID();
      return { allowed: true, requiresApproval: true, approvalId };
    }

    return { allowed: true, requiresApproval: false };
  }

  recordSpend(req: SpendRequest, status: "pending_approval" | "sent" | "rejected" | "failed", txid?: string): number {
    const result = this.db
      .prepare(
        `INSERT INTO spends (agent_id, category, amount_zatoshi, destination, memo, txid, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`
      )
      .run(
        req.agentId,
        req.category,
        req.amountZatoshi.toString(),
        req.destinationAddress,
        req.memo ?? null,
        txid ?? null,
        status,
        Date.now()
      );
    return Number(result.lastInsertRowid);
  }

  createApproval(approvalId: string, spendId: number) {
    this.db
      .prepare(
        `INSERT INTO approvals (approval_id, spend_id, status, created_at) VALUES (?, ?, 'pending', ?)`
      )
      .run(approvalId, spendId, Date.now());
  }

  getAgent(agentId: string): { subAccountIndex: number } | undefined {
    const row = this.db
      .prepare(`SELECT sub_account_index FROM agents WHERE agent_id = ?`)
      .get(agentId) as { sub_account_index: number } | undefined;
    return row ? { subAccountIndex: row.sub_account_index } : undefined;
  }

  nextSubAccountIndex(): number {
    const row = this.db
      .prepare(`SELECT COALESCE(MAX(sub_account_index), -1) + 1 AS n FROM agents`)
      .get() as { n: number };
    return row.n;
  }

  listSpends(agentId: string, limit = 50) {
    return this.db
      .prepare(
        `SELECT id, category, amount_zatoshi, destination, memo, txid, status, created_at
         FROM spends WHERE agent_id = ? ORDER BY id DESC LIMIT ?`
      )
      .all(agentId, limit);
  }
}