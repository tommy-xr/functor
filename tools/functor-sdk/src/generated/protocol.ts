// Generated from the Rust debug-protocol types by ts-rs. Do not edit.
// Regenerate: UPDATE_TS=1 cargo test -p functor_runtime_common --features ts ts_bindings

/**
 * A device battery reading, as `GET /state` reports it. A field the device
 * does not report is `null`.
 */
export type BatteryState = { level_pct: number | null, 
/**
 * `charging`, `discharging`, `not charging`, `full`, or `unknown`.
 */
status: "charging" | "discharging" | "not charging" | "full" | "unknown", 
/**
 * Connected to any charger (AC, USB, wireless, or dock).
 */
plugged: boolean, 
/**
 * Instantaneous battery current: negative = discharging, positive =
 * charging. On Quest 3 (µA from the device): about -2.6 A under a 120 Hz
 * scene, 0 when full on AC and asleep.
 */
current_now_ma: number | null, 
/**
 * The device's averaged current, same sign; Quest 3 doesn't report it.
 */
current_avg_ma: number | null, 
/**
 * Remaining charge, µAh.
 */
charge_uah: number | null, voltage_mv: number | null, temp_c: number | null, 
/**
 * Device wall clock (Unix epoch ms) when this reading was taken.
 */
sampled_ms: number, };

/**
 * A camera description produced by the game's `draw3d`. Stores plain scalars
 * (so it serializes cleanly across the wasm boundary); the runtime turns it
 * into view/projection matrices at render time via `view_matrix` /
 * `projection_matrix`.
 */
export type Camera = { eye: [number, number, number], target: [number, number, number], up: [number, number, number], 
/**
 * Vertical field of view, in radians.
 */
fov_radians: number, near: number, far: number, };

/**
 * One frame's `Scene.cells` portal culling, summed over its eye and
 * render-target passes: portal walks, the cells they reached, and
 * `Scene.inCells` subtrees hidden.
 */
export type CellsCounts = { walks: number, visible: number, hidden: number, };

/**
 * One frame's frustum culling, summed over every pass (shadow, render
 * targets, each eye): draws tested against the pass's frustum, and those
 * skipped. A hardware-instanced node counts once.
 */
export type CullCounts = { tested: number, culled: number, };

/**
 * A headset display's refresh-rate state, in Hz, as `GET /state` reports it.
 */
export type DisplayRefreshRate = { 
/**
 * The rate the display is running at, as last confirmed by the runtime.
 */
current: number, 
/**
 * The rate this session last asked for; `None` before any request.
 */
requested: number | null, 
/**
 * The rates the display offers.
 */
available: Array<number>, };

/**
 * Where a shell's environment depth comes from, as `GET /state.xr.depth`
 * reports it.
 */
export type EnvironmentDepthMode = "device" | "waiting" | "emulated" | "unavailable";

/**
 * `GET /state.xr.depth` (debug protocol v25): present when the game's
 * `config` requests `Xr.scene`.
 */
export type EnvironmentDepthState = { mode: EnvironmentDepthMode, 
/**
 * The depth image's size in texels; 0 until one exists.
 */
width: number, height: number, 
/**
 * Depth images acquired (device) or stand-in depth passes rendered
 * (emulated) since launch.
 */
frames: number, warning?: string, };

/**
 * Fixed foveated rendering level (`Xr.Off` … `Xr.High`): how coarsely the
 * eye-buffer periphery is shaded. Maps 1:1 onto `XR_FB_foveation`'s levels.
 */
export type FoveationLevel = "off" | "low" | "medium" | "high";

/**
 * `GET /state.xr.mesh` (debug protocol v25): the room fused from the
 * environment depth (`Frame.withFusedRoom`).
 */
export type FusedMeshState = { 
/**
 * Chunks with surface (each 8 voxels on a side).
 */
chunks: number, triangles: number, 
/**
 * Depth frames fused since `Xr.scene` turned on.
 */
fused_frames: number, 
/**
 * Voxel edge, metres.
 */
voxel_m: number, 
/**
 * Chunk colliders of a `Physics.fusedRoom` body (0 without one).
 */
colliders: number, 
/**
 * The floor height those colliders are snapped to, metres; absent until
 * the floor has been seen.
 */
floor_m?: number, };

/**
 * The launch-time `config` value.
 */
export type GameConfig = { xr: XrConfig, };

/**
 * The per-frame `settings(model)` value.
 */
export type GameSettings = { xr: XrSettings, };

/**
 * Target-neutral state for the primary connected gamepad, aligned to the
 * standard mapping GLFW (`glfwGetGamepadState`) and the Web Gamepad API
 * share.
 *
 * Face buttons are POSITIONAL (`south` is the bottom face button — A on
 * Xbox, Cross on PlayStation, B on Nintendo), because letter names swap
 * between vendors. Sticks are `-1..1` with **up-positive Y**, matching the
 * XR thumbstick convention — a shell that polls a pad must negate GLFW's
 * and the browser's down-positive axes at its boundary. Triggers are
 * `0..1`. Values are raw (no deadzone shaping); games apply their own.
 *
 * The desktop windowed shell polls the first standard-mapping pad via GLFW
 * (`desktop_gamepad` in `functor-runtime-desktop`), with debug injection
 * (`POST /input` `{"type":"gamepad",…}`) winning over the poll; the web
 * shell polls the browser Gamepad API each frame (`web_gamepad` in
 * `functor-runtime-web`). Presence is sticky on both: while input is gated
 * (window/document unfocused, clock pinned) a connected pad reads
 * rest-level controls rather than vanishing. Headless has no GLFW instance,
 * so there the domain exists only while injected.
 *
 * Levels only, no edge sets: pads are polled, so a press always spans at
 * least one render frame and shows up in at least one sample — games detect
 * edges against their model, exactly as XR games do.
 *
 * Deserialization defaults every missing field, so a debug-injected sample
 * (`POST /input` `{"type":"gamepad",…}`) can name only the controls it
 * drives.
 */
export type GamepadSnapshot = { left_stick: [number, number], right_stick: [number, number], left_trigger: number, right_trigger: number, south: boolean, east: boolean, west: boolean, north: boolean, left_bumper: boolean, right_bumper: boolean, left_stick_pressed: boolean, right_stick_pressed: boolean, dpad_up: boolean, dpad_down: boolean, dpad_left: boolean, dpad_right: boolean, start: boolean, select: boolean, };

/**
 * An event injected by `POST /input`.
 */
export type InputCommand = { "type": "key", key: string, down: boolean, } | { "type": "mouse_move", x: number, y: number, } | { "type": "mouse_wheel", delta: number, } | { "type": "mouse_button", button: string, down: boolean, } | { "type": "ui_event", slot: number, kind: UiEventKind, } | { "type": "webview_event", slot: number, kind: UiEventKind, } | { "type": "xr" } & XrInputSnapshot | { "type": "xr_clear" } | { "type": "gamepad" } & GamepadSnapshot | { "type": "gamepad_clear" } | { "type": "touch", phase: TouchPhase, id: number, x: number, y: number, };

/**
 * Runtime-owned input sampled for one fixed simulation step.
 *
 * Keyboard and mouse retain their event entry points, while this plain-data
 * snapshot exposes both levels and de-duplicated transitions through the
 * extensible shell → producer seam. XR, gamepad, and touch are typed device
 * domains; further devices add sibling fields without turning device
 * capabilities into stringly-typed maps or adding target-specific producer
 * methods.
 */
export type InputSnapshot = { 
/**
 * Keys currently held, in canonical discriminant order.
 */
held_keys: Array<Key>, 
/**
 * Keys whose level changed from up to down since the previous simulation
 * step, in canonical discriminant order.
 */
pressed_keys: Array<Key>, 
/**
 * Keys whose level changed from down to up since the previous simulation
 * step, in canonical discriminant order.
 */
released_keys: Array<Key>, 
/**
 * Last known mouse position and the logical surface extent sharing its
 * coordinate space.
 */
mouse: MouseSnapshot, 
/**
 * Live XR tracking/controller state when the target supplies it.
 *
 * Omitted rather than serialized as `null` on non-XR targets, preserving
 * the existing desktop `/state` wire shape.
 */
xr?: XrInputSnapshot, 
/**
 * Live state of the primary connected gamepad when the target supplies
 * one. `None` means no pad is connected — a capability signal, so a game
 * can key control hints off it.
 *
 * Omitted rather than serialized as `null` when absent, like `xr`.
 */
gamepad?: GamepadSnapshot, 
/**
 * Live touch contacts when the target supplies them. `Some` signals
 * touch CAPABILITY (a touch surface exists), so a game can decide to
 * show touch UI; an idle touchscreen is `Some` with empty lists, and
 * `None` means no touch surface at all. The web shell declares
 * capability up front on touch-capable devices (`maxTouchPoints > 0`)
 * and folds the page's touch events through the shared reducer; desktop
 * has no touch hardware source, so there the domain exists only while
 * debug-injected.
 *
 * Omitted rather than serialized as `null` when absent, like `xr`.
 */
touch?: TouchSnapshot, };

/**
 * One frame's `Scene.instanced` instance-buffer uploads.
 */
export type InstanceUploads = { count: number, bytes: number, };

export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]: JsonValue } | null;

/**
 * Canonical key identifier shared across the F# <-> Rust boundary and all
 * runtimes (desktop GLFW, web). Producers (e.g. the desktop runtime's
 * `glfw::Key` mapping in `functor-runtime-desktop`) translate their platform
 * key into this enum and pass its `as i32` discriminant across the dylib/wasm
 * boundary. The F# `Input.Key` DU mirrors these discriminants in
 * `Input.ofKeyCode` — keep the two in sync when adding keys.
 */
export type Key = "Unknown" | "A" | "B" | "C" | "D" | "E" | "F" | "G" | "H" | "I" | "J" | "K" | "L" | "M" | "N" | "O" | "P" | "Q" | "R" | "S" | "T" | "U" | "V" | "W" | "X" | "Y" | "Z" | "Up" | "Down" | "Left" | "Right" | "Space" | "Enter" | "Escape" | "Num0" | "Num1" | "Num2" | "Num3" | "Num4" | "Num5" | "Num6" | "Num7" | "Num8" | "Num9";

/**
 * A fixed set of modeled mouse buttons.
 *
 * A record rather than a `Vec<MouseButton>`: buttons are a tiny fixed set,
 * and a game asks a named question (`snapshot.mouse.buttons.left` or
 * `snapshot.mouse.pressed.left`) rather than iterating them. Further buttons
 * (back/forward) can join as sibling fields.
 */
export type MouseButtons = { left: boolean, right: boolean, middle: boolean, };

/**
 * Last known mouse position in logical shell coordinates, the matching
 * surface extent, held buttons, and this step's button transitions.
 *
 * Desktop supplies GLFW window points and web supplies CSS pixels. Keeping
 * the extent beside the position makes pointer mapping independent of
 * framebuffer/Retina scale.
 */
export type MouseSnapshot = { x: number, y: number, 
/**
 * Logical width in the same coordinate space as [`Self::x`].
 */
surface_width: number, 
/**
 * Logical height in the same coordinate space as [`Self::y`].
 */
surface_height: number, 
/**
 * Buttons held right now — the level complement of the `mouseButton`
 * edges. Defaulted on the wire so an older `/state` capture (and a
 * hand-written debug injection) still deserializes.
 */
buttons: MouseButtons, 
/**
 * Buttons whose level changed from up to down since the previous
 * simulation step.
 */
pressed: MouseButtons, 
/**
 * Buttons whose level changed from down to up since the previous
 * simulation step.
 */
released: MouseButtons, };

/**
 * How a shell is showing the real world behind `Frame.withPassthrough`
 * frames, as `GET /state.passthrough` reports it.
 */
export type PassthroughMode = "composited" | "emulated" | "paused" | "unavailable";

/**
 * `GET /state.passthrough` (debug protocol v20): present when the game's
 * `config` requests passthrough.
 */
export type PassthroughState = { mode: PassthroughMode, 
/**
 * Why the feed is unavailable, when it is.
 */
warning?: string, };

/**
 * Snapshot returned by `GET /state`.
 *
 * `viewport`, `input`, and `model` retain the desktop wire shape. `views` is
 * the target-neutral representation of mono or stereo output; `viewport` is
 * retained as the primary/legacy output extent for SDK compatibility.
 */
export type RuntimeState = { frame: number, tts: number, 
/**
 * Clock steps queued by `POST /time` `advance` and not yet run. Zero means
 * every requested step has been simulated, which is how a harness knows a
 * batched advance has fully landed without guessing a frame target.
 */
pending_steps: number, 
/**
 * How many times the game's model has been REPLACED by game logic since
 * the program loaded — every `tick`/`input`/`update` return and every
 * effect or network fold, counted at the producer's single model
 * assignment.
 *
 * This is the version label for a networked model, and `frame` is not:
 * pausing pins the CLOCK, not the transport, so a paused session keeps
 * folding inbound messages through `update` while `frame` stands still.
 * A driver that wants "did anything land since my snapshot" compares this.
 *
 * It counts replacements BY GAME LOGIC. Replacing the model from OUTSIDE
 * the game deliberately does not count: a hot reload (which rebinds it),
 * a whole-project load, a `/rewind` or a timeline seek. Those are things
 * the driver itself just performed, and each hands back fresh state to
 * re-baseline from — a counter that moved for them too could not tell
 * "the network changed my model" from "I rewound it". `default` because a
 * pre-v10 runtime omits it.
 */
model_revision: number, 
/**
 * Inbound network events the shell has accepted from its transport and
 * not yet delivered into the game
 * ([`crate::net::inbound_pending`]) — connection events and completed
 * HTTP responses. Zero is the quiescence signal a harness waits on before
 * snapshotting a baseline; it cannot see a packet still on the wire, so it
 * is a lower bound on outstanding network work. `default` because a
 * pre-v10 runtime omits it.
 */
pending_net: number, viewport: RuntimeViewport, views: Array<RuntimeView>, 
/**
 * The structured JSON view of the model
 * ([`crate::protocol::GameProducer::state_json`]) — the default thing to
 * read: parseable, total, lossy (callables/host values are sigil
 * placeholders). `Null` for producers without a structured model.
 * `default` because pre-v4 payloads carry Debug TEXT under this key —
 * version-gate before reading (see [`DEBUG_PROTOCOL_VERSION`]).
 */
model: JsonValue, 
/**
 * The Rust-`Debug` pretty-print of the model — the human/eyeball view,
 * strictly more faithful where `model` is lossy (full depth,
 * construction order, closure params). Opaque text; don't parse it.
 */
model_debug: string, input: InputSnapshot, 
/**
 * Where the game logic runs (`inline` on the render thread, or
 * `pipelined` on its own sim thread) and that thread's scheduling class
 * (v16). Omitted by runtimes that don't report it.
 */
sim_thread?: string, 
/**
 * The headset display's refresh rate (v15). `None` — and omitted from the
 * JSON — on runtimes that do not control one, such as desktop.
 */
display_refresh_rate?: DisplayRefreshRate, 
/**
 * The last rendered frame's `Scene.instanced` instance-buffer uploads
 * (0 on a headless runtime, which renders nothing). `default` because a
 * pre-v17 runtime omits it.
 */
instance_uploads: InstanceUploads, 
/**
 * The game's launch-time `config` (v18), fixed at load. Omitted when the
 * game defines none.
 */
config?: GameConfig, 
/**
 * The game's latest `settings(model)` (v18), which a headset runtime
 * applies on change. Omitted when the game defines no `settings` hook.
 */
settings?: GameSettings, 
/**
 * The last rendered frame's terrain draws (0 on a headless runtime).
 * `default` because a pre-v19 runtime omits it.
 */
terrain: TerrainDraws, 
/**
 * How the shell shows the real world behind passthrough frames (v20).
 * Omitted unless the game's `config` requests passthrough.
 */
passthrough?: PassthroughState, 
/**
 * The device battery (v21), sampled at most once per second. Omitted on
 * runtimes without one, such as desktop.
 */
battery?: BatteryState, 
/**
 * The last rendered frame's skinned-model work (0 on a headless
 * runtime). `default` because a pre-v22 runtime omits it.
 */
skinned: SkinnedWork, 
/**
 * The last rendered frame's frustum culling (0 on a headless runtime).
 * `default` because a runtime that doesn't report it omits it.
 */
cull: CullCounts, 
/**
 * The last rendered frame's portal culling (0 on a headless runtime).
 * `default` because a runtime that doesn't report it omits it.
 */
cells: CellsCounts, 
/**
 * Mean stage cost over the shell's last completed stats window (v22).
 * Omitted until a window completes, and by runtimes that render nothing.
 */
stages?: StageTimings, 
/**
 * Which runtime build is answering (v23): `pr | <sha7>` for a CI-built
 * Quest APK, `dev` for a local one. Omitted by runtimes that don't report it.
 */
build?: string, 
/**
 * The headset's scene understanding (v25): `depth`, its environment
 * depth. Omitted unless the game's `config` requests `Xr.scene`.
 */
xr?: XrState, };

/**
 * One rendered view. Desktop reports one `main` view; stereo XR reports one
 * entry per eye. `name` is descriptive rather than a closed enum so future
 * runtimes can add views without revising the protocol.
 */
export type RuntimeView = { name: string, viewport: RuntimeViewport, };

/**
 * A pixel rectangle in the runtime's output surface.
 */
export type RuntimeViewport = { width: number, height: number, };

/**
 * One frame's skinned-model work, summed over every pass (shadow and each
 * eye): mesh draw calls (an instanced draw counts once), and pose
 * evaluations (one per skinned model per pass).
 */
export type SkinnedWork = { draws: number, pose_evals: number, };

/**
 * Mean per-frame stage cost over a shell's last completed [`StageWindow`]
 * (`frames` rendered frames): the sim stage, the render stage, the render
 * side's wait on the sim, and the wall time between rendered frames. Quest's
 * render stage includes swapchain waits and frame submission; desktop's
 * excludes the buffer swap.
 */
export type StageTimings = { frames: number, sim_ms: number, render_ms: number, sim_wait_ms: number, frame_ms: number, };

/**
 * One frame's terrain draws, summed over every draw of the frame (a stereo
 * frame draws the shared patch set once per eye, so it counts twice).
 * `vertices` is patches × the patch mesh's vertex count (grid + skirt).
 */
export type TerrainDraws = { patches: number, vertices: number, };

/**
 * One touch contact transition at the shell boundary.
 */
export type TouchPhase = "begin" | "move" | "end" | "cancel";

/**
 * One touch contact in the same top-left-origin logical coordinate space as
 * [`MouseSnapshot`] (window points natively, CSS pixels on web; fractional
 * `f32`, unlike the mouse's integer point). `id` is a small shell-assigned
 * ordinal stable for the contact's lifetime — shells remap the platform's
 * arbitrary identifiers.
 */
export type TouchPoint = { id: number, x: number, y: number, };

/**
 * Touch levels and this step's de-duplicated transitions.
 *
 * `touches` are the active contacts at their current positions — the level
 * complement of the edge lists, exactly the keyboard's
 * `held`/`pressed`/`released` contract: `pressed`/`released` survive render
 * frames with no simulation step, are consumed by the first catch-up step,
 * and are empty on later substeps. A quick tap can appear in both edge
 * lists while `touches` no longer carries the contact. A cancelled contact
 * (the platform stole the gesture) reports through `released` — a touch
 * never leaks held.
 */
export type TouchSnapshot = { touches: Array<TouchPoint>, pressed: Array<TouchPoint>, released: Array<TouchPoint>, };

/**
 * A pose reported by a tracking system, in its right-handed local space.
 *
 * The orientation is a unit quaternion in `[x, y, z, w]` order. Tracking
 * local `+X` is right, `+Y` is up, and `-Z` is forward, matching OpenXR.
 *
 * Deserialization fills missing fields from [`TrackingPose::IDENTITY`], so a
 * scripted/injected pose can name only the part it cares about.
 */
export type TrackingPose = { position: [number, number, number], orientation: [number, number, number, number], };

/**
 * What happened to the widget. `Clicked` pairs with a verbatim-msg handler
 * (a button); the payload-carrying kinds pair with a tagger applied to the
 * new value (a slider / text input).
 */
export type UiEventKind = "Clicked" | { "SliderChanged": number } | { "TextChanged": string };

/**
 * `config`'s XR knobs, read before the XR session starts rendering the game.
 */
export type XrConfig = { 
/**
 * Display refresh rate to request, in Hz. `None` = the shell's default.
 */
refresh_rate?: number, 
/**
 * `Xr.passthrough(true)`: the game may show the real world — the headset
 * prepares its camera feed (`XR_FB_passthrough`) for this run.
 */
passthrough?: boolean, 
/**
 * `Xr.scene(true)`: the game reads the room's surfaces — the headset
 * starts its environment depth (`XR_META_environment_depth`) for this
 * run; elsewhere the frame's stand-in room stands in for it.
 */
scene?: boolean, };

/**
 * Target-neutral state for one tracked XR controller.
 *
 * `active` reports whether the runtime currently has an input source for the
 * hand. Each pose is independently optional because OpenXR may have buttons
 * while positional tracking is temporarily invalid.
 */
export type XrControllerSnapshot = { active: boolean, grip: TrackingPose | null, aim: TrackingPose | null, trigger: number, squeeze: number, thumbstick: [number, number], primary_pressed: boolean, secondary_pressed: boolean, thumbstick_pressed: boolean, menu_pressed: boolean, };

/**
 * One frame of XR input in the tracking rig's local coordinates.
 *
 * Poses are relative to the center-eye reference captured when the authored
 * camera rig is established: +X right, +Y up, -Z forward. Keeping them
 * rig-local lets a pure game map them through its current authored camera
 * without a one-frame mismatch when locomotion moves that camera.
 *
 * Deserialization defaults every missing field (no head pose, both hands
 * inactive), so a debug-injected sample (`POST /input` `{"type":"xr",…}`) can
 * name only the hand and controls it drives.
 */
export type XrInputSnapshot = { 
/**
 * Center-eye pose relative to the rig reference.
 */
head: TrackingPose | null, left: XrControllerSnapshot, right: XrControllerSnapshot, };

/**
 * `settings`' XR knobs, applied live on change.
 */
export type XrSettings = { 
/**
 * A pinned foveation level. `None` = the shell's default (dynamic).
 */
foveation?: FoveationLevel, 
/**
 * `Xr.passthroughEnabled(on)`: run or pause the camera feed `config`
 * prepared. `None` = on whenever `config` requested passthrough.
 */
passthrough?: boolean, };

/**
 * `GET /state.xr`: headset scene state, omitted while it has nothing to say.
 */
export type XrState = { depth: EnvironmentDepthState, mesh: FusedMeshState, };
