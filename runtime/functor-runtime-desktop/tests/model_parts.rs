//! `Scene.modelParts` in the GL renderer: a hidden part draws in no pass.
//!
//! The golden is computed, not committed: a two-part model with `Hat`
//! deselected must render byte-identical — both eyes, shadow included — to a
//! fixture that has only `Body`. The sun is low, so the hat's shadow lands in
//! view; the whole model must differ. Covered for the ordinary and instanced
//! paths, skinned and rigid, nested selections, and an unknown name. Skinned
//! draw counts drop with the selection while pose evaluations don't.
//!
//! Self-skipping like `stereo_shadow` (needs a GL display; GLFW on the main
//! thread, so `harness = false`):
//!
//! ```sh
//! cargo test -p functor-runtime-desktop --test model_parts -- --ignored --nocapture
//! ```

use std::sync::Arc;

use functor_runtime_common::asset::AssetCache;
use functor_runtime_common::gpu_counters::gpu_counters;
use functor_runtime_common::math::Angle;
use functor_runtime_common::shadow::ShadowMap;
use functor_runtime_common::{
    render_frame_with_projection, Camera, DebugRenderMode, Frame, FrameTime, InstanceData,
    InstanceList, Light, MaterialDescription, ModelDescription, ModelHandle, Scene3D, SceneContext,
    SceneObject, Viewport,
};
use glow::HasContext;

#[path = "../../functor-runtime-common/src/model/test_glb.rs"]
mod test_glb;
use test_glb::{glb, TestNode};

const EYE: u32 = 160;

fn main() {
    if !std::env::args().any(|arg| arg == "--ignored" || arg == "--include-ignored") {
        println!("model_parts: skipped (needs a GL display; pass `-- --ignored` to run)");
        return;
    }
    let mut harness = Harness::new();
    let out = std::env::var("MODEL_PARTS_OUT").ok();

    for skinned in [true, false] {
        let kind = if skinned { "skinned" } else { "rigid" };
        let (full, body) = if skinned {
            ("full.glb", "body.glb")
        } else {
            ("rigid_full.glb", "rigid_body.glb")
        };
        for instanced in [false, true] {
            let path = if instanced { "instanced" } else { "ordinary" };
            let frame = |file: &str, selections: &[&[&str]]| {
                scene_frame(model(file, selections), instanced)
            };
            harness.warm(&frame(full, &[]));
            harness.warm(&frame(body, &[]));

            let (whole, whole_work) = harness.render(&frame(full, &[]));
            let (reference, reference_work) = harness.render(&frame(body, &[]));
            assert!(
                whole != reference,
                "{kind}/{path}: the hat (or its shadow) never showed"
            );

            let (selected, selected_work) = harness.render(&frame(full, &[&["Body"]]));
            assert!(
                selected == reference,
                "{kind}/{path}: hiding Hat left colour or shadow behind"
            );
            if let Some(dir) = &out {
                harness.save(&whole, &format!("{dir}/{kind}-{path}-whole.png"));
                harness.save(&selected, &format!("{dir}/{kind}-{path}-body.png"));
            }

            // Nested selections intersect; an unknown name matches nothing.
            let (nested, _) = harness.render(&frame(full, &[&["Body", "Hat"], &["Body"]]));
            assert!(nested == reference, "{kind}/{path}: nested selections");
            let (unknown, _) = harness.render(&frame(full, &[&["Body", "Nope"]]));
            assert!(unknown == reference, "{kind}/{path}: unknown part name");

            if skinned {
                // Shadow + 2 eyes per primitive: Body has 2, Hat 1.
                assert_eq!(whole_work.draws, 3 * 3, "{whole_work:?}");
                assert_eq!(selected_work.draws, reference_work.draws);
                assert_eq!(selected_work.draws, 3 * 2, "{selected_work:?}");
                assert_eq!(whole_work.pose_evals, selected_work.pose_evals);
                assert_eq!(selected_work.pose_evals, 1, "one pose per node per frame");
            }
            println!(
                "test {kind}_{path}_hidden_part_draws_in_no_pass ... ok ({} -> {} skinned draws)",
                whole_work.draws, selected_work.draws
            );
        }
    }
}

/// Body (two primitives: a torso and a head) and a hat above it.
fn fixture(with_hat: bool, skinned: bool) -> Vec<u8> {
    let body = TestNode::named("Body").boxes(&[
        ([-0.3, 0.0, -0.2], [0.3, 1.0, 0.2]),
        ([-0.15, 1.05, -0.15], [0.15, 1.35, 0.15]),
    ]);
    let hat = TestNode::named("Hat").boxes(&[([-0.35, 1.45, -0.35], [0.35, 1.6, 0.35])]);
    let nodes = if with_hat {
        vec![body, hat]
    } else {
        vec![body]
    };
    glb(&nodes, skinned)
}

fn model(file: &str, selections: &[&[&str]]) -> Scene3D {
    Scene3D::model(ModelDescription {
        handle: ModelHandle::File(file.to_string()),
        overrides: vec![],
        animation: None,
        while_pending: vec![],
        parts: selections
            .iter()
            .map(|names| names.iter().map(|n| n.to_string()).collect())
            .collect(),
    })
}

/// `model` (or three instanced copies of it) on a lit floor under a low sun.
fn scene_frame(model: Scene3D, instanced: bool) -> Frame {
    let model = if instanced {
        let instances: Vec<InstanceData> = [-1.0, 0.0, 1.0]
            .iter()
            .map(|x| InstanceData::at([*x, 0.0, 0.0]))
            .collect();
        Scene3D::instanced(model, InstanceList::from(instances))
    } else {
        model
    };
    let floor = Scene3D {
        obj: SceneObject::Material(
            MaterialDescription::Lit {
                color: cgmath::vec4(0.8, 0.8, 0.8, 1.0),
                texture: None,
                normal_map: None,
            },
            vec![Scene3D::cube().scale_x(8.0).scale_y(0.02).scale_z(8.0)],
        ),
        xform: cgmath::Matrix4::from_scale(1.0),
    };
    let camera = Camera::look_at(
        [0.0, 3.0, -4.0],
        [0.0, 0.5, 0.5],
        [0.0, 1.0, 0.0],
        Angle::from_degrees(60.0),
    );
    let mut frame = Frame::new(
        camera,
        Scene3D {
            obj: SceneObject::Group(vec![model, floor]),
            xform: cgmath::Matrix4::from_scale(1.0),
        },
    );
    frame.lights = vec![
        Light::ambient(0.2, 0.2, 0.2),
        Light::Directional {
            direction: [0.8, -0.6, -0.5],
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            casts_shadows: true,
            baked: false,
        },
    ];
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
            .create_window(EYE, EYE, "model-parts-test", glfw::WindowMode::Windowed)
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

        let asset_cache = Arc::new(AssetCache::new());
        for (file, with_hat, skinned) in [
            ("full.glb", true, true),
            ("body.glb", false, true),
            ("rigid_full.glb", true, false),
            ("rigid_body.glb", false, false),
        ] {
            asset_cache.replace_uploaded(file, fixture(with_hat, skinned));
        }

        let shadow_map = ShadowMap::new(&gl, 512);
        Harness {
            gl,
            fbo,
            asset_cache,
            scene_context: SceneContext::new(),
            shadow_map,
            frame_id: 0,
            _window: window,
            _glfw: glfw,
        }
    }

    /// Render until the model has streamed in: the image is stable and not
    /// the empty scene.
    fn warm(&mut self, frame: &Frame) {
        let empty = scene_frame(
            Scene3D {
                obj: SceneObject::Group(vec![]),
                xform: cgmath::Matrix4::from_scale(1.0),
            },
            false,
        );
        let (blank, _) = self.render(&empty);
        let mut previous = self.render(frame).0;
        for _ in 0..500 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            let (pixels, _) = self.render(frame);
            if pixels == previous && pixels != blank {
                return;
            }
            previous = pixels;
        }
        panic!("fixture never loaded");
    }

    /// Render both eyes as one frame id; their pixels side by side, and the
    /// frame's skinned work.
    fn render(
        &mut self,
        frame: &Frame,
    ) -> (Vec<u8>, functor_runtime_common::gpu_counters::SkinnedWork) {
        self.frame_id += 1;
        let gl = &self.gl;
        let (left, right) = frame.camera.stereo_eyes(0.064);
        for (eye, camera) in [left, right].iter().enumerate() {
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
        let work = gpu_counters().frame_skinned();
        let mut pixels = vec![0u8; (2 * EYE * EYE * 4) as usize];
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.read_pixels(
                0,
                0,
                (2 * EYE) as i32,
                EYE as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        (pixels, work)
    }

    /// Write a read-back (bottom-up) image as a PNG.
    fn save(&self, pixels: &[u8], path: &str) {
        let png =
            functor_runtime_common::frame_capture::encode_bottom_up_rgba_png(2 * EYE, EYE, pixels)
                .expect("encode png");
        std::fs::write(path, png).expect("save png");
    }
}
