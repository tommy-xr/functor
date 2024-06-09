namespace Functor.Math

type Point2 = { x: float32; y: float32 }

module Point2 =

    let zero = { x = 0.0f; y = 0.0f }

    let xy x y = { x = x; y = y }

    let add (v: Vector2) p = { x = p.x + v.x; y = p.y + v.y }
