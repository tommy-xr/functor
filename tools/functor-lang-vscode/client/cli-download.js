// Download-on-demand for the `functor` CLI the live preview spawns (see
// extension.js resolveFunctorCli): when neither functor.functorPath nor
// PATH resolves, the newest GitHub release's platform archive is offered as
// a one-click download into the extension's global storage.
//
// Decision logic (asset selection, paths) is pure and node-tested
// (cli-download.test.js); the network/extract helpers are the thin IO shell.
// NOTE: Functor releases are all PRE-releases, so the /releases/latest API
// endpoint (non-prereleases only) returns nothing — list releases and take
// the newest that carries a matching asset instead.
const path = require("path");
const fs = require("node:fs");
const https = require("node:https");
const { execFile, spawn } = require("node:child_process");

const RELEASES_URL = "https://api.github.com/repos/tommy-xr/functor/releases?per_page=10";

// Release archives exist for these four platforms (release-binaries.yml).
function assetTargetFor(platform, arch) {
  if (platform === "darwin" && arch === "arm64") return "aarch64-apple-darwin";
  if (platform === "darwin" && arch === "x64") return "x86_64-apple-darwin";
  if (platform === "linux" && arch === "x64") return "x86_64-unknown-linux-gnu";
  if (platform === "win32" && arch === "x64") return "x86_64-pc-windows-msvc";
  return null;
}

// Newest release (the API lists newest first) whose tag is a release version
// and which carries this platform's archive. Returns
// { version, assetName, url } or null.
function pickAsset(releases, platform, arch) {
  const target = assetTargetFor(platform, arch);
  if (!target) return null;
  for (const release of releases || []) {
    const m = /^v(\d+\.\d+\.\d+)$/.exec(release.tag_name || "");
    if (!m) continue;
    const version = m[1];
    const suffix = platform === "win32" ? ".zip" : ".tar.gz";
    const assetName = `functor-${version}-${target}${suffix}`;
    const asset = (release.assets || []).find((a) => a.name === assetName);
    if (asset) return { version, assetName, url: asset.browser_download_url };
  }
  return null;
}

// Where a downloaded CLI lives inside the extension's global storage.
function downloadedCliPath(storageDir, platform) {
  return path.join(storageDir, "bin", platform === "win32" ? "functor.exe" : "functor");
}

// `cmd --version`'s first stdout line, or null when it can't run (ENOENT/any
// spawn error, nonzero exit, 10s hang). Doubles as the availability probe
// (commandWorks) and the version shown in the status bar tooltip.
function commandVersion(cmd) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (v) => {
      if (!settled) resolve(v);
      settled = true;
    };
    let child;
    try {
      child = spawn(cmd, ["--version"], { stdio: ["ignore", "pipe", "ignore"] });
    } catch {
      return done(null);
    }
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.on("error", () => done(null));
    child.on("exit", (code) => done(code === 0 ? out.trim().split("\n")[0] || "unknown" : null));
    setTimeout(() => {
      done(null);
      try {
        child.kill();
      } catch {}
    }, 10000).unref();
  });
}

async function commandWorks(cmd) {
  return (await commandVersion(cmd)) !== null;
}

// GET returning parsed JSON. GitHub requires a User-Agent. Bounded: a stalled
// request must not leave "Open Live Preview" pending forever.
function fetchJson(url) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { "User-Agent": "functor-lang-vscode" } }, (res) => {
      if (res.statusCode !== 200) {
        res.resume();
        return reject(new Error(`GET ${url} → HTTP ${res.statusCode}`));
      }
      let body = "";
      res.on("data", (d) => (body += d));
      res.on("end", () => {
        try {
          resolve(JSON.parse(body));
        } catch (e) {
          reject(e);
        }
      });
    });
    req.setTimeout(15000, () => req.destroy(new Error("request timed out")));
    req.on("error", reject);
  });
}

// Download to a file, following redirects (release assets 302 to the CDN).
// onProgress(received, total) fires per chunk; total may be 0 when unknown.
// Bounded by an inactivity timeout; any failure destroys the write stream
// and removes the partial file.
function download(url, dest, onProgress, redirectsLeft = 5) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { "User-Agent": "functor-lang-vscode" } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume();
        if (redirectsLeft === 0) return reject(new Error("too many redirects"));
        return resolve(download(res.headers.location, dest, onProgress, redirectsLeft - 1));
      }
      if (res.statusCode !== 200) {
        res.resume();
        return reject(new Error(`GET ${url} → HTTP ${res.statusCode}`));
      }
      const total = Number(res.headers["content-length"]) || 0;
      let received = 0;
      const out = fs.createWriteStream(dest);
      const fail = (err) => {
        out.destroy();
        fs.rmSync(dest, { force: true });
        reject(err);
      };
      res.on("data", (chunk) => {
        received += chunk.length;
        if (onProgress) onProgress(received, total);
      });
      res.pipe(out);
      out.on("finish", () => resolve());
      out.on("error", fail);
      res.on("error", fail);
    });
    req.setTimeout(30000, () => req.destroy(new Error("download stalled")));
    req.on("error", reject);
  });
}

// Extract the archive (system tar: present on macOS/Linux and Windows 10+,
// where bsdtar also reads .zip) and install the binary it contains
// (functor-<version>-<target>/functor[.exe]) as downloadedCliPath. Returns
// the installed path.
async function extractAndInstall(archivePath, storageDir, platform, version, target) {
  const extractDir = path.join(storageDir, "extract-tmp");
  fs.rmSync(extractDir, { recursive: true, force: true });
  fs.mkdirSync(extractDir, { recursive: true });
  const tarFlags = archivePath.endsWith(".zip") ? "-xf" : "-xzf";
  await new Promise((resolve, reject) => {
    execFile("tar", [tarFlags, archivePath, "-C", extractDir], (err) =>
      err ? reject(err) : resolve()
    );
  });
  const bin = platform === "win32" ? "functor.exe" : "functor";
  // The tarballs nest a functor-<version>-<target>/ directory; the Windows
  // zip is packaged from `$staging/*` and carries the binary at the archive
  // root (release-binaries.yml). Accept both.
  const extracted = [
    path.join(extractDir, `functor-${version}-${target}`, bin),
    path.join(extractDir, bin),
  ].find(fs.existsSync);
  if (!extracted) {
    throw new Error(`archive did not contain ${bin}`);
  }
  const installed = downloadedCliPath(storageDir, platform);
  fs.mkdirSync(path.dirname(installed), { recursive: true });
  fs.copyFileSync(extracted, installed);
  if (platform !== "win32") fs.chmodSync(installed, 0o755);
  fs.rmSync(extractDir, { recursive: true, force: true });
  return installed;
}

module.exports = {
  RELEASES_URL,
  assetTargetFor,
  pickAsset,
  downloadedCliPath,
  commandVersion,
  commandWorks,
  fetchJson,
  download,
  extractAndInstall,
};
