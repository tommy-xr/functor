//! The shell options a game declares through its optional `config` and
//! `settings` entry points, as plain data the shells read.
//!
//! Split by lifetime, mirroring `init`/`tick`: [`GameConfig`] is evaluated
//! once at load (a hot reload that changes it only warns), while
//! [`GameSettings`] is `settings(model)`, re-evaluated every frame and applied
//! by the shell when the value changes. Both appear in `GET /state`.

use serde::{Deserialize, Serialize};

/// Fixed foveated rendering level (`Xr.Off` … `Xr.High`): how coarsely the
/// eye-buffer periphery is shaded. Maps 1:1 onto `XR_FB_foveation`'s levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum FoveationLevel {
    Off,
    Low,
    Medium,
    High,
}

/// The launch-time `config` value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GameConfig {
    #[serde(default)]
    pub xr: XrConfig,
    /// Identifies the `Config.external` declarations (see [`ExternalPrefix`]),
    /// so a reload that changes them warns like any other launch option.
    #[serde(skip)]
    pub external_key: u64,
}

/// `Config.external(prefix, source)`: files under `prefix` aren't the
/// project's (never pushed or bundled); `source` says where they come from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExternalPrefix {
    /// `"ss2/"`, matched against `File.bytes` paths.
    pub prefix: String,
    /// `Source.mount(name)`: this machine's mount `name`, proxied to a
    /// headset under `functor run vr`.
    pub mount: String,
    /// `Source.device(path)`: where a standalone headset finds it.
    pub device: Option<String>,
}

/// `config`'s XR knobs, read before the XR session starts rendering the game.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct XrConfig {
    /// Display refresh rate to request, in Hz. `None` = the shell's default.
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_rate: Option<f32>,
    /// `Xr.passthrough(true)`: the game may show the real world — the headset
    /// prepares its camera feed (`XR_FB_passthrough`) for this run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub passthrough: bool,
    /// `Xr.scene(true)`: the game reads the room's surfaces — the headset
    /// starts its environment depth (`XR_META_environment_depth`) for this
    /// run; elsewhere the frame's stand-in room stands in for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scene: bool,
}

/// The per-frame `settings(model)` value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct GameSettings {
    #[serde(default)]
    pub xr: XrSettings,
}

/// `settings`' XR knobs, applied live on change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct XrSettings {
    /// A pinned foveation level. `None` = the shell's default (dynamic).
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foveation: Option<FoveationLevel>,
    /// `Xr.passthroughEnabled(on)`: run or pause the camera feed `config`
    /// prepared. `None` = on whenever `config` requested passthrough.
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passthrough: Option<bool>,
}

/// Whether the game's `config` requests passthrough (`Xr.passthrough(true)`).
pub fn passthrough_requested(config: Option<GameConfig>) -> bool {
    config.is_some_and(|c| c.xr.passthrough)
}

/// Whether the game's `config` requests scene understanding (`Xr.scene(true)`).
pub fn scene_requested(config: Option<GameConfig>) -> bool {
    config.is_some_and(|c| c.xr.scene)
}

/// Whether the camera feed should run: `config` asked for it and the latest
/// `settings` (if any) has not paused it.
pub fn passthrough_wanted(config: Option<GameConfig>, settings: Option<GameSettings>) -> bool {
    passthrough_requested(config) && settings.and_then(|s| s.xr.passthrough).unwrap_or(true)
}

/// How a shell is showing the real world behind `Frame.withPassthrough`
/// frames, as `GET /state.passthrough` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum PassthroughMode {
    /// The headset's camera feed is composited under the eye buffer.
    Composited,
    /// No headset: frames show the stand-in room (desktop, web).
    Emulated,
    /// `settings` paused the feed; frames show the stand-in room.
    Paused,
    /// The headset cannot show its camera feed; frames show the stand-in
    /// room (see `PassthroughState::warning`).
    Unavailable,
}

/// `GET /state.passthrough` (debug protocol v20): present when the game's
/// `config` requests passthrough.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PassthroughState {
    pub mode: PassthroughMode,
    /// Why the feed is unavailable, when it is.
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl PassthroughState {
    /// A shell with no camera feed (desktop, web): emulated while wanted.
    pub fn emulated(config: Option<GameConfig>, settings: Option<GameSettings>) -> Option<Self> {
        passthrough_requested(config).then(|| PassthroughState {
            mode: if passthrough_wanted(config, settings) {
                PassthroughMode::Emulated
            } else {
                PassthroughMode::Paused
            },
            warning: None,
        })
    }
}

/// Where a shell's environment depth comes from, as `GET /state.xr.depth`
/// reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentDepthMode {
    /// The headset delivered a depth image this frame.
    Device,
    /// The headset's depth provider runs but has no image yet.
    Waiting,
    /// No headset depth: the frame's stand-in room is rendered as the depth.
    Emulated,
    /// The headset cannot provide depth (see `EnvironmentDepthState::warning`).
    Unavailable,
}

/// `GET /state.xr.depth` (debug protocol v25): present when the game's
/// `config` requests `Xr.scene`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct EnvironmentDepthState {
    pub mode: EnvironmentDepthMode,
    /// The depth image's size in texels; 0 until one exists.
    pub width: u32,
    pub height: u32,
    /// Depth images acquired (device) or stand-in depth passes rendered
    /// (emulated) since launch.
    pub frames: u64,
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// `GET /state.xr.mesh` (debug protocol v25): the room fused from the
/// environment depth (`Frame.withFusedRoom`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FusedMeshState {
    /// Chunks with surface (each 8 voxels on a side).
    pub chunks: usize,
    pub triangles: usize,
    /// Depth frames fused since `Xr.scene` turned on.
    pub fused_frames: u64,
    /// Voxel edge, metres.
    pub voxel_m: f32,
}

/// `GET /state.xr`: headset scene state, omitted while it has nothing to say.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct XrState {
    pub depth: EnvironmentDepthState,
    pub mesh: FusedMeshState,
}

impl XrState {
    /// A shell without depth sensing (desktop, web), while `config` requests
    /// `Xr.scene`: the stand-in room's depth so far, `(width, height, passes)`,
    /// and the room fused from it.
    pub fn emulated(
        config: Option<GameConfig>,
        (width, height, frames): (u32, u32, u64),
        mesh: FusedMeshState,
    ) -> Option<Self> {
        scene_requested(config).then_some(XrState {
            depth: EnvironmentDepthState {
                mode: EnvironmentDepthMode::Emulated,
                width,
                height,
                frames,
                warning: None,
            },
            mesh,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shape_is_pinned() {
        let config = GameConfig {
            xr: XrConfig {
                refresh_rate: Some(90.0),
                passthrough: false,
                scene: false,
            },
            external_key: 7,
        };
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::json!({ "xr": { "refresh_rate": 90.0 } })
        );
        let settings = GameSettings {
            xr: XrSettings {
                foveation: Some(FoveationLevel::High),
                passthrough: None,
            },
        };
        assert_eq!(
            serde_json::to_value(settings).unwrap(),
            serde_json::json!({ "xr": { "foveation": "high" } })
        );
        assert_eq!(
            serde_json::to_value(GameSettings::default()).unwrap(),
            serde_json::json!({ "xr": {} })
        );
        let config = GameConfig {
            xr: XrConfig {
                refresh_rate: None,
                passthrough: true,
                scene: true,
            },
            ..GameConfig::default()
        };
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::json!({ "xr": { "passthrough": true, "scene": true } })
        );
        let depth = XrState {
            depth: EnvironmentDepthState {
                mode: EnvironmentDepthMode::Device,
                width: 320,
                height: 320,
                frames: 7,
                warning: None,
            },
            mesh: FusedMeshState {
                chunks: 3,
                triangles: 120,
                fused_frames: 2,
                voxel_m: 0.08,
            },
        };
        assert_eq!(
            serde_json::to_value(depth).unwrap(),
            serde_json::json!({
                "depth": { "mode": "device", "width": 320, "height": 320, "frames": 7 },
                "mesh": { "chunks": 3, "triangles": 120, "fused_frames": 2, "voxel_m": 0.08f32 }
            })
        );
        let state = PassthroughState {
            mode: PassthroughMode::Unavailable,
            warning: Some("no feed".to_string()),
        };
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!({ "mode": "unavailable", "warning": "no feed" })
        );
    }

    #[test]
    fn passthrough_runs_when_configured_unless_settings_pause_it() {
        let config = |on| {
            Some(GameConfig {
                xr: XrConfig {
                    refresh_rate: None,
                    passthrough: on,
                    scene: false,
                },
                ..GameConfig::default()
            })
        };
        let settings = |on| {
            Some(GameSettings {
                xr: XrSettings {
                    foveation: None,
                    passthrough: on,
                },
            })
        };
        assert!(!passthrough_wanted(None, None));
        assert!(!passthrough_wanted(config(false), settings(Some(true))));
        assert!(passthrough_wanted(config(true), None));
        assert!(passthrough_wanted(config(true), settings(None)));
        assert!(!passthrough_wanted(config(true), settings(Some(false))));
        assert_eq!(PassthroughState::emulated(config(false), None), None);
        assert_eq!(
            PassthroughState::emulated(config(true), settings(Some(false))).unwrap().mode,
            PassthroughMode::Paused
        );
        assert_eq!(
            PassthroughState::emulated(config(true), None).unwrap().mode,
            PassthroughMode::Emulated
        );
    }
}
