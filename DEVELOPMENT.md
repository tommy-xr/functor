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

`watchexec` is no longer needed — Functor Lang hot-reload is built into the runtime, so
`functor develop` needs no external file watcher. There is also **no .NET / Fable
dependency**: the toolchain is Rust + Node only.

On Linux you also need the native GL/X11 dev packages (see
`.github/workflows/build-native.yml` for the exact `apt` list).

## Building the CLI

Build the CLI. **Order matters:** the CLI embeds the web runtime bundle at compile
time (via `include_bytes!`), so the wasm bundle must exist before the `functor` binary is built.

```sh
wasm-pack build runtime/functor-runtime-web --target=web     # web bundle (embedded into the CLI)
cargo build --bin functor                                    # the CLI (embeds the desktop runtime)
```

Or use the bundled convenience script, which runs both in order:

```sh
npm run build:cli
```

This produces a single binary in `target/debug/`: `functor` (the CLI, with the desktop
runtime linked in as a library and run in-process — there is no separate `functor-runner`).

## What `build`/`run` do under the hood

1. `build` loads the project (the entry `.fun` plus every sibling `.fun` — file = module)
   and typechecks the whole program; diagnostics are errors here.
2. (native) `run` runs the desktop runtime in-process on the entry `.fun` (no separate
   process); it **interprets** the `.fun` each frame and hot-reloads it on save,
   preserving the model.
3. (wasm) `run` serves the project directory — the `.fun` ships as text; the embedded
   web runtime fetches and interprets it. (File-watch hot-reload is native-only;
   reload the page to pick up saved edits.)
