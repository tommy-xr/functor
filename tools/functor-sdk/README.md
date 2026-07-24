# @functor/sdk

A Playwright-style TypeScript SDK for driving the functor **debug runtime** — the
`--debug-port` HTTP control server on the `functor` runtime (see
[`docs/debug-runtime.md`](../../docs/debug-runtime.md)). It lets a script, test, or
LLM **observe** and **drive** a running game headlessly.

## Install & build

```sh
cd tools/functor-sdk
npm install
npm run build     # tsc -> dist/
```

## Usage

```ts
import { FunctorRunner, stepAll } from "@functor/sdk";

// Launch a game and drive it deterministically. Functor Lang games are run via `functorLangPath`
// (the runner interprets the .fun — nothing to build).
await using game = await FunctorRunner.launch({
  gameDir: "examples/hello",
  functorLangPath: "examples/hello/game.fun",
});

await game.pause();              // pin the clock
await game.keyDown("up");        // inject input
await game.step();               // advance exactly one frame
const state = await game.state();// observe the result
const xr = await game.xrInput();  // rig-local head/controllers on XR targets
const png = await game.capture();// PNG bytes of the frame
// `await using` shuts the runtime down at scope exit.
```

`state.input` is one extensible sampled-input record. It always carries held
keys and the last mouse position; XR targets add typed rig-local head and
controller state (`game.xrInput()`). Future gamepad and mobile-touch domains
can extend the same record without target-specific clients.

`FunctorRunner.connect(url)` attaches to an already-running runtime instead of
spawning one (and won't kill it on dispose).

Project assets can be pushed from Node or a loopback-hosted browser without
target-specific APIs:

```ts
const quest = await FunctorRunner.connect("http://127.0.0.1:8123");
await quest.reloadAssets([
  ["models/Xbot.glb", new Uint8Array(await modelFile.arrayBuffer())],
  ["textures/grid.png", new Uint8Array(await textureFile.arrayBuffer())],
]);
await quest.loadProject([
  ["game.fun", gameSource],
  ["assets.fun", assetManifestSource],
]);
```

Files upload individually, then one manifest removes stale uploads. Upload the
initial asset set before `loadProject` so `init` and the first frame see it
resident. The same calls work against desktop and the adb-forwarded Quest runtime.
`loadProject` initializes a new game from `init`; use `reloadProject` for later
source edits that should preserve the live model.

By default the runner is launched with `--hidden`: the GL window is never shown
and never steals focus or the cursor, but keeps its GL context, so `capture()`
works. Pass `visible: true` to show the window (e.g. to watch a script drive the
game), or `headless: true` to launch with no GL window at all (`--headless`) — no
display needed, ideal for CI. Headless, `state()`, `scene()`, `input()`, and the
clock controls all work; `capture()` is unavailable (it returns a 503 — there are
no pixels).

### Observe vs. drive

- **Observe a human playing:** leave the clock alone and poll `state()`,
  `scene()`, `capture()`.
- **Drive it:** `pause()` → `keyDown`/`mouseMove` → `step()` → `state()`. Pinned
  time ignores window input but honors injected input, so it's deterministic.

## Multiplayer simulation

Launch N runners on separate debug ports, networked via `Sub.connect`/`Sub.listen`,
and drive them together — the out-of-process counterpart to the in-process
`functor-netsim` harness. `waitFor(poll, predicate, opts)` (and the
`client.waitForState(predicate, opts)` shorthand) polls until an async condition
holds, e.g. network convergence; `stepAll(clients, dt)` advances every client by
one lockstep frame.

`test/multiplayer.e2e.test.ts` does exactly this end-to-end: it launches one
server + two client runners (`examples/mp`, a multi-entry project) and waits
until the server tracks 2 players and each client converges on a 2-player
world.

```ts
const launch = (entry: string, port: number) =>
  FunctorRunner.launch({
    gameDir: "examples/mp",
    functorLangPath: `examples/mp/${entry}`,
    port,
  });
await using a = await launch("server.fun", 8077);
await using b = await launch("client.fun", 8078);
await Promise.all([a.pause(), b.pause()]);
for (let frame = 0; frame < 600; frame++) {
  await a.keyDown("up");        // per-client input
  await stepAll([a, b]);        // both advance one frame together
}
```

## Tests

```sh
npm test          # unit tests only (no runtime needed)
npm run test:e2e  # FUNCTOR_E2E=1 — launches a real functor runtime
```

The e2e tests require the `functor` binary to be built, and a display to open
the GL window. The games are Functor Lang sources (`examples/*`) interpreted in place, so
there is no per-game build step:

```sh
cargo build --bin functor
```

(The games driven by the tests are `examples/hello` — the held-input
test — and `examples/mp` (its `server.fun` / `client.fun` entries) — the
multiplayer test. A `build` step is optional: `functor -d <dir> build native`
just typechecks the `.fun`.)

The headline e2e (`held-input.e2e.test.ts`) is the durable guard for the
input→state→step loop: inject `up`, step a frame, assert the model's `held.up`
flips true (and back on release), then capture a valid PNG.
