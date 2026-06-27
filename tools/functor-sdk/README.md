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

`stepAll(clients, dt)` advances several clients by one lockstep frame. Launch N
runners on separate ports (networked via `Sub.connect`/`Sub.listen`), pin every
clock, and `stepAll` them each tick to keep simulations in sync — the
out-of-process counterpart to the in-process `functor-netsim` harness.

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

The e2e tests require the runner binary and the game dylib to be built, and a
display to open the GL window:

```sh
cargo build --bin functor-runner
./target/debug/functor -d examples/hello build native
```

The headline e2e (`held-input.e2e.test.ts`) is the durable guard for the
input→state→step loop: inject `up`, step a frame, assert the model's `held.up`
flips true (and back on release), then capture a valid PNG.
