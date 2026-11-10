// vr-lab shotgun E2E (station 3): the pump shotgun, its tube and its pellets,
// driven by injected XR samples, headlessly.
//
// Plays the shotgun scenarios in `e2e/vr-lab-scenarios.mjs` against
// `examples/vr-lab` (`--headless`, pinned clock, one sample per fixed step)
// and checks `/state.model`:
//
//   1. The loop: out of its rack (0+0); six shells out of the box, each
//      latched at the loading port, guided up it and into the tube (6+0); the
//      pump worked (5+1); a seventh in (6+1); an eighth refused, the tube full
//      (it never latches) and set down; aimed at the middle target, seven
//      shots, each pumped (its hull thrown out, the next fed); the next pull
//      dry (0+0). One shot a stroke: the hull stays in the chamber until the
//      pump goes back.
//   2. Every shot's eight pellets land on the middle target where the pattern
//      puts them: one along the bore, seven on a 1.2° cone round it, the ring
//      turned by the golden angle a shot (within 1 mm).
//   3. A live shell pumped out flies, comes to rest, is picked up where it
//      lies, turned to lie along the bore and loaded again.
//   4. Every step of every run conserves the station's rounds (the shell box,
//      the tube, chambers, loose shells, shots, and those taken away).
//   5. The loop replays in a fresh runtime to an identical model.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab-shotgun.mjs
import { check, failures } from "./mcp-rpc.mjs";
import { len, play, qrot, quat, record, session, some, sub, xyz } from "./xr-scenario.mjs";
import { SHELL_BOX, SHOTGUN, TARGETS, conserves, hullsOf, project, readout, scenarios, shellsOf, tubeOf } from "./vr-lab-scenarios.mjs";

const PORT = 8295;
const run = (name, body) => session(name, project, PORT, body);

const shotgun = (m) => m.props.find((p) => p.kind.$ctor === "Props.Shotgun");
const latched = (m) => m.props.some((p) => some(p.latch) !== undefined);
const lead = (n) => n.toFixed(1);

// Where shot `n` (0 first)'s pellets strike the target face (rig z = -5.5)
// from the muzzle of the shotgun accepted at `pose`: along the bore, and
// round it on the cone.
function pattern(pose, from, n) {
  const q = quat(pose.orientation);
  return Array.from({ length: SHOTGUN.pellets }, (_, i) => {
    const a = n * SHOTGUN.turn + (2 * Math.PI * (i - 1)) / (SHOTGUN.pellets - 1);
    const local = i === 0 ? [0, 0, -1] : [Math.sin(SHOTGUN.spread) * Math.cos(a), Math.sin(SHOTGUN.spread) * Math.sin(a), -Math.cos(SHOTGUN.spread)];
    const d = qrot(q, local);
    const t = (TARGETS[1][2] - from[2]) / d[2];
    return from.map((v, k) => v + t * d[k]);
  });
}

let loopEnd;
await run("the loop: load, pump, seven shots, dry", async (rt) => {
  const { steps, marks, index } = await record(rt, scenarios["shotgun-loop"]);
  const took = marks.up.model;
  check(some(shotgun(took).held)?.side.$ctor === "Hold.Right" && readout(took.shotgun.ammo) === "0+0", `out of its rack, empty (${readout(took.shotgun.ammo)})`);
  // Each shell: latched below the receiver, then into the tube.
  for (let i = 1; i <= SHOTGUN.tube; i++) {
    const m = marks[`loaded${i}`].model;
    check(tubeOf(m) === i && m.shellBox === SHELL_BOX - i && shellsOf(m).length === 0, `shell ${i}: into the tube (${readout(m.shotgun.ammo)}, ${m.shellBox} in the box)`);
  }
  const latchSteps = steps.slice(index("up"), index("loaded6") + 1).filter((s) => latched(s.state.model)).length;
  check(latchSteps >= SHOTGUN.tube * 5, `each latched at the port and guided up it (${latchSteps} steps latched)`);
  check(readout(marks.pumped.model.shotgun.ammo) === "5+1", `pumped: the first shell chambered (${readout(marks.pumped.model.shotgun.ammo)})`);
  check(marks.pumpedBack.model.shotgun.slide.back && !marks.pumpedBack.model.shotgun.ammo.chambered, "back: nothing chambered yet");
  check(readout(marks.loaded7.model.shotgun.ammo) === "6+1", `a seventh in (${readout(marks.loaded7.model.shotgun.ammo)})`);
  const refusal = steps.slice(index("loaded7") + 1, index("setDown") + 1);
  check(refusal.every((s) => !latched(s.state.model)) && tubeOf(marks.refused.model) === SHOTGUN.tube && shellsOf(marks.refused.model).length === 1,
    "the eighth, at the mouth with the tube full: never latches, still in the hand");
  // Seven shots, each pumped.
  for (let i = 1; i <= SHOTGUN.tube + 1; i++) {
    const shot = marks[`fired${i}`].model;
    const back = marks[`pumped${i}Back`].model;
    const home = marks[`pumped${i}`].model;
    const left = SHOTGUN.tube + 1 - i;
    check(shot.shotgun.fired === i && shot.shotgun.ammo.hull && !shot.shotgun.ammo.chambered,
      `shot ${i}: fired, its hull in the chamber (${readout(shot.shotgun.ammo)})`);
    check(!back.shotgun.ammo.hull && home.shotgun.ammo.chambered === left > 0 && Math.min(left, 1) + tubeOf(home) === left,
      `pumped: the hull out, ${left > 0 ? "the next fed" : "nothing left to feed"} (${readout(home.shotgun.ammo)})`);
  }
  const between = steps.slice(index("fired1") + 1, index("pumped1Back") + 1);
  check(between.every((s) => s.state.model.shotgun.fired === 1), "no second shot before the pump (one a stroke)");
  const dry = marks.after.model;
  check(dry.shotgun.fired === SHOTGUN.tube + 1 && dry.shotgun.dry === 1 && readout(dry.shotgun.ammo) === "0+0", `the next pull dry (${readout(dry.shotgun.ammo)}, ${dry.shotgun.dry} dry)`);
  check(dry.score === SHOTGUN.tube + 1, `seven shots, seven on the target (score ${dry.score})`);
  // The cap: the set-down shell and the newest hulls lie about; hulls go first.
  check(hullsOf(dry).length === 5 && shellsOf(dry).length === 1 && dry.lost === 0, `litter: ${hullsOf(dry).length} hulls and the set-down shell (lost ${dry.lost})`);
  const rested = marks.rested.model;
  const lying = hullsOf(rested).filter((p) => p.lying).length;
  check(lying >= 4, `two seconds on, the hulls that came to rest on the level lie bodiless (${lying} of ${hullsOf(rested).length})`);
  conserves("loop", steps);

  // The pellets.
  let worst = 0;
  let onTarget = true;
  for (let i = 1; i <= SHOTGUN.tube + 1; i++) {
    const m = marks[`fired${i}`].model;
    const pose = some(shotgun(m).held).accepted;
    const last = some(m.shotgun.last);
    const want = pattern(pose, xyz(last.from), i - 1);
    const got = m.shotgun.hits.slice(-SHOTGUN.pellets);
    onTarget &&= got.length === SHOTGUN.pellets && got.every((h) => h.tag === "target-1");
    worst = Math.max(worst, ...got.map((h, k) => len(sub(xyz(h.point), want[k]))));
  }
  check(onTarget && worst < 0.001, `every shot: ${SHOTGUN.pellets} pellets on the middle target, at most ${(worst * 1000).toFixed(3)} mm off the pattern`);
  const [a, b] = [marks.fired1.model, marks.fired2.model].map((m) => m.shotgun.hits.slice(-SHOTGUN.pellets).map((h) => xyz(h.point)));
  const spread = Math.max(...a.slice(1).map((p) => len(sub(p, a[0]))));
  const turned = Math.min(...a.slice(1).map((p, k) => len(sub(p, b[k + 1]))));
  check(spread > 0.08 && spread < 0.12 && turned > 0.02, `the ring: ${lead(spread * 100)} cm round the centre pellet on the target; turned from shot to shot (the nearest pair ${lead(turned * 100)} cm apart)`);
  loopEnd = JSON.stringify((await rt.get("/state")).model);
});

await run("replay", async (rt) => {
  await play(rt, scenarios["shotgun-loop"]);
  check(loopEnd !== undefined && loopEnd === JSON.stringify((await rt.get("/state")).model), "the loop in a fresh runtime ends on an identical model");
});

await run("a live shell pumped out, and loaded again", async (rt) => {
  const { steps, marks, index } = await record(rt, scenarios["shotgun-eject"]);
  check(readout(marks.pumped.model.shotgun.ammo) === "1+1", `two in, pumped (${readout(marks.pumped.model.shotgun.ammo)})`);
  const thrown = marks.thrown.model;
  check(readout(thrown.shotgun.ammo) === "0+1" && shellsOf(thrown).length === 1 && hullsOf(thrown).length === 0,
    `pumped again: the chambered shell thrown out live, the next fed (${readout(thrown.shotgun.ammo)})`);
  const flight = steps.slice(index("thrown"), index("landed")).map((s) => shellsOf(s.state.model)[0]).filter(Boolean);
  const fastest = Math.max(...flight.map((p) => len(xyz(p.velocity))));
  const rest = shellsOf(marks.landed.model)[0];
  check(fastest > 1 && rest.lying, `it flies out (up to ${fastest.toFixed(2)} m/s) and lands, lying at y ${rest.pose.position.y.toFixed(3)}`);
  check(some(shellsOf(marks.pickedUp.model)[0].held)?.side.$ctor === "Hold.Left", "picked up where it lies");
  const reloaded = marks.reloaded.model;
  const lay = xyz(rest.pose.position);
  check(readout(reloaded.shotgun.ammo) === "1+1" && shellsOf(reloaded).length === 0, `turned along the bore and loaded again (${readout(reloaded.shotgun.ammo)}; it lay at [${lay.map((v) => v.toFixed(2))}])`);
  conserves("eject", steps);
});

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab shotgun e2e: all checks passed");
