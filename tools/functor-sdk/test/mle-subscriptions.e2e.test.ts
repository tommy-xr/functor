import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner, waitFor } from "../src/index.js";

// C4b-2 end-to-end (docs/mle.md): subscriptions fire on the global time
// grid and their messages fold through `update`. With the debug clock
// pinned, stepping by EXACTLY one period crosses exactly one grid boundary
// regardless of phase — so every assertion is exact arithmetic:
//
//   - two 1s steps        -> two Beat messages (and Time.millis(1000)
//                            fires in lockstep — unit parity)
//   - one 4s step         -> ONE Beat (a long frame collapses missed
//                            boundaries, the F# Sub.crossedBoundary rule)
//   - reload that ADDS a subscription -> next 1s step fires once, no burst
//
// Opt-in like the other e2e suites: npm run test:e2e[:headless]
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

// Beat (seconds) and Echo (millis) share a 1s period: their counts must
// stay in lockstep, or the unit conversion is wrong.
const subscribed = `type Msg =
  | Beat
  | Echo
let init = { beats: 0.0, echoes: 0.0 }
let tick = (m, dt, tts) => m
let update = (m, msg) =>
  match msg with
  | Beat => { m with beats: m.beats + 1.0 }
  | Echo => { m with echoes: m.echoes + 1.0 }
let subscriptions = (m) => Sub.batch([
  Sub.every(Time.seconds(1.0), Beat),
  Sub.every(Time.millis(1000.0), Echo),
])
let draw = (m, tts) =>
  Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())
`;

// The same game with no subscriptions/update — the reload starting point.
const unsubscribed = `let init = { beats: 0.0, echoes: 0.0 }
let tick = (m, dt, tts) => m
let draw = (m, tts) =>
  Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())
`;

/** A named Float field of the runner's `/state` model (MLE Value display). */
function fieldOf(model: string, name: string): number {
  const m = model.match(new RegExp(`${name}:\\s*(-?[\\d.]+)`));
  assert.ok(m, `could not find ${name} in model: ${model}`);
  return Number(m[1]);
}

test(
  "Sub.every fires on the global time grid and folds through update",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "mle-subs-"));
    const mlePath = join(dir, "game.mle");
    writeFileSync(mlePath, unsubscribed);

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      mlePath,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8099),
      headless,
    });

    await runner.pause();

    // Hot-reload subscriptions IN: the timer window must start from the
    // current frame's edge, not t=0 — otherwise this first step would fire
    // a burst of catch-up Beats for every second since launch.
    writeFileSync(mlePath, subscribed);
    await waitFor(
      async () => runner.logs(),
      (lines) => lines.some((l) => l.includes("hot-reloaded")),
      { timeoutMs: 10_000, description: "hot reload observed in logs" },
    );
    await runner.step(1.0);
    const model = (await runner.state()).model;
    assert.equal(
      fieldOf(model, "beats"),
      1,
      `one period after subscribing should fire exactly one Beat: ${model}`,
    );

    // One step per period -> one message per step, exactly.
    await runner.step(1.0);
    await runner.step(1.0);
    let beats = fieldOf((await runner.state()).model, "beats");
    assert.equal(beats, 3, `two more 1s steps should make 3 beats, got ${beats}`);

    // Time.millis(1000) fires in lockstep with Time.seconds(1.0).
    const echoes = fieldOf((await runner.state()).model, "echoes");
    assert.equal(echoes, beats, `millis/seconds parity: ${echoes} vs ${beats}`);

    // A long frame collapses missed boundaries into ONE firing (the F#
    // crossedBoundary rule: floor comparison, not a counter).
    await runner.step(4.0);
    beats = fieldOf((await runner.state()).model, "beats");
    assert.equal(beats, 4, `a 4s frame fires Beat once, not four times: ${beats}`);
  },
);
