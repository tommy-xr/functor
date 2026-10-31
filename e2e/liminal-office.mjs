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
// The block (B): four turned copies of the office around a crossroads.
//   8. a tour down the streets into every copy's lobby and corridor, a push
//      into each copy's facade, and each copy's window from the street and
//      from inside, night and noon. At every tick the eye is in a cell of
//      every copy whose interior draws (the model's `uncovered` stays 0), so
//      no portal walk falls back to drawing everything;
//   9. static draw calls at street, between-buildings and inside views:
//      pushed variants of game.fun draw each cell model 0×, 1× or once per
//      material group, and `/state.cull`'s tested − culled differences count
//      the static models and their group draws that actually submitted;
//  10. shared uploads (`--json` frame stats): after copy 0's pass, the same
//      pass through copies 1–3 misses no GPU cache, uploads nothing and adds
//      no live GL object; the noon switch misses as often with four copies
//      active as with the office alone.
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
const WEST_YARD = 22; // the west strip of the collar
// layout.fun's storey and flight, and game.fun's capsule centre above the feet.
const STOREY = 3.3;
const FLIGHT = { x0: 13.0, tread: 0.3, risers: 18 };
const CENTRE = 0.92;
// The ramp's height at x: building.fun's `rampY`.
const rampY = (x) =>
  Math.min(STOREY, Math.max(0, ((x - (FLIGHT.x0 - FLIGHT.tread)) * (STOREY / FLIGHT.risers)) / FLIGHT.tread));
const WALK = 2.2 / 60; // metres per step

function launch(port, env = {}, flags = []) {
  const proc = spawn(BIN, [...flags, "-d", DIR, "run", "native", "--hidden", "--debug-port", String(port)], {
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
    base,
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

// ---- The block -----------------------------------------------------------------

// block.fun's placement: copy k turns 90k° about the crossroads (-12, -12).
const CROSS = -12;
const turnXZ = (deg, x, z) => {
  const a = (deg * Math.PI) / 180;
  return [x * Math.cos(a) + z * Math.sin(a), z * Math.cos(a) - x * Math.sin(a)];
};
const copyAt = (k) => {
  const [x, z] = turnXZ(90 * k, CROSS, CROSS);
  return { k, yaw: 90 * k, x: CROSS - x, z: CROSS - z };
};
const COPIES = [0, 1, 2, 3].map(copyAt);
const toWorld = (c, x, z) => {
  const [a, b] = turnXZ(c.yaw, x, z);
  return [a + c.x, b + c.z];
};
const toLocal = (c, x, z) => turnXZ(-c.yaw, x - c.x, z - c.z);
// A heading in copy c's space (0 is its +z, into the entrance), in the world.
const headingOf = (c, yaw) => yaw + (c.yaw * Math.PI) / 180;
const wrap = (a) => Math.atan2(Math.sin(a), Math.cos(a));

// Every sample of the block tour: whether the eye was in a cell of the
// nearest copy.
const samples = { n: 0, outside: [] };
const sample = (m) => {
  samples.n++;
  if (m.cell < 0) samples.outside.push(`(${m.at.x.toFixed(1)}, ${m.at.z.toFixed(1)}) copy ${m.copy}`);
};

async function faceYaw(game, yaw) {
  await game.turn(wrap(yaw - (await game.model()).yaw));
}

// Walk straight to world (x, z), sampling every `chunk` steps; stops early
// when blocked.
async function walkToPoint(game, [x, z], chunk = 20) {
  let m = await game.model();
  for (let leg = 0; leg < 400; leg++) {
    const [dx, dz] = [x - m.at.x, z - m.at.z];
    const left = Math.hypot(dx, dz);
    if (left < 0.05) break;
    await faceYaw(game, Math.atan2(dx, dz));
    await game.key("w", true);
    await game.step(Math.max(1, Math.min(chunk, Math.floor(left / WALK))));
    await game.key("w", false);
    await game.step(1);
    const before = m.at;
    m = await game.model();
    sample(m);
    if (Math.hypot(m.at.x - before.x, m.at.z - before.z) < 0.001) break; // blocked
  }
  return m;
}

// The streets between the copies: from copy k's approach (6 m in front of its
// entrance) to copy k+1's, round the crossroads.
const approach = (c) => toWorld(c, 10, -6);
const legs = [
  [[-6, -6]],
  [[-6, -18]],
  [[-18, -18]],
];

// Enter copy c through its entrance into the lobby and the corridor, and back
// out; push into its facade beside the entrance.
async function enterCopy(game, c) {
  await walkToPoint(game, approach(c));
  let m = await walkToPoint(game, toWorld(c, 10, 3));
  check(m.copy === c.k && m.cell === LOBBY, `copy ${c.k}: walked into its lobby (copy ${m.copy}, cell ${m.cell})`);
  m = await walkToPoint(game, toWorld(c, 10, 9));
  check(m.copy === c.k && m.cell === CORRIDOR, `copy ${c.k}: up its corridor (copy ${m.copy}, cell ${m.cell}, active ${JSON.stringify(m.active)})`);
  await walkToPoint(game, approach(c));
  // The lobby's facade, left of the entrance: blocked at its outer face
  // (local z = -0.15) less the capsule's radius.
  await walkToPoint(game, toWorld(c, 6.5, -3));
  await faceYaw(game, headingOf(c, 0));
  await game.hold("w", 150);
  m = await game.model();
  const [lx, lz] = toLocal(c, m.at.x, m.at.z);
  check(lz < -0.4 && lz > -0.6 && m.copy === c.k && m.cell === APRON, `copy ${c.k}: outside → in push blocked at the wall (local x ${lx.toFixed(2)} z ${lz.toFixed(3)}, cell ${m.cell})`);
  await walkToPoint(game, approach(c));
}

// Mean luma of the middle of a capture: the window seen from the street.
const WINDOW_RECT = [0.4, 0.4, 0.6, 0.55];
const FLOOR = [0, 0.5, 1, 0.75];

// Copy c's office 103 window: from the street (the lit room shows through
// it; the sill stops a walk in), then from inside at night and noon
// (`hours`) or just at night.
async function inspectWindow(game, c, hours = true) {
  await walkToPoint(game, toWorld(c, -4, -6));
  await walkToPoint(game, toWorld(c, -4, 9.3));
  await faceYaw(game, headingOf(c, Math.PI / 2));
  await game.step(2);
  const outside = await game.capture();
  await game.hold("w", 120);
  const m = await game.model();
  const [lx] = toLocal(c, m.at.x, m.at.z);
  check(lx < -0.3 && m.cell === WEST_YARD, `copy ${c.k}: the window's sill stops a walk in (local x ${lx.toFixed(2)}, cell ${m.cell})`);
  await game.view(4);
  const night = await game.capture();
  let noon = night;
  if (hours) {
    await game.hold("n", 1);
    noon = await game.capture();
    await game.hold("n", 1);
  }
  await game.view(1); // back out onto this copy's apron
  return { outside: meanLuma(outside, WINDOW_RECT), night: meanLuma(night, FLOOR), noon: meanLuma(noon, FLOOR) };
}

async function blockTour(game) {
  await game.view(1);
  await game.hold("b", 1);
  let m = await game.model();
  check(m.block === true && m.uncovered === 0, `B turns the office into the block (block ${m.block})`);
  const windows = [];
  for (const c of COPIES) {
    await enterCopy(game, c);
    windows.push(await inspectWindow(game, c));
    for (const p of legs[c.k] ?? []) await walkToPoint(game, p);
  }
  // Across the crossroads: every copy's interior draws.
  m = await walkToPoint(game, [CROSS, CROSS]);
  check(m.active.every((a) => a), `at the crossroads every copy is active (${JSON.stringify(m.active)})`);
  check(samples.n > 300 && samples.outside.length === 0, `the eye was in a cell of the nearest copy at all ${samples.n} samples${samples.outside.length ? `; not at ${samples.outside.slice(0, 3).join(", ")}` : ""}`);
  m = await game.model();
  check(m.uncovered === 0, `no active copy's cells ever lost the eye (uncovered ${m.uncovered} over the tour)`);

  // One overcast bake at every yaw: office 103 brightens by day in each
  // copy, about equally, and its lit room shows through from the street.
  for (const w of windows) {
    check(w.noon > 1.2 * w.night, `copy ${windows.indexOf(w)}: office 103's floor brightens by day (luma ${w.night.toFixed(1)} → ${w.noon.toFixed(1)})`);
    check(Math.abs(w.noon / windows[0].noon - 1) < 0.03 && Math.abs(w.night / windows[0].night - 1) < 0.03,
      `copy ${windows.indexOf(w)}: lit like copy 0 (night ${w.night.toFixed(1)} vs ${windows[0].night.toFixed(1)}, noon ${w.noon.toFixed(1)} vs ${windows[0].noon.toFixed(1)})`);
    check(w.outside > 40, `copy ${windows.indexOf(w)}: the lit office shows through its window from the street (luma ${w.outside.toFixed(1)})`);
  }
  return windows;
}

// Static draws from `/state.cull` (tested − culled): a static model that
// isn't portal-hidden is tested once; one inside the frustum submits a draw
// per material group. Variants of game.fun's `roomOf` draw each cell model
// 0×, 1× or once per group, so the differences count the models and the
// group draws that submitted (the street side's tests cancel).
const ROOM_OF = "Scene.modelOf(p.model) |> Scene.inCells([p.cell])";
const GROUPS = "List.length(Building.spansOf(0.0, List.nth(p.cell, Building.cellParts) |> Option.defaultValue([])))";
const times = (n) => `Scene.group(List.range(${n}) |> List.map((k) => Scene.modelOf(p.model))) |> Scene.inCells([p.cell])`;

async function staticDraws(game, label) {
  const project = await (await fetch(`${game.base}/project`)).json();
  const entry = project[0][1];
  if (!entry.includes(ROOM_OF)) throw new Error("game.fun's roomOf changed: update the e2e's draw-count variants");
  const tested = async (variant) => {
    const body = project.map(([path, src], i) => [path, i === 0 ? src.replace(ROOM_OF, variant) : src]);
    await game.post("/reload-project", body);
    await game.step(2);
    const s = await game.state();
    return { live: s.cull.tested - s.cull.culled, hidden: s.cells.hidden, walks: s.cells.walks };
  };
  const none = await tested(times("0.0"));
  const groups = await tested(times(GROUPS));
  const once = await tested(ROOM_OF);
  const m = await game.model();
  return { label, models: once.live - none.live, draws: groups.live - none.live, hidden: once.hidden, walks: once.walks, active: m.active.filter((a) => a).length };
}

async function drawCounts(game) {
  const rows = [];
  // The office alone, for #882's table: the entrance and corridor.
  await game.hold("b", 1);
  await game.view(1);
  rows.push(await staticDraws(game, "office alone: entrance (view 1)"));
  await game.view(2);
  rows.push(await staticDraws(game, "office alone: corridor (view 2)"));
  await game.hold("b", 1);
  await game.view(2);
  rows.push(await staticDraws(game, "block: copy 0 corridor (view 2)"));
  await game.view(9);
  rows.push(await staticDraws(game, "street (view 9)"));
  await walkToPoint(game, [CROSS, CROSS]);
  await faceYaw(game, Math.atan2(22, 12));
  rows.push(await staticDraws(game, "crossroads, facing copy 0's entrance"));
  await game.view(1);
  await faceYaw(game, Math.PI);
  rows.push(await staticDraws(game, "between copies 0 and 1 (copy 0's apron, facing copy 1's window)"));
  await game.view(0);
  rows.push(await staticDraws(game, "inside copy 2: office 103 window (view 0)"));
  await game.view(2);
  rows.push(await staticDraws(game, "inside copy 2: corridor (view 2)"));
  for (const r of rows) {
    check(r.draws >= r.models && r.models >= 0, `${r.label}: ${r.models} static models, ${r.draws} static draws (${r.active} active, ${r.hidden} hidden, ${r.walks} walks)`);
  }
  check((await game.model()).uncovered === 0, "no fallback walks while measuring");
  return rows;
}

// The latest `--json` frame-stats window after `n` more steps.
async function statsAfter(game, n) {
  const count = () => game.log().split("\n").filter((l) => l.includes('"frame_stats"')).length;
  const seen = count();
  await game.step(n);
  const deadline = Date.now() + 30000;
  while (count() <= seen && Date.now() < deadline) await sleep(50);
  const lines = game.log().split("\n").filter((l) => l.includes('"frame_stats"'));
  return JSON.parse(lines[lines.length - 1]);
}

// The copies share one office's uploads. A cell mesh uploads the first time
// any copy draws it, so after copy 0's pass (its lobby, corridor, facade and
// window), the same pass through copies 1–3 misses the GPU caches never,
// uploads nothing, and grows no live GL object. Switching to noon misses as
// often with four copies active as with the office alone.
async function residency(game) {
  const windows = () => game.log().split("\n").filter((l) => l.includes('"frame_stats"')).map((l) => JSON.parse(l));
  const live = (s) => `${s.gpu_live_buffers} buffers, ${s.gpu_live_textures} textures, ${s.gpu_live_vaos} VAOs`;
  await game.view(1);
  await statsAfter(game, 300);
  const office = await statsAfter(game, 300);
  await game.hold("n", 1);
  const officeNoon = await statsAfter(game, 300);
  await game.hold("n", 1);

  await game.hold("b", 1);
  const pass = async (c) => {
    await enterCopy(game, c);
    await inspectWindow(game, c, false);
    for (const p of legs[c.k] ?? []) await walkToPoint(game, p);
  };
  await pass(COPIES[0]);
  await statsAfter(game, 300); // the window holding the end of copy 0's pass
  const primed = await statsAfter(game, 300);
  const from = windows().length;
  for (const c of COPIES.slice(1)) await pass(c);
  await statsAfter(game, 300);
  const later = windows().slice(from);
  const misses = later.reduce((n, w) => n + w.gpu_cache_misses, 0);
  const bytes = later.reduce((n, w) => n + w.gpu_bytes_per_frame * w.over_n_frames, 0);
  const after = later[later.length - 1];
  check(office.gpu_cache_misses === 0 && primed.gpu_cache_misses === 0 && primed.gpu_bytes_per_frame === 0,
    `standing still uploads nothing (office ${office.gpu_cache_misses} misses; block ${primed.gpu_cache_misses} misses, ${primed.gpu_bytes_per_frame} B/frame)`);
  check(later.length >= 3 && misses === 0 && bytes === 0, `copies 1–3's passes (${later.length} stats windows) miss ${misses} times and upload ${bytes} bytes after copy 0's`);
  check(after.gpu_live_buffers === primed.gpu_live_buffers && after.gpu_live_textures === primed.gpu_live_textures && after.gpu_live_vaos === primed.gpu_live_vaos,
    `live GL objects after all four copies equal those after copy 0's (${live(after)} vs ${live(primed)}; the office from its entrance: ${live(office)})`);

  await walkToPoint(game, [CROSS, CROSS]);
  check((await game.model()).active.every((a) => a), "residency: all four copies active at the crossroads");
  await statsAfter(game, 300);
  await game.hold("n", 1);
  const blockNoon = await statsAfter(game, 300);
  check(blockNoon.gpu_cache_misses === officeNoon.gpu_cache_misses,
    `the noon switch misses as often with four copies active as with one (${blockNoon.gpu_cache_misses} vs ${officeNoon.gpu_cache_misses}; ${blockNoon.gpu_bytes_per_frame} vs ${officeNoon.gpu_bytes_per_frame} B/frame)`);
  return { office: live(office), primed: live(primed), after: live(after), misses, bytes, officeNoon, blockNoon };
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
  await blockTour(game);
  const draws = await drawCounts(game);
  console.log(`static draws: ${JSON.stringify(draws, null, 1)}`);

  const unculled = launch(8192, { FUNCTOR_CULL: "0" });
  games.push(unculled);
  await unculled.ready();
  const all = await viewCaptures(unculled, false);
  // A regression check for now: the renderer draws `Model.static` data models
  // unculled, so the interior matches by construction until that engine fix.
  culled.forEach((shot, i) => check(shot.equals(all[i]), `${i < 8 ? "night" : "noon"} view ${(i % 8) + 1}: culling on and off capture the same frame`));

  const stats = launch(8193, {}, ["--json"]);
  games.push(stats);
  await stats.ready();
  await stats.turn(0);
  const residencyStats = await residency(stats);
  console.log(`residency: ${JSON.stringify(residencyStats)}`);

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
    "liminal-office-block-street": scripted("block-street.script"),
    "liminal-office-block-street-day": scripted("noon-block-street.script"),
    "liminal-office-block-window": scripted("block-window.script"),
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
