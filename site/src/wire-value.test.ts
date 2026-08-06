// The wire decoder's contract, headless: it decodes what `Effect.sendMsg`
// actually puts on the wire, and it NEVER throws on anything else.
//
// The second half is the point. This runs inside the host page's rAF loop on
// text a game chose, so a frame that threw would stop the chrono bar — the
// hostile cases below are the regression guard for that.
//
//   node --test site/src/wire-value.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { decodeWire, fullWire } from "./wire-value.ts";

const framed = (value: unknown) => `fun:${JSON.stringify(value)}`;

test("decodes a typed variant into constructor and arguments", () => {
  const row = decodeWire(
    framed({
      Variant: [
        "Steer",
        [{ Record: [["turn", { Number: 1 }], ["thrust", { Bool: true }]] }],
      ],
    })
  );
  assert.equal(row.typed, true);
  assert.equal(row.head, "Steer");
  assert.equal(row.body, "{turn:1, thrust:true}");
});

test("elides deep and wide values, and keeps the full rendering for the tooltip", () => {
  const orb = { Record: [["id", { Number: 3 }]] };
  const row = decodeWire(framed({ Variant: ["Snapshot", [{ List: [orb, orb, orb, orb, orb] }]] }));
  assert.equal(row.head, "Snapshot");
  assert.match(row.body, /…5/);
  assert.match(fullWire(framed({ Variant: ["Snapshot", [{ List: [orb, orb, orb, orb, orb] }]] })), /id:3/);
});

test("plain Effect.send text passes through, marked untyped", () => {
  const row = decodeWire("hello");
  assert.deepEqual(row, { head: "", body: "hello", typed: false, value: null });
  assert.equal(fullWire("hello"), "hello");
});

test("never throws on hostile or skewed frames", () => {
  const hostile = [
    undefined,
    "",
    "fun:",
    "fun:{}",
    "fun:null",
    "fun:7",
    "fun:[1,2,3]",
    "fun:{not json",
    framed({ Variant: [] }),
    framed({ Variant: ["X", "not an array"] }),
    framed({ Variant: [7, [{ Number: 1 }]] }),
    framed({ List: [null, undefined, { Nope: 1 }] }),
    framed({ Record: [null, ["ok"]] }),
    framed({ Map: [[null, null]] }),
    framed({ Number: "not a number" }),
  ];
  for (const wire of hostile) {
    const row = decodeWire(wire as string | undefined);
    assert.equal(typeof row.body, "string", `body for ${String(wire)}`);
    assert.equal(typeof fullWire(wire as string | undefined), "string", `full for ${String(wire)}`);
  }
});

test("caps the tooltip's rendering rather than carrying a whole payload", () => {
  const long = { Text: "x".repeat(50_000) };
  const wire = framed({ Variant: ["Chat", [long]] });
  const row = decodeWire(wire);
  const full = fullWire(wire);
  assert.ok(full.length <= 2001, `full was ${full.length}`);
  assert.ok(row.body.length < 120, `body was ${row.body.length}`);
});
