import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner, waitFor } from "../src/index.js";

// C6 perf gate (docs/functor-lang.md): the tree-walking interpreter must hold 60fps
// with headroom on a representative load. The producer already prints a
// rolling average every 300 frames:
//
//   [functor-lang] avg over 300 frames: tick X.Xµs, physics Y.Yµs, draw Z.Zµs
//   (N.N% of a 60fps budget)
//
// This test free-runs a 100-entity lit scene ON THE WALL CLOCK (no pause —
// pinning the clock would measure nothing), waits for at least two stats
// windows (600+ frames, ~10-12s at the headless loop's ~60Hz cap), and
// asserts the LAST window's Functor Lang eval cost (tick + draw) stays under 60% of
// the 16.6ms frame budget. The gate exists to catch ORDER-OF-MAGNITUDE
// regressions (an accidental per-frame deep clone, a quadratic walk), not
// hardware spread: local Apple Silicon measures ~12% of budget at 100
// entities, while GitHub's shared macOS runners measure ~32% — a uniform
// ~2.6x that failed the original 25% gate on every PR. 60% clears the
// slowest observed CI hardware ~2x while still tripping on any real
// regression class. The spike measured ~0.4% at 51 entities, so 60%
// is very generous by design: the gate exists to catch order-of-magnitude
// regressions (an accidental deep-clone per frame, an O(n²) rebind), not
// scheduler noise. The measured numbers are printed so CI logs double as a
// perf record.
//
// OPT-IN, and NOT part of the per-PR e2e suite (`FUNCTOR_PERF=1` gates it,
// which `test:e2e[:headless]` does not set) — the golden-test precedent.
// This measurement free-runs 600 real frames on the wall clock, so it
// depends on frame THROUGHPUT, which shared CI runners cannot guarantee:
// the same eval that finishes two windows in ~13s locally repeatedly blew
// past even a 240s wait on GitHub's macOS runners (contention, not a
// regression). A flaky REQUIRED check is worse than a reliable on-demand
// one; run it deliberately (`FUNCTOR_E2E=1 FUNCTOR_PERF=1 npm run
// test:e2e:headless`) or from a dedicated non-blocking perf job. The gate
// still catches order-of-magnitude regressions when run — it just no
// longer gates merges on hardware noise.
const e2eEnabled = process.env.FUNCTOR_E2E === "1" && process.env.FUNCTOR_PERF === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

const BUDGET_US = 16_666; // one 60fps frame
const GATE_US = BUDGET_US * 0.6;

// The heaviest deterministic load we can express today: 100 entities with
// per-entity model updates in `tick` and per-entity transforms in `draw`,
// through the lit pipeline (shadow-casting sun + two orbiting point lights)
// — heavier than any shipped example (examples/primitives has ~5 nodes).
const ENTITIES = 100;
const heavyGame = `let tau = 6.2831853

let entity = (i: Float) => { i: i, phase: i * 0.618 }

let init = { entities: List.range(${ENTITIES}) |> List.map(entity) }

// Per-entity model work each frame, so tick cost scales with entities.
let tick = (m, dt, tts) =>
  { m with entities: m.entities |> List.map((e) => { e with phase: e.phase + dt }) }

let pointPos = (i: Float, tts: Float) =>
  let a = tts * 0.6 + i * (tau / 2.0) in
  { x: Math.cos(a) * 4.0, y: 2.4, z: Math.sin(a) * 4.0 }

let marker = (i: Float, tts: Float, r: Float, g: Float, b: Float) =>
  let p = pointPos(i, tts) in
  Scene.sphere()
    |> Scene.scale(0.15)
    |> Scene.emissive(r, g, b)
    |> Scene.translate(p.x, p.y, p.z)

let pointLight = (i: Float, tts: Float, r: Float, g: Float, b: Float) =>
  let p = pointPos(i, tts) in
  Light.point(p.x, p.y, p.z, r, g, b, 1.4, 6.0)

// Spiral placement: radius and angle both derive from the index (no
// mod/floor in the language yet); spin and bob derive from the phase.
let shapeFor = (e) =>
  let a = e.i * 0.618 in
  let r = 1.0 + e.i * 0.08 in
  Scene.cube()
    |> Scene.scale(0.3)
    |> Scene.rotateY(Angle.radians(e.phase))
    |> Scene.translate(Math.cos(a) * r, 0.4 + Math.sin(e.phase) * 0.2, Math.sin(a) * r)

let draw = (m, tts: Float) =>
  Frame.createLit(
    Camera.firstPerson(
      0.0, 9.0, -16.0,
      Angle.radians(0.0), Angle.radians(-0.5), Angle.degrees(60.0)),
    Scene.group([
      Scene.plane() |> Scene.scale(30.0) |> Scene.lit(0.6, 0.6, 0.62),
      m.entities |> List.map(shapeFor) |> Scene.group |> Scene.lit(0.9, 0.9, 0.9),
      marker(0.0, tts, 1.0, 0.3, 0.25),
      marker(1.0, tts, 0.35, 0.5, 1.0),
    ]),
    [
      Light.ambient(0.1, 0.1, 0.13),
      Light.directional(0.5, -1.0, 0.35, 1.0, 0.98, 0.95, 0.85) |> Light.castShadows,
      pointLight(0.0, tts, 1.0, 0.3, 0.25),
      pointLight(1.0, tts, 0.35, 0.5, 1.0),
    ])
`;

const STATS_RE =
  /\[functor-lang\] avg over (\d+) frames: tick ([\d.]+)µs, physics ([\d.]+)µs, draw ([\d.]+)µs \(([\d.]+)% of a 60fps budget\)/;

test(
  "Functor Lang eval holds 60fps with headroom at 100 entities (C6 perf gate)",
  { skip: !e2eEnabled, timeout: 360_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "functor-lang-perf-"));
    const functorLangPath = join(dir, "game.functor");
    writeFileSync(functorLangPath, heavyGame);

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      functorLangPath,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8102),
      headless,
    });

    // Free-run on the wall clock until at least two 300-frame stats windows
    // have been printed — the first window includes warm-up (lazy GL setup,
    // allocator growth), so the gate reads the LAST one. The wait is generous
    // because frame THROUGHPUT (not eval cost — that's what we measure) varies
    // wildly by host: local Apple Silicon prints a window in seconds, shared
    // CI runners took ~55s each for 300 frames, so two windows can approach
    // ~2min. A tight wait spuriously TIMES OUT on slow CI (unrelated to the
    // regression class the gate hunts); 4min clears the slowest observed
    // runner with margin.
    const started = Date.now();
    const lines = await waitFor(
      async () => runner.logs().filter((l) => STATS_RE.test(l)),
      (stats) => stats.length >= 2,
      { timeoutMs: 240_000, intervalMs: 500, description: "two [functor-lang] stats windows" },
    );
    const elapsedS = (Date.now() - started) / 1000;

    const m = lines[lines.length - 1].match(STATS_RE)!;
    const [, frames, tickUs, physicsUs, drawUs, pct] = m;
    const evalUs = Number(tickUs) + Number(drawUs);

    // The perf record — printed so CI logs keep a history of the numbers.
    console.log(
      `[perf] ${ENTITIES} entities, last ${frames}-frame window after ${elapsedS.toFixed(1)}s: ` +
        `tick ${tickUs}µs + draw ${drawUs}µs = ${evalUs.toFixed(1)}µs/frame ` +
        `(physics ${physicsUs}µs; ${pct}% of the 16.6ms budget; ` +
        `gate ${GATE_US.toFixed(0)}µs)`,
    );

    assert.ok(
      evalUs < GATE_US,
      `Functor Lang eval cost ${evalUs.toFixed(1)}µs/frame exceeds the C6 gate of ` +
        `${GATE_US.toFixed(0)}µs (60% of a 60fps frame) — the tree-walker no ` +
        `longer holds; investigate before reaching for the bytecode VM. ` +
        `Line: ${lines[lines.length - 1]}`,
    );
  },
);
