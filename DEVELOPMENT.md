# Building & developing Functor

This covers building the `functor` CLI from source and what the `build`/`run`
commands do under the hood. For installing and *using* Functor — running the
samples, writing a game, and the CLI commands — see the [README](README.md).

## Prerequisites

Install the following (the versions in parentheses are known-good):

- [Rust](https://rustup.rs/) stable (`1.91`) with the wasm target:
  `rustup target add wasm32-unknown-unknown`
- [Node.js + npm](https://nodejs.org/) (`node 22`, `npm 10`)
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) (`0.12+`) — `npm install -g wasm-pack`

On Linux you also need the native GL/X11 dev packages (see
`.github/workflows/build-native.yml` for the exact `apt` list).

## Building the CLI

Build the CLI. **Order matters:** the CLI embeds the web runtime bundle at compile
time (via `include_bytes!`), so the wasm bundle must exist before the `functor` binary is built.

```sh
wasm-pack build runtime/functor-runtime-web --target=web     # web bundle (embedded into the CLI)
cargo build --release --bin functor                          # the CLI (embeds the desktop runtime)
```

Or use the bundled convenience script, which runs both in order:

```sh
npm run build:cli          # release build → target/release/functor
npm run build:cli:debug    # debug build   → target/debug/functor
```

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
