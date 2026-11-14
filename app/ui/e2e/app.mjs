// Shared by the app UI's browser tests: launching `functor-app --headless`
// with a throwaway home (so no test reads or writes the user's saved tasks),
// and watching a page for console errors and non-loopback requests.
import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect } from "@playwright/test";

export const ROOT = fileURLToPath(new URL("../../..", import.meta.url));
const EXE = process.platform === "win32" ? ".exe" : "";
export const APP = `${ROOT}target/debug/functor-app${EXE}`;
export const FUNCTOR = process.env.FUNCTOR_BIN && existsSync(process.env.FUNCTOR_BIN) ? process.env.FUNCTOR_BIN : null;
const LOOPBACK = new Set(["127.0.0.1", "localhost"]);

/** A session's own clock (`/state`'s `frame` and `clock`), without the model. */
export async function clock(debugUrl) {
  const state = await (await fetch(`${debugUrl}/state?model=0`)).json();
  return { frame: state.frame, ...state.clock };
}

/** Connections to `port` held by anything but functor and functor-app: the page's streams. */
export function openStreams(port) {
  let out = "";
  try {
    out = execFileSync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:ESTABLISHED"], { encoding: "utf8" });
  } catch {
    // lsof exits 1 when nothing matches.
  }
  return out
    .split("\n")
    .slice(1)
    .filter((l) => l && !l.startsWith("functor") && l.includes(`->127.0.0.1:${port} `)).length;
}

/** Exits unless functor-app and the built UI exist. */
export function requireBuilt() {
  for (const [path, what] of [
    [APP, "cargo build -p functor-app"],
    [`${ROOT}app/ui/dist/index.html`, "npm run build:app-ui"],
  ]) {
    if (!existsSync(path)) {
      console.error(`FAIL: ${path} missing — run ${what}`);
      process.exit(1);
    }
  }
}

/** With APP_UI_SHOTS=<dir>, saves the page (`fullPage`: all of it) as `<dir>/<name>.png`. */
export async function shot(page, name, { fullPage = false } = {}) {
  const dir = process.env.APP_UI_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: `${dir}/${name}.png`, fullPage });
}

const temps = [];
process.on("exit", () => temps.forEach((d) => rmSync(d, { recursive: true, force: true })));

/**
 * Stores the committed path captures as the predicted path of the only
 * task under the (stopped) app's `home`: its moment at frame 5, 121 steps.
 * Resolves with the task's file and what it now holds.
 */
export function storePaths(home) {
  const evidence = join(home, "evidence");
  mkdirSync(evidence, { recursive: true });
  const before = "0123456789abcdef0123456789abcdef-before.png";
  const after = "0123456789abcdef0123456789abcdef-after.png";
  copyFileSync(`${ROOT}app/ui/e2e/golden/paths-before.png`, join(evidence, before));
  copyFileSync(`${ROOT}app/ui/e2e/golden/paths-after.png`, join(evidence, after));
  const projects = join(home, "projects");
  const file = readdirSync(projects)
    .map((p) => join(projects, p, "tasks", "1.json"))
    .find((f) => existsSync(f));
  if (!file) throw new Error("no stored task");
  const saved = JSON.parse(readFileSync(file, "utf8"));
  saved.task.moment = { kind: "taken", frame: 5, steps: 121 };
  saved.task.evidence = { tree: saved.task.change.tree, state: { kind: "shown", before, after } };
  writeFileSync(file, JSON.stringify(saved));
  return { file, saved };
}

/** A fresh temp directory, removed when the process exits. */
export function tempDir(prefix) {
  temps.push(mkdtempSync(join(tmpdir(), prefix)));
  return temps.at(-1);
}

/** A fresh app home, removed when the process exits. */
export function tempHome() {
  return tempDir("functor-app-e2e-");
}

/** Git reading no user or system config, as every git these tests start (the app's included). */
export const GIT_ENV = { GIT_CONFIG_GLOBAL: "/dev/null", GIT_CONFIG_NOSYSTEM: "1" };

/** Runs git in `dir` with GIT_ENV and a fixed identity. */
export function git(dir, ...args) {
  return execFileSync("git", ["-C", dir, "-c", "user.name=Test", "-c", "user.email=test@example.com", ...args], {
    env: { ...process.env, ...GIT_ENV },
    encoding: "utf8",
  });
}

/**
 * A throwaway git repo holding a copy of `project` (a path under this repo)
 * at the same relative path, committed; its project dir. No test runs a
 * task in this repo itself.
 */
export function tempProject(project) {
  const repo = mkdtempSync(join(tmpdir(), "functor-app-e2e-repo-"));
  process.on("exit", () => rmSync(repo, { recursive: true, force: true }));
  const dir = join(repo, project);
  mkdirSync(dir, { recursive: true });
  cpSync(join(ROOT, project), dir, { recursive: true });
  git(repo, "init", "-q", "-b", "main");
  git(repo, "add", "-A");
  git(repo, "commit", "-q", "-m", "first");
  return dir;
}

/**
 * Starts functor-app and resolves with it, its URL (carrying the launch's
 * `#token=`), port and token once it listens. `home` defaults to a fresh temp
 * dir; pass the previous server's `home` and `token` to restart it with its
 * tasks for the same open page. `fakeAgent` is a fixture for the fake agent,
 * `fakeReview` one for its reviews.
 * `dir: null` passes no `-d`, so the app picks its folder from `cwd` and its
 * recent projects. `deviceLink` turns the device link on (loopback, no mDNS)
 * and resolves with its address as `link`; its cable side runs `adb` (a fake,
 * or by default none, so no real headset is touched). `functor`: the CLI it
 * runs (default `FUNCTOR_BIN`).
 */
export function startServer(
  port = 0,
  { dir = ROOT, cwd, gpu = false, home = tempHome(), fakeAgent, fakeReview, token, deviceLink = false, adb = "/nonexistent/adb", functor = FUNCTOR } = {},
) {
  const args = ["--headless", "--port", String(port)];
  // No LAN listener or mDNS advert unless a test pairs a stub device.
  if (!deviceLink) args.push("--no-device-link");
  else args.push("--device-link", "127.0.0.1:0", "--no-mdns", "--adb", adb);
  if (dir) args.push("-d", dir);
  if (!gpu) args.push("--no-gpu");
  if (functor) args.push("--functor", functor);
  // XDG_CONFIG_HOME too: git also reads its ignore and attributes files there.
  const env = { ...process.env, ...GIT_ENV, XDG_CONFIG_HOME: home, FUNCTOR_APP_HOME: home };
  if (fakeAgent) env.FUNCTOR_APP_FAKE_AGENT = fakeAgent;
  else delete env.FUNCTOR_APP_FAKE_AGENT;
  // Without one, the fake agent runs no reviews.
  if (fakeReview) env.FUNCTOR_APP_FAKE_REVIEW = fakeReview;
  else delete env.FUNCTOR_APP_FAKE_REVIEW;
  if (token) env.FUNCTOR_APP_TOKEN = token;
  else delete env.FUNCTOR_APP_TOKEN;
  const child = spawn(APP, args, { stdio: ["ignore", "ignore", "pipe"], env, cwd });
  let err = "";
  return new Promise((resolve, reject) => {
    child.stderr.on("data", (d) => {
      err += d;
      // Up to the newline: the line can arrive split across chunks.
      const m = err.match(/^\[functor-app\] listening on (http:\/\/\S+)\r?\n/m);
      if (!m) return;
      // With the device link, also where headsets dial (the next line).
      const link = err.match(/^\[functor-app\] device link on (\S+)\r?\n/m)?.[1];
      if (deviceLink && !link) return;
      const url = new URL(m[1]);
      resolve({ child, url: m[1], port: Number(url.port), home, token: url.hash.replace(/^#token=/, ""), link });
    });
    child.on("exit", (code) => reject(new Error(`functor-app exited (${code}) before listening:\n${err}`)));
  });
}

/**
 * A reload that keeps the connection: the page holds the token in memory
 * only, so a plain reload cannot reconnect; this opens the printed URL anew.
 */
export async function reopen(page, url) {
  await page.goto("about:blank");
  await page.goto(url);
}

/** Chooses `name` (a project, Open folder…, New project…) from the Explorer's project picker. */
export async function chooseProject(page, name) {
  await page.getByRole("complementary", { name: "Explorer" }).getByRole("button", { name: /^Project: / }).click();
  await page.getByRole("menu", { name: "Projects" }).getByRole("menuitem", { name, exact: true }).click();
}

/** Each pane's tab names, in reading order. */
export function tabs(page) {
  return page.locator(".pane-box").evaluateAll((panes) =>
    panes.map((p) => [...p.querySelectorAll(".pane-tabs [role=tab]")].map((t) => t.textContent)),
  );
}

export function stopServer({ child }) {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolve) => {
    child.on("exit", resolve);
    child.kill("SIGTERM");
  });
}

export function watchErrors(page) {
  const errors = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(String(e)));
  return errors;
}

/** The app's last state, as its WebSocket delivered it. */
export function watchState(page) {
  const app = { state: null };
  page.on("websocket", (ws) =>
    ws.on("framereceived", ({ payload }) => {
      const msg = JSON.parse(payload);
      if (msg.type === "state") app.state = msg.state;
    }),
  );
  return app;
}

/** Requests the page made to anything but loopback. */
export function watchExternal(page) {
  const external = [];
  page.on("request", (r) => {
    const { hostname } = new URL(r.url());
    if (!LOOPBACK.has(hostname)) external.push(r.url());
  });
  return external;
}

/** The server's stored pairings (`devices.json`, secrets and all), or "" before any. */
export function storedPairings(server) {
  try {
    return readFileSync(join(server.home, "devices.json"), "utf8");
  } catch {
    return "";
  }
}

// A failed step must not leave stubs retrying a gone desktop.
const stubs = [];
process.on("exit", () => stubs.forEach((c) => c.exitCode === null && c.kill()));

/** A `device-stub` headset on `link`; `line(prefix)` waits for its next line starting so. */
export function stub(link, store, name, extra = []) {
  const child = spawn(APP, ["device-stub", "--desktop", link, "--name", name, "--store", store, ...extra], {
    stdio: ["ignore", "pipe", "ignore"],
  });
  stubs.push(child);
  const lines = [];
  let buf = "";
  child.stdout.on("data", (d) => {
    buf += d;
    for (let i; (i = buf.indexOf("\n")) >= 0; buf = buf.slice(i + 1)) lines.push(buf.slice(0, i));
  });
  let seen = 0;
  return {
    async line(prefix) {
      let found;
      await expect
        .poll(
          () => {
            const i = lines.slice(seen).findIndex((l) => l.startsWith(prefix));
            if (i < 0) return false;
            found = lines[seen + i];
            seen += i + 1;
            return true;
          },
          { timeout: 20_000, message: `stub ${name}: ${prefix}` },
        )
        .toBe(true);
      return found;
    },
    stop: () => stopServer({ child }),
  };
}

/**
 * The mean absolute difference of two PNGs' pixels, decoded in the page
 * (0–255); within `region` (`[x0, y0, x1, y1]`, fractions of the image) when given.
 */
export function difference(page, a, b, region = [0, 0, 1, 1]) {
  return page.evaluate(
    async ([a, b, [x0, y0, x1, y1]]) => {
      const pixels = async (b64) => {
        const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
        const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        canvas.getContext("2d").drawImage(bitmap, 0, 0);
        const [x, y] = [Math.round(x0 * bitmap.width), Math.round(y0 * bitmap.height)];
        const [w, h] = [Math.round(x1 * bitmap.width) - x, Math.round(y1 * bitmap.height) - y];
        return canvas.getContext("2d").getImageData(x, y, w, h).data;
      };
      const [pa, pb] = [await pixels(a), await pixels(b)];
      if (pa.length !== pb.length) return 255;
      let sum = 0;
      for (let i = 0; i < pa.length; i++) sum += Math.abs(pa[i] - pb[i]);
      return sum / pa.length;
    },
    [a.toString("base64"), b.toString("base64"), region],
  );
}

/** Rewrites the copy's `jumpVelocity` line, as an editor's save would. */
export function setJump(dir, from, to) {
  const file = join(dir, "game.fun");
  const text = readFileSync(file, "utf8");
  if (!text.includes(`let jumpVelocity = ${from}`)) throw new Error(`game.fun has no jumpVelocity = ${from}`);
  writeFileSync(file, text.replace(`let jumpVelocity = ${from}`, `let jumpVelocity = ${to}`));
}
