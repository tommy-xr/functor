//! Headless terrain hydration through NetSim's per-instance asset shell.

use functor_netsim::NetSim;
use functor_runtime_common::physics::{remove_world, with_world, DEFAULT_WORLD};
use functor_runtime_desktop::functor_lang_game::FunctorLangGame;

#[test]
#[ignore = "pulls the desktop runtime dev-dependency; run with --ignored"]
fn physics_only_terrain_hydrates_without_a_renderer() {
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
    let source = format!(
        "let world = Terrain.heightmap(Asset.texture({path:?}), 20.0, 20.0, 0.0, 10.0)\n\
         let terrainTag = Physics.tag(\"netsim-terrain\")\n\
         let terrainBody = Physics.heightfield(terrainTag, world)\n\
         let init = 0.0\n\
         let tick = (model, dt, tts) => model\n\
         let physics = (model) => Physics.scene(Vec3.make(0.0, -9.81, 0.0), \
           [terrainBody])\n\
         let draw = (model, tts) => Frame.create(\
           Camera.lookAt(Vec3.make(0.0, 5.0, -10.0), Vec3.make(0.0, 0.0, 0.0)), \
           Scene.group([]))\n",
        path = heightmap.to_string_lossy()
    );
    let game = dir.join("game.fun");
    std::fs::write(&game, source).unwrap();

    let mut sim = NetSim::new(1);
    sim.add_producer(Box::new(FunctorLangGame::create(
        game.to_str().unwrap(),
    )));
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
