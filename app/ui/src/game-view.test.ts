import { test } from "node:test";
import assert from "node:assert/strict";
import { fitRect, headsetChoices, headsetNote, mirrorCamera, mirrorProblem, mirrorRate, streamUrl, toSurface } from "./game-view.ts";
import type { MirrorStats } from "./game-view.ts";
import type { Device, Session, Transport } from "./generated/protocol";

test("a wider view letterboxes the frame left and right", () => {
  // A 4:3 frame in a 16:9 box, frame larger than the box.
  const rect = fitRect({ w: 1600, h: 900 }, { w: 1600, h: 1200 }, { w: 1600, h: 1200 });
  assert.deepEqual(rect, { x: 200, y: 0, w: 1200, h: 900 });
});

test("a taller view letterboxes the frame top and bottom", () => {
  const rect = fitRect({ w: 800, h: 1000 }, { w: 800, h: 600 }, { w: 800, h: 600 });
  assert.deepEqual(rect, { x: 0, y: 200, w: 800, h: 600 });
});

test("a frame smaller than the view is centred, not scaled up", () => {
  // 1600×1200 device pixels at 2× is 800×600 CSS pixels.
  const rect = fitRect({ w: 1000, h: 800 }, { w: 1600, h: 1200 }, { w: 800, h: 600 });
  assert.deepEqual(rect, { x: 100, y: 100, w: 800, h: 600 });
});

test("pointer positions map into the game's surface through the letterbox", () => {
  const rect = { x: 200, y: 0, w: 1200, h: 900 };
  const surface = { w: 800, h: 600 };
  assert.deepEqual(toSurface(200, 0, rect, surface), { x: 0, y: 0 });
  assert.deepEqual(toSurface(800, 450, rect, surface), { x: 400, y: 300 });
  // The far edge is the last pixel: the surface is 0..w-1.
  assert.deepEqual(toSurface(1400, 900, rect, surface), { x: 799, y: 599 });
  assert.deepEqual(toSurface(500, 300, rect, surface), { x: 200, y: 200 });
});

test("pointer positions in the bars clamp to the frame's edge", () => {
  const rect = { x: 200, y: 0, w: 1200, h: 900 };
  const surface = { w: 800, h: 600 };
  assert.deepEqual(toSurface(50, 450, rect, surface), { x: 0, y: 300 });
  assert.deepEqual(toSurface(1550, -10, rect, surface), { x: 799, y: 0 });
});

test("the stream is requested at the view's size, on loopback only", () => {
  assert.equal(
    streamUrl("http://127.0.0.1:52011", { w: 1599.6, h: 900 }),
    "http://127.0.0.1:52011/stream?w=1600&h=900",
  );
  assert.equal(streamUrl("http://localhost:8077", { w: 0, h: 0 }), "http://localhost:8077/stream?w=1&h=1");
  assert.equal(streamUrl("http://192.168.1.5:8077", { w: 10, h: 10 }), null);
  assert.equal(streamUrl("https://127.0.0.1:8077", { w: 10, h: 10 }), null);
});

const quest = (id: string, name: string, state: Device["state"]): Device => ({
  id,
  name,
  paired: true,
  state,
  app_version: "0.0.0",
  first_seen: 1,
  last_seen: 2,
  tunnel: "http://127.0.0.1:6000",
});
const linked: Device["state"] = {
  state: "connected",
  link: "wifi",
  battery: 80,
  charging: false,
  build: "dev",
  project: "cubes",
  latency_ms: 9,
};
const away: Device["state"] = { state: "away" };

test("headsets are a placement only while connected, or while the game is on one", () => {
  const list = [quest("aa11", "Quest 3", linked), quest("bb22", "Quest Pro", away)];
  assert.deepEqual(headsetChoices(list, null), [{ device: "aa11", label: "Quest 3" }]);
  assert.deepEqual(headsetChoices([list[1]!], null), []);
  // The game is on the away one: its choice stays.
  assert.deepEqual(
    headsetChoices(list, "bb22").map((c) => c.device),
    ["aa11", "bb22"],
  );
  // Two of one name are told apart by id.
  const twins = [quest("aa11", "Quest 3", linked), quest("cc33", "Quest 3", linked)];
  assert.deepEqual(
    headsetChoices(twins, null).map((c) => c.label),
    ["Quest 3 aa11", "Quest 3 cc33"],
  );
});

test("the headset note says where the game runs, its clock, and the fix while away", () => {
  const session = (status: Session["status"]): Session => ({ id: 1, pid: 7, status, mirror: null });
  const running = session({ kind: "running", debug_url: "http://127.0.0.1:6000", surface: null });
  const clock: Transport = {
    clock: true,
    paused: true,
    frame: 42,
    recorded: [0, 42],
    generation: 0,
    seek: 0,
    extrapolate: false,
    revision: 0,
  };
  assert.deepEqual(headsetNote(quest("a", "Quest 3", linked), running, clock), {
    title: "Running on Quest 3",
    detail: "Frame 42 · Paused",
    fix: null,
  });
  assert.equal(headsetNote(quest("a", "Quest 3", linked), session({ kind: "starting" }), null).title, "Starting on Quest 3…");
  const gone = headsetNote(quest("a", "Quest 3", away), running, clock);
  assert.equal(gone.title, "Quest 3: Away");
  assert.match(gone.fix!, /Put the headset on/);
  const failed = headsetNote(quest("a", "Quest 3", linked), session({ kind: "exited", reason: "boom" }), null);
  assert.deepEqual([failed.title, failed.detail], ["The game stopped on Quest 3", "boom"]);
  assert.match(headsetNote(null, running, null).fix!, /Choose Pane/);
});

test("the mirror's cameras park near the head, looking the way it looks along the floor", () => {
  // At the origin, 1.6 m up, looking down +X and a little down.
  const view = { eye: [0, 1.6, 0], target: [1, 1.2, 0] };
  assert.deepEqual(mirrorCamera("head", view), { type: "attach" });
  assert.deepEqual(mirrorCamera("behind", view), { type: "set", eye: [-2.5, 2.8, 0], target: [1, 1.6, 0], fov: 70 });
  assert.deepEqual(mirrorCamera("above", view), { type: "set", eye: [-0.5, 5.6, 0], target: [2.5, 1.6, 0], fov: 70 });
  // Straight down: the floor direction falls back to +Z.
  assert.deepEqual((mirrorCamera("behind", { eye: [0, 2, 0], target: [0, 0, 0] }) as { eye: number[] }).eye, [0, 3.2, -2.5]);
  assert.equal(mirrorCamera("behind", null), null, "nothing to park from yet");
});

test("the mirror says its rate while frames come, and why when they do not", () => {
  const stats = (s: Partial<MirrorStats>): MirrorStats => ({ frames: 30, fps: 19.6, latency_ms: 41.2, view: null, error: null, ...s });
  const running = { kind: "running", debug_url: "http://127.0.0.1:7000" } as const;
  assert.equal(mirrorRate(stats({})), "20 fps · 41 ms");
  assert.equal(mirrorProblem(running, stats({})), null);
  assert.equal(mirrorProblem(null, null), null, "no mirror without a GPU");
  assert.equal(mirrorProblem({ kind: "starting" }, null), "Starting the mirror…");
  assert.equal(mirrorProblem(running, stats({ frames: 0 })), "Waiting for the headset's first frame…");
  const tooBig = stats({ error: "413 Payload Too Large: the frame is 9 MiB as JSON, over the mirror's 8 MiB" });
  assert.equal(mirrorRate(tooBig), null);
  assert.match(mirrorProblem(running, tooBig)!, /^No frames from the headset: 413/);
  assert.equal(mirrorProblem({ kind: "failed", reason: "exit status: 1" }, null), "The mirror stopped: exit status: 1");
});
