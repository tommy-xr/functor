// Fit — print tolerances for parts that mate. `gap` is the space left on each
// side of a joint, in mm, and it all goes into the hole: a hole for a d mm pin
// is d + 2·gap across, the pin stays d. Printed holes come out a little small,
// so these are starting points for a 0.4 mm nozzle: print
// `examples/hardware` and switch to `Fit.gap` if yours runs tight or loose.

type t = { gap: float }

// Pushed in with force, relying on the hole printing small (magnets, dowels).
let press: t = { gap: 0.05 }
// Slides in by hand with a little friction (alignment pins, keys).
let slip: t = { gap: 0.15 }
// Moves freely (a screw through a hole, printed threads).
let clearance: t = { gap: 0.3 }

// A custom gap per side, in mm.
let gap = (mm: float): t => { gap: mm }
