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
warns without failing the otherwise successful cleanup.

These event workflows must already exist on the default branch, so the PR that
introduces them cannot preview itself. After rollout, push a new commit to an
eligible open PR to exercise the full path. The hermetic lifecycle checks are:

```sh
npm run test:pr-preview
```

## Building the CLI

Build the CLI. **Order matters:** the CLI embeds the web runtime bundle at compile
time (via `include_bytes!`), so the wasm bundle must exist before the `functor` binary is built.

```sh
wasm-pack build runtime/functor-runtime-web --target=web     # web bundle (embedded into the CLI)
cargo build --release --bin functor                          # the CLI (embeds the desktop runtime)
```

Or use the bundled convenience script, which runs both in order:

```sh
npm run build              # everything: CLI, both wasm bundles, and the site
npm run build:cli          # release build → target/release/functor
npm run build:cli:debug    # debug build   → target/debug/functor
```

`npm run build` is the one to reach for when you are unsure which pieces a
change touches: the individual targets are order-dependent, and the wrong order
fails at runtime rather than at build time (a site built against a stale runtime
bundle hangs at `loading…` instead of erroring).

**Prefer the release build (`npm run build:cli`) for interactive use** — the debug
build's CPU-bound paths (the webview overlay's software raster, rapier physics) are
far slower live. The debug build is faster to *compile*, so it's the better choice
for quick headless checks, frame captures, and the e2e scripts.

Either produces a single `functor` binary (the CLI, with the desktop runtime linked
in as a library and run in-process — there is no separate `functor-runner`), under
`target/release/` or `target/debug/` respectively.

## What `build`/`run` do under the hood

The `functor` binary is **self-contained**: the desktop runtime is linked in and the
web runtime bundle is embedded (via `include_bytes!`), so there is no separate runner
process and nothing to compile at run time — it **interprets** your `.fun` directly.

- `build` typechecks the whole project (every sibling `.fun` — file = module); diagnostics are errors.
- (native) `run` drives the desktop runtime in-process, interpreting the `.fun` each frame and
  hot-reloading it on save with the model preserved.
- (wasm) `run` serves the project directory — the `.fun` ships as text and the embedded web
  runtime fetches and interprets it. (File-watch hot-reload is native-only; reload the page.)
