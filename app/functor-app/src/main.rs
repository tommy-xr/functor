//! `functor-app`: the desktop app as its own binary, over the library that
//! `functor` also runs in-process (its `app` feature). Kept for the tests and
//! scripts that start it by this name.

fn main() -> std::io::Result<std::process::ExitCode> {
    functor_app::run(std::env::args_os(), functor_app::Host::Shim)
}
