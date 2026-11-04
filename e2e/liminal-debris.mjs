// liminal-office's dynamic debris E2E: through the real physics world,
// headless, with the clock paused between `/time` steps.
//
//   1. an untouched office has no debris bodies (it stays in the instanced
//      cube) and its world holds only the ground, the building and the player;
//   2. F, from inside office 103 (view 4), breaks its window: the litter
//      outside it gets bodies, the draft carries every piece in through the
//      opening and onto the floor, and it settles and sleeps;
//   3. E, in the lobby, blasts: the lobby's debris gets bodies the same
//      frame, several pieces are thrown, and all of it settles and sleeps;
//   4. the same script run twice leaves every body at the same pose.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/liminal-debris.mjs
import { spawn } from "node:child_process";
import { join } from "node:path";
import { BIN, ROOT, check, failures, portIsBound, sleep } from "./mcp-rpc.mjs";

const DIR = join(ROOT, "examples/liminal-office");
const STEP = 1 / 60;
// officeLayout.fun: office 103's window is on the west facade (x = 0), its
// floor at y = 0; the lobby's debris is in cell 1.
const WALL_X = 0;
const LOBBY = 1;

async function launch(port) {
  // Another game on the port would answer instead of ours.
  if (await portIsBound(port)) throw new Error(`port ${port} is taken: stop what's listening there`);
  const proc = spawn(BIN, ["-d", DIR, "run", "native", "--headless", "--debug-port", String(port)], {
    cwd: ROOT,
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
    model: async () => (await state()).model,
    // The physics world's body count after the last frame.
    bodies: async () => (await (await fetch(`${base}/frames?last=1`)).json()).frames[0].bodies,
    async ready() {
      const deadline = Date.now() + 90000;
      while (Date.now() < deadline) {
        try {
          if ((await state()).frame > 0) {
            // Pause, then back to the first step: however many frames ran
            // before the pause, every run starts from the same world.
            await post("/time", { type: "set", tts: 0 });
            return post("/rewind", { frame: 1 });
          }
        } catch {
          // not listening yet
        }
        await sleep(100);
      }
      throw new Error("runtime never reported a frame");
    },
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
    // Press and release: F and E act on the release.
    async tap(key) {
      await game.key(key, true);
      await game.step(1);
      await game.key(key, false);
      await game.step(1);
    },
  };
  return game;
}

const poseOf = (b) => (b.pose.$ctor === "Option.Some" ? b.pose.args[0] : null);
const bodiesOf = (m) => m.debris.flatMap((g) => g.bodies);

// The untouched office, a blast in its lobby, then a window; returns the
// final model.
async function script(game, log) {
  await game.ready();
  await game.step(120);
  let m = await game.model();
  const baseline = await game.bodies();
  log(m.debris.length === 0 && baseline === 3, `untouched: no debris bodies (${baseline} in the world)`);

  // 2. The lobby: walk in from the apron, blast ahead.
  await game.tap("1");
  await game.step(3);
  await game.key("w", true);
  await game.step(330);
  await game.key("w", false);
  await game.step(2);
  m = await game.model();
  log(m.debris.length === 0, "still none after walking in");
  await game.tap("e");
  m = await game.model();
  const rest = bodiesOf(m).filter((b) => b.cell >= 0);
  log(rest.length > 0 && (await game.bodies()) === baseline + rest.length, `E gives the office's ${rest.length} pieces of debris bodies`);
  await game.step(60);
  m = await game.model();
  const lobby = bodiesOf(m).filter((b) => b.cell === LOBBY);
  const thrown = lobby.filter((b) => {
    const p = poseOf(b);
    return p && Math.hypot(p.x - b.x, p.z - b.z) > 0.5;
  });
  log(lobby.length > 0 && thrown.length >= 3, `the blast throws the lobby's debris: ${thrown.length} of ${lobby.length} > 0.5 m`);
  await game.step(600);
  m = await game.model();
  const loose = bodiesOf(m).filter(poseOf);
  log(loose.length >= thrown.length && loose.every((b) => poseOf(b).still), `${loose.length} loose bodies, all asleep`);

  // 3. Office 103's window, from inside.
  await game.tap("4");
  await game.step(3);
  await game.tap("f");
  m = await game.model();
  log(m.broken.length === 1 && m.broken[0].copy === "office-0" && m.broken[0].window === 0, "F breaks office 103's window");
  const litter = () => bodiesOf(m).filter((b) => b.local.startsWith("litter"));
  log(litter().length === 6 && litter().every((b) => b.x < WALL_X), "six pieces of litter, outside it");
  log((await game.bodies()) === baseline + bodiesOf(m).length, "each a body");
  await game.step(240);
  m = await game.model();
  const inside = litter().map(poseOf);
  log(inside.every((p) => p && p.x > WALL_X + 1.5 && p.y > -0.05 && p.y < 0.5),
    `the draft carries it in, onto the floor: x ${inside.map((p) => p?.x.toFixed(2)).join(", ")}`);
  await game.step(360);
  m = await game.model();
  log(litter().every((b) => poseOf(b)?.still), "the litter settles and sleeps");
  return m;
}

const PORT = Number(process.env.LIMINAL_DEBRIS_PORT ?? 8267);
const games = [];
try {
  const first = await script((games[0] = await launch(PORT)), check);
  games[0].proc.kill("SIGKILL");
  await sleep(500);
  const second = await script((games[1] = await launch(PORT)), () => {});
  const poses = (m) => JSON.stringify(bodiesOf(m).map((b) => [b.tag, b.pose]));
  check(poses(first) === poses(second), "the same script twice leaves every body at the same pose");
} catch (e) {
  failures.push(String(e));
  console.error(e);
} finally {
  for (const g of games) g?.proc.kill("SIGKILL");
}

if (failures.length) {
  console.error(`\n${failures.length} failure(s)\n--- runtime log ---\n${games.at(-1)?.log().slice(-3000)}`);
  process.exit(1);
}
console.log("\nliminal-debris e2e: all checks passed");
