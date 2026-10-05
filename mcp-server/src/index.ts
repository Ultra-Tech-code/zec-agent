#!/usr/bin/env node
/**
 * zec-budget-mcp — MCP server entrypoint.
 *
 * Exposes tools any MCP-compatible agent framework can call:
 *   - check_balance
 *   - request_spend
 *   - list_spend_history
 *
 * Policy is enforced here (budget.ts) BEFORE anything reaches rust-signer.
 * This process holds no spending key.
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import { BudgetEngine } from "./budget.js";
import { SignerClient } from "./signerClient.js";

import { fileURLToPath } from "node:url";
import path from "node:path";
// Default DB lives next to the package, not in the caller's cwd, so the
// admin CLI and the server always share one database.
const DB_PATH =
  process.env.ZEC_BUDGET_DB ??
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "budget.sqlite3");
const SIGNER_BIN = process.env.ZEC_SIGNER_BIN ?? "../rust-signer/target/debug/rust-signer";

const budget = new BudgetEngine(DB_PATH);
const signer = new SignerClient(SIGNER_BIN, ["--stdio", "--network", "testnet"]);

const server = new Server(
  { name: "zec-agent-budget", version: "0.1.0" },
  { capabilities: { tools: {} } }
);

const RequestSpendInput = z.object({
  agentId: z.string(),
  category: z.string(),
  amountZatoshi: z.string(), // string to avoid float precision issues over JSON
  destinationAddress: z.string(),
  memo: z.string().max(512).optional(),
});

const CheckBalanceInput = z.object({ agentId: z.string() });
const ListHistoryInput = z.object({ agentId: z.string() });

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: "check_balance",
      description: "Check the shielded ZEC balance available to this agent's sub-account.",
      inputSchema: {
        type: "object",
        properties: { agentId: { type: "string" } },
        required: ["agentId"],
      },
    },
    {
      name: "request_spend",
      description:
        "Request a shielded ZEC payment from the agent's budget. Enforced against daily caps, " +
        "category caps, and an approval threshold set by the human who funded the budget. " +
        "May return requiresApproval=true, in which case the spend is queued, not sent.",
      inputSchema: {
        type: "object",
        properties: {
          agentId: { type: "string" },
          category: { type: "string", description: "e.g. 'compute', 'data', 'api', 'subagent'" },
          amountZatoshi: { type: "string" },
          destinationAddress: { type: "string" },
          memo: { type: "string", description: "Up to 512 bytes, encrypted, viewing-key readable" },
        },
        required: ["agentId", "category", "amountZatoshi", "destinationAddress"],
      },
    },
    {
      name: "list_spend_history",
      description: "List this agent's recorded spend history (server-side audit log).",
      inputSchema: {
        type: "object",
        properties: { agentId: { type: "string" } },
        required: ["agentId"],
      },
    },
  ],
}));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  if (name === "check_balance") {
    const { agentId } = CheckBalanceInput.parse(args);
    const agent = budget.getAgent(agentId);
    if (!agent) {
      return { content: [{ type: "text", text: JSON.stringify({ error: `Unknown agent: ${agentId}` }) }] };
    }
    const bal = await signer.getBalance(agent.subAccountIndex);
    return { content: [{ type: "text", text: JSON.stringify(bal) }] };
  }

  if (name === "request_spend") {
    const input = RequestSpendInput.parse(args);
    const decision = budget.evaluate({
      agentId: input.agentId,
      category: input.category,
      amountZatoshi: BigInt(input.amountZatoshi),
      destinationAddress: input.destinationAddress,
      memo: input.memo,
    });

    if (!decision.allowed) {
      return {
        content: [{ type: "text", text: JSON.stringify({ allowed: false, reason: decision.reason }) }],
      };
    }

    if (decision.requiresApproval) {
      const spendId = budget.recordSpend(
        {
          agentId: input.agentId,
          category: input.category,
          amountZatoshi: BigInt(input.amountZatoshi),
          destinationAddress: input.destinationAddress,
          memo: input.memo,
        },
        "pending_approval"
      );
      budget.createApproval(decision.approvalId, spendId);
      return {
        content: [
          {
            type: "text",
            text: JSON.stringify({
              allowed: true,
              requiresApproval: true,
              approvalId: decision.approvalId,
              message: "Spend queued pending human approval.",
            }),
          },
        ],
      };
    }

    // Clear to send — call the signer, which is the only place a real
    // shielded transaction gets built and broadcast.
    try {
      const { txid } = await signer.send({
        subAccountIndex: budget.getAgent(input.agentId)!.subAccountIndex, // evaluate() already rejected unknown agents
        amountZatoshi: input.amountZatoshi,
        destinationAddress: input.destinationAddress,
        memo: input.memo,
      });
      budget.recordSpend(
        {
          agentId: input.agentId,
          category: input.category,
          amountZatoshi: BigInt(input.amountZatoshi),
          destinationAddress: input.destinationAddress,
          memo: input.memo,
        },
        "sent",
        txid
      );
      return { content: [{ type: "text", text: JSON.stringify({ allowed: true, txid }) }] };
    } catch (err) {
      budget.recordSpend(
        {
          agentId: input.agentId,
          category: input.category,
          amountZatoshi: BigInt(input.amountZatoshi),
          destinationAddress: input.destinationAddress,
          memo: input.memo,
        },
        "failed"
      );
      return {
        content: [{ type: "text", text: JSON.stringify({ allowed: true, sent: false, error: String(err) }) }],
      };
    }
  }

  if (name === "list_spend_history") {
    const { agentId } = ListHistoryInput.parse(args);
    return { content: [{ type: "text", text: JSON.stringify({ history: budget.listSpends(agentId) }) }] };
  }

  throw new Error(`Unknown tool: ${name}`);
});

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  process.stderr.write("zec-agent-budget MCP server running on stdio\n");
}

main().catch((err) => {
  process.stderr.write(`Fatal: ${err}\n`);
  process.exit(1);
});