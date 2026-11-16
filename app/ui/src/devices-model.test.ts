import { test } from "node:test";
import assert from "node:assert/strict";
import {
  autoOpen,
  cableFor,
  cableOnly,
  cableWords,
  clock,
  details,
  fixFor,
  mayOpen,
  secondsLeft,
  spacedCode,
  TYPING_IDLE_MS,
  WORDS,
} from "./devices-model.ts";
import type { Attached, Device, Devices, PairRequest } from "./generated/protocol";

const device = (state: Device["state"]): Device => ({
  id: "ab",
  name: "Quest 3",
  paired: true,
  state,
  app_version: "0.1.0",
  first_seen: 1,
  last_seen: 2,
  tunnel: null,
});

const request = (n: number, expires_at = 10_000): PairRequest => ({
  request: n,
  device: "cd",
  name: "Quest Pro",
  app_version: "0.1.0",
  code: "482913",
  expires_at,
  allowed: false,
  repair: false,
});

test("every state has a word and, but connected, a fix", () => {
  assert.equal(Object.keys(WORDS).length, 6);
  assert.match(fixFor(device({ state: "away" })), /reconnects by itself/);
  assert.equal(fixFor(device({ state: "connected", link: "wifi", battery: null, charging: false, build: null, project: null, loading: [], latency_ms: null })), "");
  assert.match(fixFor(device({ state: "needs_update", update: "headset", reason: "", since: 0 })), /on the headset/);
  assert.match(fixFor(device({ state: "needs_update", update: "desktop", reason: "", since: 0 })), /on this computer/);
  assert.match(fixFor(device({ state: "cannot_reach", reason: "timed out", since: 0 })), /same Wi-Fi/);
});

test("the code reads in two groups and the expiry counts down", () => {
  assert.equal(spacedCode("482913"), "482 913");
  assert.equal(secondsLeft(request(1, 10_000), 0), 10);
  assert.equal(secondsLeft(request(1, 10_000), 9_001), 1);
  assert.equal(secondsLeft(request(1, 10_000), 20_000), 0);
  assert.equal(clock(75), "1:15");
  assert.equal(clock(5), "0:05");
});

test("the prompt opens the oldest request not put off", () => {
  const requests = [request(3), request(4)];
  assert.equal(autoOpen(requests, [])?.request, 3);
  assert.equal(autoOpen(requests, [3])?.request, 4);
  assert.equal(autoOpen(requests, [3, 4]), null);
  assert.equal(autoOpen([], []), null);
});

test("the prompt waits while the user types or holds a pointer down", () => {
  assert.equal(mayOpen(0, TYPING_IDLE_MS, false), true);
  assert.equal(mayOpen(1000, 1000 + TYPING_IDLE_MS - 1, false), false);
  assert.equal(mayOpen(0, 100_000, true), false);
});

test("a connected card's details", () => {
  const d = device({ state: "connected", link: "wifi", battery: 80, charging: true, build: "abc", project: "examples/hello", loading: [], latency_ms: 12 });
  assert.deepEqual(details(d), [
    ["Link", "Wi-Fi · 12 ms"],
    ["Battery", "80%, charging"],
    ["Build", "abc"],
    ["Running", "examples/hello"],
  ]);
  assert.deepEqual(details(device({ state: "away" })), []);
  // What the headset is loading takes the place of what it runs. Its
  // lines: the step's kind, then the card's own (heading first).
  const row = (loading: string[]) => details({ ...d, state: { ...d.state, loading } as Device["state"] }).at(-1);
  assert.deepEqual(row(["receiving", "RECEIVING PROJECT", "23.0 of 67.5 MB (34%), 4 of 9 files"]), [
    "Sending to headset",
    "23.0 of 67.5 MB (34%), 4 of 9 files",
  ]);
  // A push the headset already held: nothing was sent.
  assert.deepEqual(row(["up_to_date", "UP TO DATE", "Nothing to send", "71.0 MB already on the headset"]), [
    "Up to date",
    "71.0 MB already on the headset",
  ]);
  assert.deepEqual(row(["up_to_date", "UP TO DATE", "Nothing to send"]), ["Up to date", "Nothing to send"]);
  assert.deepEqual(row(["starting", "STARTING", "Checking code"]), ["Starting", "Checking code"]);
  assert.deepEqual(row(["assets", "LOADING ASSETS", "2 of 6"]), ["Loading assets", "2 of 6"]);
  assert.deepEqual(row(["reloading", "RELOADING..."]), ["Reloading", "The game is reloading"]);
  // An error the headset wrapped for its card reads as one sentence.
  assert.deepEqual(row(["failed", "LOAD FAILED", "game.fun:3:1: unknown", "name `tik`"]), ["Failed to load", "game.fun:3:1: unknown name `tik`"]);
  // Running, waiting, hidden, or a kind this desktop does not know: what it runs.
  for (const lines of [["running", "RUNNING"], ["connected", "CONNECTED", "to Studio"], ["later", "SOMETHING NEW"], []]) {
    assert.deepEqual(row(lines), ["Running", "examples/hello"]);
  }
});

const onCable = (serial: string, state: Attached["state"]): Attached => ({
  serial,
  name: "Quest 3",
  state,
  pairing: false,
  pairable: state.state === "ready",
});

test("a headset on the cable joins its paired card, or has its own", () => {
  const ours = onCable("s1", { state: "ready", device: "ab", paired: { with: "this" } });
  const stranger = onCable("s2", { state: "ready", device: "zz", paired: { with: "nobody" } });
  const noId = onCable("s3", { state: "unauthorized" });
  const devices: Devices = {
    listening: "0.0.0.0:1",
    list: [device({ state: "away" })],
    requests: [],
    headset: "away",
    adb: true,
    attached: [ours, stranger, noId],
  };
  assert.equal(cableFor(devices.list[0], devices.attached), ours);
  assert.deepEqual(cableOnly(devices), [stranger, noId]);
  // Away and paired over this cable: says how it connects.
  assert.match(fixFor(devices.list[0], ours), /connects over Wi-Fi by itself/);
  assert.match(fixFor(devices.list[0], null), /reconnects by itself/);
});

test("each cable state says one thing and its fix", () => {
  const states: Attached["state"][] = [{ state: "unauthorized" }, { state: "offline" }, { state: "no_app" }, { state: "not_debuggable" }];
  for (const s of states) {
    const { line, fix } = cableWords(onCable("s", s));
    assert.ok(line && fix, s.state);
  }
  assert.match(cableWords(onCable("s", { state: "not_debuggable" })).fix, /hold the left menu button/);
  const ready = cableWords(onCable("s", { state: "ready", device: null, paired: { with: "nobody" } }));
  assert.deepEqual([/no code/.test(ready.line), ready.fix], [true, ""]);
  const other = cableWords(onCable("s", { state: "ready", device: "x", paired: { with: "other", name: "Studio" } }));
  assert.equal(other.line, "It is paired with Studio.");
});
