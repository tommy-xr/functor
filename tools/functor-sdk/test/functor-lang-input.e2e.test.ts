import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner } from "../src/index.js";

// C4: key events reach the Functor Lang model through the optional `input` entry
// point — (model, key, isDown) => model, keys as the built-in `Key` module's
// variants (Key.Up, Key.W, …).
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

const GAME = `let init = { presses: 0.0, last: "none" }
let input = (m, key, isDown) => { m with presses: m.presses + 1.0, last: key }
let tick = (m, dt, tts) => m
let draw = (m, tts) =>
  Frame.create(Camera.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
`;

test(
  "key events reach the Functor Lang model via the input entry point",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "functor-lang-input-"));
    const functorLangPath = join(dir, "game.fun");
    writeFileSync(functorLangPath, GAME);

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      functorLangPath,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8094),
      headless,
    });
    await runner.pause();

    const model = async () => (await runner.state()).model;
    assert.match(await model(), /presses: 0/, "no input yet");

    // A press+release is two events; the key crosses as its `Key.*` variant.
    await runner.keyDown("up");
    await runner.keyUp("up");
    await runner.step();
    const after = await model();
    assert.match(after, /presses: 2/, `expected two events in: ${after}`);
    assert.match(after, /last: Key.Up/, `expected the Key variant in: ${after}`);
  },
);
