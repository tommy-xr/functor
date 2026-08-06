import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner, waitForPort } from "../src/index.js";

// End-to-end network simulation: one server + two client runners
// (examples/orbs — a multi-entry project whose two roles are inline `module
// Client` / `module Server` blocks of ONE game.fun, so the wire ADT is
// declared exactly once), each its own process on its own debug port,
// networked over a real WebSocket. The `.fun` ships as text (no dylib build),
// so this only needs the runner binary and a display:
//
//   cargo build --bin functor
//   FUNCTOR_E2E=1 node --test dist/test/
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

// The example models are exposed only as Functor Lang Debug text (the game model isn't
// Serialize yet), so these read the `model` string. An Functor Lang record renders as
// `{ field: value, ... }` and a list as `[elem, ...]` — no Fable linked list.
//
// The server model is `{ world: { pilots: [...], ... }, seats: [<Seat>, ...] }`;
// each Seat renders as `{ cid: c, pid: p }` and `cid:` appears nowhere else, so
// counting `cid:` markers counts the seated connections.
const serverSeatCount = (model: string): number =>
  (model.match(/cid:/g) ?? []).length;

// The client model is `{ conn: …, myPid: p, ships: [...], orbs: [...], … }`.
// `myPid` starts at -1 and is only replaced by a Welcome off the wire, so a
// non-negative pid IS the server's answer arriving.
const clientPid = (model: string): number =>
  Number(model.match(/myPid: (-?[\d.]+)/)?.[1] ?? NaN);

// Each Ship in `ships` renders as `{ pid: p, x: .., y: .., rot: .. }`. Orbs
// carry `id:`/`owner:` and `myPid:` is capitalised, so lowercase `pid:`
// markers count the ships the client was last sent.
const clientShipCount = (model: string): number =>
  (model.match(/\bpid: /g) ?? []).length;

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

    // Debug ports are OS-assigned per launch (the SDK default), so parallel
    // test files can never collide. (The GAME's ws port is fixed at 9101 by
    // the example game — only this one test uses it, so it stays.)
    const gameDir = join(repoRoot, "examples", "orbs");
    // Both roles are the SAME file, so the role is named rather than inferred
    // from the path — the CLI's `--entry <name>` against functor.json's roles.
    const launch = (entry: "client" | "server") =>
      FunctorRunner.launch({
        gameDir,
        repoRoot,
        functorLangPath: join(gameDir, "game.fun"),
        entry,
        launchTimeoutMs: 60_000,
        headless,
      });

    // Deliberately start client A before any listener exists and leave enough
    // time for its first connection attempt to fail. `Sub.connect` is a desired
    // connection, so the same runner must converge after the server appears.
    await using clientA = await launch("client");
    await new Promise((resolve) => setTimeout(resolve, 1_000));

    await using server = await launch("server");
    await waitForPort("127.0.0.1", 9101, {
      timeoutMs: 60_000,
      description: "orbs server ws listener",
    });

    // The client Joins on connect and streams a Steer every tick from there,
    // so no input injection is needed.
    await using clientB = await launch("client");

    const waitOpts = { timeoutMs: 90_000, intervalMs: 200 };

    // The server should accept both connections and seat a pilot for each.
    const serverState = await server.waitForState(
      (s) => serverSeatCount(s.model_debug) === 2,
      { ...waitOpts, description: "server to seat 2 clients" },
    );
    assert.equal(serverSeatCount(serverState.model_debug), 2);

    // Each client should be seated (a pid off the wire) and see both ships in
    // the snapshot the server broadcasts — i.e. the clients converge on a
    // shared world. The waitForState calls already enforce this (they throw on
    // timeout); asserting on their converged return value documents intent
    // without a racy re-fetch.
    for (const [name, client] of [
      ["A", clientA],
      ["B", clientB],
    ] as const) {
      const converged = await client.waitForState(
        (s) => clientPid(s.model_debug) >= 0 && clientShipCount(s.model_debug) === 2,
        { ...waitOpts, description: `client ${name} to see both ships` },
      );
      assert.ok(clientPid(converged.model_debug) >= 0, `client ${name} was seated`);
      assert.equal(clientShipCount(converged.model_debug), 2, `client ${name} sees both ships`);
    }
  },
);
