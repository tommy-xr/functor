// `npm run build:cli`, a part of `npm run build`: the web runtime bundle
// (the CLI embeds it), then the release `functor` → target/release/functor.
// On macOS it carries the desktop app (D40): the app's UI bundle first (only
// when its sources changed), then `--features whisper` (which turns on
// `app`). Elsewhere, where the app has no window yet, the CLI alone.
//
// Order matters: the binary embeds both bundles, so they come first.
//
//   npm run build:cli -- --cli-only    the CLI alone on macOS too (lighter)
import { appUi, run } from "./build-lib.mjs";

run("wasm-pack", ["build", "runtime/functor-runtime-web", "--target=web"]);
const app = process.platform === "darwin" && !process.argv.includes("--cli-only");
if (app) appUi();
run("cargo", ["build", "--release", "-p", "functor-cli", "--bin", "functor", ...(app ? ["--features", "whisper"] : [])]);
