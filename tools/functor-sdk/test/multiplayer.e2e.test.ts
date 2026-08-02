import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner, waitForPort } from "../src/index.js";

// End-to-end network simulation: one server + two client runners
// (examples/mp — a multi-entry project whose roles share the wire protocol
// via the Protocol sibling module), each its own process on its own debug
// port, networked over a real WebSocket. The `.fun` ships as text (no dylib
// build), so this only needs the runner binary and a display:
//
//   cargo build --bin functor
//   FUNCTOR_E2E=1 node --test dist/test/
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

// The example models are exposed only as Functor Lang Debug text (the game model isn't
// Serialize yet), so these read the `model` string. An Functor Lang record renders as
// `{ field: value, ... }` and a list as `[elem, ...]` — no Fable linked list.
//
// The server model is `{ players: [<Player>, ...], nextPid: N }`; each Player
// record carries a unique `cid:` field, so counting `cid:` markers counts the
// tracked players (equivalently, nextPid reaches 2 once both have joined).
const serverPlayerCount = (model: string): number =>
  (model.match(/cid:/g) ?? []).length;

const clientStatus = (model: string): string =>
  model.match(/status:\s*"([^"]*)"/)?.[1] ?? "";

// The client model is `{ conn: Online(id), world: [<Player>, ...], status: ... }`;
// each world Player renders as `{ pid: p, x: .., z: .. }`, and `pid:` appears
// nowhere else in the client model, so `pid:` markers count the world entries.
const clientWorldCount = (model: string): number =>
  (model.match(/pid:/g) ?? []).length;

// Every wait below is condition-based with a GENEROUS ceiling rather than a
// tight window: it returns the moment its condition holds (costing nothing on a
// healthy run) and absorbs runner/load variance otherwise. CI run 30731090273
// (job 91451547356) failed exactly there — "server to track 2 players" hit a 20s
// inner deadline on a macOS runner booting three GL processes on one shared box,
// while the TEST's own budget was 180s and nine tenths of it went unused. The
// budget leads the inner ceilings so a real hang still reports as the described
// wait, not as an opaque test timeout.
test(
  "two clients connect to a server and converge on a shared world",
  { skip: !e2eEnabled, timeout: 300_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    // All three debug ports derive from one base so the test is self-consistent
    // when relocated. (The ws port is fixed at 9001 by the example games.)
    const base = Number(process.env.FUNCTOR_E2E_PORT ?? 8095);
    const launch = (entry: string, port: number) => {
      const gameDir = join(repoRoot, "examples", "mp");
      return FunctorRunner.launch({
        gameDir,
        repoRoot,
        functorLangPath: join(gameDir, entry),
        port,
        launchTimeoutMs: 60_000,
        headless,
      });
    };

    // Server first, and wait for its Sub.listen socket to actually bind before
    // launching clients — the client connects once with no retry, so a client
    // that races ahead of the listener would land in "error" and never converge.
    await using server = await launch("server.fun", base);
    await waitForPort("127.0.0.1", 9001, {
      timeoutMs: 60_000,
      description: "mp server ws listener",
    });

    // The client auto-moves on connect, so no input injection is needed.
    await using clientA = await launch("client.fun", base + 1);
    await using clientB = await launch("client.fun", base + 2);

    const waitOpts = { timeoutMs: 90_000, intervalMs: 200 };

    // The server should accept both connections and track a player for each.
    const serverState = await server.waitForState(
      (s) => serverPlayerCount(s.model_debug) === 2,
      { ...waitOpts, description: "server to track 2 players" },
    );
    assert.equal(serverPlayerCount(serverState.model_debug), 2);

    // Each client should reach "in-world" and see both players in the snapshot
    // the server broadcasts — i.e. the clients converge on a shared world. The
    // waitForState calls already enforce this (they throw on timeout); asserting
    // on their converged return value documents intent without a racy re-fetch.
    for (const [name, client] of [
      ["A", clientA],
      ["B", clientB],
    ] as const) {
      const converged = await client.waitForState(
        (s) => clientStatus(s.model_debug) === "in-world" && clientWorldCount(s.model_debug) === 2,
        { ...waitOpts, description: `client ${name} to see both players` },
      );
      assert.equal(clientStatus(converged.model_debug), "in-world");
      assert.equal(clientWorldCount(converged.model_debug), 2, `client ${name} sees both players`);
    }
  },
);
