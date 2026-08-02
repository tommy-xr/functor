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
import { decodeWire, fromValueJson, fullWire, summarize } from "./wire-value.ts";

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

// --- The trace's value grammar (`fromValueJson`) --------------------------------
// The paused inspector relays `value_to_json`, not `EffectValue`; the tree
// renders one grammar, so the conversion is what has to be right.

test("converts the trace's value grammar into the tree's shape", () => {
  assert.deepEqual(fromValueJson({ n: 1, ok: true, who: "a" }), {
    Record: [
      ["n", { Number: 1 }],
      ["ok", { Bool: true }],
      ["who", { Text: "a" }],
    ],
  });
  assert.deepEqual(fromValueJson([1, 2]), { List: [{ Number: 1 }, { Number: 2 }] });
  assert.deepEqual(fromValueJson({ $tuple: [1, "x"] }), {
    Tuple: [{ Number: 1 }, { Text: "x" }],
  });
  assert.deepEqual(fromValueJson({ $ctor: "Steer", args: [{ turn: 1 }] }), {
    Variant: ["Steer", [{ Record: [["turn", { Number: 1 }]] }]],
  });
  assert.deepEqual(fromValueJson({ $map: [["a", 1]] }), {
    Map: [[{ Text: "a" }, { Number: 1 }]],
  });
});

test("things that are not data render as themselves, unquoted", () => {
  assert.equal(summarize(fromValueJson({ $fn: "<fn(dt)>" })), "<fn(dt)>");
  assert.equal(summarize(fromValueJson({ $host: "SceneNode" })), "<SceneNode>");
  assert.equal(summarize(fromValueJson({ $number: "NaN" })), "NaN");
  assert.equal(summarize(fromValueJson({ $truncated: "trace budget" })), "… (trace budget)");
});

test("the trace conversion is total on anything the relay could carry", () => {
  const hostile = [
    null,
    undefined,
    7,
    "x",
    [null, undefined],
    { $ctor: 7 },
    { $ctor: "X", args: "not an array" },
    { $tuple: "no" },
    { $map: [null, ["k"], [{ $fn: "f" }, 1]] },
    { nested: { deep: [{ $host: "H" }] } },
  ];
  for (const json of hostile) {
    assert.equal(typeof summarize(fromValueJson(json)), "string", `for ${JSON.stringify(json)}`);
  }
});
