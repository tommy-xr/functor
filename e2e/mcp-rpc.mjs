// The MCP e2e harnesses' shared JSON-RPC client.
//
// `functor mcp` speaks line-delimited JSON-RPC 2.0 on stdio and the harnesses
// speak it RAW — no MCP client library — so what they assert is exactly what a
// coding agent's client would see. That is worth exactly one copy: this module
// is it, imported by every `mcp-*.mjs` harness and by `code-garden.mjs`.
//
// Plain ESM with node builtins only, like every other file in `e2e/`.
import { connect } from "node:net";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

export const ROOT = fileURLToPath(new URL("..", import.meta.url));
export const BIN = process.env.FUNCTOR_BIN ?? `${ROOT}target/debug/functor`;

/** A line-delimited JSON-RPC client over a child's stdio. */
export class Rpc {
  constructor(proc) {
    this.proc = proc;
    this.pending = new Map();
    this.nextId = 1;
    this.buffer = "";
    this.stderr = "";
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", (chunk) => this.#onData(chunk));
    proc.stderr.on("data", (chunk) => (this.stderr += chunk));
  }

  #onData(chunk) {
    this.buffer += chunk;
    let index;
    while ((index = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, index).trim();
      this.buffer = this.buffer.slice(index + 1);
      if (!line) continue;
      const message = JSON.parse(line);
      const waiter = this.pending.get(message.id);
      if (waiter) {
        this.pending.delete(message.id);
        clearTimeout(waiter.timer);
        message.error ? waiter.reject(new Error(JSON.stringify(message.error))) : waiter.resolve(message.result);
      }
    }
  }

  notify(method, params) {
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
  }

  beginRequest(method, params) {
    const id = this.nextId++;
    const result = new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out\nstderr:\n${this.stderr}`));
      }, 60000);
      this.pending.set(id, { resolve, reject, timer });
    });
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    return { id, result };
  }

  request(method, params) {
    return this.beginRequest(method, params).result;
  }

  cancelRequest(id, reason) {
    this.notify("notifications/cancelled", { requestId: id, reason });
    // MCP deliberately sends no response for a cancelled request. Settle the
    // local waiter while the state assertion below proves the server did not
    // execute the queued mutation after dropping that response.
    const waiter = this.pending.get(id);
    if (waiter) {
      this.pending.delete(id);
      clearTimeout(waiter.timer);
      waiter.resolve({ cancelled: true });
    }
  }

  /** Call a tool and return its single text content block, parsed if it is JSON. */
  async call(name, args = {}) {
    const result = await this.request("tools/call", { name, arguments: args });
    const text = result.content.map((block) => block.text ?? "").join("");
    if (result.isError) throw new Error(`${name} failed: ${text}`);
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  }
}

export const failures = [];
export const check = (ok, what) => {
  console.log(`  ${ok ? "✓" : "✗"} ${what}`);
  if (!ok) failures.push(what);
};


export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Spawn `functor mcp`, complete the MCP handshake, and return `{ proc, rpc }`. */
export async function startMcp(clientName) {
  const proc = spawn(BIN, ["mcp"], { cwd: ROOT, stdio: ["pipe", "pipe", "pipe"] });
  const rpc = new Rpc(proc);
  await rpc.request("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: clientName, version: "0" },
  });
  rpc.notify("notifications/initialized", {});
  return { proc, rpc };
}

/** Poll `get_state` until `predicate` holds, or fail loudly with the last model.
 *
 * Shared by every multiplayer script: a networked assertion is always "wait for
 * a game-level fact", never "step and hope". */
export async function waitForState(rpc, session, predicate, what, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  for (;;) {
    last = await rpc.call("get_state", { session });
    if (predicate(last)) return last;
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for ${what}; last model: ${JSON.stringify(last.model)}`);
    }
    await sleep(100);
  }
}

/** Is something listening on a localhost TCP port? */
export async function portIsBound(port) {
  return new Promise((resolve) => {
    const socket = connect({ host: "127.0.0.1", port });
    socket.once("connect", () => (socket.destroy(), resolve(true)));
    socket.once("error", () => (socket.destroy(), resolve(false)));
  });
}

/** Wait until something IS listening on a localhost TCP port.
 *
 * An orbs client dials ONCE with no retry, so a client launched before the
 * server's `Sub.listen` has bound lands in "error" and never converges. */
export async function waitForPort(port, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (await portIsBound(port)) return;
    if (Date.now() > deadline) throw new Error(`nothing listening on 127.0.0.1:${port}`);
    await sleep(100);
  }
}
