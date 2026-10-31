// liminal-office E2E: walk the office through the real physics world, with a
// hidden window and the clock paused between `/time` steps.
//
//   1. a scripted route: apron → lobby → corridor → office 106 → back out;
//   2. an outside → in push against a solid facade wall is blocked;
//   3. inside, portal culling hides rooms (`/state.cells.hidden > 0`);
//   4. culling on and off (FUNCTOR_CULL=0) capture identical frames at views 1–3;
//   5. the entrance and corridor goldens render byte-identically across runs
//      (`npm run test:golden` compares them to golden/, with a tolerance).
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/liminal-office.mjs
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const DIR = join(ROOT, "examples/liminal-office");
const STEP = 1 / 60;
const MOUSE_RATE = 0.0025; // game.fun's `mouseRate`, radians per point
// Cell ids are layout.fun's room positions.
const APRON = 0;
const LOBBY = 1;
const CORRIDOR = 2;
const OFFICE_106 = 9;

function launch(port, env = {}) {
  const proc = spawn(BIN, ["-d", DIR, "run", "native", "--hidden", "--debug-port", String(port)], {
    cwd: ROOT,
    env: { ...process.env, ...env },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));
  const base = `http://127.0.0.1:${port}`;
  const post = async (path, body) => {
    const res = await fetch(`${base}${path}`, { method: "POST", body: JSON.stringify(body) });
    if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${await res.text()}`);
    return res;
  };
  const state = async () => {
    const res = await fetch(`${base}/state`);
    if (!res.ok) throw new Error(`GET /state → ${res.status}`);
    return res.json();
  };
  const game = {
    proc,
    log: () => log,
    state,
    async ready() {
      const deadline = Date.now() + 60000;
      while (Date.now() < deadline) {
        try {
          if ((await state()).frame > 0) return post("/time", { type: "set", tts: 0 });
        } catch {
          // not listening yet
        }
        await sleep(100);
      }
      throw new Error("runtime never reported a frame");
    },
    // Queue n fixed steps and wait for them all to land.
    async step(n = 1) {
      await post("/time", { type: "advance", dts: STEP, frames: n });
      const deadline = Date.now() + 120000;
      while (Date.now() < deadline) {
        if ((await state()).pending_steps === 0) return;
        await sleep(10);
      }
      throw new Error(`${n} queued step(s) never landed`);
    },
    key: (key, down) => post("/input", { type: "key", key, down }),
    async hold(key, n) {
      await game.key(key, true);
      await game.step(n);
      await game.key(key, false);
      await game.step(1);
    },
    // Turn by `radians` (left positive), through the mouse-look hook.
    mouseX: 400,
    async turn(radians) {
      // Pointer positions are whole points; the rounding is under a milliradian.
      game.mouseX -= Math.round(radians / MOUSE_RATE);
      await post("/input", { type: "mouse_move", x: game.mouseX, y: 300 });
      await game.step(1);
    },
    async view(n) {
      await game.hold(String(n), 1);
      await game.step(3);
    },
    async capture() {
      const res = await post("/capture", {});
      return Buffer.from(await res.arrayBuffer());
    },
    model: async () => (await state()).model,
  };
  return game;
}

const near = (a, b, tol) => Math.abs(a - b) <= tol;

async function route(game) {
  // Anchor the pointer: the first sample only records a position.
  await game.turn(0);

  let m = await game.model();
  check(m.cell === APRON && m.baked === true, `starts on the apron with a current bake (cell ${m.cell}, baked ${m.baked})`);

  // North through the entrance, the lobby and up the corridor to z ≈ 16.5.
  await game.hold("w", 360);
  m = await game.model();
  check(m.cell === LOBBY, `walked into the lobby (cell ${m.cell}, z ${m.at.z.toFixed(2)})`);
  await game.hold("w", 390);
  m = await game.model();
  check(m.cell === CORRIDOR && near(m.at.x, 10, 0.05), `walked up the corridor (cell ${m.cell}, x ${m.at.x.toFixed(2)}, z ${m.at.z.toFixed(2)})`);
  // The street is 3 cm lower, so standing at floor height means the floor trimesh holds.
  check(m.at.y > 0.9, `the floor trimesh carries the player (y ${m.at.y.toFixed(3)})`);
  const s = await game.state();
  check(s.cells.hidden > 0, `the corridor hides rooms (cells ${JSON.stringify(s.cells)})`);

  // Into office 106 through its door at z = 16.5.
  await game.turn(Math.PI / 2); // face +x
  await game.hold("w", 180);
  m = await game.model();
  check(m.cell === OFFICE_106 && m.at.x > 13, `walked into office 106 (cell ${m.cell}, x ${m.at.x.toFixed(2)})`);
  const inOffice = await game.state();
  check(inOffice.cells.hidden > 0, `the office hides rooms (cells ${JSON.stringify(inOffice.cells)})`);

  // Back to the corridor, then south and out onto the apron.
  await game.turn(Math.PI); // face -x
  await game.hold("w", 180);
  m = await game.model();
  check(m.cell === CORRIDOR, `back in the corridor (cell ${m.cell}, x ${m.at.x.toFixed(2)})`);
  await game.turn(-Math.PI / 2); // face -z
  await game.hold("w", 750);
  m = await game.model();
  check(m.cell === APRON && m.at.z < -5, `back out on the apron (cell ${m.cell}, z ${m.at.z.toFixed(2)})`);
}

// From the apron, step left of the entrance and walk into the solid facade.
async function wallPush(game) {
  await game.turn(Math.PI); // face +z again
  await game.hold("d", 95); // strafe to x ≈ 6.5: the lobby's facade, clear of the lamp posts
  let m = await game.model();
  check(m.at.x > 6.2 && m.at.x < 7, `lined up on a solid wall (x ${m.at.x.toFixed(2)})`);
  await game.hold("w", 420);
  m = await game.model();
  // The facade's outer face is z = -0.15 and the capsule's radius 0.3: it
  // reached the wall and stopped there.
  check(m.at.z < -0.4 && m.at.z > -0.6 && m.cell === APRON, `outside → in push blocked at the wall (z ${m.at.z.toFixed(3)}, cell ${m.cell})`);
}

async function viewCaptures(game, culling) {
  const shots = [];
  for (const n of [1, 2, 3]) {
    await game.view(n);
    const s = await game.state();
    if (culling && n > 1) check(s.cells.hidden > 0, `view ${n} hides rooms (cells ${JSON.stringify(s.cells)})`);
    shots.push(await game.capture());
  }
  return shots;
}

// Render a golden the way runtime/functor-runtime-desktop/tests/golden.rs does.
function golden(name, args) {
  const out = join(mkdtempSync(join(tmpdir(), "liminal-office-")), `${name}.png`);
  const run = spawnSync(BIN, ["-d", DIR, "run", "native", "--", "--capture-frame", out, "--capture-size", "1600x1200", ...args], {
    cwd: ROOT,
    encoding: "utf8",
  });
  if (run.status !== 0) throw new Error(`${name}: ${run.stderr}`);
  const bytes = readFileSync(out);
  rmSync(out, { force: true });
  return bytes;
}

const games = [];
try {
  const game = launch(8191);
  games.push(game);
  await game.ready();
  await route(game);
  await wallPush(game);
  const culled = await viewCaptures(game, true);

  const unculled = launch(8192, { FUNCTOR_CULL: "0" });
  games.push(unculled);
  await unculled.ready();
  const all = await viewCaptures(unculled, false);
  [1, 2, 3].forEach((n, i) => check(culled[i].equals(all[i]), `view ${n}: culling on and off capture the same frame`));

  const goldens = {
    "liminal-office-entrance": ["--fixed-time", "1", "--capture-time", "1.0"],
    "liminal-office-corridor": ["--input-script", join(DIR, "views.script"), "--capture-at-frame", "30"],
  };
  for (const [name, args] of Object.entries(goldens)) {
    check(golden(name, args).equals(golden(name, args)), `${name}: byte-identical across runs`);
  }
} catch (e) {
  failures.push(String(e));
  console.error(e);
} finally {
  for (const g of games) g.proc.kill("SIGKILL");
}

if (failures.length) {
  console.error(`\n${failures.length} failure(s)\n--- runtime log ---\n${games.map((g) => g.log().slice(-3000)).join("\n")}`);
  process.exit(1);
}
console.log("\nliminal-office e2e: all checks passed");
