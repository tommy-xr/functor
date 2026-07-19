// tproto.fun — the wire ADT shared by tclient.fun and tserver.fun
// (file = module: both roles load this sibling as `Tproto`). This is the
// typed-message story: the protocol is ONE declaration both ends typecheck
// against, sent with `Effect.sendMsg` and received as `Net.Data` — no string
// codec, no parse, no drift.

type Wire =
  | Ping(n: float)
  | Pong(n: float)
