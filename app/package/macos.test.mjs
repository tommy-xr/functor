// Bundles a stub binary and checks Functor.app's layout, Info.plist and
// signature (macOS only: plutil, iconutil, codesign).
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import assert from "node:assert/strict";

test("Functor.app from a stub binary", { skip: process.platform !== "darwin" }, () => {
  const out = mkdtempSync(join(tmpdir(), "functor-package-"));
  try {
    const script = new URL("macos.mjs", import.meta.url).pathname;
    execFileSync("node", [script, "--functor", "/usr/bin/true", "--out", out]);
    const contents = join(out, "Functor.app/Contents");
    // One executable: the app is `functor` itself (D40).
    assert.deepEqual(readdirSync(join(contents, "MacOS")), ["functor"]);
    assert.ok(!existsSync(join(contents, "Helpers")), "no second copy");
    assert.ok(existsSync(join(contents, "Resources/Functor.icns")));
    const plist = execFileSync("plutil", ["-convert", "json", "-o", "-", join(contents, "Info.plist")]);
    const info = JSON.parse(plist);
    assert.equal(info.CFBundleExecutable, "functor");
    assert.equal(info.CFBundleIconFile, "Functor");
    assert.equal(info.CFBundlePackageType, "APPL");
    assert.match(info.CFBundleShortVersionString, /^\d+(\.\d+){0,2}$/);
    assert.ok(info.NSMicrophoneUsageDescription);
    assert.ok(info.NSLocalNetworkUsageDescription);
    assert.deepEqual(info.NSBonjourServices, ["_functor-link._tcp"]);
    // Every iconset size made it into the .icns.
    const back = join(out, "back.iconset");
    execFileSync("iconutil", ["-c", "iconset", "-o", back, join(contents, "Resources/Functor.icns")]);
    assert.equal(readdirSync(back).length, 10, readdirSync(back).join(" "));
    execFileSync("codesign", ["--verify", "--deep", "--strict", join(out, "Functor.app")]);
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
});
