const { test } = require("node:test");
const assert = require("node:assert");
const path = require("path");
const {
  assetTargetFor,
  pickAsset,
  downloadedCliPath,
  commandWorks,
} = require("./cli-download");

test("asset targets map the four released platforms, others null", () => {
  assert.strictEqual(assetTargetFor("darwin", "arm64"), "aarch64-apple-darwin");
  assert.strictEqual(assetTargetFor("darwin", "x64"), "x86_64-apple-darwin");
  assert.strictEqual(assetTargetFor("linux", "x64"), "x86_64-unknown-linux-gnu");
  assert.strictEqual(assetTargetFor("win32", "x64"), "x86_64-pc-windows-msvc");
  assert.strictEqual(assetTargetFor("linux", "arm64"), null);
});

const releases = [
  // Newest first, like the GitHub API. A tag without assets for the platform
  // must be skipped in favor of an older complete release.
  { tag_name: "v0.2.0", assets: [{ name: "functor-0.2.0-x86_64-unknown-linux-gnu.tar.gz", browser_download_url: "u1" }] },
  {
    tag_name: "v0.1.0",
    assets: [
      { name: "functor-0.1.0-aarch64-apple-darwin.tar.gz", browser_download_url: "u2" },
      { name: "functor-0.1.0-x86_64-pc-windows-msvc.zip", browser_download_url: "u3" },
    ],
  },
];

test("pickAsset takes the newest release carrying the platform asset", () => {
  assert.deepStrictEqual(pickAsset(releases, "linux", "x64"), {
    version: "0.2.0",
    assetName: "functor-0.2.0-x86_64-unknown-linux-gnu.tar.gz",
    url: "u1",
  });
  // darwin-arm64 missing from v0.2.0 → falls back to v0.1.0
  assert.deepStrictEqual(pickAsset(releases, "darwin", "arm64"), {
    version: "0.1.0",
    assetName: "functor-0.1.0-aarch64-apple-darwin.tar.gz",
    url: "u2",
  });
});

test("pickAsset uses .zip on windows and null when unsupported/empty", () => {
  assert.strictEqual(pickAsset(releases, "win32", "x64").url, "u3");
  assert.strictEqual(pickAsset(releases, "linux", "arm64"), null);
  assert.strictEqual(pickAsset([], "linux", "x64"), null);
  assert.strictEqual(pickAsset([{ tag_name: "preview", assets: [] }], "linux", "x64"), null);
});

test("downloadedCliPath differs only in the exe suffix", () => {
  assert.strictEqual(downloadedCliPath("/s", "linux"), path.join("/s", "bin", "functor"));
  assert.strictEqual(downloadedCliPath("/s", "win32"), path.join("/s", "bin", "functor.exe"));
});

test("commandWorks: real spawn success and failure", async () => {
  assert.strictEqual(await commandWorks(process.execPath), true); // node --version
  assert.strictEqual(await commandWorks("functor-definitely-not-installed-xyz"), false);
});
