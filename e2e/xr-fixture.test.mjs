// Unit tests for the XR fixture format: `node --test e2e/xr-fixture.test.mjs`.
import assert from "node:assert/strict";
import { test } from "node:test";
import { decode, diff, encode, keyName, patch, startTime } from "./xr-fixture.mjs";

const DT = 0.016666668;
const pose = (x) => ({ position: [x, 1, -0.5], orientation: [0, 0, 0, 1] });
const hand = (x, extra = {}) => ({ active: true, grip: pose(x), aim: pose(x), squeeze: 0, ...extra });
const xr = (x, right = {}) => ({ head: pose(0), left: hand(-x), right: hand(x, right) });
// Each press is held until its release (a release in the same step wins).
let level = new Set();
const input = (sample, pressed = [], released = []) => {
  level = new Set([...level, ...pressed].filter((k) => !released.includes(k)));
  return { held_keys: [...level], pressed_keys: pressed, released_keys: released, xr: sample };
};
// `GET /input-log` steps from frame 10, on a clock at 0.5 s.
const log = (inputs, t = Math.fround(0.5)) =>
  inputs.map((input, i) => {
    t = Math.fround(t + Math.fround(DT));
    return { frame: 10 + i, tts: t, dts: DT, input };
  });

test("diff keeps only changed fields, nested, and patch inverts it", () => {
  const a = xr(0.2);
  const b = xr(0.2, { squeeze: 1 });
  assert.deepEqual(diff(a, b), { right: { squeeze: 1 } });
  assert.equal(diff(a, structuredClone(a)), undefined);
  assert.deepEqual(patch(a, diff(a, b)), b);
  // Losing a pose (object → null) and getting it back (null → object).
  const lost = xr(0.2, { grip: null, aim: null, active: false });
  assert.deepEqual(diff(a, lost), { right: { active: false, grip: null, aim: null } });
  assert.deepEqual(patch(lost, diff(lost, a)), a);
  assert.equal(patch(a, null), null);
  assert.deepEqual(patch(null, a), a);
});

test("keyName spells snapshot keys as POST /input takes them", () => {
  assert.equal(keyName("Num1"), "1");
  assert.equal(keyName("Enter"), "Enter");
  assert.equal(keyName("A"), "A");
});

test("startTime finds the f32 clock one step before", () => {
  for (const t0 of [0, Math.fround(0.1), Math.fround(123.456), Math.fround(4000.25)]) {
    const t1 = Math.fround(t0 + Math.fround(DT));
    assert.equal(Math.fround(startTime(t1, DT) + Math.fround(DT)), t1);
  }
  // The JSON spelling of an f32 is enough.
  assert.equal(startTime(0.016666668, 0.016666668), 0);
});

test("encode → decode reproduces every step's sample and key edges", () => {
  const inputs = [
    input(xr(0.2), ["Num1"]),
    input(xr(0.2), [], ["Num1"]),
    input(xr(0.25, { squeeze: 1 })),
    input(null),
    input(xr(0.3, { grip: null, aim: null })),
  ];
  const fixture = encode(log(inputs), { marks: { grabbed: 12 }, source: { runtime: "test" } });
  assert.equal(fixture.dt, DT);
  assert.equal(Math.fround(fixture.t0 + Math.fround(DT)), log(inputs)[0].tts);
  assert.deepEqual(fixture.marks, { grabbed: 2 });
  assert.deepEqual(fixture.steps[1], { up: ["1"] });
  assert.deepEqual(fixture.steps[3], { xr: null });
  const back = decode(JSON.parse(JSON.stringify(fixture)));
  assert.deepEqual(
    back.steps.map((s) => s.xr),
    inputs.map((i) => i.xr),
  );
  assert.deepEqual(back.steps[0].down, ["1"]);
  assert.deepEqual(back.marks, { grabbed: 2 });
});

test("encode refuses logs it can't replay", () => {
  const steps = log([input(xr(0.2)), input(xr(0.3))]);
  assert.throws(() => encode([]), /no steps/);
  assert.throws(() => encode([steps[0], { ...steps[1], frame: 12 }]), /skip/);
  assert.throws(() => encode([steps[0], { ...steps[1], dts: 0.02 }]), /dt/);
  assert.throws(() => encode([steps[0], { ...steps[1], input: null }]), /sampledInput/);
  assert.throws(() => encode([steps[0], { ...steps[1], tts: 10 }]), /clock jumped/);
  // A key held from before the recording, or released and pressed again in
  // one step, can't be replayed as press-then-release edges.
  const holding = { ...input(xr(0.2)), held_keys: ["W"] };
  assert.throws(() => encode([{ ...steps[0], input: holding }]), /holds W/);
  const repressed = { ...input(xr(0.2), ["W"], ["W"]), held_keys: ["W"] };
  assert.throws(() => encode([steps[0], { ...steps[1], input: repressed }]), /holds W/);
  // A field the next sample drops can't be written as a patch.
  const { head, ...headless } = xr(0.3);
  assert.throws(() => encode([steps[0], { ...steps[1], input: input(headless) }]), /round-trip/);
});

test("decode validates the header and steps", () => {
  const good = encode(log([input(xr(0.2)), input(xr(0.3))]));
  const bad = (change) => () => decode({ ...structuredClone(good), ...change });
  assert.throws(bad({ format: "other" }), /not an XR fixture/);
  assert.throws(bad({ version: 2 }), /version 2/);
  assert.throws(bad({ dt: 0 }), /bad dt/);
  assert.throws(bad({ t0: "0" }), /bad t0/);
  assert.throws(bad({ steps: [] }), /no steps/);
  assert.throws(bad({ steps: [{}, {}] }), /whole xr sample/);
  assert.throws(bad({ steps: [good.steps[0], { xr: {}, triger: 1 }] }), /unknown field triger/);
  assert.throws(bad({ marks: { late: 2 } }), /mark late/);
});
