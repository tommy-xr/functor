use colored::*;
use std::env;
use std::io::{self, BufRead, Error};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

use crate::util::ShellCommand;
use crate::Environment;

pub async fn execute(working_directory: &str, environment: &Environment) -> Result<(), Error> {
    // TODO: Extract these out
    let cwd_path = Path::new(working_directory);
    let build_native_path = Path::new(&"build-native");
    let build_native_wd = cwd_path.join(build_native_path);

    // The F# project (and its generated `<name>.rs`) is named after the game
    // directory by convention: examples/hello -> hello.fsproj, examples/lighting
    // -> lighting.fsproj. Transpile it directly so any game directory works
    // (not just the hardcoded sample). `--outDir .` (the CLI's cwd, the repo
    // root) writes `<dir>/<name>.rs` next to the project and `fable_modules/` at
    // the root, where the build-native/build-wasm crates expect them.
    let project_name = cwd_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::other(format!("invalid game directory: {}", working_directory)))?;
    let fsproj = format!("{}/{}.fsproj", working_directory, project_name);

    let build_wasm_path = Path::new(&"build-wasm");
    let build_wasm_wd = cwd_path.join(build_wasm_path);

    let native_build_command = ShellCommand {
        prefix: "[2: Build Native]",
        cmd: "cargo",
        cwd: build_native_wd.to_str().unwrap(),
        env: vec![],
        args: vec!["build"],
    };

    let wasm_build_command = ShellCommand {
        prefix: "[2: Build WASM]",
        cmd: "wasm-pack",
        cwd: build_wasm_wd.to_str().unwrap(),
        env: vec![],
        args: vec!["build", "--target=web"],
    };

    let build_command = match environment {
        Environment::Native => native_build_command,
        Environment::Wasm => wasm_build_command,
    };

    let commands = vec![
        ShellCommand {
            prefix: "[1: Build F#]",
            cmd: "dotnet",
            cwd: ".",
            env: vec![],
            args: vec!["fable", &fsproj, "--lang", "rust", "--outDir", "."],
        },
        build_command,
    ];

    ShellCommand::run_sequential(commands).await?;

    Ok(())
}
