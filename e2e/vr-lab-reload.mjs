// vr-lab reloading E2E (station 3): guided magazine seating, the slide and
// charging handle, the slide lock and loose rounds, driven by injected XR
// samples, headlessly.
//
// Plays the reload scenarios in `e2e/vr-lab-scenarios.mjs` against
// `examples/vr-lab` (`--headless`, pinned clock, one sample per fixed step)
// and checks `/state.model`:
//
//   1. The pistol's full loop: emptied, the slide locked back; the magazine
//      dropped; a spare from its cradle brought to the mouth tipped 45°
//      (no latch), lined up (latched, then on the well's axis, turned as the
//      well is), pushed home (seated: 12+0, still locked); the slide nudged
//      back from locked and let go (11+1, unlocked); 12 shots; dry, locked.
//   2. Upside down, or a rifle magazine, at the pistol's mouth: never latches.
//   3. Pulled out half way in: it leaves where it is (≤ 1 mm, 0.5° on that
//      step, the gun still), stays in the hand, and eases back to how the
//      hand held it.
//   4. A partial rack does nothing; a full one with a round chambered throws
//      it out loose (a prop that falls and lands) and chambers the next.
//   5. The rifle: its magazine dropped, a spare seated, the charging handle
//      racked (the chambered round out, 29+1), then it fires.
//   6. Every step of every run conserves the station's rounds (magazines,
//      chambers, loose rounds, shots, and loose rounds taken away).
//   7. The pistol's loop replays in a fresh runtime to an identical model.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab-reload.mjs
import { check, failures } from "./mcp-rpc.mjs";
import { fmt, len, play, qconj, qmul, qrot, quat, record, session, some, sub, turnOf, xyz } from "./xr-scenario.mjs";
import { CAPACITY, LOOSE_CAP, PISTOL_ROUNDS, WELL, WELL_TURN, conserves, project, readout, roundsIn, scenarios, wellPoint } from "./vr-lab-scenarios.mjs";

const PORT = 8292;
const run = (name, body) => session(name, project, PORT, body);

const isRound = (p) => p.kind.$ctor === "Props.Round";
const magOf = (m, id) => m.props.find((p) => p.kind.$ctor === "Props.Mag" && p.kind.args[1] === id);
const latched = (m, id) => some(magOf(m, id)?.latch) !== undefined;
const posOf = (p) => xyz((some(p.held)?.accepted ?? p.pose).position);
const turnOfProp = (p) => quat((some(p.held)?.accepted ?? p.pose).orientation);
const loose = (m) => m.props.filter(isRound);
const PISTOL_SPARE = 1;
const RIFLE_SPARE = 3;

// How far a point is off the held pistol's well axis.
const offAxis = (p) => {
  const local = qrot(qconj(WELL_TURN), sub(p, wellPoint(0)));
  return Math.hypot(local[0], local[2]);
};

let loopEnd;
await run("pistol: the full reload loop", async (rt) => {
  const { steps, marks, index } = await record(rt, scenarios["reload-pistol"]);
  const dry = marks.dryLocked.model;
  check(dry.gun.fired === PISTOL_ROUNDS && dry.gun.dry === 1 && dry.gun.ammo.locked, `emptied, then dry: locked back (fired ${dry.gun.fired}, dry ${dry.gun.dry}, locked ${dry.gun.ammo.locked})`);
  const lockedAt = steps.findIndex((s) => s.state.model.gun.ammo.locked);
  check(steps[lockedAt].state.model.gun.fired === PISTOL_ROUNDS, `the slide locks on the 13th shot (fired ${steps[lockedAt].state.model.gun.fired})`);
  const out = marks.ejected.model;
  check(!some(out.gun.ammo.mag) && out.gun.ammo.locked, `the magazine dropped, the slide still locked (${readout(out.gun.ammo)})`);
  check(some(magOf(marks.tookSpare.model, PISTOL_SPARE).held)?.side.$ctor === "Hold.Left", "the left hand takes a spare from its cradle");
  const refused = steps.slice(index("tookSpare"), index("misaligned") + 1).every((s) => !latched(s.state.model, PISTOL_SPARE));
  check(refused, "tipped 45° at the mouth: no latch");
  const latchedAt = steps.findIndex((s, i) => i > index("misaligned") && latched(s.state.model, PISTOL_SPARE));
  check(latchedAt > index("misaligned") && latchedAt <= index("aligned"), `turned to line up: latched (step ${latchedAt - index("misaligned")} of the turn)`);
  const held = magOf(marks.latchedHeld.model, PISTOL_SPARE);
  const off = offAxis(posOf(held));
  const turn = (turnOf(turnOfProp(held), WELL_TURN) * 180) / Math.PI;
  check(off < 0.001 && turn < 0.5, `eased onto the well: ${(off * 1000).toFixed(2)} mm off the axis, ${turn.toFixed(2)}° off its turn`);
  // Pushed up: on the axis at every step until it seats.
  const push = steps.slice(index("latchedHeld") + 1, index("seated") + 1).map((s) => magOf(s.state.model, PISTOL_SPARE)).filter(Boolean);
  const worst = Math.max(...push.map((p) => offAxis(posOf(p))));
  check(push.length > 5 && worst < 0.001, `guided up the well: at most ${(worst * 1000).toFixed(3)} mm off its axis over ${push.length} steps`);
  const seated = marks.seated.model;
  check(!magOf(seated, PISTOL_SPARE) && roundsIn(seated.gun.ammo.mag) === CAPACITY.pistol && seated.gun.ammo.locked && !seated.gun.ammo.chambered,
    `seated: the magazine in the gun (${readout(seated.gun.ammo)}), still locked`);
  const racked = marks.racked.model;
  check(racked.gun.ammo.chambered && !racked.gun.ammo.locked && roundsIn(racked.gun.ammo.mag) === CAPACITY.pistol - 1 && loose(racked).length === 0,
    `racked from locked: ${readout(racked.gun.ammo)}, unlocked, nothing thrown out`);
  const fired = marks.fired12.model;
  check(fired.gun.fired === PISTOL_ROUNDS + CAPACITY.pistol && fired.score >= CAPACITY.pistol - 1, `then 12 shots (fired ${fired.gun.fired}, score ${fired.score})`);
  const again = marks.dryAgain.model;
  check(again.gun.dry === 2 && again.gun.ammo.locked && readout(again.gun.ammo) === "0+0", `the next pull dry: locked back again (${readout(again.gun.ammo)})`);
  conserves("pistol loop", steps);
  loopEnd = JSON.stringify((await rt.get("/state")).model);
});

await run("replay", async (rt) => {
  await play(rt, scenarios["reload-pistol"]);
  check(loopEnd !== undefined && loopEnd === JSON.stringify((await rt.get("/state")).model), "the loop in a fresh runtime ends on an identical model");
});

await run("upside down at the mouth", async (rt) => {
  const { steps, marks } = await record(rt, scenarios["reload-reversed"]);
  check(steps.every((s) => !latched(s.state.model, PISTOL_SPARE)), "never latches, at the mouth or pushed up the well");
  check(some(magOf(marks.out.model, PISTOL_SPARE).held)?.side.$ctor === "Hold.Left" && !some(marks.out.model.gun.ammo.mag), "still in the hand; the gun still empty");
  conserves("reversed", steps);
});

await run("a rifle magazine at the pistol's well", async (rt) => {
  const { steps, marks } = await record(rt, scenarios["reload-wrong"]);
  check(steps.every((s) => !latched(s.state.model, RIFLE_SPARE)), "never latches");
  check(!some(marks.pushed.model.gun.ammo.mag) && some(magOf(marks.pushed.model, RIFLE_SPARE).held) !== undefined, "the pistol still empty, the magazine in the hand");
  conserves("wrong kind", steps);
});

await run("pulled out half way in", async (rt) => {
  const { steps, marks, index } = await record(rt, scenarios["reload-cancel"]);
  check(latched(marks.latched.model, PISTOL_SPARE), "latched at the mouth");
  const halfIn = magOf(marks.halfIn.model, PISTOL_SPARE);
  const depth = qrot(qconj(WELL_TURN), sub(posOf(halfIn), wellPoint(0)))[1];
  check(latched(marks.halfIn.model, PISTOL_SPARE) && Math.abs(depth + WELL.drop / 2) < 0.002, `half way in (${(depth * 100).toFixed(2)} cm from home)`);
  const leftAt = steps.findIndex((s, i) => i > index("halfIn") && !latched(s.state.model, PISTOL_SPARE));
  check(leftAt > index("halfIn") && leftAt <= index("pulledOut"), `pulled sideways it leaves (step ${leftAt - index("halfIn")} of 12, 1 cm a step)`);
  const [before, at] = [steps[leftAt - 1], steps[leftAt]].map((s) => magOf(s.state.model, PISTOL_SPARE));
  const jump = len(sub(posOf(at), posOf(before)));
  const turn = (turnOf(turnOfProp(at), turnOfProp(before)) * 180) / Math.PI;
  check(jump <= 0.001 && turn <= 0.5, `no pop as it leaves: ${(jump * 1000).toFixed(3)} mm, ${turn.toFixed(3)}° (the gun held still)`);
  check(steps.slice(leftAt).every((s) => some(magOf(s.state.model, PISTOL_SPARE).held)?.side.$ctor === "Hold.Left"), "still in the left hand");
  // Then it eases back, no faster than a settling step (2 cm) beyond the hand's motion.
  const eased = steps.slice(leftAt, leftAt + 8).map((s, i, all) => (i === 0 ? 0 : len(sub(posOf(magOf(s.state.model, PISTOL_SPARE)), posOf(magOf(all[i - 1].state.model, PISTOL_SPARE)))) - len(sub(s.left.at, all[i - 1].left.at))));
  check(Math.max(...eased) <= 0.02 + 0.0001, `easing back: at most ${(Math.max(...eased) * 1000).toFixed(1)} mm a step beyond the hand's`);
  // Settled: held as it was taken.
  const offsetOf = (s) => {
    const p = magOf(s.state.model, PISTOL_SPARE);
    return { at: qrot(qconj(s.left.orientation ?? [0, 0, 0, 1]), sub(posOf(p), s.left.at)), q: qmul(qconj(s.left.orientation ?? [0, 0, 0, 1]), turnOfProp(p)) };
  };
  const took = offsetOf(steps[index("tookSpare")]);
  const now = offsetOf(steps[index("settled")]);
  check(len(sub(now.at, took.at)) < 0.001 && turnOf(now.q, took.q) < 0.01, `settled back to how the hand took it (${fmt(now.at)} vs ${fmt(took.at)})`);
  check(!some(marks.settled.model.gun.ammo.mag), "the gun still empty");
  conserves("cancel", steps);
});

await run("rack: partial, then with a round chambered", async (rt) => {
  const { steps, marks } = await record(rt, scenarios["reload-rack"]);
  const before = marks.onSlide.model;
  check(readout(before.gun.ammo) === "12+1", `loaded (${readout(before.gun.ammo)})`);
  const partial = marks.partialLetGo.model;
  check(readout(partial.gun.ammo) === "12+1" && loose(partial).length === 0, `70% of the way and let go: nothing (${readout(partial.gun.ammo)})`);
  const back = marks.back.model;
  const [round] = loose(back);
  check(round && round.kind.args[0].$ctor === "Ammo.PistolMag" && !back.gun.ammo.chambered, `all the way back: the chambered round thrown out (${JSON.stringify(round?.kind)})`);
  const racked = marks.racked.model;
  check(readout(racked.gun.ammo) === "11+1" && loose(racked).length === 1, `let go: home, the next chambered (${readout(racked.gun.ammo)})`);
  const flight = steps.slice(steps.indexOf(steps.find((s) => s.state.frame === marks.back.frame)) + 2, -1).map((s) => loose(s.state.model)[0]);
  const fastest = Math.max(...flight.map((p) => len(xyz(p.velocity))));
  check(fastest > 1.0, `it flies out (up to ${fastest.toFixed(2)} m/s)`);
  const landed = loose(marks.landed.model)[0];
  const y = landed.pose.position.y;
  check(!some(landed.held) && len(xyz(landed.velocity)) < 0.05 && ((y > 0.9 && y < 0.93) || y < 0.05), `and lands at rest (y ${y.toFixed(4)})`);
  conserves("rack", steps);
});

await run("eight racks: the loose-round cap", async (rt) => {
  const { steps, marks } = await record(rt, scenarios["reload-rounds"]);
  const rest = marks.rest.model;
  const ids = loose(rest).map((p) => p.kind.args[1]);
  check(ids.join() === [3, 4, 5, 6, 7, 8].join() && rest.lost === 2, `eight thrown out: the newest ${LOOSE_CAP} lie about (${ids}), the oldest 2 taken away (lost ${rest.lost})`);
  check(readout(rest.gun.ammo) === "4+1", `the pistol: ${readout(rest.gun.ammo)}`);
  check(loose(rest).every((p) => p.lying && len(xyz(p.velocity)) < 0.05), "all at rest, lying bodiless where they came to rest");
  conserves("cap", steps);
});

await run("rifle: magazine and charging handle", async (rt) => {
  const { steps, marks } = await record(rt, scenarios["reload-rifle"]);
  const out = marks.ejected.model;
  check(!some(out.rifle.ammo.mag) && out.rifle.ammo.chambered, `its magazine dropped, one chambered (${readout(out.rifle.ammo)})`);
  check(latched(marks.aligned.model, RIFLE_SPARE), "a rifle spare at its mouth: latched");
  const seated = marks.seated.model;
  check(!magOf(seated, RIFLE_SPARE) && readout(seated.rifle.ammo) === "30+1", `pushed home: seated (${readout(seated.rifle.ammo)})`);
  const racked = marks.racked.model;
  const thrown = loose(racked);
  check(readout(racked.rifle.ammo) === "29+1" && thrown.length === 1 && thrown[0].kind.args[0].$ctor === "Ammo.RifleMag",
    `the charging handle racked: the chambered round out, the next in (${readout(racked.rifle.ammo)}, ${thrown.length} loose)`);
  check(marks.fired.model.rifle.fired === 1, `then it fires (${marks.fired.model.rifle.fired})`);
  conserves("rifle", steps);
});

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab reload e2e: all checks passed");
