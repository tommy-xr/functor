namespace Functor.Math

type Point2 = { x: float; y: float }

module Point2 =

    let zero = { x = 0.0; y = 0.0 }

    let xy x y = { x = x; y = y }

    let add (v: Vector2) p = { x = p.x + v.x; y = p.y + v.y }
