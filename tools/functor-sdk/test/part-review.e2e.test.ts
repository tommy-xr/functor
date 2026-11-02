import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { findRepoRoot, FunctorRunner } from "../src/index.js";

// Review notes in the default part viewer: `V` shows the PartViews sheet, a
// click on a tile pins a note where the pointer meets the part, typing into
// the notes panel's field sets its text — and the note is plain data in
// `/state`'s structured model, at the expected point in part space (mm).
const e2eEnabled = process.env.FUNCTOR_E2E === "1";
const headless = process.env.FUNCTOR_E2E_HEADLESS === "1";

// A part-only file runs in the default part viewer. 10 × 20 × 30 mm on z = 0.
const GAME = `let part = Solid.box(10.0, 20.0, 30.0) |> Solid.translate(Vec3.make(0.0, 0.0, 15.0))
`;

type Point = { x: number; y: number; z: number };
type Note = {
  id: number;
  part: string;
  view: string;
  point: Point;
  normal: Point;
  text: string;
};

test(
  "a click on the sheet pins a note at the part-space hit, readable from /state",
  { skip: !e2eEnabled, timeout: 120_000 },
  async () => {
    const repoRoot = findRepoRoot(process.cwd());
    assert.ok(repoRoot, "must run from within the functor workspace");

    const dir = mkdtempSync(join(tmpdir(), "functor-part-review-"));
    const functorLangPath = join(dir, "part.fun");
    writeFileSync(
      join(dir, "functor.json"),
      JSON.stringify({
        language: "functor-lang",
        entry: "part.fun",
        cursor: "visible",
      }),
    );
    writeFileSync(functorLangPath, GAME);

    await using runner = await FunctorRunner.launch({
      gameDir: dir,
      repoRoot,
      functorLangPath,
      headless,
    });
    await runner.pause();
    await runner.pressKey("v");

    const notes = async (): Promise<Note[]> => {
      const model = (await runner.state()).model as {
        sheet: boolean;
        review: { notes: Note[] };
      };
      assert.equal(model.sheet, true, "V shows the sheet");
      return model.review.notes;
    };
    assert.deepEqual(await notes(), []);

    // The sheet's ortho camera shows 3.2 units top to bottom around the
    // centre; the FRONT tile is the middle one, 1 × 1 unit. Aim about 0.1
    // right and 0.1 down of its centre (whole points: `/input` takes ints).
    const { surface_width: w, surface_height: h } = (await runner.state()).input
      .mouse;
    assert.ok(w > 0 && h > 0, `a surface to click on: ${w} × ${h}`);
    const perUnit = h / 3.2;
    const [px, py] = [
      Math.round(w / 2 + 0.1 * perUnit),
      Math.round(h / 2 + 0.1 * perUnit),
    ];
    const [sx, sy] = [(px - w / 2) / perUnit, (h / 2 - py) / perUnit];
    await runner.mouseMove(px, py);
    await runner.mouseDown("left");
    await runner.step();
    await runner.mouseUp("left");
    await runner.step();

    // FRONT frames the 30 mm height: half height 15 × 1.25 = 18.75 mm, eye
    // 8% of it under the centre (z 15 − 1.5); a tile spans two half heights.
    // So 0.1 right, 0.1 down ≈ x 3.75, z 13.5 − 3.75, on the face y = −10.
    const [note] = await notes();
    assert.ok(note, "the click pinned a note");
    assert.equal(note.part, "part");
    assert.equal(note.view, "FRONT");
    for (const [axis, want] of [
      ["x", 2 * sx * 18.75],
      ["y", -10],
      ["z", 13.5 + 2 * sy * 18.75],
    ] as const) {
      assert.ok(
        Math.abs(note.point[axis] - want) <= 0.5,
        `${axis}: ${note.point[axis]} vs ${want}`,
      );
    }
    assert.deepEqual(note.normal, { x: 0, y: -1, z: 0 });

    // The selected note's text field is the panel's first widget (slot 0).
    await runner.input({
      type: "ui_event",
      slot: 0,
      kind: { TextChanged: "make this edge rounder" },
    });
    await runner.step();
    assert.equal((await notes())[0].text, "make this edge rounder");
  },
);
