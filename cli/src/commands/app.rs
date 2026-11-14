//! `functor app`: finds the desktop app's `functor-app` binary and runs it
//! with this `functor` as its CLI. Process launching only — the CLI does not
//! link the app.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// What this CLI offers the desktop app, which requires a minimum: bump it
/// whenever the app starts using something new from the CLI (a flag, a
/// subcommand, an output it reads). 1: `run native --paused`/`--hidden`,
/// `run vr --device-url`, `mcp --http`/`--review`/`--app-url`,
/// `init --list --json` and `run native --replay-input`.
pub const APP_PROTOCOL: u32 = 1;

/// `functor app-compat`'s answer: one JSON line the app checks before its
/// first use of this CLI.
pub fn compat_json(version: &str) -> String {
    serde_json::json!({
        "app_protocol": APP_PROTOCOL,
        "version": version,
        "debug_protocol": functor_runtime_common::debug_protocol::DEBUG_PROTOCOL_VERSION,
    })
    .to_string()
}

/// The app binary's file name on this OS.
fn app_name() -> String {
    format!("functor-app{}", std::env::consts::EXE_SUFFIX)
}

/// The first existing of: `functor-app` next to `exe`, `FUNCTOR_APP_BIN`
/// (`env`), `functor-app` on `path`. `Err` says what was looked for.
pub fn find(
    exe: Option<&Path>,
    env: Option<OsString>,
    path: Option<OsString>,
    exists: impl Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    let name = app_name();
    let sibling = exe.and_then(Path::parent).map(|dir| dir.join(&name));
    let env = env.filter(|e| !e.is_empty()).map(PathBuf::from);
    let on_path = path
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(&name));
    if let Some(found) = sibling
        .iter()
        .chain(env.iter())
        .cloned()
        .chain(on_path)
        .find(|p| exists(p))
    {
        return Ok(found);
    }
    let mut looked = Vec::new();
    if let Some(sibling) = &sibling {
        looked.push(sibling.display().to_string());
    }
    looked.push(match &env {
        Some(env) => format!("FUNCTOR_APP_BIN ({})", env.display()),
        None => "FUNCTOR_APP_BIN (unset)".to_string(),
    });
    looked.push(format!("{name} on PATH"));
    Err(looked.join(", "))
}

/// `find` for this process: its own path, environment and PATH.
fn locate(functor: Option<&Path>) -> Result<PathBuf, String> {
    find(
        functor,
        std::env::var_os("FUNCTOR_APP_BIN"),
        std::env::var_os("PATH"),
        Path::is_file,
    )
}

/// Whether a `functor-app` can be found.
pub fn installed() -> bool {
    locate(std::env::current_exe().ok().as_deref()).is_ok()
}

/// `functor-app`'s arguments: this CLI, the project directory, then `extra`.
pub fn args(functor: &Path, dir: Option<&Path>, extra: &[String]) -> Vec<OsString> {
    let mut args = vec![OsString::from("--functor"), functor.into()];
    if let Some(dir) = dir {
        args.extend([OsString::from("-d"), dir.into()]);
    }
    args.extend(extra.iter().map(OsString::from));
    args
}

/// Replaces this process with `functor-app` (Unix), or runs it and exits
/// with its code (Windows). Returns only on failure.
pub fn execute(dir: Option<&Path>, extra: &[String]) -> io::Error {
    let functor = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return e,
    };
    let app = match locate(Some(&functor)) {
        Ok(app) => app,
        Err(looked) => {
            return io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "functor-app not found (looked for {looked})\n\
                     build it from source: npm run build:app-ui && cargo build --release -p functor-app"
                ),
            )
        }
    };
    let mut command = std::process::Command::new(&app);
    command.args(args(&functor, dir, extra));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = command.exec();
        io::Error::new(e.kind(), format!("cannot run {}: {e}", app.display()))
    }
    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => io::Error::new(e.kind(), format!("cannot run {}: {e}", app.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dirs: &[&str]) -> Option<OsString> {
        Some(std::env::join_paths(dirs).unwrap())
    }

    #[test]
    fn looks_next_to_functor_then_the_env_var_then_path() {
        let name = app_name();
        let exe = Path::new("/opt/functor/bin/functor");
        let sibling = Path::new("/opt/functor/bin").join(&name);
        let env = || Some(OsString::from("/env/functor-app"));
        let path = || paths(&["/usr/bin", "/path/a", "/path/b"]);
        let everything = |_: &Path| true;
        assert_eq!(
            find(Some(exe), env(), path(), everything),
            Ok(sibling.clone())
        );
        let no_sibling = |p: &Path| p != sibling;
        assert_eq!(
            find(Some(exe), env(), path(), no_sibling),
            Ok(PathBuf::from("/env/functor-app"))
        );
        // An empty env var counts as unset; the first PATH dir that has it wins.
        let on_b = |p: &Path| p == Path::new("/path/b").join(&name);
        assert_eq!(
            find(Some(exe), Some(OsString::new()), path(), on_b),
            Ok(Path::new("/path/b").join(&name))
        );
        assert_eq!(
            find(None, None, path(), |p: &Path| p.starts_with("/path")),
            Ok(Path::new("/path/a").join(&name))
        );
    }

    #[test]
    fn not_found_says_what_was_looked_for() {
        let exe = Path::new("/opt/functor/bin/functor");
        let err = find(Some(exe), None, paths(&["/usr/bin"]), |_| false).unwrap_err();
        let sibling = Path::new("/opt/functor/bin").join(app_name());
        assert_eq!(
            err,
            format!(
                "{}, FUNCTOR_APP_BIN (unset), {} on PATH",
                sibling.display(),
                app_name()
            )
        );
        let err = find(None, Some("/x/app".into()), None, |_| false).unwrap_err();
        assert_eq!(
            err,
            format!("FUNCTOR_APP_BIN (/x/app), {} on PATH", app_name())
        );
    }

    #[test]
    fn passes_this_functor_the_dir_and_the_extra_args() {
        let functor = Path::new("/bin/functor");
        let extra = ["--headless".to_string(), "--print".to_string()];
        assert_eq!(
            args(functor, Some(Path::new("examples/primitives")), &extra),
            [
                "--functor",
                "/bin/functor",
                "-d",
                "examples/primitives",
                "--headless",
                "--print"
            ]
            .map(OsString::from)
        );
        assert_eq!(
            args(functor, None, &[]),
            ["--functor", "/bin/functor"].map(OsString::from)
        );
    }
}
