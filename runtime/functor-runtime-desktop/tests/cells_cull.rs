//! `Scene.cells` portal culling, end to end on GL, in three rooms in a row
//! (x 0..4, 4..8, 8..12) with a doorway in each shared wall:
//!
//! - looking down the doorways draws the far room; from beside them a wall
//!   hides it, and it is skipped, byte-identical to culling off;
//! - turning away hides the other rooms and turning back uploads nothing;
//! - a hidden `Model.static` draws nothing;
//! - each eye walks from its own position;
//! - a hidden node still casts its shadow.
//!
//! Self-skipping like `frustum_cull` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test cells_cull -- --ignored --nocapture
//! ```

use std::sync::Arc;

use cgmath::{vec3, vec4, Matrix4, SquareMatrix};
use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::cells::{Cell, Cells, CellsData, Plane, Portal};
use functor_runtime_common::gpu_counters::{gpu_counters, CellsCounts};
use functor_runtime_common::math::Angle;
use functor_runtime_common::model::{GroupSource, LightmapSource, ModelData, ModelSource};
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::texture::{ColorSpace, PixelData, PixelEncoding};
use functor_runtime_common::{
    render_frame_with_projection, Camera, CellsNode, DebugRenderMode, Frame, FrameTime,
    InstanceData, Light, MaterialDescription, MeshData, ModelDescription, ModelHandle, Scene3D,
    SceneContext, SceneObject, Shape, TextureDescription, Viewport,
};
use glow::HasContext;

const EYE: u32 = 128;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("cells_cull: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();
    a_doorway_shows_the_far_room_and_a_wall_hides_it(&mut harness);
    turning_back_uploads_nothing(&mut harness);
    a_hidden_static_model_draws_nothing(&mut harness);
    each_eye_walks_from_its_own_position(&mut harness);
    a_hidden_node_still_casts_its_shadow(&mut harness);
}

fn a_doorway_shows_the_far_room_and_a_wall_hides_it(h: &mut Harness) {
    let along = |z: f32| Camera::look_at([1.0, 1.5, z], [12.0, 1.0, z], UP, fov());
    let level = |far: bool| frame(along(0.0), rooms(far, vec![]), false);

    let on = h.render(&level(true), &[0], true);
    assert_eq!(
        gpu_counters().frame_cells(),
        CellsCounts {
            walks: 1,
            visible: 3,
            hidden: 0
        }
    );
    assert!(
        on == h.render(&level(true), &[0], false),
        "culling changed the image"
    );
    assert!(
        on != h.render(&level(false), &[0], true),
        "room 2's crate isn't visible through the doorways"
    );

    // Beside the doorways the next one is out of line: room 2 is hidden.
    let beside = frame(along(1.5), rooms(true, vec![]), false);
    let on = h.render(&beside, &[0], true);
    assert_eq!(
        gpu_counters().frame_cells(),
        CellsCounts {
            walks: 1,
            visible: 2,
            hidden: 1
        }
    );
    assert!(
        on == h.render(&beside, &[0], false),
        "culling changed the image"
    );

    // Under `Scene.opacity` (drawn by the transparent pass) nothing hides.
    let translucent = Scene3D {
        obj: SceneObject::Opacity(0.5, vec![rooms(true, vec![])]),
        xform: Matrix4::identity(),
    };
    let translucent = frame(along(1.5), translucent, false);
    let on = h.render(&translucent, &[0], true);
    assert_eq!(gpu_counters().frame_cells(), CellsCounts::default());
    assert!(
        on == h.render(&translucent, &[0], false),
        "culling changed the image"
    );
    println!("test a_doorway_shows_the_far_room_and_a_wall_hides_it ... ok");
}

fn turning_back_uploads_nothing(h: &mut Harness) {
    // Each room's contents also hold a mesh (its own `Arc`), an instanced
    // row and a lightmapped static model: each is released after a frame
    // it isn't used.
    let extras = (0..3)
        .map(|i| {
            let x = 4.0 * i as f32 + 2.0;
            let row: Vec<InstanceData> = (0..4)
                .map(|k| InstanceData::at([x - 1.5 + k as f32, 2.5, -1.5]))
                .collect();
            (
                i as u32,
                group(vec![
                    lit(mesh(), [0.3, 0.8, 0.3])
                        .transform(Matrix4::from_translation(vec3(x, 0.0, -1.0))),
                    Scene3D::instanced(
                        lit(Scene3D::sphere(), [0.9, 0.9, 0.2]).transform(Matrix4::from_scale(0.2)),
                        row,
                    ),
                    static_quad(x + 1.0, 1.2),
                ]),
            )
        })
        .collect();
    let scene = rooms(true, extras);
    let toward = frame(
        Camera::look_at([2.0, 1.5, 0.0], [12.0, 1.0, 0.0], UP, fov()),
        scene.clone(),
        false,
    );
    let away = frame(
        Camera::look_at([2.0, 1.5, 0.0], [-10.0, 1.0, 0.0], UP, fov()),
        scene,
        false,
    );
    let before = h.render(&toward, &[0], true);
    let live = gpu_counters().live();
    gpu_counters().take_window();
    for _ in 0..10 {
        h.render(&away, &[0], true);
        // Rooms 1 and 2: their crates and extras.
        assert_eq!(gpu_counters().frame_cells().hidden, 4);
    }
    let after = h.render(&toward, &[0], true);
    let window = gpu_counters().take_window();
    assert_eq!(
        window.bytes_uploaded, 0,
        "turning back re-uploaded: {window:?}"
    );
    let now = gpu_counters().live();
    assert_eq!(
        (now.buffers, now.vaos, now.textures),
        (live.buffers, live.vaos, live.textures)
    );
    assert!(after == before, "turning back changed the image");
    println!("test turning_back_uploads_nothing ... ok");
}

/// Room 2 is hidden from beside the doorways: a static model marked in it
/// draws nothing, even where it would show (here, in room 0).
fn a_hidden_static_model_draws_nothing(h: &mut Harness) {
    let camera = Camera::look_at([1.0, 1.5, 1.5], [12.0, 1.0, 1.5], UP, fov());
    let misplaced = vec![(2, static_quad(3.0, 1.5))];
    let without = h.render(&frame(camera.clone(), rooms(true, vec![]), false), &[0], true);
    let with = h.render(&frame(camera.clone(), rooms(true, misplaced.clone()), false), &[0], true);
    assert_eq!(gpu_counters().frame_cells().hidden, 2);
    assert!(with == without, "the hidden static model drew");
    // Shown (in room 0), it draws.
    let shown = vec![(0, misplaced[0].1.clone())];
    assert!(
        h.render(&frame(camera, rooms(true, shown), false), &[0], true) != without,
        "the static model never draws here"
    );
    println!("test a_hidden_static_model_draws_nothing ... ok");
}

fn each_eye_walks_from_its_own_position(h: &mut Harness) {
    // Eyes 1.5 m apart facing +X: the left one (z = 0) is in line with the
    // doorways, the right one (z = 1.5) isn't.
    let head = Camera::look_at([1.0, 1.5, 0.75], [12.0, 1.0, 0.75], UP, fov());
    let stereo = frame(head, rooms(true, vec![]), false);
    h.ipd = 1.5;
    let on = h.render(&stereo, &[0, 1], true);
    assert_eq!(
        gpu_counters().frame_cells(),
        CellsCounts {
            walks: 2,
            visible: 3 + 2,
            hidden: 1
        }
    );
    assert!(
        on == h.render(&stereo, &[0, 1], false),
        "culling changed an eye"
    );
    h.ipd = 0.064;
    println!("test each_eye_walks_from_its_own_position ... ok");
}

fn a_hidden_node_still_casts_its_shadow(h: &mut Harness) {
    // A caster marked as in room 2 hangs above room 0's floor, above the
    // view. Looking away from the doorways room 2 is hidden, yet its shadow
    // falls on the floor in view.
    let camera = Camera::look_at([3.8, 1.5, 0.0], [0.0, 0.0, 0.0], UP, fov());
    let caster = lit(Scene3D::cube(), [0.9, 0.2, 0.2])
        .transform(Matrix4::from_translation(vec3(2.0, 2.8, 0.0)) * Matrix4::from_scale(0.6));
    let with = frame(camera.clone(), rooms(true, vec![(2, caster)]), true);
    let without = frame(camera, rooms(true, vec![]), true);
    let shadowed = h.render(&with, &[0], true);
    assert!(
        gpu_counters().frame_cells().hidden >= 1,
        "the caster isn't hidden"
    );
    assert!(
        shadowed == h.render(&with, &[0], false),
        "culling changed the shadow"
    );
    assert!(
        shadowed != h.render(&without, &[0], true),
        "the hidden caster cast no shadow"
    );
    println!("test a_hidden_node_still_casts_its_shadow ... ok");
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];

fn fov() -> Angle {
    Angle::from_degrees(60.0)
}

/// The three rooms' cells, a doorway (z ±0.5, 2 high) both ways in each
/// shared wall.
fn cells() -> Arc<Cells> {
    let room = |x: f32| Cell {
        planes: vec![
            Plane {
                n: [1.0, 0.0, 0.0],
                d: -x,
            },
            Plane {
                n: [-1.0, 0.0, 0.0],
                d: x + 4.0,
            },
            Plane {
                n: [0.0, 1.0, 0.0],
                d: 0.0,
            },
            Plane {
                n: [0.0, -1.0, 0.0],
                d: 3.0,
            },
            Plane {
                n: [0.0, 0.0, 1.0],
                d: 2.0,
            },
            Plane {
                n: [0.0, 0.0, -1.0],
                d: 2.0,
            },
        ],
    };
    let door = |x: f32| vec![[x, 0.0, -0.5], [x, 0.0, 0.5], [x, 2.0, 0.5], [x, 2.0, -0.5]];
    let mut portals = Vec::new();
    for (a, x) in [(0, 4.0), (1, 8.0)] {
        let polygon = door(x);
        let mut back = polygon.clone();
        back.reverse();
        portals.push(Portal {
            from: a,
            to: a + 1,
            polygon,
        });
        portals.push(Portal {
            from: a + 1,
            to: a,
            polygon: back,
        });
    }
    let data = CellsData {
        cells: vec![room(0.0), room(4.0), room(8.0)],
        portals,
    };
    Arc::new(Cells::new(data).unwrap())
}

/// The three rooms' walls and floor (unmarked: always drawn), a crate in
/// each room marked with its cell (room 2's only when `far`), and `extras`
/// marked with their cells.
fn rooms(far: bool, extras: Vec<(u32, Scene3D)>) -> Scene3D {
    let wall = [0.7, 0.7, 0.75];
    let mut items = vec![
        boxed([0.0, -0.1, -2.0], [12.0, 0.0, 2.0], [0.5, 0.45, 0.4]),
        boxed([0.0, 0.0, -2.1], [12.0, 3.0, -2.0], wall),
        boxed([0.0, 0.0, 2.0], [12.0, 3.0, 2.1], wall),
        boxed([-0.1, 0.0, -2.0], [0.0, 3.0, 2.0], wall),
        boxed([12.0, 0.0, -2.0], [12.1, 3.0, 2.0], wall),
    ];
    for x in [4.0, 8.0] {
        items.push(boxed([x - 0.05, 0.0, -2.0], [x + 0.05, 3.0, -0.5], wall));
        items.push(boxed([x - 0.05, 0.0, 0.5], [x + 0.05, 3.0, 2.0], wall));
        items.push(boxed([x - 0.05, 2.0, -0.5], [x + 0.05, 3.0, 0.5], wall));
    }
    // Room 1's crate sits off the doorway line so room 2's shows past it.
    items.push(in_cells(
        0,
        boxed([1.5, 0.0, -1.6], [2.5, 1.0, -0.6], [0.2, 0.4, 0.9]),
    ));
    items.push(in_cells(
        1,
        boxed([5.5, 0.0, 0.9], [6.5, 1.0, 1.9], [0.9, 0.5, 0.1]),
    ));
    if far {
        items.push(in_cells(
            2,
            boxed([9.5, 0.0, -0.5], [10.5, 1.0, 0.5], [0.8, 0.1, 0.6]),
        ));
    }
    for (cell, extra) in extras {
        items.push(in_cells(cell, extra));
    }
    Scene3D {
        obj: SceneObject::Cells(CellsNode::Walk(cells()), vec![group(items)]),
        xform: Matrix4::identity(),
    }
}

fn in_cells(cell: u32, scene: Scene3D) -> Scene3D {
    Scene3D {
        obj: SceneObject::Cells(CellsNode::In(vec![cell]), vec![scene]),
        xform: Matrix4::identity(),
    }
}

/// A lit box from `min` to `max`.
fn boxed(min: [f32; 3], max: [f32; 3], color: [f32; 3]) -> Scene3D {
    let center = vec3(min[0] + max[0], min[1] + max[1], min[2] + max[2]) * 0.5;
    let size = |i: usize| max[i] - min[i];
    lit(Scene3D::cube(), color).transform(
        Matrix4::from_translation(center)
            * Matrix4::from_nonuniform_scale(size(0), size(1), size(2)),
    )
}

fn group(items: Vec<Scene3D>) -> Scene3D {
    Scene3D {
        obj: SceneObject::Group(items),
        xform: Matrix4::identity(),
    }
}

/// `scene` under a lit material, in a group so a later transform applies.
fn lit(scene: Scene3D, [r, g, b]: [f32; 3]) -> Scene3D {
    group(vec![Scene3D {
        obj: SceneObject::Material(
            MaterialDescription::Lit {
                color: vec4(r, g, b, 1.0),
                texture: None,
                normal_map: None,
            },
            vec![scene],
        ),
        xform: Matrix4::identity(),
    }])
}

/// A 1 m `Model.static` square facing -x at `x`, centred on `z`: one group
/// with a pixel texture, lightmapped by another.
fn static_quad(x: f32, z: f32) -> Scene3D {
    let pixels = |space, rgb: [u8; 3]| {
        let pixels = PixelData::new(1, 1, PixelEncoding::Rgb8, space, &rgb);
        TextureDescription::Data(Arc::new(pixels.expect("valid pixels")))
    };
    let (z0, z1) = (z - 0.5, z + 0.5);
    let data = ModelData::new(ModelSource {
        positions: vec![[x, 0.5, z0], [x, 0.5, z1], [x, 1.5, z1], [x, 1.5, z0]],
        normals: vec![[-1.0, 0.0, 0.0]; 4],
        uvs: vec![[0.5, 0.5]; 4],
        uvs2: vec![[0.5, 0.5]; 4],
        joints: vec![],
        weights: vec![],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        groups: vec![GroupSource {
            first: 0,
            count: 2,
            texture: Some(pixels(ColorSpace::Srgb, [40, 200, 200])),
            unlit: false,
        }],
        lightmap: Some(LightmapSource {
            texture: pixels(ColorSpace::Linear, [255, 255, 255]),
            scale: 1.0,
            floor: [0.0; 3],
        }),
        skeleton: vec![],
        clips: vec![],
        texture: None,
    })
    .expect("valid model");
    Scene3D::model(ModelDescription {
        handle: ModelHandle::Data(Arc::new(data)),
        overrides: vec![],
        animation: None,
        while_pending: vec![],
        parts: Vec::new(),
    })
}

/// A unit tetrahedron with its own `Arc`, so each is a separate GPU mesh.
fn mesh() -> Scene3D {
    let data = MeshData {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ],
        triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
    };
    Scene3D {
        obj: SceneObject::Geometry(Shape::Mesh {
            mesh: Arc::new(data),
        }),
        xform: Matrix4::identity(),
    }
}

/// A frame of `scene` lit by an ambient light and a sun (casting when `sun`).
fn frame(camera: Camera, scene: Scene3D, sun: bool) -> Frame {
    let mut frame = Frame::new(camera, scene);
    frame.lights = vec![Light::ambient(0.4, 0.4, 0.4)];
    frame.lights.push(Light::Directional {
        direction: [0.05, -1.0, 0.1],
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        casts_shadows: sun,
        baked: false,
    });
    frame
}

struct Harness {
    gl: glow::Context,
    fbo: glow::Framebuffer,
    asset_cache: Arc<AssetCache>,
    scene_context: SceneContext,
    shadow_map: ShadowMap,
    frame_id: u64,
    ipd: f32,
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
            .create_window(EYE, EYE, "cells-cull-test", glfw::WindowMode::Windowed)
            .expect("create hidden GLFW window");
        window.make_current();
        let gl = unsafe {
            glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _)
        };
        let fbo = unsafe {
            let color = gl.create_texture().expect("texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_storage_2d(
                glow::TEXTURE_2D,
                1,
                glow::RGBA8,
                (2 * EYE) as i32,
                EYE as i32,
            );
            let depth = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT24,
                (2 * EYE) as i32,
                EYE as i32,
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
            ipd: 0.064,
            _window: window,
            _glfw: glfw,
        }
    }

    /// Render `eyes` (0 = left, 1 = right) as one frame id, culling or not,
    /// and read each eye's pixels back.
    fn render(&mut self, frame: &Frame, eyes: &[usize], cull: bool) -> Vec<Vec<u8>> {
        self.frame_id += 1;
        self.scene_context.set_frustum_culling(cull);
        let gl = &self.gl;
        let (left, right) = frame.camera.stereo_eyes(self.ipd);
        let cameras = [left, right];
        for &eye in eyes {
            let camera = &cameras[eye];
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
                &frame.camera,
                &[view_projection],
                projection.y.y.abs(),
                EYE as f32,
                self.frame_id,
                None,
                FrameTime { tts: 1.0, dts: 0.0 },
                Viewport::with_offset(eye as u32 * EYE, 0, EYE, EYE),
                DebugRenderMode::Default,
            );
        }
        eyes.iter()
            .map(|&eye| {
                let mut pixels = vec![0u8; (EYE * EYE * 4) as usize];
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
                    gl.read_pixels(
                        (eye as u32 * EYE) as i32,
                        0,
                        EYE as i32,
                        EYE as i32,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut pixels)),
                    );
                }
                pixels
            })
            .collect()
    }
}
