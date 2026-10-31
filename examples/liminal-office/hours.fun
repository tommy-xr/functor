// The hour, and what it does to the light: interiors switch between the night
// and day bakes at a threshold; outdoors follows `daylight` continuously.
// Not hashed into the bakes' provenance (the bakes don't depend on it).

// 0 by night, 1 by day: dawn is 5–8 h, dusk 17–20 h.
let daylight = (hour: float): float =>
  Math.smoothstep(5.0, 8.0, hour) * (1.0 - Math.smoothstep(17.0, 20.0, hour))

// Interiors draw the day bake from here up; there is no blend.
let dayThreshold = 0.5
let isDay = (hour: float): bool => daylight(hour) >= dayThreshold

// Outdoor colours, 0..1 channels.
type Rgb = { r: float, g: float, b: float }
let mix = (a: Rgb, b: Rgb, t: float): Rgb =>
  { r: a.r |> Math.lerp(b.r, t), g: a.g |> Math.lerp(b.g, t), b: a.b |> Math.lerp(b.b, t) }
let color = (c: Rgb): Color.t => Color.rgb(c.r, c.g, c.b)

// The sky (fog and clear colour): night → dusk mauve → day haze.
let nightSky = { r: 22.0 / 255.0, g: 17.0 / 255.0, b: 34.0 / 255.0 }
let duskSky = { r: 118.0 / 255.0, g: 94.0 / 255.0, b: 126.0 / 255.0 }
let daySky = { r: 178.0 / 255.0, g: 192.0 / 255.0, b: 204.0 / 255.0 }
let skyAt = (d: float): Rgb =>
  if d < 0.5 then mix(nightSky, duskSky, 2.0 * d) else mix(duskSky, daySky, 2.0 * d - 1.0)

// The moon's direction; by day the light turns to `Building.sunDirection`,
// the day bake's sun.
let moonDirection = Vec3.make(0.3, -0.8, 0.5)

// The outdoors at `d` = daylight: the sky, the fog's near end, the frame's
// ambient and moon-to-sun light, and how strongly the neon and the sodium
// lamps glow (1 at night).
type Outdoor = { sky: Rgb, fogNear: float, ambient: Rgb, sun: Rgb, sunIntensity: float, sunDirection: Vec3.t, neon: float, lamps: float }
let outdoorAt = (d: float): Outdoor => {
  sky: skyAt(d),
  fogNear: 35.0 |> Math.lerp(70.0, d),
  ambient: mix({ r: 0.09, g: 0.08, b: 0.14 }, { r: 1.1, g: 1.12, b: 1.16 }, d),
  sun: mix({ r: 0.35, g: 0.38, b: 0.55 }, { r: 1.0, g: 0.93, b: 0.8 }, d),
  sunIntensity: 0.6 |> Math.lerp(2.6, d),
  sunDirection: moonDirection |> Vec3.lerp(Building.sunDirection, d),
  neon: 1.0 |> Math.lerp(0.7, d),
  lamps: 1.0 |> Math.lerp(0.0, d),
}

// The fog's far end at every hour: past it the interior isn't drawn.
let fogFar = 140.0
