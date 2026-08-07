// protocol.fun — what the two role FILES agree on, declared exactly once.
//
// `file = module`, so `client.fun` and `server.fun` both see this as
// `Protocol.*` with no import and no drift: the wire ADT, the snapshot shape,
// and the court constants that the simulation and the renderer must share.
// Values cross the socket as plain Functor data through `Effect.sendMsg` /
// `Net.Data` — there is no string codec, so a typo is a check-time error.

type Phase =
  | Serving(seconds: float)
  | Rally
  | Won(seat: float)

type Point = { x: float, y: float }

// The whole world, every 50 ms. `seq` is the only bookkeeping: a client
// compares it and drops anything that is not newer. Nothing here is a delta,
// and nothing is here that the renderer does not draw — the ball's VELOCITY
// stays server-side, because this client extrapolates nothing.
type Snapshot = {
  seq: float,
  leftY: float,
  rightY: float,
  ballX: float,
  ballY: float,
  leftScore: float,
  rightScore: float,
  phase: Phase,
  hitPulse: float,
  scorePulse: float,
  trail: List<Point>,
}

type Wire =
  | Welcome(seat: float)
  | PaddleIntent(seq: float, axis: float)
  | State(snapshot: Snapshot)
  | Rematch

let bind = "127.0.0.1:9108"
let serverUrl = "ws://127.0.0.1:9108/play"

let courtHalfWidth = 13.0
let courtHalfHeight = 7.0
let paddleYLimit = 5.35
let paddleHalfHeight = 1.45
let paddleX = 11.35
let ballRadius = 0.34
let paddleSpeed = 11.5
let winningScore = 5.0

let point = (x: float, y: float): Point => { x: x, y: y }

let initialSnapshot = (): Snapshot => {
  seq: 0.0,
  leftY: 0.0,
  rightY: 0.0,
  ballX: 0.0,
  ballY: 0.0,
  leftScore: 0.0,
  rightScore: 0.0,
  phase: Serving(2.2),
  hitPulse: 0.0,
  scorePulse: 0.0,
  trail: [],
}

let phaseLabel = (phase: Phase): string =>
  match phase with
  | Serving(seconds) => $"SERVE IN {Text.fixed(Math.ceil(seconds), 0.0)}"
  | Rally => ""
  | Won(seat) => if seat == 0.0 then "CYAN WINS" else "PINK WINS"

expect initialSnapshot().leftScore == 0.0
expect initialSnapshot().phase == Serving(2.2)
