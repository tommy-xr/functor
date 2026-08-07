// `examples/code-garden` E2E: the model is the garden, the file is the botany.
//
// The sample's claim is that its growth LAWS are top-level constants and each
// plant stores only plot/species/age — so editing a law re-shapes a garden that
// is already growing instead of restarting it. That is a hot-reload property,
// which no inline `expect` can reach: it needs a running game, a pinned clock,
// and two frames. This drives it over `functor mcp` (docs/mcp.md), raw
// JSON-RPC on stdio, exactly as a coding agent's client would:
//
//   1. `init` sows five plants at staggered ages.
//   2. Stepping 8 seconds ages every plant by exactly 8 and fires the
//      `Sub.every(7s)` self-seeding timer once (5 plants -> 6).
//   3. `1` then `Space` plants a Lantern seed at the next plot (6 -> 7), a
//      brand-new plant appended at age 0 while the others keep their ages.
//   4. HOT-RELOADING a changed law (`growthRate`) leaves the model identical —
//      same plots, same species, same ages — while the RENDERED FRAME changes.
//   5. Reloading the ORIGINAL source reproduces the original frame byte for
//      byte, so the reshape is a pure function of the file with no drift.
//
// The frames are compared with `capture_frame`, not `get_scene`: a mature
// garden serializes to ~12 MB of scene JSON, past the MCP response cap.
// `capture_frame` needs pixels, hence `mode: "hidden"` rather than `headless`.
//
// Run (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/code-garden.mjs
//
// Set FUNCTOR_BIN when the build uses a shared CARGO_TARGET_DIR.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { check, failures, ROOT, startMcp } from "./mcp-rpc.mjs";

const GAME_DIR = resolve(ROOT, "examples/code-garden");
const SOURCE = readFileSync(resolve(GAME_DIR, "game.fun"), "utf8");

const near = (a, b, eps = 1e-4) => Math.abs(a - b) < eps;

/** The rendered frame as a base64 PNG — the only whole-scene read that scales. */
const frameImage = async (rpc, session) => {
  const result = await rpc.request("tools/call", { name: "capture_frame", arguments: { session } });
  const image = result.content.find((block) => block.type === "image");
  if (!image) throw new Error(`capture_frame returned no image: ${JSON.stringify(result)}`);
  return image.data;
};

const { proc, rpc } = await startMcp("code-garden");
let session;

try {
  console.log("▸ launch, paused");
  const launched = await rpc.call("launch_game", { dir: GAME_DIR, mode: "hidden" });
  session = launched.session;
  await rpc.call("pause", { session });

  // `pause` cannot pin frame 0 — a few frames always run before it lands — so
  // every age assertion below is a DELTA from this first observed state.
  const sown = (await rpc.call("get_state", { session })).model;
  const ages0 = sown.plants.map((p) => p.age);
  check(sown.plants.length === 5, `init sows 5 plants (got ${sown.plants.length})`);
  check(
    ages0.every((age, i) => near(age - ages0[4], [10, 7, 4.5, 2, 0][i], 0.01)),
    `their ages are staggered 10/7/4.5/2/0 apart: ${JSON.stringify(ages0.map((a) => +a.toFixed(3)))}`,
  );

  // 8 seconds, deliberately clear of the 7s self-seeding period: 420 steps of
  // 1/60 sum to 6.99999999999998, so a "7 second" drive would sit exactly on
  // the timer's edge and fire only thanks to the frames that ran before the
  // pause landed.
  console.log("\n▸ 8 seconds of growth (80 steps of 0.1s)");
  const grown = (await rpc.call("step", { session, frames: 80, dts: 0.1 })).model;
  check(
    grown.plants.slice(0, 5).every((p, i) => near(p.age, ages0[i] + 8.0, 1e-3)),
    "every original plant aged by exactly 8.0s — age is stored in seconds",
  );
  check(
    grown.plants.length === 6,
    `Sub.every(7s) self-seeded exactly once (${grown.plants.length} plants)`,
  );
  // It sprouted at the 7s mark and has been growing for the ~1s since, so it is
  // far younger than every plant `init` sowed.
  check(
    grown.plants[5].age > 0 && grown.plants[5].age < grown.plants[4].age,
    `the volunteer sprouted mid-drive, younger than every original (${grown.plants[5].age.toFixed(3)}s old)`,
  );

  console.log("\n▸ planting by hand: 1 then Space");
  await rpc.call("send_input", { session, command: { type: "key", key: "1", down: true } });
  await rpc.call("send_input", { session, command: { type: "key", key: "1", down: false } });
  const picked = (await rpc.call("step", { session, frames: 1, dts: 1 / 60 })).model;
  check(picked.pick === 0, `"1" selects Lantern (pick=${picked.pick})`);

  await rpc.call("send_input", { session, command: { type: "key", key: "space", down: true } });
  await rpc.call("send_input", { session, command: { type: "key", key: "space", down: false } });
  await rpc.call("step", { session, frames: 1, dts: 1 / 60 });
  // Read the SETTLED model with `get_state` rather than reusing the one `step`
  // returned. The two can disagree in the last ULP of an untouched float
  // (`15.083333702757956` vs `...55`) — tommy-xr/functor#674 — which would make
  // the byte-exact comparison after the reload below flaky; two `get_state`
  // reads agree with each other, and `get_state` across a reload is bit-exact.
  // So both sides of that comparison come from the same endpoint and the
  // assertion stays EXACT.
  const planted = (await rpc.call("get_state", { session })).model;
  check(planted.plants.length === 7, `Space plants a seed (${planted.plants.length} plants)`);
  check(planted.plants[6].species === 0, "the new plant is APPENDED, with the picked species");
  check(
    planted.plants[6].age >= 0 && planted.plants[6].age <= 1 / 60 + 1e-6,
    `the new plant is at most one step old (${planted.plants[6].age.toFixed(4)}s)`,
  );
  check(
    planted.plants.slice(0, 6).every((p, i) => near(p.age, grown.plants[i].age, 0.05)),
    "planting does not disturb the plants already growing",
  );

  console.log("\n▸ the hot-reload thesis: edit a law, keep the garden");
  const before = await frameImage(rpc, session);

  const slowed = SOURCE.replace(/^let growthRate = 0\.42/m, "let growthRate = 0.10");
  check(slowed !== SOURCE, "the edit under test is `growthRate: 0.42 -> 0.10`");
  // No `step` around the reloads: the clock is pinned, so every frame below is
  // rendered at the same `tts` and any difference is the EDIT, not the weather.
  await rpc.call("reload_source", { session, source: slowed });
  const afterReload = (await rpc.call("get_state", { session })).model;

  // The WHOLE model, not a chosen subset: every plot, species and age, plus
  // `pick`. No step ran across the reload, so this is exact equality.
  check(
    JSON.stringify(afterReload) === JSON.stringify(planted),
    `the entire model survived the reload unchanged — the model IS the garden (${afterReload.plants.length} plants)`,
  );

  const after = await frameImage(rpc, session);
  check(after !== before, "the same garden renders a different frame — the law reshaped it");

  console.log("\n▸ and back: the reshape is a pure function of the file");
  await rpc.call("reload_source", { session, source: SOURCE });
  const restored = await frameImage(rpc, session);
  check(restored === before, "restoring the source reproduces the original frame exactly");
} finally {
  if (session) await rpc.call("stop_game", { session }).catch(() => {});
  proc.stdin.end();
  proc.kill();
}

console.log(failures.length ? `\n✗ ${failures.length} failed` : "\n✓ all checks passed");
process.exit(failures.length ? 1 : 0);
