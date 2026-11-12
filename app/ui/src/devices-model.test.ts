import { test } from "node:test";
import assert from "node:assert/strict";
import { ago, clock, details, fixFor, mayInterrupt, nextRequest, secondsLeft, spacedCode, WORDS } from "./devices-model.ts";
import type { Device, Devices, PairRequest } from "./generated/protocol";

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
  assert.equal(fixFor(device({ state: "connected", link: "wifi", battery: null, charging: false, build: null, project: null, latency_ms: null })), "");
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

test("the prompt shows the oldest request, one at a time", () => {
  const devices: Devices = { listening: null, list: [], requests: [request(3), request(4)], headset: "asking_to_pair" };
  assert.equal(nextRequest(devices)?.request, 3);
  assert.equal(nextRequest({ ...devices, requests: [] }), null);
  assert.equal(nextRequest(undefined), null);
});

test("the prompt waits while the user types", () => {
  assert.equal(mayInterrupt(null), true);
  assert.equal(mayInterrupt({ tagName: "BUTTON", isContentEditable: false } as unknown as Element), true);
  assert.equal(mayInterrupt({ tagName: "TEXTAREA" } as unknown as Element), false);
  assert.equal(mayInterrupt({ tagName: "INPUT" } as unknown as Element), false);
  assert.equal(mayInterrupt({ tagName: "DIV", isContentEditable: true } as unknown as Element), false);
});

test("a connected card's details", () => {
  const d = device({ state: "connected", link: "wifi", battery: 80, charging: true, build: "abc", project: "examples/hello", latency_ms: 12 });
  assert.deepEqual(details(d), [
    ["Link", "Wi-Fi · 12 ms"],
    ["Battery", "80%, charging"],
    ["Build", "abc"],
    ["Running", "examples/hello"],
  ]);
  assert.deepEqual(details(device({ state: "away" })), []);
});

test("last seen reads in plain units", () => {
  assert.equal(ago(null, 0), "Never");
  assert.equal(ago(0, 30_000), "Just now");
  assert.equal(ago(0, 180_000), "3 min ago");
  assert.equal(ago(0, 7_200_000), "2 h ago");
});
