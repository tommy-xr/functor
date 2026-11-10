// vr-lab ammunition E2E (station 3): finite rounds and magazines, driven by
// injected XR samples, headlessly.
//
// Plays the ammo scenarios in `e2e/vr-lab-scenarios.mjs` against
// `examples/vr-lab` (`--headless`, pinned clock, one sample per fixed step)
// and checks `/state.model`:
//
//   1. Pistol (12+1): thirteen pulls at a target, thirteen shots and hits;
//      the fourteenth clicks dry (a click pulse, no shot, no hit), leaving 0+0.
//   2. Rifle, full-auto, the trigger held 200 steps: exactly 31 shots, then a
//      dry click; let out and pulled again, another.
//   3. Magazine: three shots; low over the bench the release does nothing
//      (no room to drop); higher, the thumbstick drops the magazine; it
//      lands on the bench with its 9 rounds, the left hand takes it, pouches
//      it and takes it back, still 9. With it out, the chambered round fires
//      once; then dry.
//   4. Every step of every run conserves the station's rounds: every
//      magazine's (free, held, slotted, in a well) and chamber's, plus the
//      shots fired, is what the station was reset with.
//   5. The magazine run replays in a fresh runtime to an identical model.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab-ammo.mjs
import { check, failures } from "./mcp-rpc.mjs";
import { fmt, len, play, record, session, some, sub, xyz } from "./xr-scenario.mjs";
import { MAG_DROP, RESET_ROUNDS, RIFLE_ROUNDS, SLOT, SPARES, TABLE, conserves, project, readout, roundsIn, scenarios, unspent } from "./vr-lab-scenarios.mjs";

const PORT = 8296;
// The armory bench is the grab table's.
const BENCH_TOP = TABLE.top;
const run = (name, body) => session(name, project, PORT, body);

const mags = (m) => m.props.filter((p) => p.kind.$ctor === "Props.Mag");
const pulsed = (m, event) => JSON.stringify(m.pulses).includes(`"Pulses.${event}"`);

await run("pistol: emptied, then dry", async (rt) => {
  const { steps, marks } = await record(rt, scenarios["ammo-pistol"]);
  check(unspent(steps[0].state.model) === RESET_ROUNDS, `reset with ${RESET_ROUNDS} rounds: 13 + 31 + the spares`);
  const last = marks.lastShot.model;
  check(last.gun.fired === 13 && last.score === 13, `thirteen pulls: 13 shots, 13 hits (fired ${last.gun.fired}, score ${last.score})`);
  check(readout(last.gun.ammo) === "0+0", `the readout's numbers: ${readout(last.gun.ammo)}`);
  const dry = marks.dry.model;
  check(dry.gun.fired === 13 && dry.gun.dry === 1 && dry.score === 13, `the fourteenth: dry (fired ${dry.gun.fired}, dry ${dry.gun.dry}, score ${dry.score})`);
  check(pulsed(dry, "Click") && !pulsed(dry, "Fire"), `a click pulse, no fire (${JSON.stringify(dry.pulses)})`);
  check(JSON.stringify(some(dry.gun.last)) === JSON.stringify(some(last.gun.last)), "no new shot: the last is the thirteenth");
  conserves("pistol", steps);
});

await run("rifle: full-auto to empty", async (rt) => {
  const { steps, marks } = await record(rt, scenarios["ammo-rifle"]);
  const held = marks.held.model;
  check(held.rifle.mode.$ctor === "Gun.Auto", `full-auto (${held.rifle.mode.$ctor})`);
  check(held.rifle.fired === RIFLE_ROUNDS && held.rifle.dry === 1, `held 200 steps: ${held.rifle.fired} shots, then ${held.rifle.dry} dry click`);
  // The shots land on full-auto's cycle until the last; nothing after it.
  const shots = steps.filter((s, i) => i > 0 && s.state.model.rifle.fired > steps[i - 1].state.model.rifle.fired).map((s) => s.state.frame);
  const gaps = new Set(shots.slice(1).map((f, i) => f - shots[i]));
  check(shots.length === RIFLE_ROUNDS && gaps.size === 1 && gaps.has(6), `one shot every 6 steps (${[...gaps]}), ${shots.length} of them`);
  const dryAt = steps.find((s) => s.state.model.rifle.dry === 1)?.state.frame;
  check(dryAt === shots.at(-1) + 6, `the click on the next cycle (frame ${dryAt}, the last shot ${shots.at(-1)})`);
  check(readout(held.rifle.ammo) === "0+0", `the readout's numbers: ${readout(held.rifle.ammo)}`);
  const again = marks.dryAgain.model;
  check(again.rifle.dry === 2 && again.rifle.fired === RIFLE_ROUNDS, `let out and pulled: dry again (${again.rifle.dry})`);
  conserves("rifle", steps);
});

let magEnd;
await run("magazine: drop, pouch, retrieve", async (rt) => {
  const { steps, marks, index } = await record(rt, scenarios["ammo-mag"]);
  check(marks.fired3.model.gun.fired === 3, `three shots (${marks.fired3.model.gun.fired})`);
  const low = marks.lowPush.model;
  check(some(low.gun.ammo.mag) && mags(low).length === mags(marks.fired3.model).length, "low over the bench, no room to drop: the magazine stays in");
  const ejected = marks.ejected.model;
  const [mag] = mags(ejected).filter((p) => p.kind.args[1] > SPARES.pistol + SPARES.rifle);
  check(mag && mag.kind.args[0].$ctor === "Ammo.PistolMag" && roundsIn(mag.mag) === 9, `a pistol magazine drops out with 9 (${JSON.stringify(mag?.kind)}, ${roundsIn(mag?.mag)})`);
  check(!some(ejected.gun.ammo.mag) && ejected.gun.ammo.chambered, `the pistol: no magazine, one chambered (${readout(ejected.gun.ammo)})`);
  check(len(sub(xyz(mag.at.position), MAG_DROP)) < 0.001, `below the well, clear of the pistol (${fmt(xyz(mag.at.position))})`);
  const id = JSON.stringify(mag.kind);
  const magIn = (m) => m.props.find((p) => JSON.stringify(p.kind) === id);
  const landed = magIn(marks.landed.model);
  const y = landed.pose.position.y;
  check(!some(landed.held) && y > BENCH_TOP && y < BENCH_TOP + 0.06, `it falls onto the bench (centre at y ${y.toFixed(4)})`);
  check(some(magIn(marks.tookMag.model).held)?.side.$ctor === "Hold.Left", "the left hand takes it");
  check(some(magIn(marks.pouched.model).slot) === SLOT.pouch.id, `opened at the belt: in the pouch (${JSON.stringify(magIn(marks.pouched.model).slot)})`);
  const back = magIn(marks.retrieved.model);
  check(some(back.held)?.side.$ctor === "Hold.Left" && !some(back.slot), "squeezed there: back in the hand");
  const nine = steps.slice(index("ejected")).every((s) => roundsIn(magIn(s.state.model)?.mag) === 9);
  check(nine, "9 rounds in it at every step after the drop");
  check(marks.lastRound.model.gun.fired === 4, `the chambered round fires with the magazine out (fired ${marks.lastRound.model.gun.fired})`);
  const dry = marks.dry.model;
  check(dry.gun.fired === 4 && dry.gun.dry === 1 && readout(dry.gun.ammo) === "--+0", `then dry (${readout(dry.gun.ammo)}, dry ${dry.gun.dry})`);
  conserves("magazine", steps);
  magEnd = JSON.stringify((await rt.get("/state")).model);
});

await run("replay", async (rt) => {
  await play(rt, scenarios["ammo-mag"]);
  check(magEnd !== undefined && magEnd === JSON.stringify((await rt.get("/state")).model), "the same run in a fresh runtime ends on an identical model");
});

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab ammo e2e: all checks passed");
