use colored::*;
use std::env;
use std::io::{self, BufRead, Error};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;

use crate::util::WasmDevServer;
use crate::Environment;

pub async fn execute(working_directory: &str, environment: &Environment) -> Result<(), Error> {
    match environment {
        Environment::Native => panic!("not yet implemented"),
        Environment::Wasm => WasmDevServer::start().await,
    }
}
