//! The `mle` CLI. Four subcommands:
//!
//! ```text
//! mle parse <file.mle>   # print the surface AST (pretty-Debug)
//! mle ir <file.mle>      # parse + lower; print the core IR (pretty-Debug)
//! mle run <file.mle>     # evaluate; print main's result, or every binding
//! mle trace <file.mle>   # evaluate with the call trace; print the trace
//! ```
//!
//! On failure (parse, lowering, or runtime) prints
//! `file:line:col: error: message` to stderr and exits nonzero.

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, path) = match args.as_slice() {
        [command, path] if ["parse", "ir", "run", "trace"].contains(&command.as_str()) => {
            (command.as_str(), path)
        }
        _ => {
            eprintln!("usage: mle <parse|ir|run|trace> <file.mle>");
            exit(2);
        }
    };
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
    let module = match mle::lower(program) {
        Ok(module) => module,
        Err(err) => fail(path, &src, err.span, &err.message),
    };
    if command == "ir" {
        println!("{module:#?}");
        return;
    }
    let tracing = if command == "trace" {
        mle::Tracing::On
    } else {
        mle::Tracing::Off
    };
    let record = match mle::run(&module, tracing) {
        Ok(record) => record,
        Err(err) => fail(path, &src, err.span, &err.message),
    };
    if command == "trace" {
        print!("{}", mle::render_trace(&record.trace));
        return;
    }
    match record.outcome {
        mle::RunOutcome::Main(value) => println!("{value}"),
        mle::RunOutcome::Bindings(bindings) => {
            for (name, value) in bindings {
                println!("{name} = {value}");
            }
        }
    }
}

fn fail(path: &str, src: &str, span: mle::Span, message: &str) -> ! {
    let (line, col) = mle::line_col(src, span.start);
    eprintln!("{path}:{line}:{col}: error: {message}");
    exit(1);
}
