use functor_docgen::OutputFormat;
use std::io::{self, Write};
use std::path::Path;

pub fn execute(
    format: OutputFormat,
    output: Option<&Path>,
    check: Option<&Path>,
) -> io::Result<()> {
    let generated = functor_docgen::render(format).map_err(io::Error::other)?;
    if let Some(path) = check {
        if !functor_docgen::file_is_current(path, &generated)? {
            return Err(io::Error::other(format!(
                "{} is stale; run `npm run generate:docs`",
                path.display()
            )));
        }
    } else if let Some(path) = output {
        functor_docgen::write_file(path, &generated)?;
    } else {
        io::stdout().write_all(generated.as_bytes())?;
    }
    Ok(())
}
