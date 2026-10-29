// Sub-step alpha E2E: a three-parameter `draw` interpolates between fixed
// steps on a display faster than the 60 Hz sim, headlessly over the debug
// server. Drives `examples/interpolation` (rig, prev ghost, cur ghost):
//
//   1. At a simulated 120 Hz and 90 Hz (`--display-hz`), every drawn rig lies
//      on the prev→cur segment, and some frames draw strictly between them.
//   2. A stepped frame (`/time advance`, which pauses) draws at alpha 0: the
//      rig sits exactly on the previous step.
//   3. The model recorded at fixed step K is identical at both display rates:
//      interpolation never feeds back into the sim.
//
// Run (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/interpolation.mjs
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BIN = `${ROOT}target/debug/functor`;
const SAMPLE = "examples/interpolation";
const SAMPLES = 24;
const K = 30;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function get(base, path) {
  const res = await fetch(`${base}${path}`);
  if (!res.ok) throw new Error(`GET ${path} → ${res.status}`);
  return res.json();
}

async function post(base, path, body) {
  const res = await fetch(`${base}${path}`, { method: "POST", body: JSON.stringify(body) });
  const text = await res.text();
  if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${text}`);
  return text;
}

async function waitFrame(base, atLeast, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const s = await get(base, "/state");
      if (s.frame >= atLeast) return s;
    } catch {
      // not listening yet
    }
    await sleep(50);
  }
  throw new Error(`runtime never reached frame ${atLeast}`);
}

/** Where the rig sits on prev→cur, from one `/scene` (one frame, atomic). */
async function sample(base) {
  const scene = await get(base, "/scene");
  const [rig, prev, cur] = scene.scene.obj.Group.slice(0, 3).map((n) => n.xform[3]);
  // Project onto the step's direction: 0 = prev, 1 = cur.
  const d = [cur[0] - prev[0], cur[2] - prev[2]];
  const len2 = d[0] * d[0] + d[1] * d[1];
  if (len2 === 0) return { t: 0, off: 0 };
  const r = [rig[0] - prev[0], rig[2] - prev[2]];
  const t = (r[0] * d[0] + r[1] * d[1]) / len2;
  const off = Math.abs(r[0] * d[1] - r[1] * d[0]) / Math.sqrt(len2);
  return { t, off };
}

async function drive(hz, port) {
  const base = `http://127.0.0.1:${port}`;
  const proc = spawn(
    BIN,
    ["-d", SAMPLE, "run", "native", "--headless", "--display-hz", String(hz), "--debug-port", String(port)],
    { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
  );
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  try {
    await waitFrame(base, 2, 30000);
    const live = [];
    for (let i = 0; i < SAMPLES; i++) {
      live.push(await sample(base));
      await sleep(7);
    }
    await waitFrame(base, K + 2, 30000);
    // Advance pauses the clock and runs one step; wait for it to land.
    const before = (await get(base, "/state")).frame;
    await post(base, "/time", { type: "advance", dts: 1 / 60 });
    await waitFrame(base, before + 1, 10000);
    const stepped = await sample(base);
    await post(base, "/rewind", { frame: K });
    // The model text plus the stepped poses its next frame draws.
    const model = (await get(base, "/state")).model;
    const ghosts = (await get(base, "/scene")).scene.obj.Group.slice(1, 3).map((n) => n.xform[3]);
    const recorded = JSON.stringify({ model, ghosts });
    return { live, stepped, recorded };
  } catch (e) {
    throw new Error(`${e.message}\n--- runtime log (${hz} Hz) ---\n${log}`);
  } finally {
    proc.kill("SIGKILL");
  }
}

const failures = [];
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures.push(what);
};
const EPS = 1e-3;

const runs = {};
for (const [hz, port] of [[120, 8931], [90, 8932]]) {
  const run = await drive(hz, port);
  runs[hz] = run;
  check(
    run.live.every(({ t, off }) => t > -EPS && t < 1 + EPS && off < EPS),
    `${hz} Hz: every rig on prev→cur (t = ${run.live.map(({ t }) => t.toFixed(2)).join(" ")})`,
  );
  check(run.live.some(({ t }) => t > 0.1 && t < 0.9), `${hz} Hz: some frames draw between steps`);
  check(Math.abs(run.stepped.t) < EPS, `${hz} Hz: a stepped frame draws at alpha 0 (t = ${run.stepped.t})`);
}
check(
  runs[120].recorded === runs[90].recorded,
  `model at step ${K} is identical at 120 Hz and 90 Hz:\n  ${runs[120].recorded}\n  ${runs[90].recorded}`,
);

if (failures.length) {
  console.error(`\n${failures.length} check(s) failed`);
  process.exit(1);
}
console.log("\ninterpolation e2e passed");
