//! The `functor` the app runs (D40, interim): found (`session::functor_bin`)
//! and checked before its first use, and again on Try again. While the
//! check finds a problem nothing is started with it: every spawn asks
//! [`Cli::path`], which then answers with the problem's message.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use functor_app_core::{CliCause, CliProblem};

/// The least `app_protocol` this app needs from `functor app-compat`
/// (the CLI's `APP_PROTOCOL`, `cli/src/commands/app.rs`): raise it with the
/// CLI's whenever the app starts using something new from it.
pub const MIN_APP_PROTOCOL: u32 = 1;

/// How long the check may take: a freshly built binary's first start can
/// wait on the OS's malware scan.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a CLI that failed the check may take to say its version: it has
/// just started, so the scan is behind it.
const VERSION_TIMEOUT: Duration = Duration::from_secs(2);

pub struct Cli {
    /// `--functor`, `FUNCTOR_BIN` and this executable: where to look.
    flag: Option<PathBuf>,
    env: Option<OsString>,
    exe: Option<PathBuf>,
    /// The last check's binary, or its problem's message.
    found: Mutex<Result<PathBuf, String>>,
    /// `found` is this executable, inside `functor`: never checked.
    own: bool,
}

impl Cli {
    pub fn new(flag: Option<PathBuf>, env: Option<OsString>, exe: Option<PathBuf>) -> Self {
        Self {
            flag,
            env,
            exe,
            found: Mutex::new(Err("the functor CLI has not been checked yet".to_string())),
            own: false,
        }
    }

    /// `path`, taken as checked: for tests of what is started with it.
    #[cfg(test)]
    pub fn checked(path: PathBuf) -> Self {
        Self {
            found: Mutex::new(Ok(path)),
            ..Self::new(None, None, None)
        }
    }

    /// This executable, inside `functor`: the CLI cannot differ from the
    /// app, so it is neither looked up nor checked.
    pub fn own(exe: PathBuf) -> Self {
        Self {
            found: Mutex::new(Ok(exe)),
            own: true,
            ..Self::new(None, None, None)
        }
    }

    /// The `functor` to start, or why nothing may be started with it.
    pub fn path(&self) -> Result<PathBuf, String> {
        self.found.lock().unwrap().clone()
    }

    /// Finds the `functor` anew (one may have been built meanwhile), checks
    /// it, and keeps the verdict for [`Cli::path`].
    pub async fn check(&self) -> Option<CliProblem> {
        if self.own {
            return None;
        }
        let path = self.find();
        let problem = match probe(&path, CHECK_TIMEOUT).await {
            Ok(()) => None,
            Err((cause, version)) => Some(CliProblem {
                path: shown(&path),
                version,
                cause,
                bundled: self.bundled(&path),
            }),
        };
        *self.found.lock().unwrap() = match &problem {
            None => Ok(path),
            Some(p) => Err(p.message()),
        };
        problem
    }

    /// The `functor` to check: absolute (children run in project
    /// directories), a bare name taken from PATH when it is there. Inside
    /// `Functor.app`, unless named, only the bundle's own: never one on PATH.
    fn find(&self) -> PathBuf {
        let path =
            crate::session::functor_bin(self.flag.clone(), self.env.clone(), self.exe.clone());
        if path.components().count() > 1 {
            return std::path::absolute(&path).unwrap_or(path);
        }
        let named = self.flag.is_some() || self.env.as_ref().is_some_and(|e| !e.is_empty());
        if let (Some(contents), false) = (self.contents(), named) {
            return contents.join("Helpers").join(path);
        }
        let dirs = std::env::var_os("PATH").unwrap_or_default();
        let on_path = std::env::split_paths(&dirs)
            .map(|dir| dir.join(&path))
            .find(|p| p.is_absolute() && executable(p));
        on_path.unwrap_or(path)
    }

    /// The `Contents` of the `Functor.app` this app runs from, if it does.
    fn contents(&self) -> Option<&Path> {
        let dir = self.exe.as_deref().and_then(Path::parent)?;
        dir.ends_with("Contents/MacOS").then(|| dir.parent()).flatten()
    }

    /// `path` is inside the `Functor.app` this app runs from.
    fn bundled(&self, path: &Path) -> bool {
        self.contents().is_some_and(|contents| path.starts_with(contents))
    }
}

/// A file `path` that can be run, as `execvp` looks for one on PATH.
fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    path.is_file()
}

/// `path`, with the home directory as `~`.
fn shown(path: &Path) -> String {
    let home = std::env::home_dir();
    let home = home.as_deref().and_then(Path::to_str);
    functor_app_core::cli::abbreviate(&path.to_string_lossy(), home)
}

/// Runs `functor app-compat`: `Ok` when it offers what this app needs, else
/// why not, with the version it reports.
async fn probe(path: &Path, timeout: Duration) -> Result<(), (CliCause, Option<String>)> {
    if path.components().count() == 1 {
        return Err((CliCause::Missing, None));
    }
    let out = match run(path, &["app-compat", "--json"], timeout).await {
        Ok(out) => out,
        Err(why) => return Err((CliCause::Unusable(why), None)),
    };
    match out.status.code() {
        Some(0) => {}
        // Clap's usage error: a CLI from before the check.
        Some(2) => return Err((CliCause::TooOld, version(path, VERSION_TIMEOUT).await)),
        _ => {
            let why =
                first_line(&out.stderr).unwrap_or_else(|| format!("it exited with {}", out.status));
            return Err((CliCause::Unusable(why), None));
        }
    }
    let answer: Option<(u64, Option<String>)> =
        serde_json::from_slice::<serde_json::Value>(&out.stdout)
            .ok()
            .and_then(|v| {
                let protocol = v["app_protocol"].as_u64()?;
                Some((protocol, v["version"].as_str().map(str::to_string)))
            });
    match answer {
        Some((protocol, _)) if protocol >= MIN_APP_PROTOCOL as u64 => Ok(()),
        Some((_, version)) => Err((CliCause::TooOld, version)),
        None => {
            let why = "it did not answer the app's check (functor app-compat)".to_string();
            Err((CliCause::Unusable(why), None))
        }
    }
}

/// `functor --version`'s version (`functor-cli 0.1.2` → `0.1.2`), if it says.
async fn version(path: &Path, timeout: Duration) -> Option<String> {
    let out = run(path, &["--version"], timeout).await.ok()?;
    match String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()[..]
    {
        [name, version] if name.starts_with("functor") => Some(version.to_string()),
        _ => None,
    }
}

/// `path <args>`'s output, or why there is none (it cannot be started, or
/// it took longer than `timeout` and was killed).
async fn run(
    path: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let child = tokio::process::Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "it does not exist".to_string(),
            std::io::ErrorKind::PermissionDenied => "it is not executable".to_string(),
            _ => e.to_string(),
        })?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(out) => out.map_err(|e| e.to_string()),
        Err(_) => Err(format!(
            "it did not answer within {} s",
            timeout.as_secs_f32()
        )),
    }
}

fn first_line(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const SHORT: Duration = Duration::from_millis(500);

    use crate::creation::tests::fake_functor as fake;

    const OLD: &str = r#"case "$1" in
  --version) echo 'functor-cli 0.1.2' ;;
  *) echo "error: unrecognized subcommand '$1'" >&2; exit 2 ;;
esac"#;

    #[tokio::test]
    async fn a_cli_from_before_the_check_is_too_old_with_its_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake(dir.path(), OLD);
        assert_eq!(
            probe(&path, SHORT).await,
            Err((CliCause::TooOld, Some("0.1.2".to_string())))
        );
    }

    #[tokio::test]
    async fn a_lower_protocol_is_too_old_and_the_minimum_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let answer = |protocol: u32| {
            format!(r#"echo '{{"app_protocol":{protocol},"version":"9.9.9","debug_protocol":1}}'"#)
        };
        let low = fake(dir.path(), &answer(MIN_APP_PROTOCOL - 1));
        assert_eq!(
            probe(&low, SHORT).await,
            Err((CliCause::TooOld, Some("9.9.9".to_string())))
        );
        let fine = fake(dir.path(), &answer(MIN_APP_PROTOCOL));
        assert_eq!(probe(&fine, SHORT).await, Ok(()));
    }

    #[tokio::test]
    async fn one_that_cannot_run_or_answer_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("functor");
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let Err((CliCause::Unusable(why), None)) = probe(&path, SHORT).await else {
            panic!("not executable, yet usable");
        };
        assert_eq!(why, "it is not executable");
        let gone = dir.path().join("nothing-here");
        assert_eq!(
            probe(&gone, SHORT).await,
            Err((CliCause::Unusable("it does not exist".to_string()), None))
        );
        let garbled = fake(dir.path(), "echo hello");
        assert!(matches!(
            probe(&garbled, SHORT).await,
            Err((CliCause::Unusable(_), None))
        ));
        let broken = fake(dir.path(), "echo 'dyld: missing symbol' >&2; exit 134");
        assert_eq!(
            probe(&broken, SHORT).await,
            Err((CliCause::Unusable("dyld: missing symbol".to_string()), None))
        );
        assert_eq!(
            probe(Path::new("functor"), SHORT).await,
            Err((CliCause::Missing, None))
        );
    }

    #[tokio::test]
    async fn one_that_hangs_is_cut_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake(dir.path(), "exec sleep 30");
        let started = std::time::Instant::now();
        let Err((CliCause::Unusable(why), None)) = probe(&path, SHORT).await else {
            panic!("a hang passed the check");
        };
        assert!(why.contains("did not answer within"), "{why}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn the_verdict_gates_what_is_started() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake(dir.path(), OLD);
        let cli = Cli::new(Some(path.clone()), None, None);
        assert!(cli.path().is_err(), "unchecked, yet startable");
        let problem = cli.check().await.expect("too old");
        assert_eq!(cli.path(), Err(problem.message()));
        // Built meanwhile: Try again finds it fine.
        fake(
            dir.path(),
            r#"echo '{"app_protocol":1,"version":"x","debug_protocol":1}'"#,
        );
        assert_eq!(cli.check().await, None);
        assert_eq!(cli.path(), Ok(path));
    }

    #[tokio::test]
    async fn inside_functor_the_cli_is_this_executable_unchecked() {
        // It would fail the check: it is never run for one.
        let dir = tempfile::tempdir().unwrap();
        let exe = fake(dir.path(), OLD);
        let cli = Cli::own(exe.clone());
        assert_eq!(cli.path(), Ok(exe.clone()));
        assert_eq!(cli.check().await, None);
        assert_eq!(cli.path(), Ok(exe));
    }

    #[test]
    fn a_bundle_without_its_cli_never_takes_one_from_path() {
        let exe = PathBuf::from("/nonexistent/Functor.app/Contents/MacOS/functor-app");
        let cli = Cli::new(None, None, Some(exe.clone()));
        let found = cli.find();
        assert_eq!(found, Path::new("/nonexistent/Functor.app/Contents/Helpers/functor"));
        assert!(cli.bundled(&found));
        // Named, it is the user's choice.
        let named = Cli::new(None, Some("/opt/functor".into()), Some(exe));
        assert_eq!(named.find(), Path::new("/opt/functor"));
    }

    #[test]
    fn the_bundle_is_damaged_only_for_its_own_cli() {
        let exe = PathBuf::from("/Applications/Functor.app/Contents/MacOS/functor-app");
        let cli = Cli::new(None, None, Some(exe));
        assert!(cli.bundled(Path::new(
            "/Applications/Functor.app/Contents/Helpers/functor"
        )));
        assert!(!cli.bundled(Path::new("/Users/me/.functor/bin/functor")));
        let dev = Cli::new(
            None,
            None,
            Some(PathBuf::from("/src/target/debug/functor-app")),
        );
        assert!(!dev.bundled(Path::new("/src/target/debug/functor")));
    }
}
