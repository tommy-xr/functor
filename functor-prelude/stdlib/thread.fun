// Thread — screw threads that print: ISO metric coarse sizes and the 1/4-20
// camera thread, fitted with a `Fit` gap. Threads run along Z from z = 0.
//
//   let bolt = Thread.rod(Thread.m8, 30.0)
//   let nut =
//     Hardware.hex(13.0, 6.8)
//       |> Solid.subtract(Thread.tap(Thread.m8, Fit.clearance, 8.8) |> Solid.translate(Vec3.make(0.0, 0.0, -1.0)))
//
// A rod and a tap fit when they start a whole number of pitches apart.

type size = { name: string, diameter: float, pitch: float }

let m3: size = { name: "M3", diameter: 3.0, pitch: 0.5 }
let m4: size = { name: "M4", diameter: 4.0, pitch: 0.7 }
let m5: size = { name: "M5", diameter: 5.0, pitch: 0.8 }
let m6: size = { name: "M6", diameter: 6.0, pitch: 1.0 }
let m8: size = { name: "M8", diameter: 8.0, pitch: 1.25 }
let m10: size = { name: "M10", diameter: 10.0, pitch: 1.5 }
let m12: size = { name: "M12", diameter: 12.0, pitch: 1.75 }
// Camera and tripod mounts (UNC: the same 60° profile).
let unc1_4_20: size = { name: "1/4-20 UNC", diameter: 6.35, pitch: 1.27 }

let sizes = [m3, m4, m5, m6, m8, m10, m12, unc1_4_20]

// A bolt's thread, `length` long, at its nominal size (like a pin, a rod
// carries no fit: it mates with a metal nut too).
let rod = (s: size, length: float) => Solid.thread(s.diameter, s.pitch, length, 0.0)

// How far a rod's thread reaches into a tap's, radially, in mm — what holds
// the load. `engagement(s, Fit.gap(0.0))` is the full thread depth.
let engagement = (s: size, fit: Fit.t) =>
  let h = Math.sqrt(3.0) / 2.0 * s.pitch in
  let minor = s.diameter / 2.0 - 5.0 * h / 8.0 in
  // The tap's inner flat moves out by the gap, unless it closed into a point
  // (flanks are 30° off radial, so their meeting point moves 2·gap).
  s.diameter / 2.0 - Math.max(minor + fit.gap, minor - h / 4.0 + 2.0 * fit.gap)

// The thread to subtract for a nut or a tapped hole: grown by the fit's gap
// all over, so a `rod` (or a metal screw) clears it by `gap`. Make it longer
// than the hole so it cuts through both faces. Refuses a gap that leaves
// under a third of the thread's depth engaged (it would strip): small
// threads need `Fit.slip` or `Fit.press`.
let tap = (s: size, fit: Fit.t, length: float) =>
  let full = engagement(s, Fit.gap(0.0)) in
  let mm = (x: float) => Math.round(x * 100.0) / 100.0 in
  if engagement(s, fit) < full / 3.0 then
    Binary.invalid($"Thread.tap: a {fit.gap} mm gap leaves {s.name} only {mm(engagement(s, fit))} of {mm(full)} mm engaged; use a smaller gap or a bigger thread")
  else Solid.thread(s.diameter, s.pitch, length, fit.gap)
