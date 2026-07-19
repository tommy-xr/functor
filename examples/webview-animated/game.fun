// webview-animated — CSS @keyframes + :hover transitions in the webview.
//
// Two halves of this HUD animate on DIFFERENT clocks:
//  * CSS-driven: the LIVE badge pulse and the energy-bar sweep are pure
//    `@keyframes` in the stylesheet — the model never changes for them.
//    The webview's animation clock is the GAME clock, so `--fixed-time T`
//    renders them deterministically at time T (natively they tick live).
//  * Model-driven: the boost counter re-renders through the Elm loop —
//    Attr.onClick(Boost) delivers a msg through `update`, coexisting with
//    the CSS animations.

type Model = { boosts: float }
type Msg = | Boost

let init = { boosts: 0.0 }

let update = (m: Model, msg: Msg) =>
  match msg with
  | Boost => { m with boosts: m.boosts + 1.0 }

// Nothing simulates per-frame: all motion is CSS- or tts-driven.
let tick = (m: Model, dt, tts) => m

// The 3D backdrop moves on draw's tts (NOT tick), so a --fixed-time sweep
// poses it too — same 2s period as the CSS loops, for a seamless GIF.
let draw = (m: Model, tts) =>
  Frame.createLit(
    Camera.lookAt(Vec3.make(0.0, 1.4, -4.5), Vec3.make(0.0, 0.6, 0.0)),
    Scene.group([
      Scene.sphere()
        |> Scene.lit(Color.rgb(0.9, 0.35, 0.55))
        |> Scene.translate(Vec3.make(0.6, 0.9 + 0.25 * Math.sin(tts * Math.pi), 0.0)),
      Scene.sphere()
        |> Scene.scale(0.35)
        |> Scene.emissive(Color.rgb(0.55, 0.9, 1.0))
        |> Scene.translate(Vec3.make(
             0.6 + 1.6 * Math.cos(tts * Math.pi),
             0.9,
             1.6 * Math.sin(tts * Math.pi),
           )),
      Scene.plane()
        |> Scene.lit(Color.rgb(0.16, 0.18, 0.28))
        |> Scene.scale(9.0),
    ]),
    [
      Light.ambient(Color.rgb(0.22, 0.22, 0.28)),
      Light.directional(Vec3.make(-0.5, -1.0, 0.4), Color.rgb(1.0, 1.0, 1.0), 0.9),
    ],
  )

// The stylesheet is plain CSS in a string. Both @keyframes share a 2s
// period; the button's :hover fade is a 0.3s transition.
let css = "
  .hud { display: flex; flex-direction: column; gap: 14px; width: 320px;
         margin: 24px; padding: 20px;
         background: rgba(16, 18, 34, 0.9);
         border: 2px solid #ff79c6; border-radius: 14px;
         font-family: sans-serif; color: #f8f8f2; }
  .titleRow { display: flex; align-items: center; justify-content: space-between; }
  .hud h1 { margin: 0; font-size: 20px; color: #ff79c6; }

  /* CSS-driven: pulses forever with zero model updates (2s period). */
  .live { padding: 4px 12px; border-radius: 999px;
          background: #ff5555; color: #ffffff;
          font-size: 12px; font-weight: bold; letter-spacing: 1px;
          animation: pulse 2s ease-in-out infinite; }
  @keyframes pulse { 0% { opacity: 1; } 50% { opacity: 0.25; } 100% { opacity: 1; } }

  /* CSS-driven: the fill sweeps 8% -> 92% -> 8% (2s period). */
  .bar { height: 12px; border-radius: 6px;
         background: rgba(255, 255, 255, 0.12); }
  .fill { height: 12px; border-radius: 6px;
          background: linear-gradient(90deg, #8be9fd, #50fa7b);
          animation: charge 2s ease-in-out infinite; }
  @keyframes charge { 0% { width: 8%; } 50% { width: 92%; } 100% { width: 8%; } }

  /* :hover transition — the box-shadow glow + background fade in over
     0.3s. This shows LIVE under a real cursor (run `run native` and hover
     the button); a headless capture can't hover, so the GIF shows the
     resting style. */
  button { padding: 10px 16px; font-size: 16px; font-weight: bold;
           border: none; border-radius: 8px;
           background: #44475a; color: #f8f8f2;
           transition: background 0.3s ease, box-shadow 0.3s ease; }
  button:hover { background: #bd93f9;
                 box-shadow: 0 0 18px rgba(189, 147, 249, 0.85); }
  .count { font-size: 14px; color: #8be9fd; }
"

let webview = (m: Model) =>
  Html.div([], [
    Html.style(css),
    Html.div([Attr.class("hud")], [
      Html.div([Attr.class("titleRow")], [
        Html.h1([], [Html.text("Reactor status")]),
        Html.span([Attr.class("live")], [Html.text("LIVE")]),   // CSS pulse
      ]),
      Html.div([Attr.class("bar")], [
        Html.div([Attr.class("fill")], []),                     // CSS sweep
      ]),
      // Model-driven: the click re-renders the count through `update`.
      Html.button([Attr.onClick(Boost)], [Html.text("Boost")]),
      Html.div([Attr.class("count")], [
        Html.text(Text.concat("boosts: ", Text.fixed(m.boosts, 0.0))),
      ]),
    ]),
  ])
