import { test } from "node:test";
import assert from "node:assert/strict";
import { fitRect, headsetChoices, headsetNote, streamUrl, toSurface } from "./game-view.ts";
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
  const session = (status: Session["status"]): Session => ({ id: 1, pid: 7, status });
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
