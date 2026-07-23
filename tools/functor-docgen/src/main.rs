use functor_docgen::{render, OutputFormat};
use std::path::PathBuf;
use std::{env, process};

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut format = OutputFormat::Markdown;
    let mut output = None;
    let mut check = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--format" => {
                let value = iter.next().ok_or("--format requires markdown or json")?;
                format = parse_format(&value)?;
            }
            "--output" => {
                output = Some(PathBuf::from(
                    iter.next().ok_or("--output requires a path")?,
                ));
            }
            "--check" => {
                check = Some(PathBuf::from(iter.next().ok_or("--check requires a path")?));
            }
            "-h" | "--help" => {
                println!(
                    "usage: functor-docgen [--format markdown|json] [--output PATH | --check PATH]"
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument `{arg}`")),
        }
    }
    if output.is_some() && check.is_some() {
        return Err("--output and --check cannot be used together".to_string());
    }

    let generated = render(format).map_err(|error| error.to_string())?;
    if let Some(path) = check {
        let current = functor_docgen::file_is_current(&path, &generated)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if !current {
            return Err(format!(
                "{} is stale; run `npm run generate:docs`",
                path.display()
            ));
        }
    } else if let Some(path) = output {
        functor_docgen::write_file(&path, &generated)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    } else {
        print!("{generated}");
    }
    Ok(())
}

fn parse_format(value: &str) -> Result<OutputFormat, String> {
    match value {
        "markdown" => Ok(OutputFormat::Markdown),
        "json" => Ok(OutputFormat::Json),
        _ => Err(format!(
            "unknown format `{value}`; expected markdown or json"
        )),
    }
}
