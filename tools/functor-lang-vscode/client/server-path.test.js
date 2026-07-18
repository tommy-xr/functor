const { test } = require("node:test");
const assert = require("node:assert");
const path = require("path");
const { resolveServerCommand } = require("./server-path");

const EXT = path.join("/ext", "root");
const bundledUnix = path.join(EXT, "server", "functor-lang-lsp");
const bundledWin = path.join(EXT, "server", "functor-lang-lsp.exe");

test("an explicit setting always wins", () => {
  const cmd = resolveServerCommand("/opt/lsp/functor-lang-lsp", EXT, "darwin", () => true);
  assert.strictEqual(cmd, "/opt/lsp/functor-lang-lsp");
});

test("the bundled binary is preferred when present", () => {
  const cmd = resolveServerCommand(undefined, EXT, "darwin", (p) => p === bundledUnix);
  assert.strictEqual(cmd, bundledUnix);
});

test("windows looks for the .exe", () => {
  const cmd = resolveServerCommand(undefined, EXT, "win32", (p) => p === bundledWin);
  assert.strictEqual(cmd, bundledWin);
});

test("no setting and no bundle falls back to PATH", () => {
  const cmd = resolveServerCommand(undefined, EXT, "linux", () => false);
  assert.strictEqual(cmd, "functor-lang-lsp");
});

test("an empty setting is treated as unset", () => {
  const cmd = resolveServerCommand("", EXT, "linux", () => false);
  assert.strictEqual(cmd, "functor-lang-lsp");
});
