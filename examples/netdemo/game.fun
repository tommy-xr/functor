// netdemo — the Functor Lang port of examples/netdemo (roadmap E2, docs/functor-lang.md).
//
// A minimal HTTP sample in the Elm `Http.get { expect = ... }` style: it fires
// an HTTP GET whose result is tagged into a message and reflected into the
// model. Meant to be driven HEADLESSLY — the model is fully visible via the
// debug server's /state, and a response can be injected without a real server
// (see runtime/functor-runtime-common http tests). `draw` is deliberately tiny;
// the interesting state is the textual phase.
//
//   functor -d examples/netdemo run native
//
// Port note: Functor Lang's `init` is a plain value (no F# `GameBuilder.init` effect
// seed), so the request is fired ONCE on the first frame from `tick` — a bare
// model on later frames, a `(model, effect)` pair on frame 0.

// Where the request is in its lifecycle.
type Phase =
  | Loading
  | Done(status: float, body: string)
  | Failed(text: string)

type Model = { phase: Phase, frame: float }

// The HTTP result, tagged back to us. `Net.HttpResponse` is the built-in ADT
// `| Response(status, body) | Failure(error)` (a completed request vs a
// transport error); Functor Lang patterns don't nest, so `update` inner-matches it.
type Msg =
  | GotResponse(resp: Net.HttpResponse)

let endpoint = "http://127.0.0.1:9000/hello"

let init = { phase: Loading, frame: 0.0 }

let update = (m: Model, msg: Msg) =>
  match msg with
  | GotResponse(resp) =>
    (match resp with
     | Net.Response(status, body) => { m with phase: Done(status, body) }
     | Net.Failure(err) => { m with phase: Failed(err) })

// Fire the request once, on the first frame (the F# startup-effect seed); the
// runtime applies the `GotResponse` tagger when the response lands.
let tick = (m: Model, dt: float, tts: float) =>
  match m.frame == 0.0 with
  | true => ({ m with frame: 1.0 }, Effect.httpGet(endpoint, GotResponse))
  | false => { m with frame: m.frame + 1.0 }

let draw = (m: Model, tts: float) =>
  // A single emissive cube so there's something on screen; the meaningful
  // state is the textual phase, inspected via /state.
  Frame.create(
    Camera.firstPerson(
      0.0, 0.0, -5.0,
      Angle.radians(0.0), Angle.radians(0.0), Angle.degrees(60.0)),
    Scene.cube() |> Scene.emissive(Color.rgb(0.2, 0.9, 0.6)))
