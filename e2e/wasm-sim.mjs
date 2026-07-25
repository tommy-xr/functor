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
// It also verifies the SUSPENSION invariant the design rests on: while a sim
// owns the page the single game must stop simulating, or the two contend for the
// thread's shared command queues. `window.__scrub.frame()` advances only when
// the single game ticks, so it is the observable — and the test first asserts
// the single game is genuinely RUNNING, without which "it stopped" would pass
// vacuously. (That vacuity was real until the refused-socket panic was fixed:
// booting this project used to trap the runtime, so there was nothing alive to
// suspend.) [xreview]
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
// Module-scoped so a harness throw inside main() can still dump what the page
// said before it failed.
const pageLog = [];
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
  // `--no-open`: on a headless runner there is no browser to launch, and
  // without this the dev server never becomes reachable. Its output is piped
  // (not ignored) so a failure to start is diagnosable instead of silent.
  const server = spawn(cliPath(), ["-d", PROJECT, "run", "wasm", "--no-open"], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let serverOutput = "";
  server.stdout.on("data", (d) => (serverOutput += d));
  server.stderr.on("data", (d) => (serverOutput += d));

  // The same GL flags the wasm-smoke job uses: software rendering that comes up
  // on any runner, CI included, with no real GPU.
  const browser = await chromium.launch({
    args: [
      "--use-gl=angle",
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
    ],
  });

  try {
    const page = await browser.newPage();
    const log = pageLog;
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

    // Wait for the dev server to actually be listening, and say so plainly if
    // it never starts — retrying `goto` blindly turns "the server died" into an
    // opaque timeout 60s later, which is exactly how this first failed in CI.
    let up = false;
    for (let i = 0; i < 60 && !up; i++) {
      up = await portInUse();
      if (!up) await sleep(500);
    }
    if (!up) {
      throw new Error(
        `the dev server never listened on :${PORT}. Its output was:\n${serverOutput || "(none)"}`,
      );
    }
    for (let i = 0; i < 60; i++) {
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
    // blame the sim for the single game's connect. Wait for that socket rather
    // than a fixed sleep: a slow runner would otherwise push the boot past the
    // deadline and make the no-real-sockets check flaky in exactly that way.
    for (let i = 0; i < 40; i++) {
      if (await page.evaluate(() => window.__socketsOpened > 0)) break;
      await sleep(250);
    }
    // Plus a moment for any follow-on connect to land before the snapshot.
    await sleep(1000);

    // The single game must be ALIVE before we claim the sim suspends it —
    // otherwise every assertion below passes for the wrong reason.
    const singleFrame = () => page.evaluate(() => window.__scrub?.frame() ?? null);
    // Wait in RENDER FRAMES, not wall-clock: the thing under test is whether the
    // game ticks, and a slow runner would make a fixed sleep flaky in both
    // directions (a false "not advancing", or a suspension "proved" without a
    // single render opportunity). [xreview]
    const waitRenderFrames = (n) =>
      page.evaluate(
        (count) =>
          new Promise((resolve) => {
            let seen = 0;
            const tick = () => (++seen >= count ? resolve() : requestAnimationFrame(tick));
            requestAnimationFrame(tick);
          }),
        n,
      );

    const frameA = await singleFrame();
    await waitRenderFrames(20);
    const frameB = await singleFrame();
    check(
      frameA !== null && frameB !== null && frameB > frameA,
      "the single game is running before the sim starts",
      `frames ${frameA} -> ${frameB}`,
    );

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
    // Boot is now expected to be CLEAN, which is a regression test in itself:
    // `examples/mp`'s client opens a real WebSocket to a server nothing is
    // serving, and that refusal used to trap the whole wasm runtime
    // (`RuntimeError: unreachable`, preceded by a TypeError from the glue
    // reading `message` off a plain `Event`). The only line the page may still
    // print is Chrome's own ERR_CONNECTION_REFUSED, which is the browser
    // reporting a genuinely absent server and is not ours to silence.
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

    // 7. The single game is SUSPENDED while the sim owns the page: its scene
    // frame stops advancing, because it no longer ticks. This is the invariant
    // that keeps the two from contending for the shared command queues.
    const suspendedA = await singleFrame();
    await waitRenderFrames(20);
    const suspendedB = await singleFrame();
    check(
      // `!== null` guards the vacuity this check exists to prevent: if the
      // scrubber seam vanished, `null === null` would report a suspension that
      // was really an absence. [xreview]
      suspendedA !== null && suspendedA === suspendedB,
      "the single game is suspended while the sim runs",
      `frames ${suspendedA} -> ${suspendedB}`,
    );

    // 8. Stop tears the sim down and hands the page back.
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

    // 9. ...and the single game resumes simulating once it has the page back.
    const resumedA = await singleFrame();
    await waitRenderFrames(20);
    const resumedB = await singleFrame();
    check(
      resumedA !== null && resumedB > resumedA,
      "the single game resumes after the sim stops",
      `frames ${resumedA} -> ${resumedB}`,
    );

    // 10. LAST, so it covers everything above — boot, the sim, suspension,
    // stop, and resume. Nothing in the page's life may trap the runtime; a
    // refused socket used to. Give any straggling error event a turn to be
    // delivered before reading the log. [xreview]
    await waitRenderFrames(5);
    const traps = log.filter((m) => /unreachable|panic|TypeError/i.test(m));
    check(
      traps.length === 0,
      "no wasm trap or JS type error, anywhere in the run",
      traps.slice(0, 3).join(" | "),
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
  // A harness failure with no context is unactionable in CI — dump whatever the
  // page managed to say before it went wrong.
  if (pageLog.length) {
    console.log("\n--- page log ---");
    for (const line of pageLog.slice(-30)) {
      console.log(`  ${line.replace(/\s+/g, " ").slice(0, 200)}`);
    }
  }
}
console.log(
  failures.length === 0 ? "\nALL CHECKS PASSED" : `\n${failures.length} CHECK(S) FAILED`,
);
process.exit(failures.length === 0 ? 0 : 1);
