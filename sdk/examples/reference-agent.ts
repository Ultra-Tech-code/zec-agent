/**
 * Reference agent: the thing you actually demo on stage.
 *
 * A minimal "research agent" that needs to pay for an API call mid-task.
 * The point of this file is to show how little glue code an existing
 * agent needs to gain a constrained shielded budget.
 */

import { ZecBudget } from "../src/index.ts";

async function main() {
  const budget = new ZecBudget({
    agentId: "research-agent-1",
    serverCommand: "node",
    serverArgs: ["../mcp-server/dist/index.js"],
  });

  console.log("Balance:", await budget.checkBalance());

  // Simulate the agent deciding it needs to pay for a data API call.
  const result = await budget.spend({
    category: "data",
    amountZatoshi: 100000n, // ~0.001 ZEC, within budget limits
    destinationAddress: "u1exampleplaceholderaddress...",
    memo: "dataset access: q3-market-research",
  });

  if (result.allowed && "sent" in result && result.sent) {
    console.log(`Paid. txid: ${result.txid}`);
  } else if ("requiresApproval" in result) {
    console.log(`Spend above threshold — queued for human approval: ${result.approvalId}`);
  } else if (!result.allowed) {
    console.log(`Spend blocked by budget policy: ${result.reason}`);
  } else {
    console.log("Spend attempt failed:", result);
  }

  console.log("History:", await budget.history());
}

main().catch(console.error);
