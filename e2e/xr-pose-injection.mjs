// XR pose injection E2E: prove a synthetic two-hand pose sequence drives a real
// game headlessly, with no headset and no GL window.
//
// This is the agent-verifiability proof for `POST /input {"type":"xr", …}`
// (docs/debug-runtime.md). It drives `examples/xr-controllers` — whose
// `sampledInput` reads `snapshot.xr` and whose `tick` pulls the orb toward the
// right hand's aim — over the debug server:
//
//   1. NO `--emulate-xr`: assert the game starts on its `Option.None` branch
//      (nothing tracked), so the `xr` domain provably comes from injection.
//   2. Inject a two-hand sample whose right hand sits at z = +0.12 with the
//      trigger held — a pose the desktop emulator CANNOT express (it pins both
//      hands to z = -0.55, identity orientation, and derives the trigger from
//      the keyboard). Step the clock one fixed frame per pose.
//   3. Assert the game saw it: both hands tracked, `grabbing: true`, and the orb
//      dragged from its `init` position to the injected aim point.
//   4. Replay the identical sequence in a second process and assert the final
//      model text matches byte for byte — the sequence is deterministic because
//      the injected sample rides the same `sampledInput` path a headset takes.
//
// Run manually (needs the CLI built; the wasm bundle is not required):
//
//   cargo build -p functor-cli --no-default-features
//   node e2e/xr-pose-injection.mjs
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const BIN = `${ROOT}target/debug/functor`;
const SAMPLE = "examples/xr-controllers";
const STEPS = 40;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The right hand's pose at step `i`: a straight pull from the emulator's plane
 *  (z = -0.55) back PAST the head to z = +0.12, trigger held the whole way. The
 *  left hand holds a rotated grip. Neither is expressible via `--emulate-xr`. */
function poseAt(i) {
  const t = i / (STEPS - 1);
  return {
    type: "xr",
    head: { position: [0.0, 0.0, 0.0] },
    left: {
      active: true,
      grip: { position: [-0.3, -0.1, -0.6], orientation: [0.0, 0.38, 0.0, 0.92] },
      aim: { position: [-0.3, -0.1, -0.6], orientation: [0.0, 0.38, 0.0, 0.92] },
    },
    right: {
      active: true,
      grip: { position: [-0.05, -0.05, -0.55 + t * 0.67] },
      aim: { position: [-0.05, -0.05, -0.55 + t * 0.67] },
      trigger: 1.0,
    },
  };
}

async function post(base, path, body) {
  const res = await fetch(`${base}${path}`, {
    method: "POST",
    body: JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`POST ${path} → ${res.status}: ${text}`);
  return text;
}

async function state(base) {
  const res = await fetch(`${base}/state`);
  if (!res.ok) throw new Error(`GET /state → ${res.status}`);
  return res.json();
}

async function waitReady(base, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const s = await state(base);
      if (s.frame > 0) return s;
    } catch {
      // not listening yet
    }
    await sleep(100);
  }
  throw new Error("runtime never reported a frame");
}

/** Advance exactly one fixed step and WAIT for it to land.
 *
 * `POST /time advance` sets the clock's single `pending_step` slot, which the
 * loop consumes on its next iteration. Two advances that arrive inside one
 * request-drain therefore collapse into ONE step — so a naive
 * "post pose, post advance" loop silently drops poses at localhost speed, and
 * how many it drops depends on timing. Waiting for `frame` to increment is what
 * makes one pose == one frame, and the sequence reproducible. */
async function stepOneFrame(base, timeoutMs = 10000) {
  const before = (await state(base)).frame;
  await post(base, "/time", { type: "advance", dts: 1 / 60 });
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if ((await state(base)).frame > before) return;
    await sleep(5);
  }
  throw new Error(`frame never advanced past ${before}`);
}

// The model arrives as the runtime's `Debug` text, whose record keys are
// UNQUOTED (`rightGripTracked: false`, `orb: { x: 0, … }`) — so these readers
// match bare identifiers, and nest by balancing braces rather than by a regex
// that cannot see into a sub-record.

/** Read one scalar field out of the model's `Debug` text. */
function field(model, name) {
  const m = model.match(new RegExp(`\\b${name}:\\s*([A-Za-z0-9_.+-]+)`));
  return m ? m[1] : undefined;
}

/** The balanced `{…}` block that follows `name:`, so nested records work. */
function block(text, name) {
  const m = text.match(new RegExp(`\\b${name}:\\s*\\{`));
  if (!m) throw new Error(`no ${name} record in:\n${text}`);
  const open = m.index + m[0].length - 1;
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    if (text[i] === "{") depth++;
    else if (text[i] === "}" && --depth === 0) return text.slice(open, i + 1);
  }
  throw new Error(`unbalanced ${name} record in:\n${text}`);
}

/** The `{ x, y, z }` immediately inside a block. */
function xyz(text, what) {
  const m = text.match(
    /\bx:\s*(-?[\d.e+-]+),\s*y:\s*(-?[\d.e+-]+),\s*z:\s*(-?[\d.e+-]+)/,
  );
  if (!m) throw new Error(`no x/y/z in ${what}:\n${text}`);
  return { x: +m[1], y: +m[2], z: +m[3] };
}

/** A plain point field, e.g. `orb`. */
function point(model, name) {
  return xyz(block(model, name), name);
}

/** A tracked pose's position — a pose is `{ position, forward, up }`, so the
 *  point lives one level down and a flat reader would miss it entirely. */
function posePosition(model, name) {
  return xyz(block(block(model, name), "position"), `${name}.position`);
}

/** Run an injected sequence against a fresh runtime; return the state.
 *
 * `poseFn(i)` supplies step `i`'s pose, so a caller can drive the real sweep or
 * a CONTROL sequence that holds one pose — the difference between the two is
 * what proves the sequence itself (not merely "some hand was tracked") reached
 * the simulation. */
async function drive(port, poseFn = poseAt) {
  const base = `http://127.0.0.1:${port}`;
  const proc = spawn(
    BIN,
    ["-d", SAMPLE, "run", "native", "--headless", "--debug-port", String(port)],
    { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
  );
  let log = "";
  proc.stdout.on("data", (d) => (log += d));
  proc.stderr.on("data", (d) => (log += d));

  try {
    await waitReady(base, 30000);

    // No --emulate-xr, so the game must currently be on its Option.None branch.
    const before = await state(base);
    if (before.input.xr !== undefined) {
      throw new Error(`expected no xr domain before injection, got ${JSON.stringify(before.input.xr)}`);
    }
    if (field(before.model, "rightGripTracked") !== "false") {
      throw new Error(`expected rightGripTracked false, model:\n${before.model}`);
    }

    // Pin the clock so each pose gets exactly one fixed step.
    await post(base, "/time", { type: "set", tts: 0.0 });
    for (let i = 0; i < STEPS; i++) {
      await post(base, "/input", poseFn(i));
      await stepOneFrame(base);
    }
    const after = await state(base);

    // Release the injection: the game must fall back to "no XR device", which
    // is only observable because the runtime stopped substituting our sample.
    await post(base, "/input", { type: "xr_clear" });
    await stepOneFrame(base);
    const cleared = await state(base);

    return { after, cleared, log };
  } finally {
    proc.kill("SIGKILL");
  }
}

const failures = [];
const check = (ok, what) => {
  console.log(`  ${ok ? "✓" : "✗"} ${what}`);
  if (!ok) failures.push(what);
};

const first = await drive(8141);
const { after } = first;

console.log("\n▸ the injected sample reached the game");
check(after.input.xr !== undefined, "/state reports an xr domain (injection supplied it)");
check(
  after.input.xr?.right?.grip?.position?.[2] > 0,
  "the right hand is at positive z — a pose --emulate-xr cannot produce",
);
check(field(after.model, "leftGripTracked") === "true", "left hand tracked");
check(field(after.model, "rightGripTracked") === "true", "right hand tracked");
check(field(after.model, "grabbing") === "true", "injected trigger drove `grabbing`");

console.log("\n▸ the pose sequence drove the simulation");
const orb = point(after.model, "orb");
const aim = posePosition(after.model, "rightAim");
const init = { x: 0.0, y: 1.15, z: -0.7 };
const dist = (a, b) => Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z);
const moved = dist(orb, init);
const toAim = dist(orb, aim);
check(moved > 0.1, `the orb left its init pose (moved ${moved.toFixed(3)})`);
check(toAim < 0.5, `the orb was dragged to the injected aim (${toAim.toFixed(3)} away)`);

// The control: the SAME machinery, but the hand held at the sweep's first pose
// (z = -0.55, the plane --emulate-xr pins it to). If the harness were only
// proving "a hand was tracked", this would land in the same place. It must not:
// the swept hand ends 0.67 further back, and the orb has to follow it there.
console.log("\n▸ the endpoint reflects the sequence, not just that XR was on");
const control = await drive(8143, () => poseAt(0));
const controlOrb = point(control.after.model, "orb");
const spread = dist(orb, controlOrb);
check(
  spread > 0.3,
  `a held first-pose control ends elsewhere (${spread.toFixed(3)} apart), so the sweep drove the result`,
);
// Rig-local +z is BACKWARD (OpenXR forward is -Z), and this camera sits at
// z = -3 looking toward +Z — so a hand pulled toward the face travels toward
// the camera, i.e. to SMALLER world z. The orb has to follow it there.
check(
  orb.z < controlOrb.z,
  `pulling the hand toward the face drew the orb toward the camera (swept z ${orb.z.toFixed(3)} < held z ${controlOrb.z.toFixed(3)})`,
);

console.log("\n▸ releasing the injection restores the undriven state");
check(
  first.cleared.input.xr === undefined,
  "xr_clear dropped the xr domain (no --emulate-xr to fall back to)",
);
check(
  field(first.cleared.model, "rightGripTracked") === "false",
  "the game returned to its Option.None branch",
);

console.log("\n▸ the sequence is deterministic");
// Same inputs, a fresh process: this pins the injection path's reproducibility.
// (It is not a test of recorded-log REPLAY — that path has its own coverage.)
const second = await drive(8142);
check(
  second.after.model === after.model,
  "an identical injected sequence produced an identical model",
);

if (failures.length) {
  console.error(`\n✗ xr-pose-injection failed: ${failures.join(", ")}`);
  console.error(`\nfinal model:\n${after.model}`);
  process.exit(1);
}
console.log("\n✓ xr-pose-injection passed");
