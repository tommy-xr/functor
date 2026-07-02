//! The `mle` CLI. B1 exposes one subcommand:
//!
//! ```text
//! mle parse <file.mle>
//! ```
//!
//! Prints the surface AST (pretty-Debug) on success; on failure prints
//! `file:line:col: error: message` to stderr and exits nonzero.

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "parse" => {
            let src = match std::fs::read_to_string(path) {
                Ok(src) => src,
                Err(err) => {
                    eprintln!("error: cannot read {path}: {err}");
                    exit(1);
                }
            };
            match mle::parse(&src) {
                Ok(program) => println!("{program:#?}"),
                Err(err) => {
                    let (line, col) = mle::line_col(&src, err.span.start);
                    eprintln!("{path}:{line}:{col}: error: {}", err.message);
                    exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: mle parse <file.mle>");
            exit(2);
        }
    }
}
