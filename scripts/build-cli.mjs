// `npm run build:cli`, a part of `npm run build`: the web runtime bundle
// (the CLI embeds it), then the release `functor` → target/release/functor.
// On macOS it carries the desktop app (D40): the app's UI bundle first (only
// when its sources are newer: rebuilding it would relink the app), then
// `--features whisper` (which turns on `app`). Elsewhere, where the app has no
// window yet, the CLI alone.
//
// Order matters: the binary embeds both bundles, so they come first.
//
//   npm run build:cli -- --cli-only    the CLI alone on macOS too (lighter)
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// npm-installed tools are `.cmd` shims on Windows, which only a shell runs.
export const run = (cmd, args) =>
  execFileSync(cmd, args, { stdio: "inherit", cwd: root, shell: process.platform === "win32" });

/** The newest modification time under `path` (0 when it is missing). */
export function newest(path) {
  if (!existsSync(path)) return 0;
  const stat = statSync(path);
  if (!stat.isDirectory()) return stat.mtimeMs;
  return Math.max(0, ...readdirSync(path).map((name) => newest(join(path, name))));
}

/** Builds `app/ui/dist` when its sources are newer; its `index.html`. */
export function appUi() {
  const ui = join(root, "app/ui");
  const dist = join(ui, "dist/index.html");
  const inputs = ["src", "index.html", "build.mjs"].map((p) => newest(join(ui, p)));
  if (newest(dist) < Math.max(...inputs)) run("npm", ["run", "build:app-ui"]);
  return dist;
}

// Run as a script, not imported (`scripts/app.mjs`).
if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  run("wasm-pack", ["build", "runtime/functor-runtime-web", "--target=web"]);
  const app = process.platform === "darwin" && !process.argv.includes("--cli-only");
  if (app) appUi();
  run("cargo", ["build", "--release", "-p", "functor-cli", "--bin", "functor", ...(app ? ["--features", "whisper"] : [])]);
}
