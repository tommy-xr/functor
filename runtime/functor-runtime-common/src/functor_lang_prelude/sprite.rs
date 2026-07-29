//! The plain-data `Sprite` picture algebra and its `Camera2D` frame boundary.
//!
//! Kept in its own module so this sizeable, mostly cold registration/lowering
//! path does not perturb code layout for the existing per-frame 3D prelude.

use super::*;

use crate::{scene3d::BuiltinTexture, sprite_font};

/// A center-origin, Y-up [`Camera2D`] used by sprite frame passes.
struct FunctorLangCamera2D(Camera2D);

/// A Sprite value is deliberately NOT host data. The wrapper exists only for
/// typed registry argument conversion; its inner [`Value`] is a plain
/// variant/list tree, so it compares, inspects, serializes, and survives hot
/// reload like ordinary game data.
struct FunctorLangSprite(Value);

/// A plain-data, top-left-origin source rectangle in texture pixels.
struct FunctorLangSpriteRegion([f32; 4]);

/// A 2D point in the sprite's own coordinate space — the `Input.point2` record
/// (`{ x, y }`) reused deliberately. Declaring a second `{ x, y }` record type
/// would make every bare `{ x: …, y: … }` literal in every game an AMBIGUOUS
/// record literal (a check error), since literals resolve nominally by field
/// set. One shared point type is the only workable choice.
struct FunctorLangPoint2([f32; 2]);

/// A sampled mouse position plus the logical surface extent sharing its
/// top-left-origin coordinate space.
struct FunctorLangMouse {
    x: f32,
    y: f32,
    surface_width: f32,
    surface_height: f32,
}

fn finite_record_number(
    fields: &[(String, Value)],
    field: &str,
    record_name: &str,
    path: &str,
    span: Span,
) -> Result<f32, RunError> {
    match fields.iter().find(|(name, _)| name == field) {
        Some((_, Value::Number(n))) if (*n as f32).is_finite() => Ok(*n as f32),
        Some((_, Value::Number(n))) => Err(RunError {
            message: format!("{path}: {record_name} `{field}` must be finite, got {n}"),
            span,
        }),
        Some((_, other)) => Err(RunError {
            message: format!(
                "{path}: {record_name} `{field}` must be a number, got {}",
                other.kind_name()
            ),
            span,
        }),
        None => Err(RunError {
            message: format!("{path}: expected a {record_name} record, missing `{field}`"),
            span,
        }),
    }
}

impl crate::host_registry::FromArg for FunctorLangPoint2 {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let Value::Record(fields) = value else {
            return Err(RunError {
                message: format!(
                    "{path}: expected a point record {{ x, y }}, got {}",
                    value.kind_name()
                ),
                span,
            });
        };
        Ok(FunctorLangPoint2([
            finite_record_number(fields, "x", "point", path, span)?,
            finite_record_number(fields, "y", "point", path, span)?,
        ]))
    }
}

impl crate::host_registry::FromArg for FunctorLangMouse {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let Value::Record(fields) = value else {
            return Err(RunError {
                message: format!(
                    "{path}: expected an Input.mouse record, got {}",
                    value.kind_name()
                ),
                span,
            });
        };
        Ok(FunctorLangMouse {
            x: finite_record_number(fields, "x", "mouse", path, span)?,
            y: finite_record_number(fields, "y", "mouse", path, span)?,
            surface_width: finite_record_number(fields, "surfaceWidth", "mouse", path, span)?,
            surface_height: finite_record_number(fields, "surfaceHeight", "mouse", path, span)?,
        })
    }
}

impl HostData for FunctorLangCamera2D {
    fn type_name(&self) -> &'static str {
        "Camera2D"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

crate::host_returnable!(FunctorLangCamera2D);

impl crate::host_registry::FromArg for FunctorLangCamera2D {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        if let Value::HostData(data) = value {
            if let Some(camera) = data.as_any().downcast_ref::<FunctorLangCamera2D>() {
                return Ok(FunctorLangCamera2D(camera.0.clone()));
            }
        }
        Err(RunError {
            message: format!("{path}: expected a Camera2D, got {}", value.kind_name()),
            span,
        })
    }
}

impl crate::host_registry::FromArg for FunctorLangSprite {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        if matches!(
            value,
            Value::Variant { ctor, .. } if ctor.starts_with("Sprite.")
        ) {
            Ok(FunctorLangSprite(value.clone()))
        } else {
            Err(RunError {
                message: format!("{path}: expected a Sprite, got {}", value.kind_name()),
                span,
            })
        }
    }
}

impl crate::host_registry::FromArg for FunctorLangSpriteRegion {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let Value::Variant { ctor, args } = value else {
            return Err(RunError {
                message: format!(
                    "{path}: expected a Sprite.region, got {}",
                    value.kind_name()
                ),
                span,
            });
        };
        let ("SpriteRegion.Region", [x, y, width, height]) = (ctor.as_ref(), args.as_slice())
        else {
            return Err(RunError {
                message: format!(
                    "{path}: expected a Sprite.region, got {}",
                    value.kind_name()
                ),
                span,
            });
        };
        let number = |value: &Value| match value {
            Value::Number(n) if (*n as f32).is_finite() => Ok(*n as f32),
            _ => Err(RunError {
                message: format!("{path}: malformed Sprite.region data"),
                span,
            }),
        };
        Ok(FunctorLangSpriteRegion([
            number(x)?,
            number(y)?,
            number(width)?,
            number(height)?,
        ]))
    }
}

/// Validate a positive world-space dimension — a text size, a circle radius, a
/// line thickness. Rejects zero and negatives, and NaN via the same test.
///
/// The check is applied to the NARROWED `f32` the layout actually uses, not just
/// the incoming `f64`. The registry's numeric conversion already rejects a value
/// that overflows `f32`, but a tiny positive `f64` (say 1e-60) narrows to
/// exactly `0.0` and passes it — which would let `Sprite.measure` report a
/// positive box while `Sprite.text` laid out zero-sized quads, or make a circle
/// silently invisible. Every dimension goes through here, so they cannot
/// disagree about which values are legal.
fn positive_dimension(value: f64, path: &str, noun: &str) -> Result<f64, String> {
    let narrowed = value as f32;
    if !narrowed.is_finite() || narrowed <= 0.0 {
        return Err(format!(
            "{path} {noun} must be a positive number, got {value}"
        ));
    }
    Ok(value)
}

/// Segments in a `Sprite.circle`. Fixed rather than derived from the radius,
/// because lowering cannot know the camera zoom the circle will be seen at. At 32
/// the worst-case radial error is `1 - cos(pi/32)` ~= 0.5% of the radius, which is
/// under a pixel for any circle small enough to read as a circle.
const CIRCLE_SEGMENTS: usize = 32;

/// The unit-radius ring every `Sprite.circle` lowers to, sized by a scale
/// transform. Because the points are identical for every circle, the renderer's
/// per-point-count mesh cache uploads them once for the whole process.
fn unit_circle_points() -> Vec<[f32; 2]> {
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let angle = i as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
            [angle.cos(), angle.sin()]
        })
        .collect()
}

/// Validate an author-supplied outline for the triangle-fan fill.
///
/// A fan from the first vertex only fills a CONVEX polygon; on a concave outline
/// it silently paints outside the shape. Rather than document that as undefined,
/// this rejects it with a teaching error at the construction site. Pure and
/// renderer-free, so it is unit-testable.
///
/// EITHER winding is accepted (clockwise or counter-clockwise): a game computing
/// points from angles can legitimately produce either, nothing culls back faces,
/// and demanding one would be an invisible trap. Collinear vertices that continue
/// STRAIGHT are fine — they are convex — but an outline with no area at all is
/// rejected, since it cannot be filled and would give the mesh a degenerate
/// bounding box.
fn convex_outline(points: Vec<FunctorLangPoint2>, path: &str) -> Result<Vec<[f32; 2]>, String> {
    let points: Vec<[f32; 2]> = points.into_iter().map(|point| point.0).collect();
    if points.len() < 3 {
        return Err(format!(
            "{path} needs at least 3 points to fill, got {}",
            points.len()
        ));
    }
    let count = points.len();
    // Sign-consistency alone does NOT imply convexity: a STAR (a pentagram) turns
    // the same way at every vertex yet self-intersects, so it would pass and
    // fan-fill as overlapping triangles. Catching that needs the total turning,
    // which must be exactly one revolution for a simple outline.
    //
    // It is only needed above 4 points, though. Every turn is strictly under half a
    // revolution (a 180-degree reversal is rejected just below), so with all turns
    // sharing a sign the total is under `count * pi`; it is also a whole number of
    // revolutions, hence `revolutions < count / 2`. A star needs 2+ revolutions and
    // therefore at least 5 points — a triangle or quadrilateral that turns
    // consistently is always simple. So triangles and quads, the overwhelming
    // majority of game polygons, skip the transcendentals entirely.
    let needs_turning_check = count > 4;
    let mut sign = 0.0f32;
    let mut turning = 0.0f64;
    let mut reversed = false;
    for index in 0..count {
        let a = points[index];
        let b = points[(index + 1) % count];
        let c = points[(index + 2) % count];
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        let dot = (b[0] - a[0]) as f64 * (c[0] - b[0]) as f64
            + (b[1] - a[1]) as f64 * (c[1] - b[1]) as f64;
        if cross == 0.0 {
            // Collinear. Continuing straight (or a repeated point) turns by zero and
            // is convex, but DOUBLING BACK turns by exactly pi — a spike. Those must
            // be rejected, and not only because a spike is degenerate: a reversal is
            // the one turn that is not under half a revolution, so allowing it breaks
            // the bound above and lets a self-intersecting outline total one
            // revolution and pass. `[(0,0), (0,1), (2,1), (1,1), (3,2)]` is such an
            // outline — same-signed turns, total -2pi, and yet edge (0,1)->(2,1)
            // crosses the closing edge.
            // Recorded rather than reported immediately: an outline that is ENTIRELY
            // collinear also doubles back at its closing vertex, and "all on one
            // line" is the clearer diagnosis for that, so the no-area check below
            // gets first refusal.
            if dot < 0.0 {
                reversed = true;
            }
            continue;
        }
        if needs_turning_check {
            turning += (cross as f64).atan2(dot);
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return Err(format!(
                "{path} points must form a CONVEX outline — this one turns both left and \
                 right, and the fill is a triangle fan, so it would paint outside the \
                 shape. Split it into convex pieces and draw them as a group."
            ));
        }
    }
    if sign == 0.0 {
        return Err(format!(
            "{path} points must enclose an area, but these are all on one line"
        ));
    }
    if reversed {
        return Err(format!(
            "{path} points must form a CONVEX outline — this one doubles back on itself              at a point, which is a spike rather than a corner. Remove the reversing              point, or split the shape into convex pieces."
        ));
    }
    if needs_turning_check && (turning.abs() - std::f64::consts::TAU).abs() > 1e-3 {
        return Err(format!(
            "{path} points must form a SIMPLE outline that winds around once, but these \
             wind around {:.1} times — a self-intersecting star fills as overlapping fan \
             triangles, not as the star. Draw it as a group of convex pieces.",
            turning.abs() / std::f64::consts::TAU
        ));
    }
    Ok(points)
}

fn point_list_value(points: &[[f32; 2]]) -> Value {
    Value::List(Rc::new(
        points
            .iter()
            .map(|point| {
                Value::Record(Rc::new(vec![
                    ("x".to_string(), Value::Number(point[0] as f64)),
                    ("y".to_string(), Value::Number(point[1] as f64)),
                ]))
            })
            .collect(),
    ))
}

fn sprite_node(name: &str, args: Vec<Value>) -> Value {
    Value::Variant {
        ctor: Rc::from(format!("Sprite.{name}")),
        args: Rc::new(args),
    }
}

fn sprite_region_node(x: f64, y: f64, width: f64, height: f64) -> Value {
    Value::Variant {
        ctor: Rc::from("SpriteRegion.Region"),
        args: Rc::new(vec![
            Value::Number(x),
            Value::Number(y),
            Value::Number(width),
            Value::Number(height),
        ]),
    }
}

fn sprite_children(items: Vec<FunctorLangSprite>) -> Value {
    Value::List(Rc::new(items.into_iter().map(|item| item.0).collect()))
}

fn sprite_texture_parts(texture: FunctorLangTexture) -> Result<(String, Vec<String>), String> {
    match texture.0 {
        TextureDescription::File(path) | TextureDescription::FileClamped(path) => {
            Ok((path, vec![]))
        }
        TextureDescription::FileWhilePending {
            file,
            while_pending,
        }
        | TextureDescription::FileClampedWhilePending {
            file,
            while_pending,
        } => Ok((file, while_pending)),
        TextureDescription::RenderTarget(_) => {
            Err("Sprite images expect an image asset, not a render target".to_string())
        }
        // Unreachable from game code: `Asset.texture` only ever produces file
        // locators, and the built-in atlas is reachable only through
        // `Sprite.text`. Rejected rather than ignored so a future builtin
        // exposed as an asset cannot silently lose its region.
        TextureDescription::Builtin(_) => {
            Err("Sprite images expect an image asset, not a built-in texture".to_string())
        }
    }
}

fn sprite_image_node(
    width: f64,
    height: f64,
    source_pixels: Option<[f32; 4]>,
    texture: FunctorLangTexture,
) -> Result<Value, String> {
    if width <= 0.0 || height <= 0.0 {
        let constructor = if source_pixels.is_some() {
            "Sprite.imageRegion"
        } else {
            "Sprite.image"
        };
        return Err(format!(
            "{constructor} width and height must be positive, got {width} × {height}"
        ));
    }
    let (path, pending) = sprite_texture_parts(texture)?;
    let mut args = vec![Value::Number(width), Value::Number(height)];
    if let Some([x, y, source_width, source_height]) = source_pixels {
        args.extend([
            Value::Number(x as f64),
            Value::Number(y as f64),
            Value::Number(source_width as f64),
            Value::Number(source_height as f64),
        ]);
    }
    args.push(Value::String(path.into()));
    args.push(Value::List(Rc::new(
        pending
            .into_iter()
            .map(|path| Value::String(path.into()))
            .collect(),
    )));
    Ok(sprite_node(
        if source_pixels.is_some() {
            "ImageRegion"
        } else {
            "Image"
        },
        args,
    ))
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn0("Sprite.blank", "Sprite.blank()", || {
        sprite_node("Blank", vec![])
    });
    reg.fn3(
        "Sprite.rectangle",
        "Sprite.rectangle(color, width, height)",
        |color: FunctorLangColor, width: f64, height: f64| {
            if width <= 0.0 || height <= 0.0 {
                return Err(format!(
                    "Sprite.rectangle width and height must be positive, got {width} × {height}"
                ));
            }
            let (r, g, b) = color.0;
            Ok(sprite_node(
                "Rectangle",
                vec![
                    Value::Number(width),
                    Value::Number(height),
                    Value::Number(r as f64),
                    Value::Number(g as f64),
                    Value::Number(b as f64),
                ],
            ))
        },
    );
    reg.fn2(
        "Sprite.square",
        "Sprite.square(color, size)",
        |color: FunctorLangColor, size: f64| {
            if size <= 0.0 {
                return Err(format!("Sprite.square size must be positive, got {size}"));
            }
            let (r, g, b) = color.0;
            Ok(sprite_node(
                "Rectangle",
                vec![
                    Value::Number(size),
                    Value::Number(size),
                    Value::Number(r as f64),
                    Value::Number(g as f64),
                    Value::Number(b as f64),
                ],
            ))
        },
    );
    reg.fn2(
        "Sprite.circle",
        "Sprite.circle(color, radius)",
        |color: FunctorLangColor, radius: f64| {
            let radius = positive_dimension(radius, "Sprite.circle", "radius")?;
            let (r, g, b) = color.0;
            Ok(sprite_node(
                "Circle",
                vec![
                    Value::Number(radius),
                    Value::Number(r as f64),
                    Value::Number(g as f64),
                    Value::Number(b as f64),
                ],
            ))
        },
    );
    reg.fn2(
        "Sprite.polygon",
        "Sprite.polygon(color, points)",
        |color: FunctorLangColor, points: Vec<FunctorLangPoint2>| {
            let points = convex_outline(points, "Sprite.polygon")?;
            let (r, g, b) = color.0;
            Ok(sprite_node(
                "Polygon",
                vec![
                    point_list_value(&points),
                    Value::Number(r as f64),
                    Value::Number(g as f64),
                    Value::Number(b as f64),
                ],
            ))
        },
    );
    reg.fn4(
        "Sprite.line",
        "Sprite.line(color, thickness, from, to)",
        |color: FunctorLangColor,
         thickness: f64,
         from: FunctorLangPoint2,
         to: FunctorLangPoint2| {
            let thickness = positive_dimension(thickness, "Sprite.line", "thickness")?;
            // Endpoints are individually representable but their SPAN may not be:
            // `(0,0)` to `(2e20,0)` overflows f32 when squared, which would hand
            // lowering an infinite length and midpoint. Check the derived values
            // here, at the call site, rather than emitting a broken transform.
            let span_x = to.0[0] as f64 - from.0[0] as f64;
            let span_y = to.0[1] as f64 - from.0[1] as f64;
            let derived = [
                span_x.hypot(span_y),
                (from.0[0] as f64 + to.0[0] as f64) * 0.5,
                (from.0[1] as f64 + to.0[1] as f64) * 0.5,
            ];
            if derived.iter().any(|value| !(*value as f32).is_finite()) {
                return Err(
                    "Sprite.line endpoints are too far apart to place — their length or \
                     midpoint overflows"
                        .to_string(),
                );
            }
            let (r, g, b) = color.0;
            Ok(sprite_node(
                "Line",
                vec![
                    Value::Number(thickness),
                    Value::Number(from.0[0] as f64),
                    Value::Number(from.0[1] as f64),
                    Value::Number(to.0[0] as f64),
                    Value::Number(to.0[1] as f64),
                    Value::Number(r as f64),
                    Value::Number(g as f64),
                    Value::Number(b as f64),
                ],
            ))
        },
    );
    reg.fn3(
        "Sprite.text",
        "Sprite.text(color, size, text)",
        |color: FunctorLangColor, size: f64, text: String| {
            let size = positive_dimension(size, "Sprite.text", "size")?;
            let (r, g, b) = color.0;
            Ok(sprite_node(
                "Text",
                vec![
                    Value::Number(size),
                    Value::Number(r as f64),
                    Value::Number(g as f64),
                    Value::Number(b as f64),
                    Value::String(Rc::from(text.as_str())),
                ],
            ))
        },
    );
    reg.fn2(
        "Sprite.measure",
        "Sprite.measure(size, text)",
        |size: f64, text: String| {
            let size = positive_dimension(size, "Sprite.measure", "size")?;
            let (columns, rows) = sprite_font::measure_cells(&text);
            Ok(Value::Record(Rc::new(vec![
                ("width".to_string(), Value::Number(size * columns)),
                ("height".to_string(), Value::Number(size * rows)),
            ])))
        },
    );
    reg.fn3(
        "Sprite.image",
        "Sprite.image(width, height, texture)",
        |width: f64, height: f64, texture: FunctorLangTexture| {
            sprite_image_node(width, height, None, texture)
        },
    );
    reg.fn4(
        "Sprite.region",
        "Sprite.region(x, y, width, height)",
        |x: f64, y: f64, width: f64, height: f64| {
            if x < 0.0 || y < 0.0 {
                return Err(format!(
                    "Sprite.region x and y must be non-negative, got {x}, {y}"
                ));
            }
            if width <= 0.0 || height <= 0.0 {
                return Err(format!(
                    "Sprite.region width and height must be positive, got {width} × {height}"
                ));
            }
            if [x, y, width, height]
                .into_iter()
                .any(|value| value.fract() != 0.0)
            {
                return Err(
                    "Sprite.region coordinates and size must be whole source pixels".to_string(),
                );
            }
            Ok(sprite_region_node(x, y, width, height))
        },
    );
    reg.fn4(
        "Sprite.imageRegion",
        "Sprite.imageRegion(width, height, region, texture)",
        |width: f64, height: f64, region: FunctorLangSpriteRegion, texture: FunctorLangTexture| {
            sprite_image_node(width, height, Some(region.0), texture)
        },
    );
    reg.fn1(
        "Sprite.group",
        "Sprite.group([sprite, …])",
        |items: Vec<FunctorLangSprite>| sprite_node("Group", vec![sprite_children(items)]),
    );
    reg.fn3(
        "Sprite.move",
        "Sprite.move(x, y, sprite)",
        |x: f64, y: f64, sprite: FunctorLangSprite| {
            sprite_node("Move", vec![Value::Number(x), Value::Number(y), sprite.0])
        },
    );
    reg.fn2(
        "Sprite.moveX",
        "Sprite.moveX(x, sprite)",
        |x: f64, sprite: FunctorLangSprite| {
            sprite_node("Move", vec![Value::Number(x), Value::Number(0.0), sprite.0])
        },
    );
    reg.fn2(
        "Sprite.moveY",
        "Sprite.moveY(y, sprite)",
        |y: f64, sprite: FunctorLangSprite| {
            sprite_node("Move", vec![Value::Number(0.0), Value::Number(y), sprite.0])
        },
    );
    reg.fn2(
        "Sprite.rotate",
        "Sprite.rotate(angle, sprite)",
        |angle: FunctorLangAngle, sprite: FunctorLangSprite| {
            let radians: cgmath::Rad<f32> = angle.0.into();
            sprite_node("Rotate", vec![Value::Number(radians.0 as f64), sprite.0])
        },
    );
    reg.fn2(
        "Sprite.scale",
        "Sprite.scale(scale, sprite)",
        |scale: f64, sprite: FunctorLangSprite| {
            sprite_node(
                "Scale",
                vec![Value::Number(scale), Value::Number(scale), sprite.0],
            )
        },
    );
    reg.fn3(
        "Sprite.scaleXY",
        "Sprite.scaleXY(x, y, sprite)",
        |x: f64, y: f64, sprite: FunctorLangSprite| {
            sprite_node("Scale", vec![Value::Number(x), Value::Number(y), sprite.0])
        },
    );
    reg.fn2(
        "Sprite.fade",
        "Sprite.fade(alpha, sprite)",
        |alpha: f64, sprite: FunctorLangSprite| {
            if !(0.0..=1.0).contains(&alpha) {
                return Err(format!(
                    "Sprite.fade alpha must be between 0 and 1, got {alpha}"
                ));
            }
            Ok(sprite_node("Fade", vec![Value::Number(alpha), sprite.0]))
        },
    );
    reg.fn2(
        "Sprite.tint",
        "Sprite.tint(color, sprite)",
        |color: FunctorLangColor, sprite: FunctorLangSprite| {
            let (r, g, b) = color.0;
            sprite_node(
                "Tint",
                vec![
                    Value::Number(r as f64),
                    Value::Number(g as f64),
                    Value::Number(b as f64),
                    sprite.0,
                ],
            )
        },
    );
    reg.fn1(
        "Sprite.nearest",
        "Sprite.nearest(sprite)",
        |sprite: FunctorLangSprite| sprite_node("Nearest", vec![sprite.0]),
    );
    reg.fn1(
        "Sprite.linear",
        "Sprite.linear(sprite)",
        |sprite: FunctorLangSprite| sprite_node("Linear", vec![sprite.0]),
    );

    reg.fn2(
        "Camera2D.create",
        "Camera2D.create(width, height)",
        |width: f64, height: f64| {
            if width <= 0.0 || height <= 0.0 {
                return Err(format!(
                    "Camera2D.create width and height must be positive, got {width} × {height}"
                ));
            }
            Ok(FunctorLangCamera2D(Camera2D::new(
                width as f32,
                height as f32,
            )))
        },
    );
    reg.fn3(
        "Camera2D.at",
        "Camera2D.at(x, y, camera)",
        |x: f64, y: f64, camera: FunctorLangCamera2D| {
            FunctorLangCamera2D(camera.0.with_center(x as f32, y as f32))
        },
    );
    reg.fn2(
        "Camera2D.zoom",
        "Camera2D.zoom(scale, camera)",
        |zoom: f64, camera: FunctorLangCamera2D| {
            if zoom <= 0.0 {
                return Err(format!("Camera2D.zoom scale must be positive, got {zoom}"));
            }
            Ok(FunctorLangCamera2D(camera.0.with_zoom(zoom as f32)))
        },
    );
    reg.fn2(
        "Camera2D.toWorld",
        "Camera2D.toWorld(mouse, camera)",
        |mouse: FunctorLangMouse, camera: FunctorLangCamera2D| {
            crate::input::option_value(
                camera
                    .0
                    .to_world(mouse.x, mouse.y, mouse.surface_width, mouse.surface_height)
                    .map(|[x, y]| {
                        crate::input::record([
                            ("x", Value::Number(x as f64)),
                            ("y", Value::Number(y as f64)),
                        ])
                    }),
            )
        },
    );

    reg.fn2(
        "Frame.create2D",
        "Frame.create2D(camera, sprite)",
        |camera: FunctorLangCamera2D, sprite: FunctorLangSprite| {
            let layer = SpriteLayer {
                camera: camera.0,
                scene: lower_sprite(&sprite.0, [1.0, 1.0, 1.0, 1.0], SpriteSampling::Linear)?,
            };
            Ok(FunctorLangFrame(Frame::with_2d(
                Frame::new(Camera::default(), group(vec![], Matrix4::from_scale(1.0))),
                layer,
            )))
        },
    );
    reg.fn3(
        "Frame.with2D",
        "Frame.with2D(camera, sprite, frame)",
        |camera: FunctorLangCamera2D, sprite: FunctorLangSprite, frame: FunctorLangFrame| {
            let layer = SpriteLayer {
                camera: camera.0,
                scene: lower_sprite(&sprite.0, [1.0, 1.0, 1.0, 1.0], SpriteSampling::Linear)?,
            };
            Ok(FunctorLangFrame(Frame::with_2d(frame.0, layer)))
        },
    );
}

fn sprite_number(value: &Value, node: &str) -> Result<f32, String> {
    match value {
        Value::Number(n) if (*n as f32).is_finite() => Ok(*n as f32),
        _ => Err(format!(
            "invalid {node} sprite data: expected a finite number"
        )),
    }
}

/// Fold the inherited tint into a node's own color channels.
fn tinted(
    tint: [f32; 4],
    r: &Value,
    g: &Value,
    b: &Value,
    node: &str,
) -> Result<[f32; 4], String> {
    Ok([
        sprite_number(r, node)? * tint[0],
        sprite_number(g, node)? * tint[1],
        sprite_number(b, node)? * tint[2],
        tint[3],
    ])
}

/// A filled convex polygon leaf in the sprite's own coordinate space.
fn convex_polygon_scene(points: Vec<[f32; 2]>) -> Scene3D {
    Scene3D {
        obj: SceneObject::Geometry(Shape::ConvexPolygon { points }),
        xform: Matrix4::from_scale(1.0),
    }
}

fn lower_sprite(
    value: &Value,
    tint: [f32; 4],
    sampling: SpriteSampling,
) -> Result<Scene3D, String> {
    let Value::Variant { ctor, args } = value else {
        return Err(format!(
            "invalid Sprite data: expected a sprite node, got {}",
            value.kind_name()
        ));
    };
    match (ctor.as_ref(), args.as_slice()) {
        ("Sprite.Blank", []) => Ok(group(vec![], Matrix4::from_scale(1.0))),
        ("Sprite.Rectangle", [width, height, r, g, b]) => {
            let (width, height) = (
                sprite_number(width, "Rectangle")?,
                sprite_number(height, "Rectangle")?,
            );
            let color = [
                sprite_number(r, "Rectangle")? * tint[0],
                sprite_number(g, "Rectangle")? * tint[1],
                sprite_number(b, "Rectangle")? * tint[2],
                tint[3],
            ];
            let leaf = material_scene(
                MaterialDescription::emissive(color[0], color[1], color[2], color[3]),
                FunctorLangScene(Scene3D::quad()),
            );
            Ok(transformed(leaf, Matrix4::from_nonuniform_scale(width, height, 1.0)).0)
        }
        ("Sprite.Circle", [radius, r, g, b]) => {
            let radius = sprite_number(radius, "Circle")?;
            let color = tinted(tint, r, g, b, "Circle")?;
            // Every circle is the SAME unit ring plus a scale, so the renderer's
            // mesh cache holds one polygon mesh for all of them and re-uploads
            // nothing between circles or between frames.
            let leaf = material_scene(
                MaterialDescription::emissive(color[0], color[1], color[2], color[3]),
                FunctorLangScene(convex_polygon_scene(unit_circle_points())),
            );
            Ok(transformed(leaf, Matrix4::from_nonuniform_scale(radius, radius, 1.0)).0)
        }
        ("Sprite.Polygon", [Value::List(points), r, g, b]) => {
            let color = tinted(tint, r, g, b, "Polygon")?;
            let mut outline = Vec::with_capacity(points.len());
            for point in points.iter() {
                let Value::Record(fields) = point else {
                    return Err("invalid Polygon sprite data: expected point records".to_string());
                };
                let coordinate = |name: &str| match fields
                    .iter()
                    .find(|(field, _)| field == name)
                {
                    Some((_, value)) => sprite_number(value, "Polygon"),
                    None => Err(format!(
                        "invalid Polygon sprite data: point is missing `{name}`"
                    )),
                };
                outline.push([coordinate("x")?, coordinate("y")?]);
            }
            if outline.len() < 3 {
                return Err("invalid Polygon sprite data: needs at least 3 points".to_string());
            }
            // The points are the geometry, in the sprite's own space — no
            // centering, so game-computed outlines land where they were computed.
            Ok(material_scene(
                MaterialDescription::emissive(color[0], color[1], color[2], color[3]),
                FunctorLangScene(convex_polygon_scene(outline)),
            )
            .0)
        }
        ("Sprite.Line", [thickness, x1, y1, x2, y2, r, g, b]) => {
            let thickness = sprite_number(thickness, "Line")?;
            let (x1, y1) = (sprite_number(x1, "Line")?, sprite_number(y1, "Line")?);
            let (x2, y2) = (sprite_number(x2, "Line")?, sprite_number(y2, "Line")?);
            let color = tinted(tint, r, g, b, "Line")?;
            let (dx, dy) = (x2 - x1, y2 - y1);
            // `hypot`, not `sqrt(dx*dx + dy*dy)`: the squares can overflow f32 for
            // far-apart endpoints even when the length itself is representable.
            let length = dx.hypot(dy);
            // A zero-length line has no direction to orient, so it draws nothing
            // at all rather than issuing a degenerate draw call.
            if length == 0.0 {
                return Ok(group(vec![], Matrix4::from_scale(1.0)));
            }
            let leaf = material_scene(
                MaterialDescription::emissive(color[0], color[1], color[2], color[3]),
                FunctorLangScene(Scene3D::quad()),
            );
            // Thickness is applied in the SEGMENT's own frame (scale, then
            // rotate), so it is exact at every angle — the artifact a game gets
            // if it rotates an assembled group instead.
            Ok(transformed(
                leaf,
                Matrix4::from_translation(cgmath::vec3(
                    (x1 + x2) * 0.5,
                    (y1 + y2) * 0.5,
                    0.0,
                )) * Matrix4::from_angle_z(cgmath::Rad(dy.atan2(dx)))
                    * Matrix4::from_nonuniform_scale(length, thickness, 1.0),
            )
            .0)
        }
        ("Sprite.Text", [size, r, g, b, Value::String(text)]) => {
            let size = sprite_number(size, "Text")?;
            let color = [
                sprite_number(r, "Text")? * tint[0],
                sprite_number(g, "Text")? * tint[1],
                sprite_number(b, "Text")? * tint[2],
                tint[3],
            ];
            Ok(lower_sprite_text(size, color, text, sampling))
        }
        ("Sprite.Image", [width, height, path, pending]) => {
            lower_sprite_image(width, height, None, path, pending, tint, sampling)
        }
        (
            "Sprite.ImageRegion",
            [width, height, x, y, source_width, source_height, path, pending],
        ) => lower_sprite_image(
            width,
            height,
            Some([
                sprite_number(x, "ImageRegion")?,
                sprite_number(y, "ImageRegion")?,
                sprite_number(source_width, "ImageRegion")?,
                sprite_number(source_height, "ImageRegion")?,
            ]),
            path,
            pending,
            tint,
            sampling,
        ),
        ("Sprite.Group", [Value::List(items)]) => {
            let scenes = items
                .iter()
                .map(|item| lower_sprite(item, tint, sampling))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(group(scenes, Matrix4::from_scale(1.0)))
        }
        ("Sprite.Move", [x, y, child]) => Ok(transformed(
            FunctorLangScene(lower_sprite(child, tint, sampling)?),
            Matrix4::from_translation(cgmath::vec3(
                sprite_number(x, "Move")?,
                sprite_number(y, "Move")?,
                0.0,
            )),
        )
        .0),
        ("Sprite.Rotate", [angle, child]) => Ok(transformed(
            FunctorLangScene(lower_sprite(child, tint, sampling)?),
            Matrix4::from_angle_z(cgmath::Rad(sprite_number(angle, "Rotate")?)),
        )
        .0),
        ("Sprite.Scale", [x, y, child]) => Ok(transformed(
            FunctorLangScene(lower_sprite(child, tint, sampling)?),
            Matrix4::from_nonuniform_scale(
                sprite_number(x, "Scale")?,
                sprite_number(y, "Scale")?,
                1.0,
            ),
        )
        .0),
        ("Sprite.Fade", [alpha, child]) => {
            let mut next = tint;
            next[3] *= sprite_number(alpha, "Fade")?;
            lower_sprite(child, next, sampling)
        }
        ("Sprite.Tint", [r, g, b, child]) => {
            let mut next = tint;
            next[0] *= sprite_number(r, "Tint")?;
            next[1] *= sprite_number(g, "Tint")?;
            next[2] *= sprite_number(b, "Tint")?;
            lower_sprite(child, next, sampling)
        }
        ("Sprite.Nearest", [child]) => lower_sprite(child, tint, SpriteSampling::Nearest),
        ("Sprite.Linear", [child]) => lower_sprite(child, tint, SpriteSampling::Linear),
        _ => Err(format!("invalid Sprite data: malformed {ctor} node")),
    }
}

/// Expand a text node into one textured quad per VISIBLE glyph, every quad
/// sampling its own cell of the compiled-in font atlas.
///
/// Expansion happens here, at lowering, and never in the `Sprite.t` value: the
/// sprite tree keeps the string itself, so a picture containing text still
/// compares, inspects, serializes, and survives time travel as plain data. A
/// blank character (a space, or anything outside printable ASCII) emits no quad
/// at all but still advances the pen, so unsupported text reads as gaps rather
/// than sliding the rest of the line.
///
/// The run is centered on its own box, like every other sprite primitive, so
/// `Sprite.move` places text the same way it places a rectangle. `\n` starts a
/// new line, stacked at exactly one `size` of line height and centered within
/// the run's box (left-aligned blocks are the follow-up `textBlock`'s job).
fn lower_sprite_text(
    size: f32,
    color: [f32; 4],
    text: &str,
    sampling: SpriteSampling,
) -> Scene3D {
    let (_, rows) = sprite_font::measure_cells(text);
    let rows = rows as f32;
    let mut glyphs = Vec::new();
    for (row, line) in sprite_font::lines(text).enumerate() {
        let count = sprite_font::advance_count(line) as f32;
        // Lines run top to bottom in +Y-up space, so row 0 sits highest. The
        // stride is exactly `size`, which is also `measure(...).height / rows`
        // -- the invariant that lets callers stack blocks without overlap.
        let y = size * (rows * 0.5 - row as f32 - 0.5);
        let mut pen = size * (0.5 - count * 0.5);
        for character in line.chars() {
            if let Some((cell_x, cell_y, cell_width, cell_height)) =
                sprite_font::glyph_cell(character)
            {
                let leaf = material_scene(
                    MaterialDescription::sprite_texture_tinted(
                        TextureDescription::Builtin(BuiltinTexture::FontAtlas),
                        Some([cell_x, cell_y, cell_width, cell_height]),
                        sampling,
                        color[0],
                        color[1],
                        color[2],
                        color[3],
                    ),
                    FunctorLangScene(Scene3D::quad()),
                );
                // Negative Y for the same reason `Sprite.image` flips: the atlas
                // is uploaded top-row-first while GL's v = 0 is the bottom, and
                // the glyph cell is addressed with a top-left origin.
                glyphs.push(
                    transformed(
                        leaf,
                        Matrix4::from_translation(cgmath::vec3(pen, y, 0.0))
                            * Matrix4::from_nonuniform_scale(size, -size, 1.0),
                    )
                    .0,
                );
            }
            pen += size;
        }
    }
    group(glyphs, Matrix4::from_scale(1.0))
}

fn lower_sprite_image(
    width: &Value,
    height: &Value,
    source_pixels: Option<[f32; 4]>,
    path: &Value,
    pending: &Value,
    tint: [f32; 4],
    sampling: SpriteSampling,
) -> Result<Scene3D, String> {
    let (width, height) = (
        sprite_number(width, "Image")?,
        sprite_number(height, "Image")?,
    );
    let Value::String(path) = path else {
        return Err("invalid Image sprite data: expected a texture path".to_string());
    };
    let Value::List(pending) = pending else {
        return Err("invalid Image sprite data: expected placeholder texture paths".to_string());
    };
    let mut while_pending = Vec::with_capacity(pending.len());
    for item in pending.iter() {
        let Value::String(path) = item else {
            return Err(
                "invalid Image sprite data: expected placeholder texture paths".to_string(),
            );
        };
        while_pending.push(path.to_string());
    }
    let texture = if while_pending.is_empty() {
        TextureDescription::FileClamped(path.to_string())
    } else {
        TextureDescription::FileClampedWhilePending {
            file: path.to_string(),
            while_pending,
        }
    };
    let leaf = material_scene(
        MaterialDescription::sprite_texture_tinted(
            texture,
            source_pixels,
            sampling,
            tint[0],
            tint[1],
            tint[2],
            tint[3],
        ),
        FunctorLangScene(Scene3D::quad()),
    );
    // File textures upload top-row-first while GL's v=0 is the bottom; flip
    // the leaf locally so source PNGs and top-left atlas regions appear
    // upright in Y-up space.
    Ok(transformed(leaf, Matrix4::from_nonuniform_scale(width, -height, 1.0)).0)
}
