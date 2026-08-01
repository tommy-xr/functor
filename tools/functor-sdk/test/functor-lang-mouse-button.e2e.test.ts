import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner } from "../src/index.js";

// Mouse buttons reach the Functor Lang model three ways, and this pins all:
//   * the edge, through the optional `mouseButton` entry point —
//     (model, button, isDown) => model, buttons as the built-in `Mouse`
//     module's variants (Mouse.Left, Mouse.Right, Mouse.Middle);
//   * the held level, through `sampledInput`'s `snapshot.mouse.buttons`;
//   * deterministic fixed-step edges through `.pressed` / `.released`.
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

// `edges` counts left PRESSES, `heldSteps` counts fixed steps during which the
// left button was held, and `rights` proves the variant is discriminated rather
// than collapsed to "a click happened".
const GAME = `let init = {
  edges: 0.0,
  heldSteps: 0.0,
  sampledPresses: 0.0,
  sampledReleases: 0.0,
  rights: 0.0
}
let mouseButton = (m, button, isDown) =>
  match button with
  | Mouse.Left => (if isDown then { m with edges: m.edges + 1.0 } else m)
  | Mouse.Right => (if isDown then { m with rights: m.rights + 1.0 } else m)
  | _ => m
let sampledInput = (m, snapshot: Input.snapshot) =>
  {
    m with
      heldSteps:
        m.heldSteps + (if snapshot.mouse.buttons.left then 1.0 else 0.0),
      sampledPresses:
        m.sampledPresses + (if snapshot.mouse.pressed.left then 1.0 else 0.0),
      sampledReleases:
        m.sampledReleases + (if snapshot.mouse.released.left then 1.0 else 0.0)
  }
let tick = (m, dt, tts) => m
let draw = (m, tts) =>
  Frame.create(Camera3D.lookAt(Vec3.make(0.0, 2.0, -6.0), Vec3.make(0.0, 0.0, 0.0)), Scene.cube())
`;

/** Read one numeric field out of the (stringly-typed) Debug model. */
function field(model: string, name: string): number {
  const m = model.match(new RegExp(`\\b${name}:\\s*(-?[0-9.]+)`));
  assert.ok(m, `could not find ${name} in model: ${model.slice(0, 200)}`);
  return Number(m[1]);
}

test(
  "mouse hooks, held levels, and sampled edges agree",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "functor-lang-mouse-button-"));
    const functorLangPath = join(dir, "game.fun");
    writeFileSync(functorLangPath, GAME);

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      functorLangPath,
      port: Number(process.env.FUNCTOR_E2E_PORT ?? 8096),
      headless,
    });
    await runner.pause();

    const model = async () => (await runner.state()).model_debug;
    // `buttons` is optional on the type (older runtimes omit it); a runtime
    // that supports this feature at all must report it.
    const buttons = async () => {
      const held = (await runner.state()).input.mouse.buttons;
      assert.ok(held, "/state must report mouse.buttons");
      return held;
    };

    assert.equal(field(await model(), "edges"), 0, "no clicks yet");
    assert.deepEqual(
      await buttons(),
      { left: false, right: false, middle: false },
      "nothing held at rest",
    );

    // One press: exactly one edge, and the level goes hot.
    await runner.mouseDown("left");
    await runner.step();
    assert.equal(field(await model(), "edges"), 1, "the press fired mouseButton once");
    assert.equal(field(await model(), "sampledPresses"), 1);
    assert.equal((await buttons()).left, true, "the runtime reports left held");

    // Held across further steps: no new edges, but the level keeps sampling —
    // this is what makes full-auto fire scriptable.
    const heldAfterFirst = field(await model(), "heldSteps");
    await runner.step();
    await runner.step();
    assert.equal(field(await model(), "edges"), 1, "holding does not re-fire the edge");
    assert.equal(
      field(await model(), "sampledPresses"),
      1,
      "sampled press is consumed after one fixed step",
    );
    assert.ok(
      field(await model(), "heldSteps") > heldAfterFirst,
      "the held level keeps accumulating while the button is down",
    );

    // REGRESSION: moving the cursor must not release the held button. A
    // mouse-move that replaced the whole mouse snapshot would silently clear
    // `buttons`, so a drag would drop the click on replay but not live.
    await runner.mouseMove(42, 7);
    await runner.step();
    const dragged = await runner.state();
    assert.equal(dragged.input.mouse.x, 42, "the move applied");
    assert.equal(dragged.input.mouse.y, 7, "the move applied");
    assert.equal(
      dragged.input.mouse.surface_width,
      800,
      "debug injection retains the runtime-owned logical surface width",
    );
    assert.equal(
      dragged.input.mouse.surface_height,
      600,
      "debug injection retains the runtime-owned logical surface height",
    );
    assert.equal(
      dragged.input.mouse.buttons?.left,
      true,
      "moving the cursor must NOT release a held button",
    );

    // Release: the level clears and stops accumulating.
    await runner.mouseUp("left");
    await runner.step();
    assert.equal((await buttons()).left, false, "the release cleared the level");
    assert.equal(field(await model(), "sampledReleases"), 1);
    const heldAtRelease = field(await model(), "heldSteps");
    await runner.step();
    assert.equal(
      field(await model(), "heldSteps"),
      heldAtRelease,
      "a released button stops sampling",
    );
    assert.equal(
      field(await model(), "sampledReleases"),
      1,
      "sampled release is consumed after one fixed step",
    );

    // A complete click between fixed steps appears in BOTH sampled edge sets,
    // while the final held level remains false.
    await runner.mouseDown("left");
    await runner.mouseUp("left");
    await runner.step();
    assert.equal(field(await model(), "sampledPresses"), 2);
    assert.equal(field(await model(), "sampledReleases"), 2);
    assert.equal((await buttons()).left, false);

    // The right button lands on its own match arm and its own level bit.
    await runner.mouseDown("right");
    await runner.step();
    assert.equal(field(await model(), "rights"), 1, "Mouse.Right is discriminated");
    assert.equal(field(await model(), "edges"), 2, "...and did not count as a left click");
    assert.deepEqual(await buttons(), { left: false, right: true, middle: false });
  },
);
