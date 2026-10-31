// liminal-office E2E: walk the office through the real physics world, with a
// hidden window and the clock paused between `/time` steps.
//
//   1. a scripted route: apron → lobby → corridor → office 106 → back out;
//   2. the stairs: entrance → corridor → stairwell → up the flight (grounded
//      on its ramp the whole way) → upstairs office 206 → back down;
//   3. an outside → in push against a solid facade wall is blocked;
//   4. inside, the portal walk hides cells' subtrees (`/state.cells.hidden`),
//      more upstairs than in the lobby. (The baked rooms are `Model.static`
//      data models, which the renderer draws whether or not the walk hides
//      them; these numbers are the walk's, not the GPU's.)
//   5. the hour: rewinding restores it and its lighting; the XR controller's
//      X and Y step and flip it; by day, daylight brightens office 103
//      (window) while the corridor stays fixture-lit;
//   6. culling on and off (FUNCTOR_CULL=0) capture identical frames at views
//      1–8, the stairwell looking up and down among them;
//   7. the night and day goldens render byte-identically across runs
//      (`npm run test:golden` compares them to golden/, with a tolerance).
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/liminal-office.mjs
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { decodePng } from "../scripts/png.mjs";
import { BIN, ROOT, check, failures, sleep } from "./mcp-rpc.mjs";

const DIR = join(ROOT, "examples/liminal-office");
const STEP = 1 / 60;
const MOUSE_RATE = 0.0025; // game.fun's `mouseRate`, radians per point
// Cell ids are layout.fun's room positions.
const APRON = 0;
const LOBBY = 1;
const CORRIDOR = 2;
const OFFICE_106 = 9;
const STAIR_FOOT = 11;
const STAIR_HEAD = 12;
const CORRIDOR_2 = 13;
const TURN_2 = 14;
const OFFICE_206 = 20;
// layout.fun's storey and flight, and game.fun's capsule centre above the feet.
const STOREY = 3.3;
const FLIGHT = { x0: 13.0, tread: 0.3, risers: 18 };
const CENTRE = 0.92;
// The ramp's height at x: building.fun's `rampY`.
const rampY = (x) =>
  Math.min(STOREY, Math.max(0, ((x - (FLIGHT.x0 - FLIGHT.tread)) * (STOREY / FLIGHT.risers)) / FLIGHT.tread));
const WALK = 2.2 / 60; // metres per step

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
    post,
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
    // Face `yaw` (0 is +z, π/2 is +x).
    async face(yaw) {
      await game.turn(yaw - (await game.model()).yaw);
    },
    // Hold W until the player passes `target` along `axis`, `chunk` steps at
    // a time; `onStep` sees the model after each chunk.
    async walkTo(axis, target, { chunk = 30, onStep } = {}) {
      let m = await game.model();
      const sign = Math.sign(target - m.at[axis]);
      await game.key("w", true);
      while ((target - m.at[axis]) * sign > 0.02) {
        const before = m.at[axis];
        await game.step(Math.max(1, Math.min(chunk, Math.floor(((target - m.at[axis]) * sign) / WALK))));
        m = await game.model();
        onStep?.(m);
        if (Math.abs(m.at[axis] - before) < 0.001) break; // blocked
      }
      await game.key("w", false);
      await game.step(1);
      return game.model();
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
let lobbyHidden;

async function route(game) {
  // Anchor the pointer: the first sample only records a position.
  await game.turn(0);

  let m = await game.model();
  check(m.cell === APRON && m.baked === true, `starts on the apron with a current bake (cell ${m.cell}, baked ${m.baked})`);

  // North through the entrance, the lobby and up the corridor to z ≈ 16.5.
  await game.hold("w", 360);
  m = await game.model();
  check(m.cell === LOBBY, `walked into the lobby (cell ${m.cell}, z ${m.at.z.toFixed(2)})`);
  lobbyHidden = (await game.state()).cells.hidden;
  await game.hold("w", 390);
  m = await game.model();
  check(m.cell === CORRIDOR && near(m.at.x, 10, 0.05), `walked up the corridor (cell ${m.cell}, x ${m.at.x.toFixed(2)}, z ${m.at.z.toFixed(2)})`);
  // The street is 3 cm lower, so standing at floor height means the floor trimesh holds.
  check(m.at.y > 0.9, `the floor trimesh carries the player (y ${m.at.y.toFixed(3)})`);
  const s = await game.state();
  check(s.cells.hidden > 0, `the corridor's portal walk hides subtrees (cells ${JSON.stringify(s.cells)})`);

  // Into office 106 through its door at z = 16.5.
  await game.turn(Math.PI / 2); // face +x
  await game.hold("w", 180);
  m = await game.model();
  check(m.cell === OFFICE_106 && m.at.x > 13, `walked into office 106 (cell ${m.cell}, x ${m.at.x.toFixed(2)})`);
  const inOffice = await game.state();
  check(inOffice.cells.hidden > 0, `the office's portal walk hides subtrees (cells ${JSON.stringify(inOffice.cells)})`);

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

// On the flight, the capsule rides its ramp: grounded, its centre a capsule's
// settle above the ramp (more on a slope than on the flat), never below it.
function rampChecker(label) {
  const misses = [];
  let samples = 0;
  const onStep = (m) => {
    if (m.at.x < FLIGHT.x0 || m.at.x > FLIGHT.x0 + (FLIGHT.risers - 1) * FLIGHT.tread) return;
    samples++;
    const above = m.at.y - (rampY(m.at.x) + CENTRE);
    if (!m.grounded || above < -0.02 || above > 0.12) misses.push(`x ${m.at.x.toFixed(2)} y ${m.at.y.toFixed(3)} (+${above.toFixed(3)}) grounded ${m.grounded}`);
  };
  const report = () =>
    check(samples >= 20 && misses.length === 0, `${label}: grounded on the ramp at ${samples} samples${misses.length ? `; off at ${misses.slice(0, 3).join(", ")}` : ""}`);
  return { onStep, report };
}

// Entrance → corridor → the stairwell → up the flight → office 206 upstairs
// → back down to the corridor.
async function stairs(game) {
  await game.view(1);
  await game.face(0);
  let m = await game.walkTo("z", 23.3);
  check(m.cell === CORRIDOR, `walked up the corridor to the stairwell door (cell ${m.cell}, z ${m.at.z.toFixed(2)})`);
  await game.face(Math.PI / 2);
  m = await game.walkTo("x", 12.0);
  check(m.cell === STAIR_FOOT, `into the stairwell (cell ${m.cell}, x ${m.at.x.toFixed(2)})`);
  await game.face(Math.PI);
  await game.walkTo("z", 20.8);
  await game.face(Math.PI / 2);
  const up = rampChecker("up the flight");
  m = await game.walkTo("x", 19.2, { chunk: 4, onStep: up.onStep });
  up.report();
  check(m.cell === STAIR_HEAD && near(m.at.y, STOREY + CENTRE, 0.05), `on the landing (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
  await game.face(0);
  m = await game.walkTo("z", 27.8);
  check(m.cell === TURN_2, `through the upstairs stair door (cell ${m.cell}, z ${m.at.z.toFixed(2)})`);
  await game.face(-Math.PI / 2);
  await game.walkTo("x", 10.0);
  await game.face(Math.PI);
  m = await game.walkTo("z", 16.5);
  check(m.cell === CORRIDOR_2, `down the upstairs corridor (cell ${m.cell}, z ${m.at.z.toFixed(2)})`);
  await game.face(Math.PI / 2);
  m = await game.walkTo("x", 15.0);
  const office = await game.state();
  check(m.cell === OFFICE_206 && near(m.at.y, STOREY + CENTRE, 0.05), `in office 206 upstairs (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
  check(office.cells.hidden > lobbyHidden, `upstairs hides more than the lobby (${office.cells.hidden} vs ${lobbyHidden} subtrees)`);

  // Back the way we came.
  await game.face(-Math.PI / 2);
  await game.walkTo("x", 10.0);
  await game.face(0);
  await game.walkTo("z", 27.8);
  await game.face(Math.PI / 2);
  await game.walkTo("x", 19.2);
  await game.face(Math.PI);
  m = await game.walkTo("z", 20.8);
  check(m.cell === STAIR_HEAD, `back on the landing (cell ${m.cell}, z ${m.at.z.toFixed(2)})`);
  await game.face(-Math.PI / 2);
  const down = rampChecker("down the flight");
  m = await game.walkTo("x", 12.0, { chunk: 4, onStep: down.onStep });
  down.report();
  check(m.cell === STAIR_FOOT && near(m.at.y, CENTRE, 0.05), `at the stair foot (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
  await game.face(0);
  await game.walkTo("z", 23.3);
  await game.face(-Math.PI / 2);
  m = await game.walkTo("x", 10.0);
  check(m.cell === CORRIDOR && near(m.at.y, CENTRE, 0.05), `back in the ground-floor corridor (cell ${m.cell}, y ${m.at.y.toFixed(3)})`);
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

// Mean luma (0–255) of a captured PNG over the rect given as fractions of
// the image, [x0, y0, x1, y1].
function meanLuma(png, [x0, y0, x1, y1]) {
  const { width, height, channels, pixels } = decodePng(png);
  let sum = 0;
  let n = 0;
  for (let y = Math.floor(y0 * height); y < Math.floor(y1 * height); y++) {
    for (let x = Math.floor(x0 * width); x < Math.floor(x1 * width); x++) {
      const p = pixels.subarray((y * width + x) * channels);
      sum += 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
      n++;
    }
  }
  return sum / n;
}

// The largest per-channel difference between two same-sized captures.
function maxDiff(a, b) {
  const [pa, pb] = [decodePng(a).pixels, decodePng(b).pixels];
  let max = 0;
  for (let i = 0; i < pa.length; i++) max = Math.max(max, Math.abs(pa[i] - pb[i]));
  return max;
}

// The hour is model state: N toggles midnight ⇄ noon, T steps an hour, and a
// rewind restores both the hour and the bake it draws with.
async function hours(game) {
  // Stand still at view 1 first, so the rewound pose matches to the pixel.
  await game.view(1);
  await game.step(30);
  // Hold each hour for 10 frames and rewind to the middle of that run:
  // `/state.frame` and the recorder's frame numbers are a couple apart.
  const press = async (key) => {
    await game.hold(key, 10);
    return { frame: (await game.state()).frame - 5, hour: (await game.model()).hour };
  };
  check((await game.model()).hour === 0, "starts at midnight");
  const one = await press("t");
  check(one.hour === 1, `T steps the hour (hour ${one.hour})`);
  const noon = await press("n");
  check(noon.hour === 12, `N jumps to noon (hour ${noon.hour})`);
  const day = await game.capture();
  const midnight = await press("n");
  check(midnight.hour === 0, `N jumps back to midnight (hour ${midnight.hour})`);
  const night = await game.capture();
  check(!day.equals(night), "noon and midnight draw differently");

  await game.post("/rewind", { frame: noon.frame });
  let hour = (await game.model()).hour;
  check(hour === 12, `rewinding to frame ${noon.frame} restores noon (hour ${hour})`);
  // The rewound frame is a few frames before `day`, whose pose differs by the
  // character controller's sub-millimetre settle: allow 1–2 levels.
  const diff = maxDiff(await game.capture(), day);
  check(diff <= 2, `the rewound frame draws as noon did (max diff ${diff}; night differs by ${maxDiff(night, day)})`);
  await game.post("/rewind", { frame: one.frame });
  hour = (await game.model()).hour;
  check(hour === 1, `rewinding to frame ${one.frame} restores 1:00 (hour ${hour})`);
  await press("n"); // 1:00 is night: on to noon,
  await press("n"); // then midnight
  check((await game.model()).hour === 0, "back at midnight");
}

// In a headset, the left controller's X steps the hour and Y jumps midnight ⇄
// noon, once per press however long it's held.
async function xrHours(game) {
  const pad = (buttons) => game.post("/input", { type: "xr", head: { position: [0, 0, 0] }, left: { active: true, ...buttons }, right: { active: true } });
  const press = async (buttons) => {
    await pad(buttons);
    await game.step(10);
    await pad({});
    await game.step(1);
    return (await game.model()).hour;
  };
  check((await game.model()).hour === 0, "XR: starts at midnight");
  let hour = await press({ primary_pressed: true });
  check(hour === 1, `XR: holding X steps one hour (hour ${hour})`);
  hour = await press({ secondary_pressed: true });
  check(hour === 12, `XR: Y jumps to noon (hour ${hour})`);
  hour = await press({ secondary_pressed: true });
  check(hour === 0, `XR: Y jumps back to midnight (hour ${hour})`);
  await game.post("/input", { type: "xr_clear" });
  await game.step(1);
}

// Daylight reaches only what an opening connects to: office 103's floor
// brightens by day; the windowless corridor stays fixture-lit.
async function daylight(game) {
  const at = async (view) => {
    await game.view(view);
    const night = await game.capture();
    await game.hold("n", 1);
    const day = await game.capture();
    await game.hold("n", 1);
    return { night, day };
  };
  // View 4 looks at the window; the floor band below it excludes the sky.
  const room = await at(4);
  const floor = [0, 0.5, 1, 0.75];
  const [roomNight, roomDay] = [meanLuma(room.night, floor), meanLuma(room.day, floor)];
  check(roomDay > 1.2 * roomNight, `office 103's floor brightens by day (luma ${roomNight.toFixed(1)} → ${roomDay.toFixed(1)})`);
  // View 2 looks down the corridor, away from the lobby.
  const corridor = await at(2);
  const whole = [0, 0, 1, 1];
  const [corridorNight, corridorDay] = [meanLuma(corridor.night, whole), meanLuma(corridor.day, whole)];
  check(Math.abs(corridorDay / corridorNight - 1) < 0.02, `the corridor stays fixture-lit (luma ${corridorNight.toFixed(1)} → ${corridorDay.toFixed(1)})`);
}

// Views 1–8 at midnight, then at noon.
async function viewCaptures(game, culling) {
  const shots = [];
  for (const n of [1, 2, 3, 4, 5, 6, 7, 8, "n", 1, 2, 3, 4, 5, 6, 7, 8, "n"]) {
    if (n === "n") {
      await game.hold("n", 1);
      continue;
    }
    await game.view(n);
    const s = await game.state();
    if (culling && n > 1) check(s.cells.hidden > 0, `view ${n}'s portal walk hides subtrees (cells ${JSON.stringify(s.cells)})`);
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
  await stairs(game);
  await hours(game);
  await xrHours(game);
  await daylight(game);
  const culled = await viewCaptures(game, true);

  const unculled = launch(8192, { FUNCTOR_CULL: "0" });
  games.push(unculled);
  await unculled.ready();
  const all = await viewCaptures(unculled, false);
  // A regression check for now: the renderer draws `Model.static` data models
  // unculled, so the interior matches by construction until that engine fix.
  culled.forEach((shot, i) => check(shot.equals(all[i]), `${i < 8 ? "night" : "noon"} view ${(i % 8) + 1}: culling on and off capture the same frame`));

  const scripted = (script) => ["--input-script", join(DIR, script), "--capture-at-frame", "30"];
  const goldens = {
    "liminal-office-entrance": ["--fixed-time", "1", "--capture-time", "1.0"],
    "liminal-office-corridor": scripted("views.script"),
    "liminal-office-window": scripted("window.script"),
    "liminal-office-entrance-day": scripted("noon-entrance.script"),
    "liminal-office-corridor-day": scripted("noon-corridor.script"),
    "liminal-office-window-day": scripted("noon-window.script"),
    "liminal-office-stairwell": scripted("stairwell.script"),
    "liminal-office-stairwell-day": scripted("noon-stairwell.script"),
    "liminal-office-upstairs": scripted("upstairs.script"),
    "liminal-office-upstairs-day": scripted("noon-upstairs.script"),
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
