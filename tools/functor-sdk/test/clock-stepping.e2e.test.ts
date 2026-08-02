import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner } from "../src/index.js";

// End-to-end contract for `POST /time` stepping (docs/debug-runtime.md), the
// seam every scripted/LLM-driven session steers the game through:
//
//   1. N advances run exactly N model steps — they ACCUMULATE. A single
//      `pending_step` slot used to let two advances landing between one frame's
//      consumption collapse into one, silently dropping whatever input was
//      injected for the lost step.
//   2. One advance is one observable stepped frame, so a step/observe loop
//      never skips a pose.
//   3. `frames: n` is the same thing in one round trip.
//
//   npm run test:e2e:headless
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

test(
  "clock advances accumulate: N steps requested is N frames run",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const gameDir = join(repoRoot, "examples", "counter");
    await using game = await FunctorRunner.launch({
      gameDir,
      repoRoot,
      functorLangPath: join(gameDir, "game.fun"),
      headless,
    });

    await game.pause();
    // Settle: after a pause the loop stops stepping, so the frame counter
    // stops moving and every later delta is caused only by our advances.
    const settle = async () => {
      let last = (await game.state()).frame;
      for (;;) {
        await new Promise((r) => setTimeout(r, 150));
        const now = (await game.state()).frame;
        if (now === last) return now;
        last = now;
      }
    };

    // One advance == one stepped frame, checked after EVERY step rather than
    // only in total: a run that grouped several steps into one rendered frame
    // would still reach the right total, but is not the property harnesses
    // step/observe against.
    let before = await settle();
    for (let i = 1; i <= 10; i++) {
      await game.step();
      const s = await game.state();
      assert.equal(s.frame, before + i, `advance ${i} ran exactly one frame`);
      assert.equal(s.pending_steps, 0, `advance ${i} left nothing queued`);
    }

    // The batch form runs every requested step too.
    before = await settle();
    await game.stepFrames(50);
    assert.equal(
      (await game.state()).frame - before,
      50,
      "stepFrames(50) must run 50 frames and return once they have",
    );

    // And the clock holds afterwards — a batch parks like a single step.
    const parked = await settle();
    await new Promise((r) => setTimeout(r, 300));
    assert.equal((await game.state()).frame, parked, "clock holds after a batch");
  },
);
