//! The `mle` CLI. Five subcommands:
//!
//! ```text
//! mle parse <file.mle>   # print the surface AST (pretty-Debug)
//! mle ir <file.mle>      # parse + lower; print the core IR (pretty-Debug)
//! mle check <file.mle>   # typecheck; silent when clean, all diagnostics when not
//! mle run <file.mle>     # evaluate; print main's result, or every binding
//! mle trace <file.mle>   # evaluate with the call trace; print the trace
//! ```
//!
//! On failure (parse, lowering, checking, or runtime) prints
//! `file:line:col: error: message` to stderr and exits nonzero. `check` is
//! the one command that reports *every* diagnostic, one per line, rather
//! than stopping at the first; `run` deliberately does not typecheck.

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, path) = match args.as_slice() {
        [command, path] if ["parse", "ir", "check", "run", "trace"].contains(&command.as_str()) => {
            (command.as_str(), path)
        }
        _ => {
            eprintln!("usage: mle <parse|ir|check|run|trace> <file.mle>");
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
    if command == "check" {
        let diags = mle::check(&module);
        for diag in &diags {
            let (line, col) = mle::line_col(&src, diag.span.start);
            eprintln!("{path}:{line}:{col}: error: {}", diag.message);
        }
        if !diags.is_empty() {
            exit(1);
        }
        return;
    }
    let tracing = if command == "trace" {
        mle::Tracing::On
    } else {
        mle::Tracing::Off
    };
    let record = match mle::run(&module, tracing) {
        Ok(record) => record,
        Err(failure) => {
            // A failing run is when the execution story matters most: print
            // the partial trace before the diagnostic.
            if command == "trace" {
                print!("{}", mle::render_trace(&failure.trace));
            }
            fail(path, &src, failure.error.span, &failure.error.message)
        }
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
