namespace Functor.Math

/// A linear RGB color, each channel nominally 0..1 (values may exceed 1 for
/// bright/HDR lights). Used by the lighting API.
type Color = { r: float32; g: float32; b: float32 }

module Color =

    let rgb r g b = { r = r; g = g; b = b }

    let black = { r = 0.0f; g = 0.0f; b = 0.0f }
    let white = { r = 1.0f; g = 1.0f; b = 1.0f }
