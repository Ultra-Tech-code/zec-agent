/**
 * zec-agent-budget SDK
 *
 * Goal: integrating a constrained ZEC budget into an existing agent
 * should be a few lines, not a protocol implementation.
 *
 *   const budget = new ZecBudget({ agentId: "research-agent-1" });
 *   const result = await budget.spend({
 *     category: "api",
 *     amountZatoshi: 50_000n,
 *     destinationAddress: "u1...",
 *     memo: "invoice #4471",
 *   });
 */

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

export interface ZecBudgetConfig {
  agentId: string;
  /** Path to the zec-budget-mcp server binary/entrypoint. */
  serverCommand?: string;
  serverArgs?: string[];
}

export interface SpendParams {
  category: string;
  amountZatoshi: bigint;
  destinationAddress: string;
  memo?: string;
}

export type SpendResult =
  | { allowed: true; sent: true; txid: string }
  | { allowed: true; requiresApproval: true; approvalId: string }
  | { allowed: true; sent: false; error: string }
  | { allowed: false; reason: string };

export class ZecBudget {
  private client: Client;
  private connected = false;
  private agentId: string;

  constructor(config: ZecBudgetConfig) {
    this.agentId = config.agentId;
    const transport = new StdioClientTransport({
      command: config.serverCommand ?? "zec-budget-mcp",
      args: config.serverArgs ?? [],
    });
    this.client = new Client({ name: "zec-budget-sdk-client", version: "0.1.0" }, { capabilities: {} });
    this.client.connect(transport).then(() => {
      this.connected = true;
    });
  }

  private async ensureConnected() {
    if (!this.connected) {
      // simple poll; fine for hackathon scaffold, replace with a proper
      // ready-promise if this becomes a real dependency
      await new Promise((r) => setTimeout(r, 100));
    }
  }

  async checkBalance(): Promise<{ zatoshi: string }> {
    await this.ensureConnected();
    const res = await this.client.callTool({
      name: "check_balance",
      arguments: { agentId: this.agentId },
    });
    return JSON.parse((res.content as { text: string }[])[0].text);
  }

  async spend(params: SpendParams): Promise<SpendResult> {
    await this.ensureConnected();
    const res = await this.client.callTool({
      name: "request_spend",
      arguments: {
        agentId: this.agentId,
        category: params.category,
        amountZatoshi: params.amountZatoshi.toString(),
        destinationAddress: params.destinationAddress,
        memo: params.memo,
      },
    });
    return JSON.parse((res.content as { text: string }[])[0].text);
  }

  async history(): Promise<unknown[]> {
    await this.ensureConnected();
    const res = await this.client.callTool({
      name: "list_spend_history",
      arguments: { agentId: this.agentId },
    });
    const parsed = JSON.parse((res.content as { text: string }[])[0].text);
    return parsed.history ?? [];
  }
}
