import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner, waitForPort } from "../src/index.js";

// End-to-end network simulation: one mpserver + two mpclient runners,
// each its own process on its own debug port, networked over a real WebSocket.
// These are the Functor Lang ports of examples/mpserver + examples/mpclient — same wire
// protocol and same auto-move-on-connect, so convergence is identical. The
// `.fun` ships as text (no dylib build), so this only needs the runner binary
// and a display:
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

test(
  "two clients connect to a server and converge on a shared world",
  { skip: !e2eEnabled, timeout: 180_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    // All three debug ports derive from one base so the test is self-consistent
    // when relocated. (The ws port is fixed at 9001 by the example games.)
    const base = Number(process.env.FUNCTOR_E2E_PORT ?? 8095);
    const launch = (game: string, port: number) => {
      const gameDir = join(repoRoot, "examples", game);
      return FunctorRunner.launch({
        gameDir,
        repoRoot,
        functorLangPath: join(gameDir, "game.fun"),
        port,
        launchTimeoutMs: 30_000,
        headless,
      });
    };

    // Server first, and wait for its Sub.listen socket to actually bind before
    // launching clients — mpclient connects once with no retry, so a client that
    // races ahead of the listener would land in "error" and never converge.
    await using server = await launch("mpserver", base);
    await waitForPort("127.0.0.1", 9001, {
      timeoutMs: 15_000,
      description: "mpserver ws listener",
    });

    // mpclient auto-moves on connect, so no input injection is needed.
    await using clientA = await launch("mpclient", base + 1);
    await using clientB = await launch("mpclient", base + 2);

    const waitOpts = { timeoutMs: 20_000, intervalMs: 200 };

    // The server should accept both connections and track a player for each.
    const serverState = await server.waitForState(
      (s) => serverPlayerCount(s.model) === 2,
      { ...waitOpts, description: "server to track 2 players" },
    );
    assert.equal(serverPlayerCount(serverState.model), 2);

    // Each client should reach "in-world" and see both players in the snapshot
    // the server broadcasts — i.e. the clients converge on a shared world. The
    // waitForState calls already enforce this (they throw on timeout); asserting
    // on their converged return value documents intent without a racy re-fetch.
    for (const [name, client] of [
      ["A", clientA],
      ["B", clientB],
    ] as const) {
      const converged = await client.waitForState(
        (s) => clientStatus(s.model) === "in-world" && clientWorldCount(s.model) === 2,
        { ...waitOpts, description: `client ${name} to see both players` },
      );
      assert.equal(clientStatus(converged.model), "in-world");
      assert.equal(clientWorldCount(converged.model), 2, `client ${name} sees both players`);
    }
  },
);
