import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner } from "../src/index.js";

// End-to-end against a real functor-runner. Requires the runner binary and the
// `hello` game dylib to be built, and a display to open the GL window, so it's
// opt-in:
//
//   npm run test:e2e        (or FUNCTOR_E2E=1 node --test dist/test/)
const e2eEnabled = process.env.FUNCTOR_E2E === "1";

const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

/** Best-effort read of `held.up` from the model's Debug string, e.g.
 * `... held: HeldKeys { up: true, down: false, ... } ...`. Whitespace-tolerant,
 * but still coupled to the Rust Debug layout (model is stringly-typed today —
 * see types.ts; a structured snapshot would let us drop this regex). */
function heldUp(model: string): boolean {
  const m = model.match(/HeldKeys\s*\{\s*up:\s*(true|false)/);
  assert.ok(m, `could not find HeldKeys.up in model: ${model.slice(0, 200)}`);
  return m[1] === "true";
}

test(
  "injected key is reflected in game state across a manual step",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    await using game = await FunctorRunner.launch({
      gameDir: join(repoRoot, "examples", "hello"),
      repoRoot,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8090),
    });

    // Pin the clock so the only thing that changes state is what we inject.
    await game.pause();

    // Baseline: nothing held.
    await game.step();
    assert.equal(heldUp((await game.state()).model), false, "up should start released");

    // Positive: press 'up', step a frame, observe held.up flip true.
    await game.keyDown("up");
    await game.step();
    assert.equal(
      heldUp((await game.state()).model),
      true,
      "up should be held after keyDown + step (regression: input not reaching the game)",
    );

    // Negative: release 'up', step, observe it flip back.
    await game.keyUp("up");
    await game.step();
    assert.equal(heldUp((await game.state()).model), false, "up should release after keyUp + step");

    // The render path produces a valid PNG.
    const png = await game.capture();
    assert.ok(png.length > 0, "capture should return bytes");
    assert.ok(
      png.subarray(0, 8).equals(PNG_MAGIC),
      "capture should be a PNG (magic bytes)",
    );
  },
);
