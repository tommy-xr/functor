// vr-lab haptics E2E: each hold event buzzes its hand once, headlessly.
//
// Plays `e2e/vr-lab-scenarios.mjs` against `examples/vr-lab` (`--headless`,
// pinned clock, one injected XR sample per fixed step) and reads each step's
// new pulses from `GET /state.haptics` (`Effect.haptic`), naming each by its
// amplitude (`examples/vr-lab/pulses.fun`):
//
//   1. grab-throw: grab, then release — right hand, nothing dropped.
//   2. wall-push: grab, a stop at the wall (once per push, not per step held
//      there: twice), release.
//   3. break-grip: grab, stop, break.
//   4. inventory-reach: grab the brick, ready entering the empty holster,
//      stow, then the empty hand back in the holster takes it out: unstow
//      (no second ready while it is already in the slot).
//   5. inventory-refuse: neither the stick (fits no slot) nor the bottle (the
//      holster is full) is ready or stowed; both release.
//   6. armory-fire: grab, then one fire per pull (the trigger held down is
//      one); armory-wall: grab, stop at the wall, the refused pull's click,
//      then a fire clear of it; armory-holster: grab, ready, stow, unstow (the
//      draw, trigger held: no fire), then a fire once let out and pulled.
//   7. rifle-auto: unstow, the foregrip's grab, then each shot in both
//      hands (semi held: one; full-auto held 30 steps: five); rifle-wall: the
//      stop, the refused pull's click in the trigger hand, a fire clear of
//      it; rifle-break: the support's break; rifle-handoff: silent handed on,
//      the retake's grab, the support's release.
//   8. Ammunition: ammo-pistol, a fire for each of 13 rounds, then the dry
//      pull's click; ammo-rifle, every full-auto shot in both hands, then a
//      click in the trigger hand alone for each dry pull.
//   9. Reloading: reload-pistol, the spare's unstow, its latch, the seat in
//      both hands, the slide's grab, back and home (home in both hands);
//      reload-rack, a partial pull only its grab; reload-rifle, the same at
//      the rifle's well and charging handle.
//  10. Replay: `POST /rewind` and a hot reload (`POST /reload-project`) add no
//      pulse. A fixture replayed through `/input` is a live run: the recorded
//      grab-throw pulses exactly the live run's events.
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/vr-lab-haptics.mjs
import { check, failures } from "./mcp-rpc.mjs";
import { advance, play, session } from "./xr-scenario.mjs";
import { project, scenarios } from "./vr-lab-scenarios.mjs";

const PORT = 8297;

// Each event's amplitude, as `Pulses.pulse` has it.
const PULSE = {
  ready: 0.15, latch: 0.2, click: 0.25, release: 0.3, grab: 0.45, slideBack: 0.5, unstow: 0.55, slideHome: 0.65,
  stow: 0.7, seat: 0.8, blocked: 0.85, fire: 0.95, break: 1.0,
};
const EVENT = new Map(Object.entries(PULSE).map(([name, a]) => [a, name]));

const run = (name, body) => session(name, project, PORT, body);

// Play a scenario; each step's new pulses as `{ event, hand, frame }`, and its marks.
async function pulsesOf(rt, scenario) {
  const start = (await rt.get("/state")).haptics;
  let sent = start.sent;
  const pulses = [];
  const marks = await play(rt, scenario, async ({ state }) => {
    const { haptics } = state;
    const fresh = haptics.sent - sent;
    sent = haptics.sent;
    if (fresh > haptics.recent.length) throw new Error(`${fresh} pulses in one step`);
    for (const p of fresh > 0 ? haptics.recent.slice(-fresh) : []) {
      pulses.push({ event: EVENT.get(p.amplitude) ?? `?${p.amplitude}`, hand: p.hand, frame: p.frame });
    }
  });
  const end = (await rt.get("/state")).haptics;
  return { pulses, marks, dropped: end.dropped - start.dropped, sent: end.sent };
}

const names = (pulses) => pulses.map((p) => `${p.hand} ${p.event}`).join(", ");
const expectPulses = (got, want) =>
  check(names(got.pulses) === want.join(", "), `pulses: ${names(got.pulses) || "none"} (want ${want.join(", ")})`);
const between = (p, after, upTo) => p !== undefined && p.frame > after && p.frame <= upTo;

let live;
await run("grab and throw", async (rt) => {
  const got = await pulsesOf(rt, scenarios["grab-throw"]);
  expectPulses(got, ["right grab", "right release"]);
  const [grab, release] = got.pulses;
  check(between(grab, got.marks.reached.frame, got.marks.grabbed.frame), `the grab buzzes as the squeeze lands (frame ${grab?.frame})`);
  check(release?.frame === got.marks.release.frame, `the release on the step it opens (frame ${release?.frame})`);
  check(got.dropped === 0, `none dropped (${got.dropped})`);
  live = got;

  // A hot reload rebinds the model without re-running effects (then a quiet
  // live step: the ball has landed and the hand is open).
  await rt.post("/reload-project", await rt.get("/project"));
  await advance(rt);
  check((await rt.get("/state")).haptics.sent === got.sent, "a hot reload adds no pulse");
  // Nor does a rewind, which restores frames without re-running theirs.
  await rt.post("/rewind", { frame: got.marks.grabbed.frame });
  check((await rt.get("/state")).haptics.sent === got.sent, "a rewind adds no pulse");
});

await run("wall push", async (rt) => {
  const got = await pulsesOf(rt, scenarios["wall-push"]);
  expectPulses(got, ["right grab", "right blocked", "right blocked", "right release"]);
  const [, first, second] = got.pulses;
  const { grabbed, pushed, pulled, pushedAgain } = got.marks;
  check(between(first, grabbed.frame, pushed.frame), `stopped at the wall: once (frame ${first?.frame})`);
  check(between(second, pulled.frame, pushedAgain.frame), `pulled free and pushed again: once more (frame ${second?.frame})`);
});

await run("break grip", async (rt) => {
  const got = await pulsesOf(rt, scenarios["break-grip"]);
  expectPulses(got, ["right grab", "right blocked", "right break"]);
  check(between(got.pulses[2], got.marks.grabbed.frame, got.marks.broke.frame), `the break while dragged (frame ${got.pulses[2]?.frame})`);
});

await run("inventory: stow and take back", async (rt) => {
  const got = await pulsesOf(rt, scenarios["inventory-reach"]);
  expectPulses(got, ["right grab", "right ready", "right stow", "right unstow"]);
  const [, ready, stow, unstow] = got.pulses;
  const { tookBrick, atHolster, stored, beforeRetrieve, retrieved } = got.marks;
  check(between(ready, tookBrick.frame, atHolster.frame), `ready entering the holster (frame ${ready?.frame})`);
  check(stow?.frame === stored.frame, `stowed on the step it opens (frame ${stow?.frame})`);
  check(between(unstow, beforeRetrieve.frame, retrieved.frame), `taken out on the squeeze (frame ${unstow?.frame})`);
});

await run("inventory: refusals", async (rt) => {
  const got = await pulsesOf(rt, scenarios["inventory-refuse"]);
  expectPulses(got, [
    "right grab", "right release", // the stick: fits no slot
    "right grab", "right ready", "right stow", // the brick
    "right grab", "right release", // the bottle: the holster is full
  ]);
});

await run("armory: fire", async (rt) => {
  const got = await pulsesOf(rt, scenarios["armory-fire"]);
  expectPulses(got, ["right grab", "right fire", "right fire"]);
  check(got.pulses[1]?.frame === got.marks.fired.frame, `the fire on the pull's step (frame ${got.pulses[1]?.frame})`);
  check(got.pulses[2]?.frame === got.marks.missed.frame, `none while held; the next on the next pull (frame ${got.pulses[2]?.frame})`);
});

await run("armory: barrel into the wall", async (rt) => {
  const got = await pulsesOf(rt, scenarios["armory-wall"]);
  expectPulses(got, ["right grab", "right blocked", "right click", "right fire"]);
  check(got.pulses[2]?.frame === got.marks.clicked.frame, `the click on the refused pull (frame ${got.pulses[2]?.frame})`);
  check(got.pulses[3]?.frame === got.marks.fired.frame, `the fire clear of it (frame ${got.pulses[3]?.frame})`);
});

await run("armory: holster and draw", async (rt) => {
  const got = await pulsesOf(rt, scenarios["armory-holster"]);
  expectPulses(got, ["right grab", "right ready", "right stow", "right unstow", "right fire"]);
  check(got.pulses[4]?.frame === got.marks.fired.frame, `no fire on the draw; one once let out and pulled (frame ${got.pulses[4]?.frame})`);
});

await run("rifle: semi and full-auto", async (rt) => {
  const got = await pulsesOf(rt, scenarios["rifle-auto"]);
  // Out of the rack, the foregrip taken; every shot buzzes both hands on it.
  const shot = ["left fire", "right fire"];
  expectPulses(got, ["right unstow", "left grab", ...shot, ...Array(5).fill(shot).flat()]);
  check(got.pulses[2]?.frame === got.marks.semiHeld.frame - 29, `semi: one shot, on the pull (frame ${got.pulses[2]?.frame})`);
});

await run("ammo: the pistol emptied, then dry", async (rt) => {
  const got = await pulsesOf(rt, scenarios["ammo-pistol"]);
  expectPulses(got, ["right grab", ...Array(13).fill("right fire"), "right click"]);
  check(got.pulses.at(-1)?.frame === got.marks.dry.frame, `the click on the dry pull (frame ${got.pulses.at(-1)?.frame})`);
});

await run("ammo: the rifle on full-auto to empty", async (rt) => {
  const got = await pulsesOf(rt, scenarios["ammo-rifle"]);
  expectPulses(got, ["right unstow", "left grab", ...Array(31).fill(["left fire", "right fire"]).flat(), "right click", "right click"]);
  check(got.pulses.at(-1)?.frame === got.marks.dryAgain.frame, `the second click on the next pull (frame ${got.pulses.at(-1)?.frame})`);
});

await run("rifle: barrel into the wall", async (rt) => {
  const got = await pulsesOf(rt, scenarios["rifle-wall"]);
  expectPulses(got, ["right unstow", "left grab", "right blocked", "right click", "left fire", "right fire"]);
  check(got.pulses[3]?.frame === got.marks.clicked.frame, `the click on the refused pull, the trigger hand only (frame ${got.pulses[3]?.frame})`);
});

await run("rifle: support break", async (rt) => {
  const got = await pulsesOf(rt, scenarios["rifle-break"]);
  expectPulses(got, ["right unstow", "left grab", "left break"]);
  check(between(got.pulses[2], got.marks.slidBack.frame, got.marks.dragged.frame), `the break as it is dragged off (frame ${got.pulses[2]?.frame})`);
});

await run("rifle: hand-off", async (rt) => {
  const got = await pulsesOf(rt, scenarios["rifle-handoff"]);
  // Handed on: silent. Retaken: the right's grab. The support opening: its release.
  expectPulses(got, ["right unstow", "left grab", "right grab", "left release"]);
  check(got.pulses[2]?.frame === got.marks.retaken.frame && got.pulses[3]?.frame === got.marks.supportOpened.frame, "on the retake and the support's opening");
});

await run("reload: the pistol's full loop", async (rt) => {
  const got = await pulsesOf(rt, scenarios["reload-pistol"]);
  // Emptied and dry; the spare out of its cradle; latched, seated (both
  // hands); the slide taken, back, home (both hands); 12 shots and dry.
  expectPulses(got, [
    "right grab", ...Array(13).fill("right fire"), "right click",
    "left unstow", "left latch", "left seat", "right seat",
    "left grab", "left slideBack", "left slideHome", "right slideHome",
    ...Array(12).fill("right fire"), "right click",
  ]);
  const at = (event) => got.pulses.find((p) => p.event === event)?.frame;
  check(at("latch") > got.marks.misaligned.frame && at("latch") <= got.marks.aligned.frame, `the latch as it lines up (frame ${at("latch")})`);
  check(at("seat") <= got.marks.seated.frame && at("seat") > got.marks.latchedHeld.frame, `the seat as it goes home (frame ${at("seat")})`);
  check(at("slideBack") <= got.marks.pulled.frame && at("slideHome") === got.marks.racked.frame, `back as it is pulled, home as it is let go (frames ${at("slideBack")}, ${at("slideHome")})`);
});

await run("reload: partial and full racks", async (rt) => {
  const got = await pulsesOf(rt, scenarios["reload-rack"]);
  // The partial pull: only its grab. The full one: back, home.
  expectPulses(got, ["right grab", "left grab", "left grab", "left slideBack", "left slideHome", "right slideHome"]);
});

await run("reload: the rifle", async (rt) => {
  const got = await pulsesOf(rt, scenarios["reload-rifle"]);
  expectPulses(got, [
    "right unstow", "left unstow", "left latch", "left seat", "right seat",
    "left grab", "left slideBack", "left slideHome", "right slideHome", "right fire",
  ]);
});

await run("recorded fixture", async (rt) => {
  const got = await pulsesOf(rt, scenarios["grab-throw-recorded"]);
  // A replay through `/input` is a live run: the recording's own grab and throw.
  expectPulses(got, ["right grab", "right release"]);
  check(live === undefined || names(got.pulses) === names(live.pulses), "the same events as the scripted throw");
});

if (failures.length) {
  console.error(`\n${failures.length} failure(s)`);
  process.exit(1);
}
console.log("\nvr-lab haptics e2e: all checks passed");
