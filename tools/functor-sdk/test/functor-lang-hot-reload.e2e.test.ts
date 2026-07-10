import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner, waitFor } from "../src/index.js";

// The C3 payoff, end-to-end (docs/functor-lang.md): edit a running Functor Lang game's source
// and assert the model SURVIVED the reload while the behavior CHANGED — with
// the debug clock pinned, the assertion is exact arithmetic, not a race.
// Headless-friendly (no GL needed); opt-in like the other e2e suites:
//
//   npm run test:e2e[:headless]
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

const game = (speed: string) => `let speed = ${speed}
let init = { spin: 0.0 }
let tick = (m, dt, tts) => { m with spin: m.spin + dt * speed }
let draw = (m, tts) =>
  Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())
`;

/** The spin field of the runner's `/state` model (Functor Lang Value display). */
function spinOf(model: string): number {
  const m = model.match(/spin:\s*(-?[\d.]+)/);
  assert.ok(m, `could not find spin in model: ${model}`);
  return Number(m[1]);
}

test(
  "editing a running .fun game preserves the model and rebinds behavior",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "functor-lang-reload-"));
    const functorLangPath = join(dir, "game.fun");
    writeFileSync(functorLangPath, game("1.0"));

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      functorLangPath,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8093),
      headless,
    });

    // Pin the clock: every state change below is an explicit, exact step.
    // (The game free-runs on the wall clock between launch and pause, so all
    // assertions are RELATIVE to the pinned baseline.)
    await runner.pause();
    const base = spinOf((await runner.state()).model);
    const DT = 0.1;
    for (let i = 0; i < 3; i++) await runner.step(DT);
    const before = spinOf((await runner.state()).model);
    assert.ok(
      Math.abs(before - base - 3 * DT) < 1e-4,
      `three steps at speed 1.0 should add ~0.3 to ${base}, got ${before}`,
    );

    // Edit the SOURCE of the running game: reverse and amplify the speed.
    writeFileSync(functorLangPath, game("-5.0"));
    // The reload is polled once per frame; in pinned-clock mode frames keep
    // running (the loop isn't blocked), so give it a moment, then step once.
    await waitFor(
      async () => runner.logs(),
      (lines) => lines.some((l) => l.includes("hot-reloaded")),
      { timeoutMs: 10_000, description: "hot reload observed in logs" },
    );
    await runner.step(DT);
    const after = spinOf((await runner.state()).model);

    // THE assertion: the new spin is the OLD value plus one step of the NEW
    // behavior — state survived the edit AND the edit took effect.
    assert.ok(
      Math.abs(after - (before + DT * -5.0)) < 1e-4,
      `expected ${before} + ${DT}*-5 = ${before + DT * -5}, got ${after} ` +
        `(a reset-to-init would give ${DT * -5})`,
    );

    // The reload was fast: the runner logs its re-parse+reload latency.
    const line = runner.logs().find((l) => l.includes("hot-reloaded"));
    assert.ok(line, "reload log line present");
    const ms = Number(line!.match(/in ([\d.]+)ms/)?.[1]);
    assert.ok(ms < 100, `reload should be well under 100ms, took ${ms}ms`);

    // A BROKEN edit fails loud but keeps the old program running.
    writeFileSync(functorLangPath, "let broken = ((");
    await waitFor(
      async () => runner.logs(),
      (lines) => lines.some((l) => l.includes("reload failed")),
      { timeoutMs: 10_000, description: "broken reload reported" },
    );
    await runner.step(DT);
    const still = spinOf((await runner.state()).model);
    assert.ok(
      Math.abs(still - (after + DT * -5.0)) < 1e-4,
      `the old program should keep running after a broken edit, got ${still}`,
    );
  },
);
