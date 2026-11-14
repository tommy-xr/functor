// `npm run app [-- <app arguments>]`: the developer loop. Builds what is
// stale, then opens the desktop app inside the `functor` just built, on the
// directory you ran npm from (else your last project). Release by default:
// a debug build's live game view is far too slow (`npm run app:debug` for a
// fast compile when you only check that something loads). E.g.
//
//   npm run app -- -d examples/platformer
//   npm run app:debug -- --headless
//
// In order, as the binary embeds both bundles: the web runtime bundle (only
// when missing: `wasm-pack` costs a minute and is only for `run wasm`; run
// `npm run build` after changing the web runtime), the app UI bundle (when
// its sources changed), then cargo (incremental). Never a stale half: the app
// and every game and tool it starts are this one executable (D40).
import { spawnSync } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import { join } from "node:path";
import { appUi, newest, root, run } from "./build-lib.mjs";

const debug = process.argv[2] === "--debug";
const args = process.argv.slice(debug ? 3 : 2);

if (!existsSync(join(root, "runtime/functor-runtime-web/pkg"))) {
  run("wasm-pack", ["build", "runtime/functor-runtime-web", "--target=web"]);
}
const dist = appUi();
// Speech-to-text on macOS (whisper.cpp), and not in debug: its C++ build
// would make the fast loop slow. (`build:cli` elsewhere and `build:cli:debug`
// build without the app, so switching between them relinks.)
const features = process.platform === "darwin" && !debug ? "whisper" : "app";
run("cargo", ["build", ...(debug ? [] : ["--release"]), "-p", "functor-cli", "--bin", "functor", "--features", features]);

const exe = join(root, "target", debug ? "debug" : "release", `functor${process.platform === "win32" ? ".exe" : ""}`);
// The UI is embedded at compile time: a binary older than it would be stale.
if (statSync(exe).mtimeMs < newest(dist)) throw new Error(`${exe} is older than the UI bundle it embeds`);
// From where npm was run, so a project there opens.
const opened = spawnSync(exe, ["app", ...args], { stdio: "inherit", cwd: process.env.INIT_CWD ?? process.cwd() });
if (opened.error) throw opened.error;
process.exit(opened.status ?? 1);
