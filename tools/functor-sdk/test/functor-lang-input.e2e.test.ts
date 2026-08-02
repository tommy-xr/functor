import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner } from "../src/index.js";

// Key events preserve the optional `input` hook while the sampled snapshot
// exposes deterministic pressed/released sets for one fixed step.
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

const GAME = `let init = {
  events: 0.0,
  sampledPressed: 0.0,
  sampledReleased: 0.0,
  heldSteps: 0.0,
  last: "none"
}
let input = (m, key, isDown) => { m with events: m.events + 1.0, last: key }
let has = (key, keys) => keys |> List.any((candidate) => candidate == key)
let sampledInput = (m, snapshot: Input.snapshot) => {
  m with
    sampledPressed:
      m.sampledPressed + (if has(Key.Up, snapshot.pressedKeys) then 1.0 else 0.0),
    sampledReleased:
      m.sampledReleased + (if has(Key.Up, snapshot.releasedKeys) then 1.0 else 0.0),
    heldSteps:
      m.heldSteps + (if has(Key.Up, snapshot.heldKeys) then 1.0 else 0.0)
}
let tick = (m, dt, tts) => m
let draw = (m, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
`;

function field(model: string, name: string): number {
  const match = model.match(new RegExp(`\\b${name}:\\s*(-?[0-9.]+)`));
  assert.ok(match, `could not find ${name} in model: ${model.slice(0, 240)}`);
  return Number(match[1]);
}

test(
  "key hooks and sampled pressed/released edges agree",
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
      headless,
    });
    await runner.pause();

    const model = async () => (await runner.state()).model_debug;
    assert.equal(field(await model(), "events"), 0, "no input yet");

    // A quick tap before one fixed step is both sampled edges, with the final
    // held level released. The legacy hook still receives both raw events.
    await runner.keyDown("up");
    await runner.keyUp("up");
    await runner.step();
    const after = await model();
    assert.equal(field(after, "events"), 2, `expected two events in: ${after}`);
    assert.match(after, /last: Key.Up/, `expected the Key variant in: ${after}`);
    assert.equal(field(after, "sampledPressed"), 1);
    assert.equal(field(after, "sampledReleased"), 1);
    assert.equal(field(after, "heldSteps"), 0, "tap ended released");

    // Consumed edges do not repeat on a later fixed step.
    await runner.step();
    const coasted = await model();
    assert.equal(field(coasted, "sampledPressed"), 1);
    assert.equal(field(coasted, "sampledReleased"), 1);

    // Repeated down injection preserves the legacy input-hook behavior (two
    // events), but only one physical up→down transition enters pressedKeys.
    await runner.keyDown("up");
    await runner.keyDown("up");
    await runner.step();
    const repeated = await model();
    assert.equal(field(repeated, "events"), 4);
    assert.equal(field(repeated, "sampledPressed"), 2);
    assert.equal(field(repeated, "sampledReleased"), 1);
    assert.equal(field(repeated, "heldSteps"), 1);

    await runner.keyUp("up");
    await runner.step();
    const released = await model();
    assert.equal(field(released, "events"), 5);
    assert.equal(field(released, "sampledReleased"), 2);
  },
);
