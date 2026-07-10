import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner, waitFor } from "../src/index.js";

// B8 part 1, end-to-end (docs/functor-lang.md): a multi-file Functor Lang project where the
// entry pulls its speed from a SIBLING module. Editing the sibling — not the
// entry — must hot-reload with the model preserved, exactly like editing the
// entry does (the producer watches every project file). With the debug clock
// pinned, the assertion is exact arithmetic, not a race.
// Headless-friendly (no GL needed); opt-in like the other e2e suites:
//
//   npm run test:e2e[:headless]
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

// The entry never changes: it reads `Config.speed` from the sibling module
// (qualified access, no `open` needed).
const game = `let init = { spin: 0.0 }
let tick = (m, dt, tts) => { m with spin: m.spin + dt * Config.speed }
let draw = (m, tts) =>
  Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())
`;

const config = (speed: string) => `let speed = ${speed}
`;

/** The spin field of the runner's `/state` model (Functor Lang Value display). */
function spinOf(model: string): number {
  const m = model.match(/spin:\s*(-?[\d.]+)/);
  assert.ok(m, `could not find spin in model: ${model}`);
  return Number(m[1]);
}

test(
  "editing a NON-entry module hot-reloads with the model preserved",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "functor-lang-modules-"));
    const entryPath = join(dir, "game.functor");
    const configPath = join(dir, "config.functor");
    writeFileSync(entryPath, game);
    writeFileSync(configPath, config("1.0"));

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      functorLangPath: entryPath,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8102),
      headless,
    });

    // Pin the clock: every state change below is an explicit, exact step.
    await runner.pause();
    const base = spinOf((await runner.state()).model);
    const DT = 0.1;
    for (let i = 0; i < 3; i++) await runner.step(DT);
    const before = spinOf((await runner.state()).model);
    assert.ok(
      Math.abs(before - base - 3 * DT) < 1e-4,
      `three steps at Config.speed 1.0 should add ~0.3 to ${base}, got ${before}`,
    );

    // Edit the SIBLING module only — the entry file is untouched.
    writeFileSync(configPath, config("-5.0"));
    await waitFor(
      async () => runner.logs(),
      (lines) => lines.some((l) => l.includes("hot-reloaded")),
      { timeoutMs: 10_000, description: "sibling edit observed as hot reload" },
    );
    await runner.step(DT);
    const after = spinOf((await runner.state()).model);

    // THE assertion: the new spin is the OLD value plus one step of the NEW
    // sibling constant — state survived a non-entry edit AND it took effect.
    assert.ok(
      Math.abs(after - (before + DT * -5.0)) < 1e-4,
      `expected ${before} + ${DT}*-5 = ${before + DT * -5}, got ${after} ` +
        `(a reset-to-init would give ${DT * -5})`,
    );

    // A BROKEN sibling edit fails loud but keeps the old program running.
    writeFileSync(configPath, "let speed = ((");
    await waitFor(
      async () => runner.logs(),
      (lines) => lines.some((l) => l.includes("reload failed")),
      { timeoutMs: 10_000, description: "broken sibling reload reported" },
    );
    await runner.step(DT);
    const still = spinOf((await runner.state()).model);
    assert.ok(
      Math.abs(still - (after + DT * -5.0)) < 1e-4,
      `the old program should keep running after a broken sibling edit, got ${still}`,
    );

    // Fixing the sibling reloads again, and the model STILL survives —
    // three generations of state across two sibling edits and one break.
    writeFileSync(configPath, config("2.0"));
    await waitFor(
      async () => runner.logs(),
      (lines) =>
        lines.filter((l) => l.includes("hot-reloaded")).length >= 2,
      { timeoutMs: 10_000, description: "fixed sibling reloaded" },
    );
    await runner.step(DT);
    const fixed = spinOf((await runner.state()).model);
    assert.ok(
      Math.abs(fixed - (still + DT * 2.0)) < 1e-4,
      `expected ${still} + ${DT}*2 after the fix, got ${fixed}`,
    );
  },
);
