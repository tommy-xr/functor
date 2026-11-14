// `npm run build:cli`: the web runtime bundle (the CLI embeds it), then the
// release `functor` → target/release/functor. On macOS it carries the
// desktop app (D40): the app's UI bundle first (only when its sources are
// newer: rebuilding it would relink the app), then `--features whisper`
// (which turns on `app`), as `npm run build:app` builds `functor-app`.
// Elsewhere, where the app has no window yet, the CLI alone.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// npm-installed tools are `.cmd` shims on Windows, which only a shell runs.
const run = (cmd, args) => execFileSync(cmd, args, { stdio: "inherit", cwd: root, shell: process.platform === "win32" });

/** The newest modification time under `path` (0 when it is missing). */
function newest(path) {
  if (!existsSync(path)) return 0;
  const stat = statSync(path);
  if (!stat.isDirectory()) return stat.mtimeMs;
  return Math.max(0, ...readdirSync(path).map((name) => newest(join(path, name))));
}

run("wasm-pack", ["build", "runtime/functor-runtime-web", "--target=web"]);
const app = process.platform === "darwin";
if (app) {
  const ui = join(root, "app/ui");
  const inputs = ["src", "index.html", "build.mjs"].map((p) => newest(join(ui, p)));
  if (newest(join(ui, "dist/index.html")) < Math.max(...inputs)) run("npm", ["run", "build:app-ui"]);
}
run("cargo", ["build", "--release", "-p", "functor-cli", "--bin", "functor", ...(app ? ["--features", "whisper"] : [])]);
