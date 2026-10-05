#!/usr/bin/env node
/**
 * Human-side admin CLI. Deliberately NOT exposed as an MCP tool: if the
 * agent could register itself or raise its own limits, the budget would
 * mean nothing.
 *
 *   npx tsx src/admin.ts register <agentId> --daily <zatoshi> \
 *       --approval <zatoshi> [--cap category=zatoshi ...]
 *   npx tsx src/admin.ts show <agentId>
 */
import { fileURLToPath } from "node:url";
import path from "node:path";
import { BudgetEngine } from "./budget.js";

const DB_PATH =
    process.env.ZEC_BUDGET_DB ??
    path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "budget.sqlite3");
const budget = new BudgetEngine(DB_PATH);

const [cmd, agentId, ...rest] = process.argv.slice(2);

function flag(name: string): string[] {
    const out: string[] = [];
    for (let i = 0; i < rest.length; i++) if (rest[i] === `--${name}` && rest[i + 1]) out.push(rest[i + 1]);
    return out;
}

if (cmd === "register" && agentId) {
    const daily = flag("daily")[0];
    const approval = flag("approval")[0];
    if (!daily || !approval) {
        console.error("usage: register <agentId> --daily <zatoshi> --approval <zatoshi> [--cap category=zatoshi]");
        process.exit(1);
    }
    const caps: Record<string, bigint> = {};
    for (const c of flag("cap")) {
        const [k, v] = c.split("=");
        caps[k] = BigInt(v);
    }
    const index = budget.nextSubAccountIndex();
    budget.registerAgent(agentId, index, {
        dailyCapZatoshi: BigInt(daily),
        categoryCapsZatoshi: caps,
        approvalThresholdZatoshi: BigInt(approval),
    });
    console.log(`Registered ${agentId} (sub-account ${index}) in ${DB_PATH}`);
} else if (cmd === "show" && agentId) {
    console.log(budget.getAgent(agentId) ?? "not found");
} else {
    console.error("commands: register | show");
    process.exit(1);
}