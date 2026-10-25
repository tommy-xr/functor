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
#[serde(rename_all = "snake_case")]
pub enum FoveationLevel {
    Off,
    Low,
    Medium,
    High,
}

/// The launch-time `config` value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameConfig {
    #[serde(default)]
    pub xr: XrConfig,
}

/// `config`'s XR knobs, read before the XR session starts rendering the game.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XrConfig {
    /// Display refresh rate to request, in Hz. `None` = the shell's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_rate: Option<f32>,
    /// `Xr.passthrough(true)`: the game may show the real world — the headset
    /// prepares its camera feed (`XR_FB_passthrough`) for this run.
    #[serde(default, skip_serializing_if = "is_false")]
    pub passthrough: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The per-frame `settings(model)` value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameSettings {
    #[serde(default)]
    pub xr: XrSettings,
}

/// `settings`' XR knobs, applied live on change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XrSettings {
    /// A pinned foveation level. `None` = the shell's default (dynamic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foveation: Option<FoveationLevel>,
    /// `Xr.passthroughEnabled(on)`: run or pause the camera feed `config`
    /// prepared. `None` = on whenever `config` requested passthrough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passthrough: Option<bool>,
}

/// Whether the camera feed should run: `config` asked for it and the latest
/// `settings` (if any) has not paused it.
pub fn passthrough_wanted(config: Option<GameConfig>, settings: Option<GameSettings>) -> bool {
    config.is_some_and(|c| c.xr.passthrough)
        && settings.and_then(|s| s.xr.passthrough).unwrap_or(true)
}

/// How a shell is showing the real world behind `Frame.withPassthrough`
/// frames, as `GET /state.passthrough` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassthroughMode {
    /// The headset's camera feed is composited under the eye buffer.
    Composited,
    /// No headset: frames clear to the flat placeholder (desktop, web).
    Emulated,
    /// `settings` paused the feed; frames clear to the placeholder.
    Paused,
    /// The headset cannot show its camera feed; frames clear to the
    /// placeholder (see `PassthroughState::warning`).
    Unavailable,
}

/// `GET /state.passthrough` (debug protocol v20): present when the game's
/// `config` requests passthrough.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassthroughState {
    pub mode: PassthroughMode,
    /// Why the feed is unavailable, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl PassthroughState {
    /// A shell with no camera feed (desktop, web): emulated while wanted.
    pub fn emulated(config: Option<GameConfig>, settings: Option<GameSettings>) -> Option<Self> {
        config.is_some_and(|c| c.xr.passthrough).then(|| PassthroughState {
            mode: if passthrough_wanted(config, settings) {
                PassthroughMode::Emulated
            } else {
                PassthroughMode::Paused
            },
            warning: None,
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
            },
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
            },
        };
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::json!({ "xr": { "passthrough": true } })
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
                },
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
