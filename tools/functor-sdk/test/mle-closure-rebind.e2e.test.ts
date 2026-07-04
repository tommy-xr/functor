import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner, waitFor } from "../src/index.js";

// B5 part 2 end-to-end (docs/mle.md): a closure STORED IN THE MODEL rebinds
// across a hot reload — it adopts the edited body while keeping its captured
// environment. The model holds `vel = makeSpin(2.0)` (a closure capturing
// k = 2); editing makeSpin's inner lambda changes how every already-spawned
// vel behaves, without resetting anything. Pinned clock ⇒ the assertions are
// exact arithmetic. Opt-in like the other e2e suites:
//
//   npm run test:e2e[:headless]
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

const game = (body: string) => `let makeSpin = (k) => (dt) => ${body}
let init = { vel: makeSpin(2.0), x: 0.0 }
let tick = (m, dt, tts) => { m with x: m.x + m.vel(dt) }
let draw = (m, tts) =>
  Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())
`;

function xOf(model: string): number {
  const m = model.match(/x:\s*(-?[\d.]+)/);
  assert.ok(m, `could not find x in model: ${model}`);
  return Number(m[1]);
}

test(
  "a closure stored in the model adopts an edited body and keeps its captured env",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "mle-rebind-"));
    const mlePath = join(dir, "game.mle");
    writeFileSync(mlePath, game("k * dt"));

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      mlePath,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8100),
      headless,
    });

    await runner.pause();
    const base = xOf((await runner.state()).model);
    const DT = 0.25;
    await runner.step(DT);
    await runner.step(DT);
    const before = xOf((await runner.state()).model);
    // The stored closure is makeSpin(2.0): each step adds k*dt = 0.5.
    assert.ok(
      Math.abs(before - base - 2 * (2.0 * DT)) < 1e-4,
      `two steps of k*dt should add 1.0 to ${base}, got ${before}`,
    );

    // Edit the INNER lambda's body — the code the stored closure points at.
    writeFileSync(mlePath, game("k * dt * 10.0"));
    await waitFor(
      async () => runner.logs(),
      (lines) => lines.some((l) => l.includes("hot-reloaded")),
      { timeoutMs: 10_000, description: "hot reload observed in logs" },
    );
    // The reload log reports the rebind.
    const line = runner.logs().find((l) => l.includes("hot-reloaded"));
    assert.ok(
      line!.includes("1 stored closure(s) rebound"),
      `reload should report the rebind: ${line}`,
    );

    await runner.step(DT);
    const after = xOf((await runner.state()).model);
    // THE assertion: new body (k*dt*10) with the OLD captured k = 2 —
    // and the old x, since the model itself survived too. (The edit must
    // stay dt-proportional: paused frames still tick with dt = 0, so a
    // constant term would accumulate while we wait for the reload.)
    assert.ok(
      Math.abs(after - (before + 2.0 * DT * 10.0)) < 1e-4,
      `expected ${before} + ${2.0 * DT * 10.0}, got ${after} ` +
        `(old-body would give ${before + 2.0 * DT})`,
    );
  },
);
