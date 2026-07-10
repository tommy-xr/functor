// A sibling module: `open`ed by the entry, so `add`/`origin`/`V2` resolve
// bare there; `Vec.make` works with no open at all.

type V2 = { x: float, y: float }

let origin = { x: 0.0, y: 0.0 }

let make = (x: float, y: float): V2 => { x: x, y: y }

let add = (a: V2, b: V2): V2 => { x: a.x + b.x, y: a.y + b.y }
