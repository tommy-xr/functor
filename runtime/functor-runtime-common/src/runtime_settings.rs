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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shape_is_pinned() {
        let config = GameConfig {
            xr: XrConfig {
                refresh_rate: Some(90.0),
            },
        };
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::json!({ "xr": { "refresh_rate": 90.0 } })
        );
        let settings = GameSettings {
            xr: XrSettings {
                foveation: Some(FoveationLevel::High),
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
    }
}
