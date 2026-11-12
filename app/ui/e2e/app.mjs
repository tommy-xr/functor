// Shared by the app UI's browser tests: launching `functor-app --headless`
// with a throwaway home (so no test reads or writes the user's saved tasks),
// and watching a page for console errors and non-loopback requests.
import { execFileSync, spawn } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = fileURLToPath(new URL("../../..", import.meta.url));
const EXE = process.platform === "win32" ? ".exe" : "";
export const APP = `${ROOT}target/debug/functor-app${EXE}`;
export const FUNCTOR = process.env.FUNCTOR_BIN && existsSync(process.env.FUNCTOR_BIN) ? process.env.FUNCTOR_BIN : null;
const LOOPBACK = new Set(["127.0.0.1", "localhost"]);

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

/** With APP_UI_SHOTS=<dir>, saves the page as `<dir>/<name>.png`. */
export async function shot(page, name) {
  const dir = process.env.APP_UI_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: `${dir}/${name}.png` });
}

/** A fresh app home, removed when the process exits. */
export function tempHome() {
  const home = mkdtempSync(join(tmpdir(), "functor-app-e2e-"));
  process.on("exit", () => rmSync(home, { recursive: true, force: true }));
  return home;
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
 * Starts functor-app and resolves with it, its URL and port once it listens.
 * `home` defaults to a fresh temp dir; pass the previous server's `home` to
 * restart it with its tasks. `fakeAgent` is a fixture for the fake agent.
 */
export function startServer(port = 0, { dir = ROOT, gpu = false, home = tempHome(), fakeAgent } = {}) {
  const args = ["--headless", "--port", String(port), "-d", dir];
  if (!gpu) args.push("--no-gpu");
  if (FUNCTOR) args.push("--functor", FUNCTOR);
  // XDG_CONFIG_HOME too: git also reads its ignore and attributes files there.
  const env = { ...process.env, ...GIT_ENV, XDG_CONFIG_HOME: home, FUNCTOR_APP_HOME: home };
  if (fakeAgent) env.FUNCTOR_APP_FAKE_AGENT = fakeAgent;
  else delete env.FUNCTOR_APP_FAKE_AGENT;
  const child = spawn(APP, args, { stdio: ["ignore", "ignore", "pipe"], env });
  let err = "";
  return new Promise((resolve, reject) => {
    child.stderr.on("data", (d) => {
      err += d;
      // Up to the newline: the line can arrive split across chunks.
      const m = err.match(/^\[functor-app\] listening on (http:\/\/\S+)\r?\n/m);
      if (m) resolve({ child, url: m[1], port: Number(new URL(m[1]).port), home });
    });
    child.on("exit", (code) => reject(new Error(`functor-app exited (${code}) before listening:\n${err}`)));
  });
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
