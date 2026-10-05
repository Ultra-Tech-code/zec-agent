/**
 * Thin client for talking to the rust-signer sidecar.
 *
 * Hackathon-speed choice: JSON lines over the sidecar's stdio, one request
 * per line, one response per line. Swap for gRPC if there's time later —
 * the interface below is what mcp-server actually depends on, so the
 * transport can change without touching budget.ts or index.ts.
 *
 * IMPORTANT: this process never sees a spending key. It sends
 * (sub-account index, amount, destination, memo) and gets back a txid
 * or an error. Key material lives only inside rust-signer.
 */

import { spawn, ChildProcessByStdio } from "node:child_process";
import { createInterface } from "node:readline";
import type { Writable, Readable } from "node:stream";

interface SignerRequest {
  id: string;
  method: "get_balance" | "derive_subaccount" | "send" | "scan_memos";
  params: Record<string, unknown>;
}

interface SignerResponse {
  id: string;
  ok: boolean;
  result?: unknown;
  error?: string;
}

export class SignerClient {
  private proc: ChildProcessByStdio<Writable, Readable, null>;
  private pending = new Map<string, (res: SignerResponse) => void>();

  constructor(binaryPath: string, args: string[] = []) {
    this.proc = spawn(binaryPath, args, { 
        stdio: ["pipe", "pipe", "inherit"],
        cwd: new URL("../../rust-signer", import.meta.url).pathname
    });
    const rl = createInterface({ input: this.proc.stdout });
    rl.on("line", (line) => {
      try {
        const res: SignerResponse = JSON.parse(line);
        const resolver = this.pending.get(res.id);
        if (resolver) {
          resolver(res);
          this.pending.delete(res.id);
        }
      } catch {
        // Non-JSON line from the signer (e.g. a log line) — ignore here,
        // rust-signer should log to stderr, not stdout, but be defensive.
      }
    });
  }

  private call(method: SignerRequest["method"], params: Record<string, unknown>): Promise<unknown> {
    const id = crypto.randomUUID();
    const req: SignerRequest = { id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, (res) => {
        if (res.ok) resolve(res.result);
        else reject(new Error(res.error ?? "unknown signer error"));
      });
      this.proc.stdin.write(JSON.stringify(req) + "\n");
    });
  }

  async getBalance(subAccountIndex: number): Promise<{ zatoshi: string }> {
    return this.call("get_balance", { sub_account_index: subAccountIndex }) as Promise<{
      zatoshi: string;
    }>;
  }

  async deriveSubAccount(agentId: string): Promise<{ index: number; address: string }> {
    return this.call("derive_subaccount", { agent_id: agentId }) as Promise<{
      index: number;
      address: string;
    }>;
  }

  async send(params: {
    subAccountIndex: number;
    amountZatoshi: string;
    destinationAddress: string;
    memo?: string;
  }): Promise<{ txid: string }> {
    return this.call("send", {
      sub_account_index: params.subAccountIndex,
      amount_zatoshi: params.amountZatoshi,
      destination_address: params.destinationAddress,
      memo: params.memo ?? "",
    }) as Promise<{ txid: string }>;
  }

  close() {
    this.proc.kill();
  }
}
