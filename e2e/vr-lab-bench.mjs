// vr-lab per-step cost: the inventory station's logic, measured headlessly.
//
// Plays `inventory-tour` (e2e/vr-lab-scenarios.mjs) on the inventory station,
// then the same hand motion on the grab table, which has the same props in
// the same rig-local places but wears no slots. Each step reads its stage
// times back from `GET /frames` and reports per-step `sim` (all producer
// work: sampledInput, tick, physics, draw), `tick` and `draw`, as min,
// median, p95 and max. The difference between the two medians is what the
// hips frame and the slots cost; set against the 120 Hz Quest frame
// (8.33 ms) at the sweep_bench convention of ×4 for desktop → Quest.
//
// Use a release build (a debug one is many times slower):
//
//   cargo build --release -p functor-cli --no-default-features
//   FUNCTOR_BIN=target/release/functor node e2e/vr-lab-bench.mjs
import { launch, play } from "./xr-scenario.mjs";
import { project, scenarios } from "./vr-lab-scenarios.mjs";

const PORT = 8294;
const BUDGET_US = 1e6 / 120;
const QUEST = 4;
const STAGES = ["sim_us", "tick_us", "draw_us"];

async function measure(scenario) {
  const rt = await launch(project, PORT, ["--headless"]);
  const rows = [];
  try {
    await play(rt, scenario, async () => {
      const [r] = (await rt.get("/frames?last=1")).frames;
      if (r.steps === 1) rows.push(r);
    });
  } finally {
    await rt.kill();
  }
  // The first steps include the station key's reset and warm-up.
  return rows.slice(20);
}

const stats = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const at = (q) => s[Math.min(s.length - 1, Math.floor(q * s.length))];
  return { min: s[0], med: at(0.5), p95: at(0.95), max: s.at(-1) };
};

const tour = scenarios["inventory-tour"];
const runs = [
  ["inventory (station 2)", tour],
  ["grab table (station 1), same hands", { ...tour, station: 1 }],
];
const results = [];
for (const [name, scenario] of runs) {
  const rows = await measure(scenario);
  results.push(Object.fromEntries(STAGES.map((k) => [k, stats(rows.map((r) => r[k]))])));
  console.log(`\n${name}: ${rows.length} steps, µs per step`);
  console.log(`  ${"stage".padEnd(6)} ${"min".padStart(7)} ${"med".padStart(7)} ${"p95".padStart(7)} ${"max".padStart(7)}`);
  for (const k of STAGES) {
    const s = results.at(-1)[k];
    console.log(`  ${k.replace("_us", "").padEnd(6)} ${[s.min, s.med, s.p95, s.max].map((v) => String(v).padStart(7)).join(" ")}`);
  }
}
const [inv, base] = results;
const delta = inv.sim_us.med - base.sim_us.med;
const pct = (us) => ((100 * us) / BUDGET_US).toFixed(1);
console.log(`\nslots + hips: ${delta} µs per step on desktop (median sim, station 2 − station 1)`);
console.log(`Quest estimate (×${QUEST}): ${delta * QUEST} µs, ${pct(delta * QUEST)}% of the ${BUDGET_US.toFixed(0)} µs frame`);
console.log(`whole station: ${inv.sim_us.med} µs median, ${inv.sim_us.p95} p95 → Quest ≈ ${inv.sim_us.med * QUEST} µs (${pct(inv.sim_us.med * QUEST)}%), p95 ${pct(inv.sim_us.p95 * QUEST)}%`);
