//! The `mle` CLI. Two subcommands:
//!
//! ```text
//! mle parse <file.mle>   # print the surface AST (pretty-Debug)
//! mle ir <file.mle>      # parse + lower; print the core IR (pretty-Debug)
//! ```
//!
//! On failure (parse or lowering) prints `file:line:col: error: message` to
//! stderr and exits nonzero.

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "parse" || command == "ir" => {
            let src = match std::fs::read_to_string(path) {
                Ok(src) => src,
                Err(err) => {
                    eprintln!("error: cannot read {path}: {err}");
                    exit(1);
                }
            };
            let program = match mle::parse(&src) {
                Ok(program) => program,
                Err(err) => fail(path, &src, err.span, &err.message),
            };
            if command == "parse" {
                println!("{program:#?}");
                return;
            }
            match mle::lower(program) {
                Ok(module) => println!("{module:#?}"),
                Err(err) => fail(path, &src, err.span, &err.message),
            }
        }
        _ => {
            eprintln!("usage: mle <parse|ir> <file.mle>");
            exit(2);
        }
    }
}

fn fail(path: &str, src: &str, span: mle::Span, message: &str) -> ! {
    let (line, col) = mle::line_col(src, span.start);
    eprintln!("{path}:{line}:{col}: error: {message}");
    exit(1);
}
