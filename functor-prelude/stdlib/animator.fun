// Animator — the engine-bundled derived crossfade over Anim clips, in pure
// Functor Lang.
//
// The engine's Anim.* algebra is stateless: a pose is a function of explicit
// playheads and weights. This module derives smooth clip transitions from
// plain data stored in the game model — the elm-animator pattern reduced to
// depth 1:
//
//   { current, since, prevClip, prevSince, fadeStart }
//
// `play` records a state change (what changed, when); `pose` derives the
// blend at draw time. Playheads are CLIP-LOCAL (`tts - since`), so a clip
// always enters at its first frame. Because the state is plain model data
// and the weights derive from `tts`, scrubbing time-travel through a
// transition replays the fade exactly, and hot-reload carries it over.
//
// Interruption policy: `play` during an in-flight fade TRUNCATES — the old
// `current` becomes `prev` as-is and the fade restarts, so the outgoing pose
// can pop under rapid re-targeting (mash two keys to see it). The bounded
// alternative (snapshotting the mid-fade pose as data) can layer on later
// without changing this surface.

// The state for one animated character, stamped at `tts`.
let start = (clip: string, tts: float) =>
  {
    current: clip,
    since: tts,
    prevClip: clip,
    prevSince: tts,
    // Far in the past: the first pose is fully `current`, no fade-in.
    fadeStart: -1000.0,
  }

// Record a state change. Re-playing the current clip is a no-op (it keeps
// looping rather than restarting — call `start` to hard-reset).
let play = (clip: string, tts: float, st) =>
  match clip == st.current with
  | true => st
  | false =>
    {
      current: clip,
      since: tts,
      prevClip: st.current,
      prevSince: st.since,
      fadeStart: tts,
    }

// The derived pose: a smoothstep crossfade from `prev` to `current` over
// `fade` seconds, collapsing to the bare current clip once settled. A
// non-positive fade is a hard cut (guards the divide).
let pose = (st, fade: float, tts: float): Anim.t =>
  let w =
    (match fade > 0.0 with
     | true =>
       let c = Math.clamp01((tts - st.fadeStart) / fade) in
       c * c * (3.0 - 2.0 * c)
     | false => 1.0) in
  match w < 1.0 with
  | true =>
    Anim.blend([
      (Anim.clip(st.prevClip, tts - st.prevSince), 1.0 - w),
      (Anim.clip(st.current, tts - st.since), w),
    ])
  | false => Anim.clip(st.current, tts - st.since)
