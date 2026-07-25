//! Headless terrain hydration through NetSim's per-instance asset shell.

use std::sync::Mutex;

use functor_netsim::NetSim;
use functor_runtime_common::physics::{remove_world, with_world, DEFAULT_WORLD};

mod support;
use support::{add_source_game, install_controlled_fetcher};

static TERRAIN_LOCK: Mutex<()> = Mutex::new(());

fn heightmap_png(sample: u16) -> Vec<u8> {
    let pixels = image::ImageBuffer::from_pixel(2, 2, image::Luma([sample]));
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma16(pixels)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    png.into_inner()
}

fn terrain_game_source(declarations: &str, tag: &str, remove_on_frame_three: bool) -> String {
    let bodies = if remove_on_frame_three {
        "if model == 3.0\n\
         then Physics.scene(Vec3.make(0.0, -9.81, 0.0), [])\n\
         else Physics.scene(Vec3.make(0.0, -9.81, 0.0), [terrainBody])"
    } else {
        "Physics.scene(Vec3.make(0.0, -9.81, 0.0), [terrainBody])"
    };
    let tick = if remove_on_frame_three {
        "model + 1.0"
    } else {
        "model"
    };
    format!(
        "{declarations}\n\
         let terrainBody = Physics.heightfield(Physics.tag({tag:?}), world)\n\
         let init = 0.0\n\
         let tick = (model, dt, tts) => {tick}\n\
         let physics = (model) => {bodies}\n\
         let draw = (model, tts) => Frame.create(\
           Camera.lookAt(Vec3.make(0.0, 5.0, -10.0), Vec3.make(0.0, 0.0, 0.0)), \
           Scene.group([]))\n"
    )
}

#[test]
fn physics_only_terrain_hydrates_without_a_renderer() {
    let _guard = TERRAIN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    remove_world(DEFAULT_WORLD);
    let dir = std::env::temp_dir().join(format!(
        "functor-netsim-terrain-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let heightmap = dir.join("height.png");
    let pixels = image::ImageBuffer::from_fn(2, 2, |x, y| {
        image::Luma([if x == 1 && y == 1 { u16::MAX } else { 0 }])
    });
    pixels.save(&heightmap).unwrap();
    let declarations = format!(
        "let world = Terrain.heightmap(Asset.texture({path:?}), 20.0, 20.0, 0.0, 10.0)",
        path = heightmap.to_string_lossy()
    );
    let source = terrain_game_source(&declarations, "netsim-terrain", false);
    let mut sim = NetSim::new(1);
    add_source_game(&mut sim, &dir, source);
    sim.step_n(3);

    with_world(DEFAULT_WORLD, |world| {
        assert!(
            world.body_transform("netsim-terrain").is_some(),
            "physics-only terrain should hydrate through NetSim's asset shell"
        );
    });

    remove_world(DEFAULT_WORLD);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn terrain_stand_in_and_primary_replay_at_their_original_frames() {
    let _guard = TERRAIN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    remove_world(DEFAULT_WORLD);
    let fetches = install_controlled_fetcher();

    let dir = std::env::temp_dir().join(format!(
        "functor-netsim-terrain-replay-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let primary = format!(
        "https://netsim.invalid/terrain-primary-{}.png",
        std::process::id()
    );
    let stand_in = format!(
        "https://netsim.invalid/terrain-stand-in-{}.png",
        std::process::id()
    );
    let declarations = format!(
        "let primary = Asset.texture({primary:?})\n\
         let standIn = Asset.texture({stand_in:?})\n\
         let heightmap = primary |> Asset.whilePending(standIn)\n\
         let world = Terrain.heightmap(heightmap, 20.0, 20.0, 0.0, 10.0)"
    );
    let source = terrain_game_source(&declarations, "replay-terrain", true);
    let mut sim = NetSim::new(2);
    add_source_game(&mut sim, &dir, source);
    sim.step_n(2);
    assert_eq!(fetches.lock().unwrap().len(), 2);
    let mut pending = std::mem::take(&mut *fetches.lock().unwrap());
    let stand_in_index = pending
        .iter()
        .position(|(url, _)| url == &stand_in)
        .expect("the stand-in request");
    let (_, stand_in_sender) = pending.swap_remove(stand_in_index);
    let (primary_url, primary_sender) = pending.pop().expect("the primary request");
    assert_eq!(primary_url, primary);

    let terrain_height = || {
        with_world(DEFAULT_WORLD, |world| {
            world
                .raycast([0.0, 20.0, 0.0], [0.0, -1.0, 0.0], 100.0)
                .map(|hit| hit.position[1].round() as i32)
        })
        .flatten()
    };
    stand_in_sender
        .send(Ok(heightmap_png(0)))
        .expect("the pending stand-in future still owns its receiver");
    // The stand-in publishes on frame 2, but that frame deliberately removes
    // the body. The replay anchor therefore owns no old Rapier collider even
    // though the pending terrain request retains the active stand-in surface.
    sim.step();
    let anchor = sim.frame() - 1;
    assert_eq!(terrain_height(), None);

    let mut original = Vec::new();
    sim.step();
    original.push(terrain_height());
    sim.step();
    original.push(terrain_height());

    primary_sender
        .send(Ok(heightmap_png(u16::MAX)))
        .expect("the pending primary future still owns its receiver");
    sim.step();
    original.push(terrain_height());
    for _ in 0..3 {
        sim.step();
        original.push(terrain_height());
    }
    assert_eq!(
        original,
        vec![
            Some(0),
            Some(0),
            Some(10),
            Some(10),
            Some(10),
            Some(10),
        ]
    );

    sim.seek(anchor).expect("seek to the pending heightmap");
    assert_eq!(terrain_height(), None);
    let mut replayed = Vec::new();
    for _ in 0..original.len() {
        sim.step();
        replayed.push(terrain_height());
    }
    assert_eq!(replayed, original);
    assert!(
        fetches.lock().unwrap().is_empty(),
        "replay should reuse the warm decoded heightmap"
    );

    remove_world(DEFAULT_WORLD);
    let _ = std::fs::remove_dir_all(dir);
}
