// Floor-locked XR E2E: `--emulate-xr` stands a floor-locked game's head at the
// configured eye height, and leaves other games' head-centred rig unchanged.
//
// The fixture (`e2e/fixtures/floor-locked`) records `xr.head` and the head
// mapped through a level rig at the player's feet (y = 0). Three headless runs:
//
//   1. `Xr.floorLocked(true)` with `--xr-eye-height 1.75`: head.y = 1.75,
//      upright, so the mapped eye is 1.75 m up with world up.
//   2. The same with the default eye height (1.6 m).
//   3. The fixture without its `config` (a temp copy): the legacy identity
//      head, so the mapped eye sits on the rig.
//
// Run (needs the CLI; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/floor-locked.mjs
import { spawn } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const FIXTURE = join(ROOT, "e2e/fixtures/floor-locked");
const PORT = 8193;
const near = (a, b) => Math.abs(a - b) < 1e-4;

async function state() {
  const res = await fetch(`http://127.0.0.1:${PORT}/state`);
  if (!res.ok) throw new Error(`GET /state → ${res.status}`);
  return res.json();
}

/** Run `dir` headless under `--emulate-xr` until the model has seen a head. */
async function sampled(dir, extraArgs) {
  const proc = spawn(
    BIN,
    ["-d", dir, "run", "native", "--headless", "--emulate-xr", "--debug-port", String(PORT), ...extraArgs],
    { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
  );
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  // Registered now: an early exit must still settle cleanup and stop polling.
  let exited = false;
  const exit = new Promise((r) => proc.once("close", () => r((exited = true))));
  proc.once("error", (e) => (log += String(e)));
  try {
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline && !exited) {
      try {
        const s = await state();
        if (s.model?.eye?.$ctor === "Option.Some") return s;
      } catch {
        // not listening yet
      }
      await sleep(200);
    }
    throw new Error(`no XR head sampled in ${dir}\n${log.slice(-2000)}`);
  } finally {
    proc.kill("SIGKILL");
    await exit;
  }
}

function expectRig(label, s, { floorLocked, headY }) {
  const head = s.model.head.args[0];
  const eye = s.model.eye.args[0];
  check(Boolean(s.config?.xr?.floor_locked) === floorLocked, `${label}: config.xr.floor_locked = ${floorLocked}`);
  check(near(s.input.xr.head.position[1], headY), `${label}: /state input.xr.head.y = ${headY} (got ${s.input.xr.head.position[1]})`);
  check(near(head.position.y, headY), `${label}: the game's xr.head.y = ${headY} (got ${head.position.y})`);
  const q = head.orientation;
  check(near(q.x, 0) && near(q.y, 0) && near(q.z, 0) && near(Math.abs(q.w), 1), `${label}: head upright`);
  check(near(eye.position.y, headY) && near(eye.position.x, 2) && near(eye.position.z, 3), `${label}: mapped eye at rig + head (y ${eye.position.y})`);
  check(near(eye.up.y, 1), `${label}: mapped up is world up`);
}

const legacy = mkdtempSync(join(tmpdir(), "floor-locked-legacy-"));
try {
  console.log("\n▸ floor-locked, --xr-eye-height 1.75");
  expectRig("1.75", await sampled(FIXTURE, ["--xr-eye-height", "1.75"]), { floorLocked: true, headY: 1.75 });

  console.log("\n▸ floor-locked, default eye height");
  expectRig("default", await sampled(FIXTURE, []), { floorLocked: true, headY: 1.6 });

  console.log("\n▸ legacy: no config");
  cpSync(FIXTURE, legacy, { recursive: true });
  const game = join(legacy, "game.fun");
  const stripped = readFileSync(game, "utf8").replace(/^let config = .*\n/m, "");
  check(!stripped.includes("Xr.floorLocked"), "legacy copy has no Xr.floorLocked");
  writeFileSync(game, stripped);
  expectRig("legacy", await sampled(legacy, ["--xr-eye-height", "1.75"]), { floorLocked: false, headY: 0 });
} catch (e) {
  failures.push(String(e));
  console.error(e);
} finally {
  rmSync(legacy, { recursive: true, force: true });
}

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nfloor-locked e2e: all checks passed");
