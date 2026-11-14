// `npm run build:cli`: the web runtime bundle (the CLI embeds it), then the
// release `functor` → target/release/functor. On macOS it carries the
// desktop app (D40): the app's UI bundle first, then `--features whisper`
// (which turns on `app`), as `npm run build:app` builds `functor-app`.
// Elsewhere, where the app has no window yet, the CLI alone.
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// npm-installed tools are `.cmd` shims on Windows, which only a shell runs.
const run = (cmd, args) => execFileSync(cmd, args, { stdio: "inherit", cwd: root, shell: process.platform === "win32" });

run("wasm-pack", ["build", "runtime/functor-runtime-web", "--target=web"]);
const app = process.platform === "darwin";
if (app) run("npm", ["run", "build:app-ui"]);
run("cargo", ["build", "--release", "-p", "functor-cli", "--bin", "functor", ...(app ? ["--features", "whisper"] : [])]);
