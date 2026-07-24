//! The plain-data `Sprite` picture algebra and its `Camera2D` frame boundary.
//!
//! Kept in its own module so this sizeable, mostly cold registration/lowering
//! path does not perturb code layout for the existing per-frame 3D prelude.

use super::*;

/// A center-origin, Y-up [`Camera2D`] used by sprite frame passes.
struct FunctorLangCamera2D(Camera2D);

/// A Sprite value is deliberately NOT host data. The wrapper exists only for
/// typed registry argument conversion; its inner [`Value`] is a plain
/// variant/list tree, so it compares, inspects, serializes, and survives hot
/// reload like ordinary game data.
struct FunctorLangSprite(Value);

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

fn sprite_node(name: &str, args: Vec<Value>) -> Value {
    Value::Variant {
        ctor: Rc::from(format!("Sprite.{name}")),
        args: Rc::new(args),
    }
}

fn sprite_children(items: Vec<FunctorLangSprite>) -> Value {
    Value::List(Rc::new(items.into_iter().map(|item| item.0).collect()))
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
    reg.fn3(
        "Sprite.image",
        "Sprite.image(width, height, texture)",
        |width: f64, height: f64, texture: FunctorLangTexture| {
            if width <= 0.0 || height <= 0.0 {
                return Err(format!(
                    "Sprite.image width and height must be positive, got {width} × {height}"
                ));
            }
            let (path, pending) = match texture.0 {
                TextureDescription::File(path) => (path, vec![]),
                TextureDescription::FileWhilePending {
                    file,
                    while_pending,
                } => (file, while_pending),
                TextureDescription::FileClamped(path) => (path, vec![]),
                TextureDescription::FileClampedWhilePending {
                    file,
                    while_pending,
                } => (file, while_pending),
                TextureDescription::RenderTarget(_) => {
                    return Err(
                        "Sprite.image expects an image asset, not a render target".to_string()
                    )
                }
            };
            Ok(sprite_node(
                "Image",
                vec![
                    Value::Number(width),
                    Value::Number(height),
                    Value::String(path.into()),
                    Value::List(Rc::new(
                        pending
                            .into_iter()
                            .map(|path| Value::String(path.into()))
                            .collect(),
                    )),
                ],
            ))
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
        "Frame.create2D",
        "Frame.create2D(camera, sprite)",
        |camera: FunctorLangCamera2D, sprite: FunctorLangSprite| {
            let layer = SpriteLayer {
                camera: camera.0,
                scene: lower_sprite(&sprite.0, [1.0, 1.0, 1.0, 1.0])?,
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
                scene: lower_sprite(&sprite.0, [1.0, 1.0, 1.0, 1.0])?,
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

fn lower_sprite(value: &Value, tint: [f32; 4]) -> Result<Scene3D, String> {
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
        ("Sprite.Image", [width, height, path, pending]) => {
            let (width, height) = (
                sprite_number(width, "Image")?,
                sprite_number(height, "Image")?,
            );
            let Value::String(path) = path else {
                return Err("invalid Image sprite data: expected a texture path".to_string());
            };
            let Value::List(pending) = pending else {
                return Err(
                    "invalid Image sprite data: expected placeholder texture paths".to_string(),
                );
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
                MaterialDescription::emissive_texture_tinted(
                    texture, tint[0], tint[1], tint[2], tint[3],
                ),
                FunctorLangScene(Scene3D::quad()),
            );
            // File textures upload top-row-first while GL's v=0 is the bottom;
            // flip the leaf locally so source PNGs appear upright in Y-up space.
            Ok(transformed(leaf, Matrix4::from_nonuniform_scale(width, -height, 1.0)).0)
        }
        ("Sprite.Group", [Value::List(items)]) => {
            let scenes = items
                .iter()
                .map(|item| lower_sprite(item, tint))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(group(scenes, Matrix4::from_scale(1.0)))
        }
        ("Sprite.Move", [x, y, child]) => Ok(transformed(
            FunctorLangScene(lower_sprite(child, tint)?),
            Matrix4::from_translation(cgmath::vec3(
                sprite_number(x, "Move")?,
                sprite_number(y, "Move")?,
                0.0,
            )),
        )
        .0),
        ("Sprite.Rotate", [angle, child]) => Ok(transformed(
            FunctorLangScene(lower_sprite(child, tint)?),
            Matrix4::from_angle_z(cgmath::Rad(sprite_number(angle, "Rotate")?)),
        )
        .0),
        ("Sprite.Scale", [x, y, child]) => Ok(transformed(
            FunctorLangScene(lower_sprite(child, tint)?),
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
            lower_sprite(child, next)
        }
        ("Sprite.Tint", [r, g, b, child]) => {
            let mut next = tint;
            next[0] *= sprite_number(r, "Tint")?;
            next[1] *= sprite_number(g, "Tint")?;
            next[2] *= sprite_number(b, "Tint")?;
            lower_sprite(child, next)
        }
        _ => Err(format!("invalid Sprite data: malformed {ctor} node")),
    }
}
