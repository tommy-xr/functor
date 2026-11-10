// XR fixtures: a recorded session's XR input, replayable step by step.
//
// Recording reads the runtime's input log (`GET /input-log`, the sampled
// snapshot each fixed step consumed) from a desktop or Quest debug server;
// playback feeds each step back through `POST /input` + one pinned
// `/time advance`, so a fresh process sees the same input (and, from the
// same start state, ends bit-identically).
//
// Format (version 1), JSON:
//
//   { "format": "functor-xr-fixture", "version": 1,
//     "dt": 0.016666668,        // every step's delta (s); the fixed step
//     "t0": 0,                  // the game clock before the first step (s)
//     "marks": { "release": 51 },   // optional: named step indices
//     "source": { ... },        // optional: where it was recorded
//     "steps": [ { "xr": {...}, "down": ["1"], "up": [] }, {}, ... ] }
//
// A step's `xr` is the `Input.snapshot.xr` sample (`GET /state` shape:
// metres and [x, y, z, w] quaternions in the floor-locked rig frame, +X
// right, +Y up, -Z forward), written as a patch against the previous step's:
// only the fields that changed, nested; the first step's is whole; `null`
// means no XR domain that step. `down`/`up` are the key presses/releases
// that step saw, as `POST /input` key names. Omitted fields mean no change.
// Mouse, gamepad and touch aren't carried. Numbers are the runtime's own f32
// spellings, so the fixture is lossless.

export const FORMAT = "functor-xr-fixture";
export const VERSION = 1;

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/** The fields of `next` that differ from `prev`, nested; `undefined` if none. */
export function diff(prev, next) {
  if (same(prev, next)) return undefined;
  if (!isObject(prev) || !isObject(next)) return next;
  const out = {};
  for (const k of Object.keys(next)) {
    const d = diff(prev[k], next[k]);
    if (d !== undefined) out[k] = d;
  }
  return out;
}

/** `prev` with `delta` applied (the inverse of `diff`). */
export function patch(prev, delta) {
  if (delta === undefined) return prev;
  if (!isObject(prev) || !isObject(delta)) return delta;
  const out = { ...prev };
  for (const [k, v] of Object.entries(delta)) out[k] = patch(prev[k], v);
  return out;
}

// `Num1` (the snapshot's spelling) → `1` (what `POST /input` takes).
export const keyName = (k) => k.replace(/^Num(\d)$/, "$1");

const f32 = new Float32Array(1);
const i32 = new Int32Array(f32.buffer);
const ulps = (x, n) => {
  f32[0] = x;
  i32[0] += x >= 0 ? n : -n;
  return f32[0];
};

/**
 * An f32 clock `t0` from which one f32 step of `dt` lands exactly on `t1`.
 * There may be several; any replays alike (the clock is `t1` after the step).
 */
export function startTime(tts, dts) {
  // JSON carries the f32s' shortest spellings; compare the f32s themselves.
  const [t1, dt] = [Math.fround(tts), Math.fround(dts)];
  const guess = Math.fround(t1 - dt);
  for (const n of [0, -1, 1, -2, 2]) {
    const t0 = ulps(guess, n);
    if (Math.fround(t0 + dt) === t1) return t0;
  }
  throw new Error(`no f32 clock steps by ${dt} onto ${t1}`);
}

/**
 * A fixture from `GET /input-log` steps (consecutive frames, one fixed `dt`).
 * `marks` names state frames (`{ release: 412 }`); they're stored as indices.
 */
export function encode(logSteps, { marks = {}, source } = {}) {
  if (logSteps.length === 0) throw new Error("no steps to record");
  const dt = logSteps[0].dts;
  const first = logSteps[0].frame;
  // Replay rebuilds the clock as t0 + dt + dt ... and key levels from the
  // edges alone (press, then release, from nothing held): refuse a log that
  // didn't run that way.
  let held = new Set();
  const steps = logSteps.map((s, i) => {
    if (s.frame !== first + i) throw new Error(`steps skip from frame ${first + i - 1} to ${s.frame}`);
    if (s.dts !== dt) throw new Error(`frame ${s.frame} ran dt ${s.dts}, not ${dt}`);
    const tts = i && Math.fround(Math.fround(logSteps[i - 1].tts) + Math.fround(dt));
    if (i && tts !== Math.fround(s.tts)) throw new Error(`the clock jumped at frame ${s.frame} (${s.tts}, not ${tts})`);
    if (!s.input) throw new Error(`frame ${s.frame} has no sampled input: does the game define sampledInput?`);
    held = new Set([...held, ...s.input.pressed_keys].filter((k) => !s.input.released_keys.includes(k)));
    if (!same([...held].sort(), [...s.input.held_keys].sort())) {
      throw new Error(`frame ${s.frame} holds ${s.input.held_keys}, which its key edges don't replay to`);
    }
    return s.input;
  });
  const fixture = {
    format: FORMAT,
    version: VERSION,
    dt,
    t0: startTime(logSteps[0].tts, dt),
    ...(Object.keys(marks).length && {
      marks: Object.fromEntries(Object.entries(marks).map(([k, frame]) => [k, frame - first])),
    }),
    ...(source && { source }),
    steps: steps.map((input, i) => {
      const step = {};
      const xr = input.xr ?? null;
      const d = i === 0 ? xr : diff(steps[i - 1].xr ?? null, xr);
      if (d !== undefined) step.xr = d;
      if (input.pressed_keys.length) step.down = input.pressed_keys.map(keyName);
      if (input.released_keys.length) step.up = input.released_keys.map(keyName);
      return step;
    }),
  };
  // A shape `diff` can't express (a dropped field) must fail here, not on replay.
  decode(fixture).steps.forEach((s, i) => {
    if (!same(s.xr, steps[i].xr ?? null)) throw new Error(`step ${i}'s xr doesn't round-trip`);
  });
  return fixture;
}

/** Validate a fixture and expand it: `{ dt, t0, marks, steps: [{ xr, down, up }] }`. */
export function decode(fixture) {
  if (fixture?.format !== FORMAT) throw new Error(`not an XR fixture (format ${JSON.stringify(fixture?.format)})`);
  if (fixture.version !== VERSION) throw new Error(`XR fixture version ${fixture.version}; this reader takes ${VERSION}`);
  if (!(fixture.dt > 0)) throw new Error(`bad dt ${fixture.dt}`);
  if (typeof fixture.t0 !== "number") throw new Error(`bad t0 ${fixture.t0}`);
  if (!Array.isArray(fixture.steps) || fixture.steps.length === 0) throw new Error("no steps");
  if (!("xr" in fixture.steps[0])) throw new Error("the first step must carry a whole xr sample");
  const marks = fixture.marks ?? {};
  for (const [k, i] of Object.entries(marks)) {
    if (!Number.isInteger(i) || i < 0 || i >= fixture.steps.length) throw new Error(`mark ${k} is not a step: ${i}`);
  }
  let xr = null;
  const steps = fixture.steps.map((s, i) => {
    for (const k of Object.keys(s)) {
      if (!["xr", "down", "up"].includes(k)) throw new Error(`step ${i}: unknown field ${k}`);
    }
    xr = patch(xr, s.xr);
    return { xr, down: s.down ?? [], up: s.up ?? [] };
  });
  return { dt: fixture.dt, t0: fixture.t0, marks, steps };
}

/** `GET /input-log` steps from state frame `from` on; throws if any were lost. */
export async function readLog(rt, from) {
  const log = await rt.get(`/input-log?from=${from}`);
  if (log.steps.length && log.steps[0].frame !== from) {
    throw new Error(`steps ${from}..${log.steps[0].frame - 1} already left the runtime's history`);
  }
  return log;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * Record a live (unpinned) session attached over `rt` (`{ get, post }`):
 * restart the game from `init` (`POST /load-project` with its own sources),
 * tap `resetKey`, poll the input log until `stopped` resolves, then read it
 * once more. The steps start at the reset's, so a replay from a fresh
 * launch starts from the same state. Returns `GET /input-log` steps.
 */
export async function recordLive(rt, { resetKey, stopped }) {
  const { protocol_version: version } = await rt.get("/");
  if (version < 33) throw new Error(`the runtime speaks debug protocol v${version}; recording needs v33 (GET /input-log)`);
  // A reset alone keeps state from before it (vr-lab's ball generation).
  await rt.post("/load-project", await rt.get("/project"));
  const deadline = Date.now() + 10000;
  while ((await rt.get("/state")).frame === 0) {
    if (Date.now() > deadline) throw new Error("the restarted game never stepped: is its clock paused?");
    await sleep(50);
  }
  const { generation } = await rt.get(`/input-log?from=${Number.MAX_SAFE_INTEGER}`);
  let next = (await rt.get("/state")).frame + 1;
  // Tapped after `next` was read, so the reset lands in the recording.
  await rt.post("/input", { type: "key", key: resetKey, down: true });
  await sleep(100);
  await rt.post("/input", { type: "key", key: resetKey, down: false });
  let stop = false;
  stopped.then(() => (stop = true));
  const steps = [];
  // The runtime keeps up to 15 s of steps; `readLog` throws if any were lost.
  for (;;) {
    const done = stop;
    const log = await readLog(rt, next);
    if (log.generation !== generation) throw new Error("the runtime's history was replaced (a rewind or project load): record again");
    steps.push(...log.steps);
    if (log.steps.length) next = log.steps.at(-1).frame + 1;
    if (done) break;
    await Promise.race([sleep(500), stopped]);
  }
  const reset = steps.findIndex((s) => s.input?.pressed_keys.some((k) => keyName(k).toLowerCase() === resetKey.toLowerCase()));
  if (reset < 0) throw new Error(`the reset key ${resetKey} never reached the game`);
  return steps.slice(reset);
}
