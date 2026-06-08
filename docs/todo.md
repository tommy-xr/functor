# NOTES

- Does CI run tests?

## Games to build

- 3D: Simple Terrain (synthwave vibes)
- 3D: Multiplayer Asteroids
- 3D: Simple FPS (Weapons / recoil) Shooting Range
- 3D: Multiplayer FPS (Weapons / recoil)

Goals?

- Address deficiencies in previous approach:
  - Rapid iteration!!!
  - Have in-built shadows + lighting
  - Have in-built cubemap (emissive lighting + reflection)
  - Have in-built hands models

## Presentation Brainstorming

- Do a 3d-focused presentation - showcasing the architecture

## VR Dev experience brainstorming

- Table-top / light-table-esque experience?

## CLI Tool Brainstorming

- `functor init`
  - `functor init 3d`
  - `functor init fps`

MVP:

- 1p Asteroids game
  - [ ] Model loading
  - [ ] Lighting
  - [ ] Sprites
  - [ ] Sound effect
  - [ ] Input
- Battle royale asteroids
  - [ ] 1p bots

## Next

- ## Materials

  - Sub.ofMsg
    - dispatch message to game
    - Create a 'poll' method for Subs
    - Just fires once
  - Sub.renderFrame
    - Always returns a message via poll
  - Sub.timer
  - Sub.Net.httpRequest
  - Sub.Net.webSocket

  - Add test project

  - Start processing effects

    - Implement effect queue - going to need a custom type that codes debug
      - EffectQueue
        - length
        - enqueue
        - dequeue
      - Implement debug

  - EffectRunner::poll
  - EffectRunner::get_completed_result(): Array<T>

  - Are we going to have to re-think our approach to async IO on desktop runtime?

    - Maybe we create a tokio runtime in a separate thread
    - Use the receiver / channel to push state

  - Add timeout effect to implement futures
    `Effect.timeout(~duration, msg)`

  `Assets.Effect.load(texturePipeline, "my_texture.png"): AssetHandle<Texture>`
  `Assets.Effect.load(animationPipeline, "my_texture.png"): AssetHandle<Animations>`

  - [x] `init` function (startup effect via `GameBuilder.init`, seeded into the effect queue at construction)

  - Get update function working

  - Create SkinnedModel material - can it be just vertex shader? Probably

    - Render weights in rgba

  - Hopefully get animating!
  - API Design:

    - Effect to load model
    - Actual model object
    - animatedModel bones
    - Debug.skeleton bones

  - Assets -> Add options to the pipeline

  - Add transform override

    - Include name in model info
    - Add Mesh Selector by name
    - Test with Glock model

  - example assets folder

  - quad

  - Prototype input API

    <!-- - System.Input.render(world) -> InputState
    - System.Input.Effect.rumble(amount, inputDevice)
    - System.Input.Sub.onKeyDown()
    - System.Input.Sub.onKeyUp() -->

    - System.Input.update(input: InputState, world) -> world
    - input.keyboard
    - input.mouse
    - input.vr.head
    - input.vr.controllers
    <!-- - System.Input.event(event, world) -> msg -->

  Asset Brainstorming:

  - When bringing in an asset, automatically generate types (joint names, animations, etc)
  - Have an opionated structure on asset loading?

    - Content that is immediately available vs not

  - Animation

    - Load skinning data
      - Refresh on inverse bind pose - how does that work again?
      - Create new vertex format
      - Add joints and weights
      - Render bones
    - Bring over shader to handle skinning
      - Hard code some transforms - can we play with specific tweaks?
      - How to test in pure rust land?
    - Create animation pipeline
      - Info here: https://gabormakesgames.com/blog_animation_skinspace.html#:~:text=The%20inverse%20bind%20pose%20matrix%20maps%20a%20vertex%20from%20model,the%20bone%20it%20belongs%20to.
      - Load animations from gltf
      - Shark animating
        - Test out particular frames of the animation
    - Test out hand pose data

  - Image verification test, to verify hasn't changed?

    - Command line option for test (--test testName)
      - If test mode:
        - Use software rendering
      - Set up actions for the test
    - Implement way to check if assets are pending being loaded
    - Wait for all assets to be loaded
    - Render single frame
    - Run with output

  - Interface for input?
  - Manually create module?

    - rust
    - fs

  - Chore:

    - Load materials ahead of time for scene context

      - Happens in two places:
        - The place where we initialize `basic_material` as a default for models
        - The place where we actually load supplied materials
      - We have to rethink how we actually load these, because they take arguments
      - We may need to move away from the <dyn Material> and have a separate way to set parameters

    - F# API
      - So we can iterate quickly!

  - Steam VR models

    - Does animation work for these too?

  - Preliminary mesh improvements

    - Update primitives
    - Add quad
    - Add plane
    - Add heightmap

  - 'hello' example

    - Synthwave ground
    - Model loading:
    - Dynamic mesh:
    - Quad with texture
    - Skybox

  - Pass assetCache to scene
  - Get crate example working via F#
  - Color primitive

  ### Material Definition

  - Texture2D.color in Scene3D -> test rendering the shapes diff colors via the F# API
  - Texture2D.dynamic -> hook up the raw stuff from above

  - Add F# API for texture
    Asset.Texture.raw({ width, height, format, bytes})
    Asset.Texture.path("jjk")

  - Use Asset.Texture.path("proto") as a prototype

  ### Material Definition

  - Scene3D:

    - MaterialDefinition
      - basic
        - diffuse
        - normal
    - TextureDefinition
      - path('')
      - raw(width, height, format, '');

  ### Hot reloading of assets

  - How to manage, along with time travel?

  - Runtime:

    - Material
    - Texture

  - Load asset
  - Add asset loading for textures
    - Create loader trait that takes a path &str and is async and returns an array of bytes
  - Add texture material
  - Implement loading solution that works on both platforms

- ## Transform / Visuals

  - Add 'transform' as Mat4 to the Scene3D outside the type
    - 4. Add vector3 + translate + scale
    - 5. Add quaternion + rotate
  - Add plane and quad primitives

- ## VR Runtime

- ## Model Loading

  - gltf loader
  - custom loaders?

- ## Physics

- ## Live Variables

  - Set up live variable debugging
  - Temporary project

    - Use
    - Create temporary folder w/ scaffolding
      - Copy over fs/fsproj
    - Copy FS files on change (for transform)
    - `.functor` folder with some context

  - Materials
    - Add color material, where color can be specified
    - Add materials - default material is the prototype one
    - See the scene start to come to life!
  - Figure out textures

- Build commands for wasm:

  - functor cli
    - set up build.rs to build functor runtime desktop, functor runtime web
    - simplify CI
  - Add `functor develop wasm`
    - Add a special 'hot-reload' websocket
      - Push changes
      - On change, save state, reload, and rehydrate state

- ## Mesh

  - Add dynamic mesh

    - Add texture material w/ emissive

  - CLI Part 3:
    - `functor init`
      - Use rust-embed along with template folder
      - Bundle up entire template folder
      - cargo install to get in path
    - Create template
  - Lighting
    - Ambient Light
    - Multi-pass lighting
    - Point light
    - Directional light
    - Ambient fog
    - Positional fog
    - Spot light -> shadow mapping
  - Camera
  - How could we get the experience of dynamically editing a value?

    - 'Constant' function that we manually add, with a rust implementation
      - name
      - value
    - Maintain map to string any value
    - Example of similar project: https://github.com/tversteeg/const-tweaker
    - Token reloading here: https://github.com/fable-compiler/Fable/blob/76a33dc107ce2009acac3429b27999f8776597d2/src/Fable.Transforms/Rust/AST/Rust.AST.Helpers.fs#L323
    - Can we do a custom build of fable compiler?

  -

  - Middleware
    - FPSCamera
    - OrbitCamera

- Simple rendering

  - Add transform
  - Run through and render tree

- Rendering: Textures

  - Get basic primitives working from shock2quest

    - How to load for both native and web?

    - In-memory texture
      - Port over TextureTrait
        - bind0
        - bind1
      - Port over Texture
      - Port over RawTextureData
      - Port over TextureOptions
      - Port over init_from_memory2
    - Port over TextureFormat, Texture trait
    - Port over PixelFormat
    - Port over RawTextureData

    - Get textures loading

      - Do a hard-coded texture
      - Load a texture from files

    - Use a basic camera example

      - maybe this one is a good one: https://github.com/bwasty/learn-opengl-rs/blob/6357f7ca55508cd8ed9389a73207a8b954d362b4/src/_1_getting_started/_7_1_camera_circle.rs#L227

    - Game loop:
      - https://gafferongames.com/post/fix_your_timestep/

  - Webassembly

    - Loading textures: https://github.com/kettle11/LD46/blob/365613e6089e29921a36b672217c245d9980e078/src/image.rs#L27

  - Bring in basic camera
  - Move cube up and down

  - Add a mouselook primitive

  - Review rendering primitives in my old project (citadel)

    - Geometry
      - Plane
      - Cube
      - Sphere
      - Cylinder
      - HeightMap
      - Custom
    - Material
      - Color
    - Primitive

      - Mesh (Geometry, Material)
      - SkinnedMesh
      - Transform
      - Group []
      - PointLight

      - Scene3d
        - camera
        - Primitive

  - Get basic rendering primitives working
  - Bring back in this project

- Complete Pong
  - Add `Key` type -> interop with glfw key type?
  - Add input function to pong
  - Add update function to pong
- Runtime
  - Get pong running on desktop
    - Create a window using GLFW
    - Add game loop in F#
      - Call game update
      - Call game render
      - Default loading spinner
  - Get pong running on webasm
    - Render to canvas in web assembly
    - How will game loop work?
  -
  - How to interface with runtime? Can we compile and load web assembly?
