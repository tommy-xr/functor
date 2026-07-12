// Headless tests for the inspector decision logic (client/inspector.js).
// Runs under `node --test` — no VS Code host, no LSP server, no new deps.
const test = require("node:test");
const assert = require("node:assert/strict");
const inspector = require("./inspector.js");

test("relayTrace forwards a functor-inspector-trace message", () => {
  const trace = { frame: 7, paused: true, invocations: [] };
  assert.deepEqual(inspector.relayTrace({ type: "functor-inspector-trace", trace }), {
    notification: "functor/inspector/trace",
    params: trace,
  });
});

test("relayTrace ignores unrelated / malformed messages", () => {
  assert.equal(inspector.relayTrace(null), null);
  assert.equal(inspector.relayTrace(undefined), null);
  assert.equal(inspector.relayTrace({}), null);
  assert.equal(inspector.relayTrace({ type: "functor-lang-set-source" }), null);
  assert.equal(inspector.relayTrace({ type: "functor-lang-preview-ready" }), null);
});

test("attach / detach notifications carry the right params", () => {
  assert.deepEqual(inspector.attachNotification(8077), {
    notification: "functor/inspector/attach",
    params: { port: 8077 },
  });
  assert.deepEqual(inspector.detachNotification(), {
    notification: "functor/inspector/attach",
    params: { port: null },
  });
});

test("parsePort accepts valid ports", () => {
  assert.deepEqual(inspector.parsePort("8077"), { port: 8077 });
  assert.deepEqual(inspector.parsePort("  8077 "), { port: 8077 });
  assert.deepEqual(inspector.parsePort("1"), { port: 1 });
  assert.deepEqual(inspector.parsePort("65535"), { port: 65535 });
});

test("parsePort rejects out-of-range and non-numeric input", () => {
  assert.ok(inspector.parsePort("0").error);
  assert.ok(inspector.parsePort("65536").error);
  assert.ok(inspector.parsePort("").error);
  assert.ok(inspector.parsePort("abc").error);
  assert.ok(inspector.parsePort("80a").error);
  assert.ok(inspector.parsePort(null).error);
});

test("initialState remembers a valid last port, else the default", () => {
  assert.deepEqual(inspector.initialState(9000), { attached: false, port: 9000 });
  assert.deepEqual(inspector.initialState("9000"), { attached: false, port: 9000 });
  assert.deepEqual(inspector.initialState(undefined), {
    attached: false,
    port: inspector.DEFAULT_PORT,
  });
  assert.deepEqual(inspector.initialState("garbage"), {
    attached: false,
    port: inspector.DEFAULT_PORT,
  });
});

test("reduce transitions attach/detach and keeps the port on detach", () => {
  const s0 = inspector.initialState(undefined);
  const s1 = inspector.reduce(s0, { type: "attach", port: 9001 });
  assert.deepEqual(s1, { attached: true, port: 9001 });
  const s2 = inspector.reduce(s1, { type: "detach" });
  assert.deepEqual(s2, { attached: false, port: 9001 });
  // Unknown actions are a no-op.
  assert.equal(inspector.reduce(s2, { type: "nope" }), s2);
});

test("promptDefault reflects the remembered port", () => {
  assert.equal(inspector.promptDefault({ attached: false, port: 8077 }), "8077");
  assert.equal(inspector.promptDefault({ attached: true, port: 9001 }), "9001");
});

test("statusBar derives text + toggle command from attach state", () => {
  const detached = inspector.statusBar({ attached: false, port: 8077 });
  assert.match(detached.text, /inspector/);
  assert.equal(detached.command, inspector.ATTACH_COMMAND);

  const attached = inspector.statusBar({ attached: true, port: 8077 });
  assert.equal(attached.text, "$(debug) inspector :8077");
  assert.equal(attached.command, inspector.DETACH_COMMAND);
});
