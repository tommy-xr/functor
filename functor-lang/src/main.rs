//! The `functor_lang` CLI. Six subcommands:
//!
//! ```text
//! functor-lang parse <file.fun>   # print the surface AST (pretty-Debug; this file only)
//! functor-lang ir <file.fun>      # load the project; print the merged core IR
//! functor-lang check <file.fun>   # typecheck the project; all diagnostics, exit 1
//! functor-lang run <file.fun>     # evaluate; print main's result, or the entry's bindings
//! functor-lang trace <file.fun>   # evaluate with the call trace; print the trace
//! functor-lang test <file.fun>    # evaluate the project's `expect` tests; exit 1 on failure
//! functor-lang bench [--all] [--json] [<file.fun>|<dir>]  # time interpreter eval (see bench.rs)
//! ```
//!
//! `ir`/`check`/`run`/`trace`/`test` treat the file as a project entry (B8): every
//! sibling `.fun` file in its directory loads with it — file = module,
//! whole-program checking. `parse` stays single-file (it shows one file's
//! surface syntax).
//!
//! On failure (parse, lowering, checking, or runtime) prints
//! `file:line:col: error: message` to stderr and exits nonzero. `check` is
//! the one command that reports *every* diagnostic, one per line, rather
//! than stopping at the first; `run` deliberately does not typecheck.

use std::process::exit;

mod bench;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `bench` takes its own flags (`--all`, `--json`) and an optional path, so
    // it is dispatched before the rigid single-path subcommand parsing.
    if args.first().is_some_and(|a| a == "bench") {
        bench::main(&args[1..]);
    }
    let (command, path) = match args.as_slice() {
        [command, path]
            if ["parse", "ir", "check", "run", "trace", "test"].contains(&command.as_str()) =>
        {
            (command.as_str(), path)
        }
        _ => {
            eprintln!(
                "usage: functor-lang <parse|ir|check|run|trace|test> <file.fun>\n       functor-lang bench [--all] [--json] [<file.fun>|<dir>]"
            );
            exit(2);
        }
    };
    if command == "parse" {
        let src = match std::fs::read_to_string(path) {
            Ok(src) => src,
            Err(err) => {
                eprintln!("error: cannot read {path}: {err}");
                exit(1);
            }
        };
        match functor_lang::parse(&src) {
            Ok(program) => println!("{program:#?}"),
            Err(err) => {
                let (line, col) = functor_lang::line_col(&src, err.span.start);
                eprintln!("{path}:{line}:{col}: error: {}", err.message);
                exit(1);
            }
        }
        return;
    }

    // Project commands: the file is the entry; siblings load with it.
    let project = match functor_lang::project::load(std::path::Path::new(path)) {
        Ok(project) => project,
        Err(err) => {
            eprintln!(
                "{}:{}:{}: error: {}",
                err.path.display(),
                err.line,
                err.col,
                err.message
            );
            exit(1);
        }
    };
    if command == "ir" {
        println!("{:#?}", project.module);
        return;
    }
    if command == "check" {
        let diags = project.check();
        for diag in &diags {
            let (file, line, col) = project.sources.resolve(diag.span.start);
            eprintln!(
                "{}:{line}:{col}: error: {}",
                file.path.display(),
                diag.message
            );
        }
        if !diags.is_empty() {
            exit(1);
        }
        return;
    }
    if command == "test" {
        // Like `run`, `test` does not typecheck first — `check` is the
        // static gate; here a non-bool expect reports as its own error.
        let reports = match functor_lang::run_expects(&project.module, &mut functor_lang::NoHost) {
            Ok(reports) => reports,
            Err(failure) => {
                let (file, line, col) = project.sources.resolve(failure.error.span.start);
                eprintln!(
                    "{}:{line}:{col}: error: {}",
                    file.path.display(),
                    failure.error.message
                );
                exit(1);
            }
        };
        if reports.is_empty() {
            println!("no `expect` tests found");
            return;
        }
        let mut failed = 0usize;
        for report in &reports {
            let (file, line, _col) = project.sources.resolve(report.span.start);
            let at = format!("{}:{line}", file.path.display());
            match &report.outcome {
                functor_lang::ExpectOutcome::Pass => println!("{at}: ok"),
                functor_lang::ExpectOutcome::Fail(detail) => {
                    failed += 1;
                    match detail {
                        Some(cmp) => println!(
                            "{at}: FAILED: left {} right — left: {}, right: {}",
                            cmp.op, cmp.lhs, cmp.rhs
                        ),
                        None => println!("{at}: FAILED: expected true, got false"),
                    }
                }
                functor_lang::ExpectOutcome::Error(error) => {
                    failed += 1;
                    let (efile, eline, ecol) = project.sources.resolve(error.span.start);
                    println!(
                        "{at}: ERROR: {}:{eline}:{ecol}: {}",
                        efile.path.display(),
                        error.message
                    );
                }
            }
        }
        let passed = reports.len() - failed;
        println!("{} expects: {passed} passed, {failed} failed", reports.len());
        if failed > 0 {
            exit(1);
        }
        return;
    }
    let tracing = if command == "trace" {
        functor_lang::Tracing::On
    } else {
        functor_lang::Tracing::Off
    };
    let record = match functor_lang::run(&project.module, tracing) {
        Ok(record) => record,
        Err(failure) => {
            // A failing run is when the execution story matters most: print
            // the partial trace before the diagnostic.
            if command == "trace" {
                print!("{}", functor_lang::render_trace(&failure.trace));
            }
            let (file, line, col) = project.sources.resolve(failure.error.span.start);
            eprintln!(
                "{}:{line}:{col}: error: {}",
                file.path.display(),
                failure.error.message
            );
            exit(1);
        }
    };
    if command == "trace" {
        print!("{}", functor_lang::render_trace(&record.trace));
        return;
    }
    match record.outcome {
        functor_lang::RunOutcome::Main(value) => println!("{value}"),
        functor_lang::RunOutcome::Bindings(bindings) => {
            for (name, value) in bindings {
                // Sibling modules' bindings are qualified ("Utils.x") —
                // report the ENTRY's bindings, the program the user ran.
                if !name.contains('.') {
                    println!("{name} = {value}");
                }
            }
        }
    }
}
