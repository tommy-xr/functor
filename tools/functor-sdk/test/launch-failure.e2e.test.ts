import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner } from "../src/index.js";

// Verifies a launch failure surfaces a useful error rather than hanging.
const e2eEnabled = process.env.FUNCTOR_E2E === "1";

test(
  "launching against a missing game source rejects with an actionable error",
  { skip: !e2eEnabled, timeout: 30_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    await assert.rejects(
      FunctorRunner.launch({
        gameDir: join(repoRoot, "examples", "mle-hello-gltf"),
        repoRoot,
        mlePath: join(repoRoot, "does", "not", "exist.mle"),
        port: Number(process.env.FUNCTOR_E2E_PORT ?? 8091),
      }),
      (error: Error) => {
        assert.match(error.message, /mle game source not found/);
        assert.match(error.message, /Build it first/);
        return true;
      },
    );
  },
);
