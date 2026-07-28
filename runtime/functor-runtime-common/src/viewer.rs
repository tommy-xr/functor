use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Who owns captured mouse camera control for the main viewport.
///
/// This is intentionally separate from the authored [`crate::Camera`]. `Game`
/// routes captured mouse input into the game's ordinary input hooks; future
/// detached modes will keep their camera state in the shell instead.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraControl {
    /// Keep the pointer free and render through the game-authored camera.
    #[default]
    None,
    /// Let the game own captured mouse input and derive its camera in `draw`.
    Game,
}

impl CameraControl {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Game => "game",
        }
    }
}

impl fmt::Display for CameraControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CameraControl {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "game" => Ok(Self::Game),
            other => Err(format!(
                "unknown camera control `{other}` (expected `none` or `game`)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CameraControl;

    #[test]
    fn string_and_json_spellings_are_stable() {
        for (mode, spelling) in [(CameraControl::None, "none"), (CameraControl::Game, "game")] {
            assert_eq!(mode.to_string(), spelling);
            assert_eq!(spelling.parse::<CameraControl>().unwrap(), mode);
            assert_eq!(
                serde_json::to_string(&mode).unwrap(),
                format!("\"{spelling}\"")
            );
            assert_eq!(
                serde_json::from_str::<CameraControl>(&format!("\"{spelling}\"")).unwrap(),
                mode
            );
        }
    }

    #[test]
    fn detached_modes_are_not_accepted_before_they_exist() {
        assert!("orbit".parse::<CameraControl>().is_err());
        assert!("fps".parse::<CameraControl>().is_err());
    }
}
