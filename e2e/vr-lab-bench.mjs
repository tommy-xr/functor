// vr-lab per-step cost: the inventory station's logic, measured headlessly.
//
// Plays `inventory-reach` (e2e/vr-lab-scenarios.mjs) up to the hand reaching
// into the holster (the brick slotted, the hips pinned), then holds that pose
// for a burst of steps queued at once (`/time advance` with `frames`), so the
// CPU never idles between them: spaced-out steps measure DVFS, not work. The
// shell runs a burst several steps per frame and draws once a frame. Then the
// same on the grab table, which has the same props in the same rig-local
// places but wears no slots; then the armory, the pistol held aimed down
// range (its trigger read each step), the slots worn; then the rifle there in
// two hands, aimed by the support (one more held prop: one more sweep a step);
// then a magazine latched half way into the pistol's well (the guided seat),
// and the pistol held with six loose rounds lying at rest (the cap); then
// the shotgun in two hands, aimed (loaded, a shell lying on the bench).
//
// A shot is one step, so a burst can't time it: the shotgun's seven shots
// are played a step at a time instead (a spinning worker thread keeps the
// clock up between them), and each firing step (eight pellet rays) is set
// against the pump steps around it in the same run. The extra is added to
// the aimed shotgun's burst minimum for its worst frame.
//
// A hand's glove is rebuilt only when its fingers move (`Scene.memo`), and a
// burst holds them still. So, also a step at a time: both hands raised at
// the grab table, held still, then their grip and trigger swept every step
// (both gloves rebuilt each frame). The extra draw is added to the aimed
// shotgun's worst frame too: a trigger moving under the aim.
//
// From `GET /frames`: `step` (one fixed step's producer work: sampledInput,
// tick, physics) and `draw` (once a frame), min / median / p95 / max. The
// summary compares the MIN column: wall-clock medians swing with DVFS and
// load from run to run. At 120 Hz a frame runs a 60 Hz step every other
// frame, so the worst frame costs a step plus a draw: set against the
// 8.33 ms Quest frame at the sweep_bench convention of ×4 for desktop → Quest.
//
//   npm run bench:vr-lab        # a release build; a debug one is many times slower
import { Worker } from "node:worker_threads";
import { sleep } from "./mcp-rpc.mjs";
import { DT, launch, play } from "./xr-scenario.mjs";
import { project, scenarios } from "./vr-lab-scenarios.mjs";

const PORT = 8294;
const BURST = 1200;
const BUDGET_US = 1e6 / 120;
const QUEST = 4;

async function measure(scenario) {
  const rt = await launch(project, PORT, ["--headless"]);
  try {
    await play(rt, scenario);
    const from = (await rt.get("/state")).frame;
    await rt.post("/time", { type: "advance", dts: DT, frames: BURST });
    while ((await rt.get("/state")).frame < from + BURST) await sleep(50);
    const frames = (await rt.get("/frames?last=600")).frames.filter((f) => f.steps > 0 && f.frame > from);
    return {
      steps: frames.reduce((n, f) => n + f.steps, 0),
      step: frames.map((f) => (f.sim_us - f.draw_us) / f.steps),
      draw: frames.map((f) => f.draw_us),
    };
  } finally {
    await rt.kill();
  }
}

// Play `scenario` a step at a time (a spinning worker thread keeps the clock
// up between them), calling `onStep` with each; `/frames`' one-step frames.
async function stepwise(scenario, onStep) {
  const rt = await launch(project, PORT, ["--headless"]);
  const spin = new Worker("for (;;) {}", { eval: true });
  try {
    await play(rt, scenario, onStep);
    return (await rt.get("/frames?last=600")).frames.filter((f) => f.steps === 1);
  } finally {
    await spin.terminate();
    await rt.kill();
  }
}

// The step times of the steps that fired the shotgun and of the others since
// `from` (a mark).
async function measureShots(scenario, from) {
  const fired = new Set();
  let start;
  let shots = 0;
  const frames = await stepwise(scenario, async ({ state, mark }) => {
    if (mark === from) start = state.frame;
    if (start !== undefined && state.model.shotgun.fired > shots) fired.add(state.frame);
    shots = state.model.shotgun.fired;
  });
  const since = frames.filter((f) => f.frame > start);
  const stepOf = (f) => f.sim_us - f.draw_us;
  return { fire: since.filter((f) => fired.has(f.frame)).map(stepOf), other: since.filter((f) => !fired.has(f.frame)).map(stepOf) };
}

// Both hands raised at the grab table: `still` steps held, then `moving`
// steps with their grip and trigger swept (a different curl every step).
const HAND_AT = { right: [0.07, 1.3, -0.32], left: [-0.07, 1.3, -0.32] };
const sweep = (i) => 0.5 + 0.5 * Math.sin(i * 0.37);
const curling = {
  station: 1,
  steps: [
    { frames: 20, right: { to: HAND_AT.right }, left: { to: HAND_AT.left }, mark: "placed" },
    { frames: 60, right: {}, left: {}, mark: "still" },
    ...Array.from({ length: 60 }, (_, i) => ({
      frames: 1,
      right: { squeeze: sweep(i), trigger: sweep(i + 2) },
      left: { squeeze: sweep(i + 4), trigger: sweep(i + 6) },
    })),
  ],
};

// The held and the moving steps' step and draw times, `curling` played a step
// at a time.
async function measureCurling() {
  const at = {};
  const frames = (await stepwise(curling, async ({ state, mark }) => {
    if (mark) at[mark] = state.frame;
  })).filter((f) => f.frame > at.placed);
  const part = (fs) => ({ step: fs.map((f) => f.sim_us - f.draw_us), draw: fs.map((f) => f.draw_us) });
  return { still: part(frames.filter((f) => f.frame <= at.still)), moving: part(frames.filter((f) => f.frame > at.still)) };
}

const stats = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const at = (q) => s[Math.min(s.length - 1, Math.floor(q * s.length))];
  return { min: s[0], med: at(0.5), p95: at(0.95), max: s.at(-1) };
};
const row = (name, s) => `  ${name.padEnd(5)} ${[s.min, s.med, s.p95, s.max].map((v) => v.toFixed(0).padStart(7)).join(" ")}`;

const reach = scenarios["inventory-reach"];
const held = { ...reach, steps: reach.steps.slice(0, reach.steps.findIndex((s) => s.mark === "inZone") + 1) };
const fire = scenarios["armory-fire"];
const aimed = { ...fire, steps: fire.steps.slice(0, fire.steps.findIndex((s) => s.mark === "aimed") + 1) };
const rifle = scenarios["rifle-aim"];
const twoHands = { ...rifle, steps: rifle.steps.slice(0, rifle.steps.findIndex((s) => s.mark === "aimedRight") + 1) };
const cancel = scenarios["reload-cancel"];
const latched = { ...cancel, steps: cancel.steps.slice(0, cancel.steps.findIndex((s) => s.mark === "halfIn") + 1) };
const loop = scenarios["shotgun-loop"];
const sgAimed = { ...loop, steps: loop.steps.slice(0, loop.steps.findIndex((s) => s.mark === "aimed") + 1) };
const runs = [
  ["inventory (station 2): brick slotted, hand in the holster", held],
  ["grab table (station 1): the same hands", { ...held, station: 1 }],
  ["armory (station 3): the pistol held aimed", aimed],
  ["armory (station 3): the rifle in two hands, aimed", twoHands],
  ["armory (station 3): a magazine latched half way into the pistol's well", latched],
  ["armory (station 3): the pistol held, six loose rounds at rest", scenarios["reload-rounds"]],
  ["armory (station 3): the shotgun in two hands, aimed", sgAimed],
];
const results = [];
for (const [name, scenario] of runs) {
  const m = await measure(scenario);
  const r = { step: stats(m.step), draw: stats(m.draw) };
  results.push(r);
  console.log(`\n${name}: ${m.steps} steps in ${m.step.length} frames, µs`);
  console.log(`  ${"".padEnd(5)} ${["min", "med", "p95", "max"].map((h) => h.padStart(7)).join(" ")}`);
  console.log(row("step", r.step));
  console.log(row("draw", r.draw));
}
const [inv, base, armory, rifled, seating, strewn, sg] = results;
const pct = (us) => `${((100 * us) / BUDGET_US).toFixed(1)}%`;
const step = inv.step.min - base.step.min;
const draw = inv.draw.min - base.draw.min;
console.log(`\nslots + hips, desktop mins: +${step.toFixed(0)} µs a step, +${draw.toFixed(0)} µs a draw`);
const worst = (r) => r.step.min + r.draw.min;
console.log(`Quest (×${QUEST}), a frame with a step: inventory ${(worst(inv) * QUEST).toFixed(0)} µs (${pct(worst(inv) * QUEST)} of ${BUDGET_US.toFixed(0)}), grab table ${(worst(base) * QUEST).toFixed(0)} µs (${pct(worst(base) * QUEST)}); the slots' share ${pct((step + draw) * QUEST)}`);
console.log(`armory, desktop mins: step ${armory.step.min.toFixed(0)} µs, draw ${armory.draw.min.toFixed(0)} µs; Quest (×${QUEST}) ${(worst(armory) * QUEST).toFixed(0)} µs (${pct(worst(armory) * QUEST)} of ${BUDGET_US.toFixed(0)})`);
console.log(`rifle, desktop mins: step ${rifled.step.min.toFixed(0)} µs, draw ${rifled.draw.min.toFixed(0)} µs; Quest (×${QUEST}) ${(worst(rifled) * QUEST).toFixed(0)} µs (${pct(worst(rifled) * QUEST)} of ${BUDGET_US.toFixed(0)}); over the pistol +${(rifled.step.min - armory.step.min).toFixed(0)} µs a step, +${(rifled.draw.min - armory.draw.min).toFixed(0)} µs a draw`);
const versus = (name, r) =>
  console.log(`${name}, desktop mins: step ${r.step.min.toFixed(0)} µs, draw ${r.draw.min.toFixed(0)} µs; Quest (×${QUEST}) ${(worst(r) * QUEST).toFixed(0)} µs (${pct(worst(r) * QUEST)} of ${BUDGET_US.toFixed(0)}); over the pistol aimed +${(r.step.min - armory.step.min).toFixed(0)} µs a step, +${(r.draw.min - armory.draw.min).toFixed(0)} µs a draw`);
versus("magazine latched", seating);
versus("six loose rounds", strewn);
versus("shotgun aimed", sg);
// Up to the last shot's pump (`/frames` keeps the last 150 or so).
const shots = await measureShots({ ...loop, steps: loop.steps.slice(0, loop.steps.findIndex((s) => s.mark === "after") + 1) }, "aimed");
const [firing, other] = [stats(shots.fire), stats(shots.other)];
console.log(`\nshotgun, stepped one at a time from the aim: ${shots.fire.length} firing steps, ${shots.other.length} others, µs`);
console.log(`  ${"".padEnd(5)} ${["min", "med", "p95", "max"].map((h) => h.padStart(7)).join(" ")}`);
console.log(row("fire", firing));
console.log(row("other", other));
const extra = firing.min - other.min;
const fireFrame = (sg.step.min + extra + sg.draw.min) * QUEST;
console.log(`a shot's step: +${extra.toFixed(0)} µs (mins); its frame, Quest (×${QUEST}) ${fireFrame.toFixed(0)} µs (${pct(fireFrame)} of ${BUDGET_US.toFixed(0)})`);
const curl = await measureCurling();
const [held2, moving] = [curl.still, curl.moving].map((c) => ({ step: stats(c.step), draw: stats(c.draw) }));
console.log(`\nhands at the grab table, a step at a time: ${curl.still.draw.length} held, ${curl.moving.draw.length} curling every step, µs`);
console.log(`  ${"".padEnd(5)} ${["min", "med", "p95", "max"].map((h) => h.padStart(7)).join(" ")}`);
console.log(row("held", held2.draw));
console.log(row("curl", moving.draw));
const regrip = moving.draw.min - held2.draw.min;
const regripFrame = (sg.step.min + sg.draw.min + regrip / 2) * QUEST;
console.log(`both gloves rebuilt: +${regrip.toFixed(0)} µs a draw (mins); one rebuilt under the aimed shotgun, Quest (×${QUEST}) ${regripFrame.toFixed(0)} µs (${pct(regripFrame)} of ${BUDGET_US.toFixed(0)})`);
