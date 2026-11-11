//! `functor-app`: the desktop app's shell around `functor-app-core`.

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "The Functor desktop app")]
struct Args {
    /// Start directory for project discovery (defaults to the current directory).
    #[arg(short, long)]
    dir: Option<PathBuf>,

    /// Run without a window.
    #[arg(long)]
    headless: bool,

    /// Print the state as text and exit.
    #[arg(long)]
    print: bool,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    if !args.headless {
        eprintln!("functor-app: only --headless is supported yet");
        std::process::exit(2);
    }
    if !args.print {
        println!("functor-app: nothing to serve yet");
        return Ok(());
    }
    let start = std::fs::canonicalize(args.dir.unwrap_or_else(|| PathBuf::from(".")))?;
    print!(
        "{}",
        functor_app_core::print(&functor_app_core::discover(&start))
    );
    Ok(())
}
