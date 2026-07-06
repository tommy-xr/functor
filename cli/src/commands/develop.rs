use std::io;
use std::path::Path;

use crate::util::{self, get_nearby_bin, ShellCommand};
use crate::Environment;

// `develop` is native-only (it always runs the hot-reload runner), so the
// environment is currently ignored.
pub async fn execute(
    working_directory: &str,
    _environment: &Environment,
    runner_args: &[String],
) -> io::Result<()> {
    let cwd_path = Path::new(working_directory);
    let build_native_path = Path::new(&"build-native");
    let build_native_wd = cwd_path.join(build_native_path);

    // Transpile the game's F# project directly (mirroring `build`), named after
    // the game directory by convention (examples/foo -> foo.fsproj). `--outDir .`
    // (the CLI's cwd, the repo root) writes `fable_modules/` where the
    // build-native crate expects it.
    let project_name = cwd_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other(format!("invalid game directory: {working_directory}")))?;
    let fsproj = format!("{working_directory}/{project_name}.fsproj");
    let fable_cmd = format!("dotnet fable {fsproj} --lang rust --outDir .");

    let functor_runner_exe =
        get_nearby_bin(&"functor-runner").expect("functor-runner should be available");

    let target_dir = Path::new(&"target/debug");
    let library_name = libloading::library_filename("game_native");
    let game_lib = build_native_path.join(target_dir.join(Path::new(&library_name)));

    // The hot-reload runner gets the standard --hot/--game-path plus any extra
    // args forwarded from the CLI (e.g. --debug-port).
    let mut runner_command_args = vec!["--hot", "--game-path", game_lib.to_str().unwrap()];
    runner_command_args.extend(runner_args.iter().map(|s| s.as_str()));

    let commands = vec![
        ShellCommand {
            prefix: "[1: Build F#]",
            cmd: "watchexec",
            cwd: ".",
            env: vec![],
            args: vec!["-e", "fs", "--no-process-group", "--", &fable_cmd],
        },
        ShellCommand {
            prefix: "[2: Build Rust]",
            cmd: "watchexec",
            cwd: build_native_wd.to_str().unwrap(),
            env: vec![],
            args: vec!["-w", "..", "-e", "rs", "--", "cargo build"],
        },
        ShellCommand {
            prefix: "[3: Functor Runner]",
            cmd: functor_runner_exe.to_str().unwrap(),
            cwd: working_directory,
            env: vec![],
            args: runner_command_args,
        },
    ];

    util::ShellCommand::run_parallel(commands).await?;

    Ok(())
}
