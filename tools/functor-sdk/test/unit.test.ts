import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import {
  findRepoRoot,
  formatCrashOutput,
  FunctorClient,
  HttpClient,
  stepAll,
  waitFor,
} from "../src/index.js";

// Pure unit tests — no runtime required, always run.

test("findRepoRoot walks up to the cargo workspace root", () => {
  const root = findRepoRoot(process.cwd());
  assert.ok(root !== undefined, "should find a workspace root from the SDK dir");
  assert.ok(
    existsSync(join(root, "Cargo.toml")),
    "found root should contain Cargo.toml",
  );
});

test("findRepoRoot returns undefined when there's no workspace above", () => {
  assert.equal(findRepoRoot("/"), undefined);
});

test("formatCrashOutput keeps the panic and its context", () => {
  const lines = ["init", "loading", "ok", "panicked at foo.rs:1", "stack:", "  0: x"];
  const out = formatCrashOutput(lines);
  assert.match(out, /panicked at foo\.rs:1/);
  assert.match(out, /0: x/, "should include lines after the panic");
  assert.match(out, /ok/, "should include a little context before the panic");
});

test("formatCrashOutput falls back to the tail when there's no panic", () => {
  const lines = Array.from({ length: 50 }, (_, i) => `line ${i}`);
  const out = formatCrashOutput(lines);
  assert.match(out, /line 49/, "should include the last line");
  assert.doesNotMatch(out, /line 0\b/, "should drop the earliest lines");
});

test("stepAll advances every client by the same dt, concurrently", async () => {
  const calls: number[] = [];
  const fake = () => ({
    step: async (dt: number) => {
      calls.push(dt);
    },
  });
  const clients = [fake(), fake(), fake()];

  await stepAll(clients, 0.25);

  assert.deepEqual(calls, [0.25, 0.25, 0.25]);
});

test("waitFor returns once the predicate holds", async () => {
  let n = 0;
  const value = await waitFor(
    async () => ++n,
    (v) => v >= 3,
    { intervalMs: 1 },
  );
  assert.equal(value, 3);
});

test("waitFor retries when poll throws, then resolves", async () => {
  let n = 0;
  const value = await waitFor(
    async () => {
      n++;
      if (n < 3) throw new Error("transient");
      return n;
    },
    (v) => v >= 3,
    { intervalMs: 1 },
  );
  assert.equal(value, 3);
});

test("waitFor surfaces the last poll error on timeout", async () => {
  await assert.rejects(
    waitFor(
      async () => {
        throw new Error("boom");
      },
      () => true,
      { timeoutMs: 20, intervalMs: 5, description: "x" },
    ),
    /timed out after 20ms waiting for x \(last error: Error: boom\)/,
  );
});

test("waitFor throws on timeout with the description", async () => {
  await assert.rejects(
    waitFor(
      async () => false,
      (v) => v === true,
      { timeoutMs: 20, intervalMs: 5, description: "the impossible" },
    ),
    /timed out after 20ms waiting for the impossible/,
  );
});

test("reloadAssets uploads binary envelopes then finalizes the manifest", async () => {
  const calls: Array<{ path: string; body: unknown }> = [];
  const http = {
    postRawBinary: async (path: string, body: Uint8Array) => {
      calls.push({ path, body });
      return "reloaded";
    },
    postText: async (path: string, body: unknown) => {
      calls.push({ path, body });
      return "synced";
    },
  } as HttpClient;
  const client = new FunctorClient(http);

  assert.equal(
    await client.reloadAssets([["textures/grid.png", Uint8Array.of(0, 1, 255)]]),
    "synced",
  );
  assert.equal(calls[0].path, "/reload-asset");
  const envelope = calls[0].body as Uint8Array;
  const pathLength = new DataView(
    envelope.buffer,
    envelope.byteOffset,
    envelope.byteLength,
  ).getUint32(0, false);
  assert.equal(
    new TextDecoder().decode(envelope.slice(4, 4 + pathLength)),
    "textures/grid.png",
  );
  assert.deepEqual([...envelope.slice(4 + pathLength)], [0, 1, 255]);
  assert.deepEqual(calls[1], {
    path: "/sync-assets",
    body: ["textures/grid.png"],
  });
});

test("project load and reload use distinct lifecycle routes", async () => {
  const calls: Array<{ path: string; body: unknown }> = [];
  const http = {
    postText: async (path: string, body: unknown) => {
      calls.push({ path, body });
      return "ok";
    },
  } as HttpClient;
  const client = new FunctorClient(http);
  const files: [string, string][] = [["game.fun", "let init = 0"]];

  await client.loadProject(files);
  await client.reloadProject(files);

  assert.deepEqual(calls, [
    { path: "/load-project", body: files },
    { path: "/reload-project", body: files },
  ]);
});
