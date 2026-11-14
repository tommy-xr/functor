// What `build-cli.mjs` and `app.mjs` share: running a step from the repo
// root, and building the app UI bundle when its sources changed.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

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

/** Builds `app/ui/dist` when its sources are newer (rebuilding it would relink the app); its `index.html`. */
export function appUi() {
  const ui = join(root, "app/ui");
  const dist = join(ui, "dist/index.html");
  const inputs = ["src", "index.html", "build.mjs"].map((p) => newest(join(ui, p)));
  if (newest(dist) < Math.max(...inputs)) run("npm", ["run", "build:app-ui"]);
  return dist;
}
