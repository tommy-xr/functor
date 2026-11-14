//! The `functor` CLI the app runs (D40, interim): what the shell's check
//! found wrong with it, said once, the same everywhere it shows.

/// Why the app cannot use the `functor` it found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliCause {
    /// Nothing named `functor` next to the app or on PATH.
    Missing,
    /// It answers, but offers less than this app needs.
    TooOld,
    /// It cannot be run, or did not answer the check: why.
    Unusable(String),
}

/// What the check found wrong with the `functor` the app found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliProblem {
    /// The binary, as found, with the home directory as `~`.
    pub path: String,
    /// What it says its version is, when it says.
    pub version: Option<String>,
    pub cause: CliCause,
    /// It is the one inside `Functor.app`, so the bundle is damaged.
    pub bundled: bool,
}

impl CliProblem {
    /// The one message: which binary, what is wrong with it, the fix.
    pub fn message(&self) -> String {
        let named = match &self.version {
            Some(v) => format!("{} ({v})", self.path),
            None => self.path.clone(),
        };
        let what = match &self.cause {
            CliCause::Missing => "is missing".to_string(),
            CliCause::TooOld => "is older than this app".to_string(),
            CliCause::Unusable(why) => format!("cannot be used: {why}"),
        };
        let stopped = "so games and the agent's tools cannot start";
        match (&self.cause, self.bundled) {
            (_, true) => format!(
                "This Functor.app is damaged: its functor CLI at {named} {what}, {stopped}. \
Reinstall Functor.app."
            ),
            (CliCause::Missing, false) => format!(
                "No functor CLI was found next to this app or on PATH, {stopped}. {}",
                FIX
            ),
            (_, false) => format!("The functor CLI at {named} {what}, {stopped}. {FIX}"),
        }
    }
}

const FIX: &str = "Build it in this checkout (npm run build:cli), or pass --functor <path>.";

/// `path` with `home` (when it starts with it) as `~`.
pub fn abbreviate(path: &str, home: Option<&str>) -> String {
    let home = home
        .map(|h| h.trim_end_matches('/'))
        .filter(|h| !h.is_empty());
    match home.and_then(|h| path.strip_prefix(h)) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(cause: CliCause, version: Option<&str>, bundled: bool) -> CliProblem {
        CliProblem {
            path: "~/.functor/bin/functor".to_string(),
            version: version.map(str::to_string),
            cause,
            bundled,
        }
    }

    #[test]
    fn says_which_binary_what_is_wrong_and_the_fix() {
        assert_eq!(
            problem(CliCause::TooOld, Some("0.1.2"), false).message(),
            "The functor CLI at ~/.functor/bin/functor (0.1.2) is older than this app, so games \
and the agent's tools cannot start. Build it in this checkout (npm run build:cli), or pass \
--functor <path>."
        );
        assert_eq!(
            problem(CliCause::Unusable("Permission denied".into()), None, false).message(),
            "The functor CLI at ~/.functor/bin/functor cannot be used: Permission denied, so \
games and the agent's tools cannot start. Build it in this checkout (npm run build:cli), or \
pass --functor <path>."
        );
        assert!(problem(CliCause::Missing, None, false)
            .message()
            .starts_with("No functor CLI was found next to this app or on PATH"));
    }

    #[test]
    fn inside_the_bundle_it_is_damaged() {
        let message = problem(CliCause::TooOld, Some("0.1.2"), true).message();
        assert!(
            message.starts_with("This Functor.app is damaged"),
            "{message}"
        );
        assert!(message.ends_with("Reinstall Functor.app."), "{message}");
        assert!(!message.contains("npm"), "{message}");
    }

    #[test]
    fn abbreviates_the_home_directory_only_as_a_whole_component() {
        let home = Some("/Users/me");
        assert_eq!(
            abbreviate("/Users/me/.functor/bin/functor", home),
            "~/.functor/bin/functor"
        );
        assert_eq!(abbreviate("/Users/me/", Some("/Users/me/")), "~/");
        assert_eq!(abbreviate("/Users/meg/functor", home), "/Users/meg/functor");
        assert_eq!(abbreviate("/usr/bin/functor", home), "/usr/bin/functor");
        assert_eq!(abbreviate("/usr/bin/functor", None), "/usr/bin/functor");
    }
}
