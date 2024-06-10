namespace Functor.Math

type Vector2 = { x: float32; y: float32 }

module Vector2 =

    let zero = { x = 0.0f; y = 0.0f }

    let xy x y = { x = x; y = y }

    let scale s (v: Vector2) = { x = s * v.x; y = s * v.y }