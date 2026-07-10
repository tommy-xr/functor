import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner } from "../src/index.js";

// B6 end-to-end (docs/functor-lang.md): a key press returns `(model, effect)`; the
// runner performs the effect through its real EffectRunner and folds the
// tagger's message back through `update`. Real-world values aren't exact,
// so the assertions are structural: the roll lands in [0, 1), the stamp is
// a plausible wall-clock epoch, and both replaced their sentinels — proof
// the whole chain (pair split → perform → tagger → update) ran in the
// shipped runner. Exact-value determinism is pinned at the unit level
// (fake/replay runners in functor_lang_prelude).
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

const game = `type Msg =
  | Rolled(n: Float)
  | Stamped(t: Float)
let init = { roll: -1.0, at: -1.0 }
let tick = (m, dt, tts) => m
let update = (m, msg) =>
  match msg with
  | Rolled(n) => ({ m with roll: n }, Effect.now(Stamped))
  | Stamped(t) => { m with at: t }
let input = (m, key, isDown) =>
  match isDown with
  | true => (m, Effect.random(Rolled))
  | false => m
let draw = (m, tts) =>
  Frame.create(Camera.lookAt(0.0, 2.0, -6.0, 0.0, 0.0, 0.0), Scene.cube())
`;

function fieldOf(model: string, name: string): number {
  const m = model.match(new RegExp(`${name}:\\s*(-?[\\d.]+)`));
  assert.ok(m, `could not find ${name} in model: ${model}`);
  return Number(m[1]);
}

test(
  "Effect.random/now perform through the runner and fold back via update",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "functor-lang-effects-"));
    const functorLangPath = join(dir, "game.fun");
    writeFileSync(functorLangPath, game);

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      functorLangPath,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8101),
      headless,
    });

    await runner.pause();
    let model = (await runner.state()).model;
    assert.equal(fieldOf(model, "roll"), -1, `sentinel intact: ${model}`);

    // One key press = one Effect.random, whose update chains an Effect.now.
    await runner.key("Space", true);
    await runner.step(0.1);
    model = (await runner.state()).model;
    const roll = fieldOf(model, "roll");
    const at = fieldOf(model, "at");
    assert.ok(roll >= 0 && roll < 1, `roll in [0,1): ${model}`);
    // A real wall clock: after 2020, before 2100.
    assert.ok(at > 1.6e9 && at < 4.1e9, `epoch stamp plausible: ${model}`);
  },
);
