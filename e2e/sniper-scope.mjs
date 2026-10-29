// Sniper-scope E2E: three scripted shots through the real physics world, with
// no window and no GPU.
//
// Drives `examples/sniper-scope` over the debug server (`--headless`), one
// pinned fixed step at a time, and reads the outcome from `/state`'s model:
//
//   1. shoulder (RMB) and fire at the 200 m plate the rifle starts laid on → Hit
//   2. raise the aim over the backstop and fire                            → Miss
//   3. drop the aim into the dirt short of the plates and fire             → Blocked
//
// The hitscan is a `Physics.raycast` from the muzzle, so each outcome is the
// rapier world's answer, not a stub. Run (needs the CLI; the wasm bundle is not
// required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/sniper-scope.mjs
import { spawn } from "node:child_process";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const PORT = 8187;
const BASE = `http://127.0.0.1:${PORT}`;

async function post(path, body) {
  const res = await fetch(`${BASE}${path}`, { method: "POST", body: JSON.stringify(body) });
  const text = await res.text();
  if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${text}`);
}

async function state() {
  const res = await fetch(`${BASE}/state`);
  if (!res.ok) throw new Error(`GET /state → ${res.status}`);
  return res.json();
}

async function waitReady(timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      if ((await state()).frame > 0) return;
    } catch {
      // not listening yet
    }
    await sleep(100);
  }
  throw new Error("runtime never reported a frame");
}

// Queue n fixed steps and wait until they have all landed.
async function step(n = 1) {
  await post("/time", { type: "advance", dts: 1 / 60, frames: n });
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) {
    if ((await state()).pending_steps === 0) return;
    await sleep(5);
  }
  throw new Error(`${n} queued step(s) never landed`);
}

const input = (body) => post("/input", body);

// A click: press, one step (the trigger edge), release, then a few steps so the
// raycast answers after a physics step and folds back through `update`.
async function shoot() {
  await input({ type: "mouse_button", button: "left", down: true });
  await step();
  await input({ type: "mouse_button", button: "left", down: false });
  await step(3);
  return (await state()).model;
}

const proc = spawn(
  BIN,
  ["-d", "examples/sniper-scope", "run", "native", "--headless", "--debug-port", String(PORT)],
  { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
);
let log = "";
proc.stdout.on("data", (d) => (log += d));
proc.stderr.on("data", (d) => (log += d));

try {
  await waitReady(30000);
  await post("/time", { type: "set", tts: 0.0 });

  // Anchor the mouse (the first sample only records a position), then shoulder.
  await input({ type: "mouse_move", x: 400, y: 300 });
  await input({ type: "mouse_button", button: "right", down: true });
  await step(20);
  check((await state()).model.shoulder === 1, "rifle shouldered after 20 steps");

  const hit = await shoot();
  check(hit.lastShot.$ctor === "Hit" && hit.lastShot.args[0] === 2, `hit the 200 m plate: ${JSON.stringify(hit.lastShot)}`);
  check(hit.score === 200 && hit.hits === 1, `scored 200 (score ${hit.score}, hits ${hit.hits})`);
  check(hit.plates[2].ring > 0.9, "the 200 m plate rings");

  // Let the bolt cycle, then aim ~2.6 degrees up: over the 8 m backstop.
  await step(70);
  await input({ type: "mouse_move", x: 400, y: 240 });
  const miss = await shoot();
  check(miss.lastShot.$ctor === "Miss", `sailed over the backstop: ${JSON.stringify(miss.lastShot)}`);
  check(miss.score === 200 && miss.shots === 2, `no score for a miss (score ${miss.score}, shots ${miss.shots})`);

  // Aim ~3.6 degrees below level: the dirt about 24 m out, short of every plate.
  await step(70);
  await input({ type: "mouse_move", x: 400, y: 380 });
  const blocked = await shoot();
  const dirt = blocked.lastShot.$ctor === "Blocked" ? blocked.lastShot.args[0] : NaN;
  check(dirt > 10 && dirt < 40, `obstructed by the ground short of the plates: ${JSON.stringify(blocked.lastShot)}`);
  check(blocked.score === 200 && blocked.shots === 3 && blocked.hits === 1, "an obstructed shot scores nothing");
  // The plate and the dirt each keep a marker; the sky round leaves none.
  check(blocked.marks.length === 2, `two impact markers (got ${blocked.marks.length})`);
} catch (e) {
  failures.push(String(e));
  console.error(e);
} finally {
  proc.kill("SIGKILL");
}

if (failures.length) {
  console.error(`\n${failures.length} failure(s)\n--- runtime log ---\n${log.slice(-4000)}`);
  process.exit(1);
}
console.log("\nsniper-scope e2e: all checks passed");
