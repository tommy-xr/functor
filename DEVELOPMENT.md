# Building & developing Functor

This covers building the `functor` CLI from source and what the `build`/`run`
commands do under the hood. For installing and *using* Functor — running the
samples, writing a game, and the CLI commands — see the [README](README.md).

## Prerequisites

Install the following (the versions in parentheses are known-good):

- [Rust](https://rustup.rs/) stable (`1.91`) with the wasm target:
  `rustup target add wasm32-unknown-unknown`
- [Node.js + npm](https://nodejs.org/) (`node >=22.18`, `npm 10`) — the site build imports
  TypeScript sources directly, and Node strips types by default only from 22.18
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) (`0.12+`) — `npm install -g wasm-pack`

On Linux you also need the native GL/X11 dev packages (see
`.github/workflows/build-native.yml` for the exact `apt` list).

## PR preview infrastructure

Eligible pull requests build the static site without secrets, then a trusted
default-branch workflow independently authorizes and deploys the artifact to
`pr-<number>.preview.functor.games`. Preview deployment is currently limited to
PRs authored and pushed by `tommy-xr` from branches in `tommy-xr/functor`.

The deploy and close-cleanup workflows require these repository secrets:

- `CLOUDFLARE_ACCOUNT_ID` — the account containing the `functor.games` zone.
- `CLOUDFLARE_PREVIEW_API_TOKEN` — a dedicated token scoped to that account and
  zone with **Account / Workers Scripts: Edit** and
  **Zone / SSL and Certificates: Edit** for `functor.games`. Workers Scripts
  covers Worker deploy/delete and Custom Domain attach/detach; the zone
  permission is only for best-effort cleanup of the certificate Cloudflare
  generates for each Custom Domain.

The split is intentional: `.github/workflows/pr-preview-build.yml` runs PR code
with read-only repository access and no secrets. After it uploads `site/dist`,
`.github/workflows/pr-preview-deploy.yml` runs code from the default branch,
rechecks the workflow, actor, repository, open PR, and current head SHA, validates
the artifact as static data, and only then exposes the preview token. Authorization
rejects duplicate/expired artifacts and archives over 50 MiB; validation rejects
expanded bundles over 100 MiB, individual assets over 25 MiB, symlinks, and
Cloudflare deployment-control files. On close,
`.github/workflows/pr-preview-cleanup.yml` detaches the domain and deletes the
temporary Worker. A domain or Worker cleanup failure fails the job and marks the
sticky PR comment with a warning; certificate cleanup is non-quota-critical and
warns without failing the otherwise successful cleanup. It resolves the
certificate pack from Cloudflare's exact domain certificate ID, with bounded
inventory retries, before detaching the domain; it never deletes a certificate
using only a hostname match.

These event workflows must already exist on the default branch, so the PR that
introduces them cannot preview itself. After rollout, push a new commit to an
eligible open PR to exercise the full path. The hermetic lifecycle checks are:

```sh
npm run test:pr-preview
```

## Trying a PR without building it

CI uploads, for every main push and PR head, a dev-signed Quest APK
(`Build Android`) and a release `functor` CLI for macOS arm64 and Linux x64
(`Build CLI`), kept 30 days. `npm run pr` fetches the ones for a
PR's head commit (needs `gh`; cached under `~/.functor/pr-artifacts/<sha>/`):

```sh
npm run pr -- 123                    # install + launch the APK, print the CLI path
npm run pr -- 123 --cli              # just the CLI
npm run pr -- 123 --apk --run hello  # install the APK, then run examples/hello on it
```

It refuses fork PRs and a head commit whose build is missing, running or failed.
Replacing an install signed with another key, or a newer one, needs
`--allow-uninstall`. The example comes from this checkout, not the PR. Delete
`~/.functor/pr-artifacts` (or `$FUNCTOR_PR_CACHE`) to reclaim the cache.
Tests: `npm run test:pr`.

`f` runs either CLI with any `functor` args, from any directory:

```sh
f -d examples/hello run native            # this checkout's target/{release,debug}/functor, the newer
f --pr 123 -d examples/hello test         # PR 123's CI-built CLI (same cache)
f --pr 123 -d examples/hello run vr       # also installs PR 123's APK unless the headset runs it
```

`--pr` may go anywhere before a `--`. With `run vr`, `f --pr` downloads the CLI
and the APK before touching the headset, then installs the APK only when the
headset's runner reports another build (`/state.build`), or, if the runner is
down, when `~/.functor/apk-installed/<serial>` (written by `f` and `npm run pr`
installs; void once the package is reinstalled elsewhere) records another. `--force-apk` reinstalls anyway, `--no-apk` skips
all APK handling, and `--allow-uninstall` works as above. Without `--pr`,
`run vr` refuses a headset runner on another debug protocol than this
checkout's, and warns when it runs a PR build or can't be reached. Put `f` on your PATH with
`npm link` (once, from the repo root) or `alias f="node ~/code/functor/scripts/f.mjs"`;
`npm run f -- …` works too, but resolves `-d` from the repo root. The downloaded
binary is not notarized.

## Building

Three commands cover it:

```sh
npm run build                          # everything, release: ./target/release/functor works with nothing else
npm run app [-- -d <project> …]        # the app loop: build what changed, open the app (release)
npm run package:app                    # macOS: target/package/Functor.app
```

- **`npm run build`** builds, in order: the web runtime bundle, the app's UI
  bundle (macOS), the release `functor` (on macOS with the desktop app and its
  speech-to-text built in), the editor's language wasm, and the site. The
  binary embeds the two bundles, so they come first. Afterwards
  `./target/release/functor` with no arguments opens the app (macOS) and
  `./target/release/functor -d <project> run` runs a game.
- **`npm run app`** is the loop while working on the app or the engine: it
  rebuilds the UI bundle when its sources changed, then the combined binary
  (cargo is incremental), and opens the app inside it on the directory you ran
  it from (else your last project). Arguments after `--` go to the app
  (`-d <project>`, `--headless`, `--no-device-link`). The web runtime bundle is
  built only when missing (it is only for `run wasm`; run `npm run build` after
  changing the web runtime). Release by default, as the live game view is far
  too slow in debug; `npm run app:debug` compiles faster when you only check
  that something loads. The app and every game and tool it starts are that one
  executable, so they never mismatch. The window is macOS-only for now:
  elsewhere use `npm run app -- --headless` and open the printed URL.
- **`npm run package:app`** bundles the release `functor` as `Functor.app`
  (below).

The parts, for CI and scripts:

| Part | What it builds |
| --- | --- |
| `npm run build:cli` | Web runtime bundle, then `target/release/functor` (macOS: with the app; `-- --cli-only` for the CLI alone). The first step of `build`. |
| `npm run build:cli:debug` | Web runtime bundle, then `target/debug/functor`, the CLI alone. |
| `npm run build:app-ui` | The app's UI bundle, `app/ui/dist`. |
| `npm run build:app` | The standalone `functor-app` shim, for the app's tests. |

By hand: `wasm-pack build runtime/functor-runtime-web --target=web`, then
`cargo build --release --bin functor` (the CLI alone; `-p functor-cli
--features app` or `whisper` adds the app, after `npm run build:app-ui`).

**Prefer release builds for interactive use** — the debug build's CPU-bound
paths (the webview overlay's software raster, rapier physics) are far slower
live. Debug compiles faster, so it suits quick headless checks, frame captures
and the e2e scripts.

Every build produces a single `functor` binary (the CLI, with the desktop
runtime linked in as a library and run in-process — there is no separate
`functor-runner`), under `target/release/` or `target/debug/`.

## Desktop app (preview)

The desktop app (under `app/`) is a chat-and-game-view app around a project;
using it is covered in [the manual](https://functor.games/manual/app/). It is a
library the `functor` CLI links behind its `app` feature (`whisper` adds
speech-to-text and implies it): `functor` with no subcommand opens it
in-process, and every game session and agent tool server it starts is that
same executable again. `npm run build` and `npm run app` build it in on macOS;
a plain `cargo build` leaves it out. Elsewhere the window does not exist yet:
`--features app` gives `functor app -- --headless`.

`whisper` compiles whisper.cpp. Its model (about 148 MB) is fetched on first
use into `~/.functor/models` (`FUNCTOR_MODELS` overrides), not at build time.
`npm run app:debug` and `--features app` leave it out: the composer shows no
mic, and a headset's voice clip is answered with "speech-to-text is not in this
build".

The app's own binary, `functor-app`, is still built for the tests and scripts
that start it by name (`npm run build:app`, `cargo build -p functor-app`). It
runs the `functor` it finds (`--functor`, `FUNCTOR_BIN`, next to itself, then
PATH) and checks it first: one too old for the app is named in a banner with
the build that fixes it, and nothing is started with it. A `functor` built
without the app runs that `functor-app` instead (next to itself,
`FUNCTOR_APP_BIN`, PATH).

Run it:

```sh
./target/release/functor -d examples/primitives                     # the window (macOS only for now); `functor app` is the same
./target/release/functor app -d examples/primitives -- --headless   # no window: open the printed URL in a browser
```

`--headless` prints `[functor-app] listening on http://127.0.0.1:<port>/#token=…`.
The token is new each launch and the server refuses the page's socket without
it, so open that exact URL (a reload of the page needs it again). Saved tasks,
transcripts, headset pairings and the window's size and place live in
`~/.functor/app/` (`FUNCTOR_APP_HOME` overrides).

Besides the loopback UI server, the app opens the **device link**: a listener on
every interface (a free port) with an mDNS advert, for headsets to pair, and an
`adb devices` poll for headsets on a cable. `--no-device-link` turns all three
off; `--device-link <addr>` chooses where it listens.

### Testing the app

| Command | What it runs | Needs |
| --- | --- | --- |
| `cargo test -p functor-app-core -p functor-app -p functor-stt` | The core's state machine, and the app end to end over its socket under a fake agent (`FUNCTOR_APP_FAKE_AGENT=<fixture>`). | `FUNCTOR_BIN=<a built functor>` for the tests that start a real game or tool server; they print `skipped` without it. |
| `npm run test:app-ui-unit` | The UI's pure models (`app/ui/src/*.test.ts`) under `node --test`. | Nothing built. |
| `npm run check:app-ui` | `tsc` over the UI. | — |
| `cargo test -p functor-cli --features app --test default_app` | The app in-process in `functor`: a game session started with that same executable. | — |
| `npm run test:app-ui` | Builds the UI and a debug `functor-app`, then drives the page in Playwright's Chromium (`app/ui/e2e/*.mjs`) against `--headless --no-gpu`, with the fake agent and `device-stub` headsets. | `FUNCTOR_BIN` for the scripts that need a real `functor` (new project, agent views, the Headset placement); `APP_UI_SHOTS=<dir>` saves screenshots. |
| `npm run metrics:app` | Each scenario in `app/metrics/scenarios.json` through `functor-app task --review` on a fresh git copy of its example, judged mechanically; writes `app/metrics/results/<stamp>-<mode>.jsonl` and a `.md` table. Replays recorded runs by default. | Debug `functor-app` and `functor` (`FUNCTOR_APP_BIN` / `FUNCTOR_BIN` override). `FUNCTOR_APP_LIVE=1 npm run metrics:app -- --live` runs the real agent and **spends tokens**; `-- --compare a.jsonl b.jsonl` prints deltas. |
| `npm run test:metrics-app` | The metrics runner's own tests. | — |

The wire types the UI imports (`app/ui/src/generated/protocol.ts`) are generated
from the core; after changing one, regenerate with
`UPDATE_TS=1 cargo test -p functor-app-core --features ts ts_bindings` (CI fails
if stale). `cargo test -p functor-stt --features whisper` builds whisper.cpp but
fetches no model; its real transcription test runs only with
`FUNCTOR_STT_REAL=1`.

**Tests pass `--no-device-link`.** Every test that starts the app does, unless
it is about the link: a test run must not open a port on the LAN, advertise the
machine by mDNS, or talk to a headset that happens to be plugged in. The tests
that do pair use `--device-link 127.0.0.1:0 --no-mdns --adb <a fake>` against
`functor-app device-stub` processes, so they stay on loopback too.

### Quest tests

These drive a real headset and are skipped unless `FUNCTOR_QUEST_E2E=1` (the npm
scripts set it). All need one Quest on `adb` with a debuggable
`dev.functor.runner` installed (`npm run build:oculus:apk`), on the same Wi-Fi
as this machine. Controller presses are injected, so nobody wears the headset.
Each puts the headset's own pairing back afterwards;
`FUNCTOR_QUEST_CAPTURES=<dir>` keeps stills.

| Command | What it proves | Also needs |
| --- | --- | --- |
| `npm run test:quest-link` | Pairing by code over Wi-Fi and mDNS, the tunnel, relinking, Forget. | — |
| `npm run test:quest-cable` | Pairing over the cable with no code; the headset then links over Wi-Fi. | — |
| `npm run test:quest-run` | `functor run vr --device-url` pushes source and a large asset through the tunnel alone (no `adb forward`), and an edit hot-reloads. | `FUNCTOR_BIN` |
| `npm run test:quest-placement` | The Headset placement from the app: run there, pause and step, capture through the tunnel, return to the desktop. | `FUNCTOR_BIN` |
| `npm run test:quest-voice` | Push-to-talk and every state of the status toast, against a stub desktop. The microphone is replaced by a test tone. | — |

### Functor.app (macOS)

```sh
npm run package:app        # builds the release functor (with the app) → target/package/Functor.app
open target/package/Functor.app
open target/package/Functor.app --args -d ~/code/my-game
cp -R target/package/Functor.app /Applications/
```

The bundle holds one executable, `functor` with the app and its UI built in
(`Contents/MacOS`), and the icon (`app/package/icon.svg`). Its game sessions
and tool servers are that executable too, run by `Contents/Helpers/functor`, a
symbolic link to it: from outside `MacOS` they do not take the app's bundle
id. A hidden game also declares itself a background process, so it takes no
Dock tile of its own and Quit quits the app. App flags follow `app`:
`open target/package/Functor.app --args app --no-device-link`. It is ad-hoc
signed: it runs on the Mac that built it, but Gatekeeper rejects it elsewhere
(a downloaded copy is quarantined and blocked as from an unidentified
developer). Opened from Finder or the Dock, it opens the last project and takes
`PATH` from your login shell, so `claude`, `codex` and `adb` are found. While it
runs, `open` only brings it forward: `--args` reach a fresh launch. The first
mic recording and the first headset search ask for microphone and
local-network access, and ask again after each rebuild (an ad-hoc signature
changes with the binary).

## What `build`/`run` do under the hood

The `functor` binary is **self-contained**: the desktop runtime is linked in and the
web runtime bundle is embedded (via `include_bytes!`), so there is no separate runner
process and nothing to compile at run time — it **interprets** your `.fun` directly.

- `build` typechecks the whole project (every sibling `.fun` — file = module); diagnostics are errors.
- (native) `run` drives the desktop runtime in-process, interpreting the `.fun` each frame and
  hot-reloading it on save with the model preserved.
- (wasm) `run` serves the project directory — the `.fun` ships as text and the embedded web
  runtime fetches and interprets it. (File-watch hot-reload is native-only; reload the page.)
