namespace Functor.Math

type Vector3 = { x: float32; y: float32; z: float32 }

module Vector3 =

    let zero = { x = 0.0f; y = 0.0f; z = 0.0f }

    let xyz x y z = { x = x; y = y; z = z }

    let add (a: Vector3) (b: Vector3) = { x = a.x + b.x; y = a.y + b.y; z = a.z + b.z }

    let scale s (v: Vector3) = { x = s * v.x; y = s * v.y; z = s * v.z }
