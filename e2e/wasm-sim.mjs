// wasm netsim test: a WHOLE multiplayer session — an authoritative server and
// two clients — must run inside one browser page, deterministically, and rewind
// as a single environment.
//
// This is the browser counterpart of `functor-netsim`'s native `scrub` suite,
// and it guards the class of "works native, broken in wasm" bugs that a Rust
// test cannot see: the producers here are the real `WebPlatform` ones, built
// from the real fetched project sources, stepped through the real virtual
// network in wasm.
//
// It asserts the properties a multi-pane IDE scrubber depends on:
//
//   1. N instances of a multi-entry project (`examples/mp`: server.fun +
//      client.fun x2 over a shared protocol.fun) load and step in ONE page;
//   2. they genuinely talk to each other — the clients converge on the
//      server's authoritative world — with NO real sockets involved;
//   3. a whole-environment seek rewinds every instance at once, each to its
//      OWN view of that frame;
//   4. it is non-destructive while parked, and stepping on commits the branch
//      and replays the original timeline exactly.
//
// Scope split, deliberately: this test covers the WASM INTEGRATION — that real
// WebPlatform producers, built from really-fetched sources, step and rewind in a
// browser. It runs on perfect links, so it does not exercise the harder
// restore-packets-that-were-mid-flight case; `functor-netsim`'s native `scrub`
// suite covers that under impaired links, and link impairment is not exposed to
// JS yet (it arrives with the IDE's latency knobs).
//
// Known gap, stated rather than papered over: this does NOT verify that the
// single game is suspended while the sim runs. Doing that needs a live single
// game to poke, and booting this project trips the pre-existing refused-socket
// panic below — which may already have killed it, so a "suspension works"
// assertion here could pass vacuously. The suspension gates are covered by
// construction and review; a real test wants a project whose default entry opens
// no socket, which is worth building when the panic is fixed. [xreview]
//
// Run manually (owns its own server on :8080):
//
//   npm run build:cli:debug   # so target/debug/functor embeds this runtime
//   node e2e/wasm-sim.mjs
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BASE = "http://127.0.0.1:8080";
const PORT = 8080;
const PROJECT = "examples/mp";
// One server and two clients, so the test covers a client's view diverging from
// BOTH the server's and the other client's.
const ENTRIES = ["server.fun", "client.fun", "client.fun"];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function portInUse() {
  return new Promise((resolve) => {
    const sock = net
      .connect(PORT, "127.0.0.1")
      .on("connect", () => {
        sock.destroy();
        resolve(true);
      })
      .on("error", () => resolve(false));
  });
}

async function waitPortFree(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!(await portInUse())) return true;
    await sleep(200);
  }
  return false;
}

function cliPath() {
  for (const p of ["target/debug/functor", "target/release/functor"]) {
    if (existsSync(`${ROOT}${p}`)) return `${ROOT}${p}`;
  }
  throw new Error(
    "no functor binary — run `npm run build:cli:debug` first (the CLI embeds the web runtime)",
  );
}

const failures = [];
function check(ok, label, detail = "") {
  if (ok) {
    console.log(`  PASS  ${label}`);
  } else {
    failures.push(label);
    console.log(`  FAIL  ${label}${detail ? ` — ${detail}` : ""}`);
  }
}

async function main() {
  if (!(await waitPortFree(10000))) {
    throw new Error(`:${PORT} is busy — a stale dev server would serve the wrong project`);
  }
  const server = spawn(cliPath(), ["-d", PROJECT, "run", "wasm"], {
    cwd: ROOT,
    stdio: "ignore",
  });
  const browser = await chromium.launch({
    args: ["--use-gl=swiftshader", "--enable-unsafe-swiftshader"],
  });

  try {
    const page = await browser.newPage();
    const log = [];
    page.on("console", (m) => log.push(m.text()));
    page.on("pageerror", (e) => log.push(`pageerror: ${e}`));

    // Count every real WebSocket the page constructs, from before the first
    // script runs. The sim must open NONE: it routes the games' own
    // connect/send commands through the virtual network instead of the real
    // dispatcher, and this is what would catch that regression.
    await page.addInitScript(() => {
      window.__socketsOpened = 0;
      window.__socketLog = [];
      const Real = window.WebSocket;
      window.WebSocket = function (...args) {
        window.__socketsOpened++;
        window.__socketLog.push(`${Math.round(performance.now())}ms ${args[0]}`);
        return new Real(...args);
      };
      window.WebSocket.prototype = Real.prototype;
      Object.assign(window.WebSocket, Real);
    });

    for (let i = 0; i < 120; i++) {
      try {
        await page.goto(BASE);
        break;
      } catch {
        await sleep(500);
      }
    }
    // The single-game runtime loads first; the sim seam is installed with it.
    await page.waitForFunction(() => typeof window.__sim === "object", null, {
      timeout: 60000,
    });

    // Let the single game finish booting FIRST — it opens its own real socket
    // within the first frames, and snapshotting the counter before that would
    // blame the sim for the single game's connect.
    await sleep(3000);

    // 1. Start the sim: three producers built from this project's own sources.
    const socketsBefore = await page.evaluate(() => window.__socketsOpened);
    const count = await page.evaluate((entries) => window.__sim.start(entries, 1), ENTRIES);
    check(count === 3, "three instances start in one page", `got ${count}`);

    // Bad input is rejected rather than silently mangled (a bare string would
    // otherwise be iterated character by character into 12 "entries").
    const badArgs = await page.evaluate(async () => {
      const errs = [];
      for (const bad of ["server.fun", ["server.fun", null], []]) {
        try {
          await window.__sim.start(bad, 1);
          errs.push("accepted");
        } catch (e) {
          errs.push(String(e));
        }
      }
      return errs;
    });
    check(
      badArgs.every((e) => e !== "accepted"),
      "malformed entries are rejected, not mangled",
      badArgs.join(" | "),
    );

    // 2. Step, and read every instance's view.
    await page.evaluate(() => window.__sim.step(60));
    const views = async () =>
      page.evaluate((n) => Array.from({ length: n }, (_, i) => window.__sim.state(i)), 3);
    const anchorViews = await views();
    const anchor = (await page.evaluate(() => window.__sim.timeline())).frame - 1;

    check(
      anchorViews[1].includes("in-world") && anchorViews[2].includes("in-world"),
      "both clients joined the server's world (no real sockets)",
      anchorViews[1],
    );
    check(
      anchorViews[0].includes("players") && !anchorViews[0].includes("players: []"),
      "the server tracks the connected players",
      anchorViews[0],
    );

    // 3. Step well past the anchor, recording the timeline to reproduce.
    const timeline = [];
    for (let i = 0; i < 40; i++) {
      await page.evaluate(() => window.__sim.step(1));
      timeline.push(await views());
    }
    const liveViews = timeline[timeline.length - 1];
    check(
      liveViews.every((v, i) => v !== anchorViews[i]),
      "every instance advanced past the anchor",
    );

    // 4. Whole-environment seek: all three rewind at once, each to its own view.
    await page.evaluate((f) => window.__sim.seek(f), anchor);
    const rewound = await views();
    check(
      JSON.stringify(rewound) === JSON.stringify(anchorViews),
      "a seek rewinds every instance to its own view of the anchor frame",
      `got ${JSON.stringify(rewound)}`,
    );
    const parked = await page.evaluate(() => window.__sim.timeline());
    check(parked.scrubPos === anchor, "the sim reports where it is parked", `${parked.scrubPos}`);
    check(
      parked.hi > anchor,
      "the recorded future survives while parked (non-destructive)",
      `hi=${parked.hi}`,
    );

    // 5. Stepping on commits the branch and replays the timeline exactly.
    const replayed = [];
    for (let i = 0; i < 40; i++) {
      await page.evaluate(() => window.__sim.step(1));
      replayed.push(await views());
    }
    check(
      JSON.stringify(replayed) === JSON.stringify(timeline),
      "replay from a scrub reproduces the original timeline exactly",
    );

    // Assert on the SIM's own log only, keyed off the runtime's in-band "sim
    // started" line (console events flush asynchronously, so wall-clock phase
    // tagging in this process is racy).
    //
    // Boot noise is deliberately excluded, and it is NOT this feature's: the
    // page first boots the project's default entry as an ordinary single game,
    // and `examples/mp`'s client immediately opens a REAL WebSocket to its
    // server — which nothing is serving here, so the connection is refused.
    //
    // That refusal trips a wasm panic (`RuntimeError: unreachable`) in the
    // single-game socket path. It is PRE-EXISTING: booting this project on an
    // unmodified main produces the byte-identical three errors with no sim
    // involved at all. Tracked separately; deliberately not fixed here so this
    // change stays reviewable. Once the sim starts, the single game is
    // suspended and the sim opens no sockets whatsoever.
    const startedAt = log.findIndex((m) => m.includes("sim started"));
    check(startedAt >= 0, "the runtime logged the sim starting");
    const simLog = startedAt >= 0 ? log.slice(startedAt) : log;
    const errors = simLog.filter((m) => /error|panic|unreachable/i.test(m));
    check(errors.length === 0, "the sim logs no errors", errors.slice(0, 5).join(" | "));

    // 6. The sim opened no REAL sockets — the whole session crossed the virtual
    // network only. (Boot-time sockets belong to the single game, before the
    // sim existed, so compare against the count at start.)
    const socketsAfter = await page.evaluate(() => window.__socketsOpened);
    const socketLog = await page.evaluate(() => window.__socketLog);
    check(
      socketsAfter === socketsBefore,
      "the sim opened no real WebSockets",
      `${socketsBefore} -> ${socketsAfter}; sockets: ${socketLog.join(", ")}`,
    );

    // 7. Stop tears the sim down and hands the page back.
    await page.evaluate(() => window.__sim.stop());
    check((await page.evaluate(() => window.__sim.len())) === 0, "stop tears the sim down");
    const afterStop = await page.evaluate(() => {
      try {
        window.__sim.state(0);
        return "no error";
      } catch (e) {
        return String(e);
      }
    });
    check(
      afterStop.includes("no simulation is running"),
      "the exports report cleanly once stopped",
      afterStop,
    );
    if (process.env.SIM_DEBUG) {
      console.log("\n--- full page log ---");
      for (const line of log) console.log(`  ${line.replace(/\s+/g, " ").slice(0, 200)}`);
    }
  } finally {
    await browser.close();
    server.kill("SIGKILL");
    await waitPortFree(10000);
  }
}

console.log(`wasm netsim: ${PROJECT} as ${ENTRIES.join(" + ")}\n`);
try {
  await main();
} catch (e) {
  failures.push(`harness error: ${e}`);
  console.log(`  FAIL  harness — ${e}`);
}
console.log(
  failures.length === 0 ? "\nALL CHECKS PASSED" : `\n${failures.length} CHECK(S) FAILED`,
);
process.exit(failures.length === 0 ? 0 : 1);
