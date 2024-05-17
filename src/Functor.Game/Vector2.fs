namespace Functor.Math

type Vector2 = { x: float; y: float }

module Vector2 =

    let zero = { x = 0.0; y = 0.0 }

    let xy x y = { x = x; y = y }