# @functor/sdk

A Playwright-style TypeScript SDK for driving the functor **debug runtime** — the
`--debug-port` HTTP control server on `functor-runner` (see
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

// Launch a game and drive it deterministically.
await using game = await FunctorRunner.launch({ gameDir: "examples/hello" });

await game.pause();              // pin the clock
await game.keyDown("up");        // inject input
await game.step();               // advance exactly one frame
const state = await game.state();// observe the result
const png = await game.capture();// PNG bytes of the frame
// `await using` shuts the runtime down at scope exit.
```

`FunctorRunner.connect(url)` attaches to an already-running runtime instead of
spawning one (and won't kill it on dispose).

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
`mpserver` + two `mpclient` runners and waits until the server tracks 2 players
and each client converges on a 2-player world.

```ts
await using a = await FunctorRunner.launch({ gameDir: "examples/pong", port: 8077 });
await using b = await FunctorRunner.launch({ gameDir: "examples/pong", port: 8078 });
await Promise.all([a.pause(), b.pause()]);
for (let frame = 0; frame < 600; frame++) {
  await a.keyDown("up");        // per-client input
  await stepAll([a, b]);        // both advance one frame together
}
```

## Tests

```sh
npm test          # unit tests only (no runtime needed)
npm run test:e2e  # FUNCTOR_E2E=1 — launches a real functor-runner
```

The e2e tests require the runner binary and the relevant game dylibs to be built,
and a display to open the GL window:

```sh
cargo build --bin functor-runner
./target/debug/functor -d examples/hello build native      # held-input test
./target/debug/functor -d examples/mpserver build native   # multiplayer test
./target/debug/functor -d examples/mpclient build native   # multiplayer test
```

(If a build reports a missing `*.rs`, the Fable cache is stale — regenerate with
`dotnet fable <example>/<name>.fsproj --lang rust --outDir . --noCache`.)

The headline e2e (`held-input.e2e.test.ts`) is the durable guard for the
input→state→step loop: inject `up`, step a frame, assert the model's `held.up`
flips true (and back on release), then capture a valid PNG.
