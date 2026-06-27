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
// Headless (no GL window) is the CI path; capture is unavailable there.
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

/** Whether the hello game's model shows `held.up` set. This reads the (stringly-
 * typed) Debug model on purpose — as an independent check that the injected key
 * reaches the *game*, not just the runtime's own input snapshot (which is mutated
 * in the same handler as the game key event). */
function gameSawUp(model: string): boolean {
  const m = model.match(/HeldKeys\s*\{\s*up:\s*(true|false)/);
  assert.ok(m, `could not find HeldKeys.up in model: ${model.slice(0, 200)}`);
  return m[1] === "true";
}

test(
  "injected key is reflected in both the runtime input state and the game",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    await using game = await FunctorRunner.launch({
      gameDir: join(repoRoot, "examples", "hello"),
      repoRoot,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8090),
      headless,
    });

    // Pin the clock so the only thing that changes state is what we inject.
    await game.pause();

    // Baseline: nothing held — structured snapshot AND game model.
    assert.equal(await game.isKeyDown("Up"), false, "Up should start released");
    assert.equal(gameSawUp((await game.state()).model), false, "game should start with up released");

    // Positive: press 'up', step a frame.
    await game.keyDown("up");
    await game.step();
    assert.equal(await game.isKeyDown("Up"), true, "runtime should report Up held");
    assert.equal(
      gameSawUp((await game.state()).model),
      true,
      "game should see up held (regression: input not reaching the game)",
    );

    // Negative: release 'up', step.
    await game.keyUp("up");
    await game.step();
    assert.equal(await game.isKeyDown("Up"), false, "runtime should report Up released");
    assert.equal(gameSawUp((await game.state()).model), false, "game should see up released");

    // The render path produces a valid PNG — windowed only (headless has no GL).
    if (!headless) {
      const png = await game.capture();
      assert.ok(png.length > 0, "capture should return bytes");
      assert.ok(
        png.subarray(0, 8).equals(PNG_MAGIC),
        "capture should be a PNG (magic bytes)",
      );
    }
  },
);
