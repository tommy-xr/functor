//! `Config` / `Settings` / `Xr`: the builders behind the optional `config`
//! value and `settings(model)` hook (see [`crate::runtime_settings`]).
//!
//! One `Xr.t` carries every headset knob; `Config.xr` and `Settings.xr` each
//! accept only the knobs their bucket can honour, so a misplaced knob is a
//! teaching error at the call — at load for `config` and the load-time
//! `settings(init)` evaluation.

use super::*;

use crate::runtime_settings::{
    ExternalPrefix, FoveationLevel, GameConfig, GameSettings, XrConfig, XrSettings,
};

/// Every headset knob, before `Config.xr` / `Settings.xr` sorts it into a
/// bucket.
#[derive(Clone, Copy, Default)]
struct XrKnobs {
    refresh_rate: Option<f32>,
    passthrough: Option<bool>,
    scene: Option<bool>,
    foveation: Option<FoveationLevel>,
    passthrough_enabled: Option<bool>,
}

struct FunctorLangXr(XrKnobs);
struct FunctorLangConfig(ConfigData);
struct FunctorLangSettings(GameSettings);
struct FunctorLangSource(SourceData);

/// A `Config.t`: the shell's launch options, plus `Config.external`'s
/// declarations, which only the CLI reads.
#[derive(Clone, Default)]
struct ConfigData {
    game: GameConfig,
    external: Vec<ExternalPrefix>,
}

/// `Source.mount(name)`, optionally `|> Source.device(path)`.
#[derive(Clone)]
struct SourceData {
    mount: String,
    device: Option<String>,
}

/// Plain data with no Functor Lang values inside, so a model may hold one
/// across a reload.
macro_rules! plain_host_data {
    ($($ty:ty => $name:literal),+ $(,)?) => {$(
        impl HostData for $ty {
            fn type_name(&self) -> &'static str {
                $name
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn is_reload_safe_snapshot(&self) -> bool {
                true
            }
        }
    )+};
}

plain_host_data!(
    FunctorLangXr => "Xr",
    FunctorLangConfig => "Config",
    FunctorLangSettings => "Settings",
    FunctorLangSource => "Source",
);

handle_arg!(
    FunctorLangXr => "Xr options (Xr.create() |> …)",
    FunctorLangConfig => "a Config (Config.create())",
    FunctorLangSettings => "a Settings (Settings.create())",
    FunctorLangSource => "a Source (Source.mount(\"name\"))",
);

impl crate::host_registry::FromArg for FoveationLevel {
    fn from_arg(value: &Value, path: &str, span: Span) -> Result<Self, RunError> {
        let level = match value {
            Value::Variant { ctor, args } if args.is_empty() => match ctor.as_ref() {
                "Xr.Off" => Some(FoveationLevel::Off),
                "Xr.Low" => Some(FoveationLevel::Low),
                "Xr.Medium" => Some(FoveationLevel::Medium),
                "Xr.High" => Some(FoveationLevel::High),
                _ => None,
            },
            _ => None,
        };
        level.ok_or_else(|| RunError {
            message: format!(
                "{path}: expected a foveation level (Xr.Off, Xr.Low, Xr.Medium, Xr.High), got {value}"
            ),
            span,
        })
    }
}

crate::host_returnable!(
    FunctorLangXr,
    FunctorLangConfig,
    FunctorLangSettings,
    FunctorLangSource
);

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn0("Xr.create", "Xr.create()", || FunctorLangXr(XrKnobs::default()));
    reg.fn2(
        "Xr.refreshRate",
        "Xr.refreshRate(hz, xr)",
        |hz: f64, xr: FunctorLangXr| {
            if hz <= 0.0 {
                return Err(format!(
                    "Xr.refreshRate: the rate must be positive Hz, got {hz}"
                ));
            }
            Ok(FunctorLangXr(XrKnobs {
                refresh_rate: Some(hz as f32),
                ..xr.0
            }))
        },
    );
    reg.fn2(
        "Xr.passthrough",
        "Xr.passthrough(on, xr)",
        |on: bool, xr: FunctorLangXr| {
            FunctorLangXr(XrKnobs {
                passthrough: Some(on),
                ..xr.0
            })
        },
    );
    reg.fn2("Xr.scene", "Xr.scene(on, xr)", |on: bool, xr: FunctorLangXr| {
        FunctorLangXr(XrKnobs {
            scene: Some(on),
            ..xr.0
        })
    });
    reg.fn2(
        "Xr.passthroughEnabled",
        "Xr.passthroughEnabled(on, xr)",
        |on: bool, xr: FunctorLangXr| {
            FunctorLangXr(XrKnobs {
                passthrough_enabled: Some(on),
                ..xr.0
            })
        },
    );
    reg.fn2(
        "Xr.foveation",
        "Xr.foveation(level, xr)",
        |level: FoveationLevel, xr: FunctorLangXr| {
            FunctorLangXr(XrKnobs {
                foveation: Some(level),
                ..xr.0
            })
        },
    );

    reg.fn0("Config.create", "Config.create()", || {
        FunctorLangConfig(ConfigData::default())
    });
    reg.fn1("Source.mount", "Source.mount(name)", |name: String| {
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            return Err(format!(
                "Source.mount: {name:?} must be a mount name (letters, digits, `_`, `-`), like \
\"ss2\" — the path it maps to is per machine, never in code"
            ));
        }
        Ok(FunctorLangSource(SourceData {
            mount: name,
            device: None,
        }))
    });
    reg.fn2(
        "Source.device",
        "Source.device(path, source)",
        |path: String, source: FunctorLangSource| {
            if !path.starts_with('/') {
                return Err(format!(
                    "Source.device: {path:?} must be an absolute path on the headset, like \
\"/sdcard/game/data.zip!res/\""
                ));
            }
            Ok(FunctorLangSource(SourceData {
                device: Some(path),
                ..source.0
            }))
        },
    );
    reg.fn3(
        "Config.external",
        "Config.external(prefix, source, config)",
        |prefix: String, source: FunctorLangSource, config: FunctorLangConfig| {
            let valid = prefix
                .strip_suffix('/')
                .is_some_and(|dir| crate::debug_protocol::validate_project_asset_path(dir).is_ok());
            if !valid {
                return Err(format!(
                    "Config.external: {prefix:?} must be a folder path ending in `/`, like \"ss2/\""
                ));
            }
            let ConfigData {
                mut game,
                mut external,
            } = config.0;
            if let Some(other) = external.iter().find(|other| {
                other.prefix.starts_with(prefix.as_str())
                    || prefix.starts_with(other.prefix.as_str())
            }) {
                return Err(format!(
                    "Config.external: {prefix:?} overlaps {:?}; declare one or the other",
                    other.prefix
                ));
            }
            external.push(ExternalPrefix {
                prefix,
                mount: source.0.mount,
                device: source.0.device,
            });
            game.external_key = {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                external.hash(&mut hasher);
                hasher.finish()
            };
            Ok(FunctorLangConfig(ConfigData { game, external }))
        },
    );
    reg.fn2(
        "Config.xr",
        "Config.xr(xr, config)",
        |xr: FunctorLangXr, config: FunctorLangConfig| {
            let live = if xr.0.foveation.is_some() {
                Some("Xr.foveation")
            } else if xr.0.passthrough_enabled.is_some() {
                Some("Xr.passthroughEnabled")
            } else {
                None
            };
            if let Some(knob) = live {
                return Err(format!(
                    "Config.xr: `{knob}` is a live setting the headset applies \
on change — return it from `settings = (model) => Settings.create() |> Settings.xr(…)`, not \
`config`"
                ));
            }
            let mut config = config.0;
            config.game.xr = XrConfig {
                refresh_rate: xr.0.refresh_rate,
                passthrough: xr.0.passthrough.unwrap_or(false),
                scene: xr.0.scene.unwrap_or(false),
            };
            Ok(FunctorLangConfig(config))
        },
    );

    reg.fn0("Settings.create", "Settings.create()", || {
        FunctorLangSettings(GameSettings::default())
    });
    reg.fn2(
        "Settings.xr",
        "Settings.xr(xr, settings)",
        |xr: FunctorLangXr, settings: FunctorLangSettings| {
            if xr.0.refresh_rate.is_some() {
                return Err(
                    "Settings.xr: `Xr.refreshRate` is read once at launch — declare it \
in `let config = Config.create() |> Config.xr(…)`, not `settings`"
                        .to_string(),
                );
            }
            if xr.0.passthrough.is_some() {
                return Err(
                    "Settings.xr: `Xr.passthrough` is read once at launch — declare it \
in `let config = Config.create() |> Config.xr(…)`, then pause or resume the feed here with \
`Xr.passthroughEnabled(on)`"
                        .to_string(),
                );
            }
            if xr.0.scene.is_some() {
                return Err(
                    "Settings.xr: `Xr.scene` is read once at launch — declare it in \
`let config = Config.create() |> Config.xr(…)`, not `settings`"
                        .to_string(),
                );
            }
            let mut settings = settings.0;
            settings.xr = XrSettings {
                foveation: xr.0.foveation,
                passthrough: xr.0.passthrough_enabled,
            };
            Ok(FunctorLangSettings(settings))
        },
    );
}

/// The `config` entry point's value, or `None` if it is not a `Config.t`.
pub fn config_of(value: &Value) -> Option<GameConfig> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangConfig>()
            .map(|c| c.0.game),
        _ => None,
    }
}

/// The `config` value's `Config.external` declarations, in order, or `None`
/// if it is not a `Config.t`.
pub fn externals_of(value: &Value) -> Option<Vec<ExternalPrefix>> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangConfig>()
            .map(|c| c.0.external.clone()),
        _ => None,
    }
}

/// A `settings(model)` result, or `None` if it is not a `Settings.t`.
pub fn settings_of(value: &Value) -> Option<GameSettings> {
    match value {
        Value::HostData(data) => data
            .as_any()
            .downcast_ref::<FunctorLangSettings>()
            .map(|s| s.0),
        _ => None,
    }
}
