//! Pins `Model.static` models on the GPU:
//!
//! - groups draw with their own base textures, in one model; an `unlit`
//!   group ignores the frame's lights, and a frame without lights draws
//!   every group fullbright;
//! - one model drawn by several nodes uploads once, steady frames upload
//!   nothing, and undrawn it is deleted;
//! - the shadow pass and the instanced (stamped) path draw it;
//! - a lightmapped texel is base × lightmap × scale, in either lightmap
//!   colour space, sampled clamped without mips; `floor` is a minimum; `Light.baked` lights skip
//!   lightmapped surfaces but still light others; unbaked lights add;
//! - `Scene.withLightmap` swaps the lightmap texture, e.g. for a layered
//!   composition of the model's own atlas.
//!
//! Self-skipping like `skinned_data` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test static_models -- --ignored --nocapture
//! ```

use std::sync::Arc;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::color_space::{linear_to_srgb, srgb_u8_to_linear};
use functor_runtime_common::model::{GroupSource, LightmapSource, ModelData, ModelSource};
use functor_runtime_common::render::VertexPositionTextureLightmap;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::texture::{ColorSpace, Layer, LayeredImage, PixelData, PixelEncoding};
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, InstanceData, Light,
    MaterialDescription, ModelDescription, ModelHandle, Scene3D, SceneContext, SceneObject,
    TextureDescription, Viewport,
};
use glow::HasContext;

const SIZE: u32 = 128;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("static_models: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();
    groups(&mut harness);
    println!("test groups ... ok");
    one_upload_for_many_nodes(&mut harness);
    println!("test one_upload_for_many_nodes ... ok");
    shadow_and_instanced(&mut harness);
    println!("test shadow_and_instanced ... ok");
    lightmap_texels(&mut harness);
    println!("test lightmap_texels ... ok");
    baked_and_unbaked_lights(&mut harness);
    println!("test baked_and_unbaked_lights ... ok");
    with_lightmap(&mut harness);
    println!("test with_lightmap ... ok");
}

/// A 1×1 sRGB texture.
fn solid(rgb: [u8; 3]) -> TextureDescription {
    solid_in(rgb, ColorSpace::Srgb)
}

fn solid_in(rgb: [u8; 3], space: ColorSpace) -> TextureDescription {
    let pixels = PixelData::new(1, 1, PixelEncoding::Rgb8, space, &rgb);
    TextureDescription::Data(Arc::new(pixels.expect("valid pixels")))
}

/// The left quad of `room`, one group with `base`, lightmapped by
/// `lightmap` at `scale` and `floor`.
fn lightmapped(
    base: TextureDescription,
    lightmap: TextureDescription,
    scale: f32,
    floor: [f32; 3],
) -> Arc<ModelData> {
    let room = room(&base, &base);
    model(ModelSource {
        positions: room.positions[..4].to_vec(),
        normals: room.normals[..4].to_vec(),
        uvs: room.uvs[..4].to_vec(),
        uvs2: vec![[0.5, 0.5]; 4],
        triangles: room.triangles[..2].to_vec(),
        groups: vec![GroupSource {
            first: 0,
            count: 2,
            texture: Some(base),
            unlit: false,
        }],
        lightmap: Some(LightmapSource {
            texture: lightmap,
            scale,
            floor,
        }),
        ..room
    })
}

/// Each lightmapped texel is base × lightmap × scale in linear light: a
/// `Linear` lightmap byte is its value / 255, an `Srgb` one is decoded
/// first (multiplying in gamma space). No frame lights are needed.
fn lightmap_texels(harness: &mut Harness) {
    for space in [ColorSpace::Linear, ColorSpace::Srgb] {
        for (base, light, scale) in [
            ([200, 120, 60], [128, 128, 128], 1.0),
            ([255, 255, 255], [64, 160, 255], 1.0),
            ([90, 200, 250], [100, 50, 200], 1.5),
        ] {
            let room = lightmapped(solid(base), solid_in(light, space), scale, [0.0; 3]);
            let image = harness.render(&frame(&room, 1, false, vec![]));
            let want: [u8; 3] = std::array::from_fn(|c| {
                let lm = match space {
                    ColorSpace::Linear => light[c] as f32 / 255.0,
                    ColorSpace::Srgb => srgb_u8_to_linear(light[c]),
                };
                let out = srgb_u8_to_linear(base[c]) * lm * scale;
                (linear_to_srgb(out.min(1.0)) * 255.0).round() as u8
            });
            assert_near(rgb(&image, 32, 64), want, &format!("{space:?} {base:?} × {light:?}"));
        }
    }
    // uv2 indexes the lightmap: across the quad it runs between the texel
    // centres of a black | white 2×1 map, so the light ramps up left to right.
    let ramp = PixelData::new(2, 1, PixelEncoding::Rgb8, ColorSpace::Linear, &[0, 0, 0, 255, 255, 255]);
    let ramp = TextureDescription::Data(Arc::new(ramp.expect("valid pixels")));
    let room = lightmapped(solid([255, 255, 255]), ramp, 1.0, [0.0; 3]);
    let room = model(ModelSource {
        uvs2: vec![[0.25, 0.5], [0.75, 0.5], [0.75, 0.5], [0.25, 0.5]],
        ..room.source().clone()
    });
    let image = harness.render(&frame(&room, 1, false, vec![]));
    for x in [8, 32, 56] {
        // Pixel x sees world x = (x / 64 - 1) × 2.31, on a quad from -3 to 0.
        let world = (x as f32 / 64.0 - 1.0) * 4.0 * 30f32.to_radians().tan();
        let light = (world + 3.0) / 3.0;
        let want = (linear_to_srgb(light) * 255.0).round() as u8;
        let seen = rgb(&image, x, 64)[0];
        assert!(seen.abs_diff(want) <= 6, "uv2 ramp at {x}: drew {seen}, want {want}");
    }
    // Lightmaps sample clamped with no mips: uv2 running far past 1 over a
    // white | black `Srgb` map (mipmapped, repeating as a texture) reads the
    // black edge texel, not the grey average of the smallest mip.
    let edge = PixelData::new(2, 1, PixelEncoding::Rgb8, ColorSpace::Srgb, &[255, 255, 255, 0, 0, 0]);
    let edge = TextureDescription::Data(Arc::new(edge.expect("valid pixels")));
    let room = lightmapped(solid([255, 255, 255]), edge, 1.0, [0.0; 3]);
    let room = model(ModelSource {
        uvs2: vec![[0.0, 0.5], [100.0, 0.5], [100.0, 0.5], [0.0, 0.5]],
        ..room.source().clone()
    });
    let image = harness.render(&frame(&room, 1, false, vec![]));
    assert_near(rgb(&image, 32, 64), [0, 0, 0], "clamped, unmipmapped lightmap");
    // A dark lightmap shows `floor` (an sRGB colour) instead.
    let room = lightmapped(
        solid([255, 255, 255]),
        solid_in([0, 0, 0], ColorSpace::Linear),
        1.0,
        [0.5, 0.25, 0.0],
    );
    let image = harness.render(&frame(&room, 1, false, vec![]));
    assert_near(rgb(&image, 32, 64), [128, 64, 0], "floor");
}

/// A white quad whose own lightmap is a black 1×1 atlas with one white
/// layer: drawn as is it is black; `with_lightmap` of that layer at 0.5
/// lights it to linear 0.5, and of another texture uses that one.
fn with_lightmap(harness: &mut Harness) {
    let base = PixelData::new(1, 1, PixelEncoding::Rgb8, ColorSpace::Linear, &[0; 3]).unwrap();
    let image = LayeredImage {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        padding: 0,
        layers: vec![Layer { id: 1.0, rgba8: vec![255; 4] }],
    };
    let atlas = Arc::new(base.with_layers(vec![image]).unwrap());
    let room = lightmapped(solid([255, 255, 255]), TextureDescription::Data(atlas.clone()), 1.0, [0.0; 3]);
    let draw = |harness: &mut Harness, lightmap: Option<TextureDescription>| {
        let mut frame = frame(&room, 1, false, vec![]);
        if let Some(lightmap) = lightmap {
            frame.scene = frame.scene.with_lightmap(lightmap);
        }
        rgb(&harness.render(&frame), 32, 64)
    };
    assert_near(draw(harness, None), [0, 0, 0], "own lightmap");
    let half = TextureDescription::Layered { base: atlas.clone(), intensities: vec![(1.0, 0.5)] };
    assert_near(draw(harness, Some(half)), [188, 188, 188], "layer at 0.5");
    let grey = solid_in([64, 64, 64], ColorSpace::Linear);
    assert_near(draw(harness, Some(grey)), [137, 137, 137], "another texture");
    assert_near(draw(harness, None), [0, 0, 0], "own lightmap again");
}

/// A `Light.baked` ambient leaves the lightmapped quad as the lightmap has
/// it but brightens a lit cube; an unbaked point light adds to the quad.
fn baked_and_unbaked_lights(harness: &mut Harness) {
    let room = lightmapped(
        solid([255, 255, 255]),
        solid_in([64, 64, 64], ColorSpace::Linear),
        1.0,
        [0.0; 3],
    );
    let with_cube = |lights: Vec<Light>| {
        let mut frame = frame(&room, 1, false, lights);
        let cube = Scene3D::cube().scale_x(1.0).translate_x(1.5);
        let cube = Scene3D {
            obj: SceneObject::Material(MaterialDescription::lit(1.0, 1.0, 1.0, 1.0), vec![cube]),
            xform: cgmath::Matrix4::from_scale(1.0),
        };
        let SceneObject::Group(items) = &mut frame.scene.obj else {
            unreachable!()
        };
        items.push(cube);
        frame
    };
    let dark = harness.render(&with_cube(vec![Light::ambient(0.05, 0.05, 0.05)]));
    let baked = harness.render(&with_cube(vec![Light::ambient(1.0, 1.0, 1.0).baked()]));
    let unbaked = harness.render(&with_cube(vec![Light::ambient(1.0, 1.0, 1.0)]));
    let quad = |image: &[u8]| rgb(image, 32, 64);
    let cube = |image: &[u8]| rgb(image, 96, 64);
    // 64 / 255 linear encodes to ~137.
    assert_near(quad(&baked), [137, 137, 137], "baked ambient skips the quad");
    assert!(cube(&baked)[0] > cube(&dark)[0] + 100, "baked ambient lights the cube");
    assert!(quad(&unbaked)[0] > 250, "an unbaked ambient adds to the quad");
    let point = Light::point(-1.5, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 4.0);
    let lit = harness.render(&with_cube(vec![point]));
    assert!(quad(&lit)[0] > quad(&baked)[0] + 40, "an unbaked point adds");
}

/// Two quads filling the view, facing the camera: the left lit and red,
/// the right unlit and green.
fn room(red: &TextureDescription, green: &TextureDescription) -> ModelSource {
    let quad = |x0: f32, x1: f32| [[x0, -3.0, 0.0], [x1, -3.0, 0.0], [x1, 3.0, 0.0], [x0, 3.0, 0.0]];
    ModelSource {
        positions: [quad(-3.0, 0.0), quad(0.0, 3.0)].concat(),
        normals: vec![[0.0, 0.0, 1.0]; 8],
        uvs: vec![[0.5, 0.5]; 8],
        uvs2: vec![],
        joints: vec![],
        weights: vec![],
        triangles: vec![[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        groups: vec![
            GroupSource {
                first: 0,
                count: 2,
                texture: Some(red.clone()),
                unlit: false,
            },
            GroupSource {
                first: 2,
                count: 2,
                texture: Some(green.clone()),
                unlit: true,
            },
        ],
        lightmap: None,
        skeleton: vec![],
        clips: vec![],
        texture: None,
    }
}

fn model(source: ModelSource) -> Arc<ModelData> {
    Arc::new(ModelData::new(source).expect("valid model"))
}

/// Each group shows its own texture; only the lit one dims under a dim
/// ambient light, and neither in a frame without lights.
fn groups(harness: &mut Harness) {
    let (red, green) = (solid([255, 0, 0]), solid([0, 255, 0]));
    let room = model(room(&red, &green));
    // sRGB 0.5 is ~0.21 linear: the lit red texel encodes back to ~128.
    let dim = harness.render(&frame(&room, 1, false, vec![Light::ambient(0.5, 0.5, 0.5)]));
    assert_near(rgb(&dim, 32, 64), [128, 0, 0], "lit group, dim ambient");
    assert_near(rgb(&dim, 96, 64), [0, 255, 0], "unlit group");
    let unlit_frame = harness.render(&frame(&room, 1, false, vec![]));
    assert_near(rgb(&unlit_frame, 32, 64), [255, 0, 0], "no lights: fullbright");
    // No groups: one untextured (white) lit group.
    let plain = model(ModelSource {
        groups: vec![],
        lightmap: None,
        ..self::room(&red, &green)
    });
    let white = harness.render(&frame(&plain, 1, false, vec![Light::ambient(1.0, 1.0, 1.0)]));
    assert_near(rgb(&white, 96, 64), [255, 255, 255], "no groups: white");
}

/// Five nodes share one upload of the mesh; steady frames upload nothing;
/// undrawn, its buffers and VAO are deleted.
fn one_upload_for_many_nodes(harness: &mut Harness) {
    let (red, green) = (solid([255, 0, 0]), solid([0, 255, 0]));
    let lights = || vec![Light::ambient(1.0, 1.0, 1.0)];
    // Warm the textures with one model, so the next frame uploads only the
    // second model's mesh.
    harness.render(&frame(&model(room(&red, &green)), 1, false, lights()));
    let before = gpu_counters().live();
    let room = model(room(&red, &green));
    let mesh_bytes = room.source().positions.len()
        * std::mem::size_of::<VertexPositionTextureLightmap>()
        + room.source().triangles.len() * 12;
    gpu_counters().take_window();
    harness.render(&frame(&room, 5, false, lights()));
    let first = gpu_counters().take_window();
    assert_eq!(first.bytes_uploaded as usize, mesh_bytes, "mesh uploaded once");
    harness.render(&frame(&room, 5, false, lights()));
    assert_eq!(gpu_counters().take_window().bytes_uploaded, 0, "steady frame");
    for _ in 0..3 {
        harness.render(&frame(&room, 0, false, lights()));
    }
    let after = gpu_counters().live();
    // The warm-up model was evicted too, so at most `before`'s objects remain.
    assert!(
        after.buffers + 2 <= before.buffers && after.vaos < before.vaos,
        "evicted: {before:?} -> {after:?}"
    );
}

/// A static occluder off-screen (between the sun and the room) shadows the
/// room through the depth pass, and an instanced copy (stamped) draws what
/// a plain node does.
fn shadow_and_instanced(harness: &mut Harness) {
    let (red, green) = (solid([255, 0, 0]), solid([0, 255, 0]));
    let room = model(room(&red, &green));
    // Seen from the camera it lies left of the view; the sun, travelling
    // +x and -z, casts it onto the room at x ≈ -2.4 … -1.4.
    let occluder = model(ModelSource {
        positions: vec![[-3.5, -1.0, 1.5], [-2.5, -1.0, 1.5], [-2.5, 1.0, 1.5], [-3.5, 1.0, 1.5]],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uvs: vec![],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        groups: vec![],
        lightmap: None,
        ..self::room(&red, &green)
    });
    let sun = vec![
        Light::ambient(0.2, 0.2, 0.2),
        Light::directional(0.5, 0.0, -1.0, 1.0, 1.0, 1.0, 1.0).cast_shadows(),
    ];
    let mut shadowed = frame(&room, 1, false, sun.clone());
    let SceneObject::Group(items) = &mut shadowed.scene.obj else {
        unreachable!()
    };
    items.push(frame(&occluder, 1, false, vec![]).scene);
    let lit = harness.render(&frame(&room, 1, false, sun.clone()));
    let dark = harness.render(&shadowed);
    // World x = -1.9 on the room plane is pixel 11; x = -0.5 is pixel 50.
    let (lit_r, dark_r) = (rgb(&lit, 11, 64)[0], rgb(&dark, 11, 64)[0]);
    assert!(lit_r > 200 && dark_r < 140, "shadow: lit {lit_r}, shadowed {dark_r}");
    assert_eq!(rgb(&dark, 50, 64), rgb(&lit, 50, 64), "outside the shadow");
    let instanced = harness.render(&frame(&room, 1, true, sun));
    assert!(lit == instanced, "the stamped copy differs");
}

/// Pixel `(x, y)` of a read-back frame (row 0 at the bottom).
fn rgb(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

fn assert_near(seen: [u8; 3], want: [u8; 3], what: &str) {
    let close = seen.iter().zip(want).all(|(s, w)| s.abs_diff(w) <= 3);
    assert!(close, "{what}: drew {seen:?}, want {want:?}");
}

/// `nodes` nodes of `model` at the origin (one instanced copy when
/// `instanced`), seen from 4 units in front.
fn frame(model: &Arc<ModelData>, nodes: usize, instanced: bool, lights: Vec<Light>) -> Frame {
    let node = || {
        Scene3D::model(ModelDescription {
            handle: ModelHandle::Data(model.clone()),
            overrides: vec![],
            animation: None,
            while_pending: vec![],
        })
    };
    let items = (0..nodes)
        .map(|_| match instanced {
            true => Scene3D::instanced(node(), vec![InstanceData::at([0.0, 0.0, 0.0])]),
            false => node(),
        })
        .collect();
    let camera = Camera::look_at(
        [0.0, 0.0, 4.0],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        Angle::from_degrees(60.0),
    );
    let mut frame = Frame::new(
        camera,
        Scene3D {
            obj: SceneObject::Group(items),
            xform: cgmath::Matrix4::from_scale(1.0),
        },
    );
    frame.lights = lights;
    frame
}

struct Harness {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    asset_cache: Arc<AssetCache>,
    scene_context: SceneContext,
    shadow_map: ShadowMap,
    frame_id: u64,
    _window: glfw::PWindow,
    _glfw: glfw::Glfw,
}

impl Harness {
    fn new() -> Harness {
        use glfw::Context as _;
        let mut glfw = glfw::init(glfw::fail_on_errors).expect("init GLFW");
        glfw.window_hint(glfw::WindowHint::ContextVersion(4, 1));
        glfw.window_hint(glfw::WindowHint::OpenGlProfile(
            glfw::OpenGlProfileHint::Core,
        ));
        glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true));
        glfw.window_hint(glfw::WindowHint::Visible(false));
        let (mut window, _events) = glfw
            .create_window(SIZE, SIZE, "static-models-test", glfw::WindowMode::Windowed)
            .expect("create hidden GLFW window");
        window.make_current();
        let gl = unsafe {
            glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)
        };
        let fbo = unsafe {
            let color = gl.create_texture().expect("texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, SIZE as i32, SIZE as i32);
            let depth = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT24,
                SIZE as i32,
                SIZE as i32,
            );
            let fbo = gl.create_framebuffer().expect("fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(color),
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            assert_eq!(
                gl.check_framebuffer_status(glow::FRAMEBUFFER),
                glow::FRAMEBUFFER_COMPLETE
            );
            fbo
        };
        let shadow_map = ShadowMap::new(&gl, 512);
        Harness {
            gl,
            fbo,
            asset_cache: Arc::new(AssetCache::new()),
            scene_context: SceneContext::new(),
            shadow_map,
            frame_id: 0,
            _window: window,
            _glfw: glfw,
        }
    }

    /// Render `frame` as a new frame id and read its pixels back.
    fn render(&mut self, frame: &Frame) -> Vec<u8> {
        self.frame_id += 1;
        let gl = &self.gl;
        let camera = &frame.camera;
        let projection = camera.projection_matrix(1.0);
        let view_projection = projection * camera.view_matrix();
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::DEPTH_TEST);
        }
        render_frame_with_projection(
            gl,
            "#version 410",
            self.asset_cache.clone(),
            &self.scene_context,
            &self.shadow_map,
            frame,
            camera,
            &projection,
            camera,
            &[view_projection],
            projection.y.y.abs(),
            SIZE as f32,
            self.frame_id,
            None,
            FrameTime { tts: 1.0, dts: 0.0 },
            Viewport::with_offset(0, 0, SIZE, SIZE),
            DebugRenderMode::Default,
        );
        let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
        unsafe {
            gl.read_pixels(
                0,
                0,
                SIZE as i32,
                SIZE as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        pixels
    }
}
