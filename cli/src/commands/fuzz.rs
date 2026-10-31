//! `functor fuzz`: the time-travel fuzzer over a project, headless
//! ([`functor_runtime_common::timetravel::fuzz`]; docs/time-travel.md,
//! "Fuzzing").

use std::io::Error;

use functor_runtime_common::timetravel::fuzz::{self, format_ops, parse_ops, Spec, Stats};

use super::functor_lang_project::{read_project_files, FunctorLangProject};
use crate::output::{emit, Event};

pub struct Options<'a> {
    pub seeds: u64,
    pub ops: usize,
    pub seed: Option<u64>,
    pub replay: Option<&'a str>,
    /// `functor -d … --entry … --mount …`, to print reproducing commands.
    pub command: String,
}

/// Fuzz the project's selected entry; `Err` (exit 1) on any failure, after
/// printing each shrunk repro and the command that replays it.
pub fn execute(
    project: &FunctorLangProject,
    working_directory: &str,
    options: &Options,
) -> Result<(), Error> {
    let sources = read_project_files(&project.entry_path(working_directory)?)?;
    let name = match project.role.as_str() {
        "" => project.entry.clone(),
        role => role.to_string(),
    };
    // `Persistence.save` writes a real slot: each run gets an empty scratch
    // store, so the project's own saves are untouched and a repro depends on
    // its ops alone. File reads keep resolving through the project reader
    // `build` installed.
    let scratch = tempfile::tempdir()?;
    let spec = Spec {
        saves: Some(scratch.path().join("saves")),
        ..Spec::derive(&name, sources, project.entry_role()).map_err(Error::other)?
    };
    emit(Event::Info {
        message: format!(
            "fuzzing {name}: keys {:?}, mouse {}, xr {}, http {}, reload knob {}",
            spec.keys,
            spec.mouse,
            spec.xr,
            spec.http,
            spec.knob.as_ref().map_or("none", |(text, _)| text.as_str())
        ),
    });
    let (stats, found, budget) = match options.replay {
        Some(text) => {
            let ops = parse_ops(text).map_err(Error::other)?;
            let (failure, stats) = fuzz::execute(&spec, &ops);
            let found = failure.map(|failure| fuzz::Found {
                seed: 0,
                failure,
                repro: ops.clone(),
            });
            (stats, Vec::from_iter(found), format!("{} replayed ops", ops.len()))
        }
        None => {
            let seeds: Vec<u64> = match options.seed {
                Some(seed) => vec![seed],
                None => (1..=options.seeds).collect(),
            };
            let budget = format!("{} seed(s) x {} ops", seeds.len(), options.ops);
            let (stats, found) = fuzz::run(&spec, seeds, options.ops);
            (stats, found, budget)
        }
    };
    emit(Event::Info {
        message: format!("{name}: {budget}: {}", summary(&stats)),
    });
    for f in &found {
        let seed = match options.replay {
            Some(_) => String::new(),
            None => format!("seed {}: ", f.seed),
        };
        emit(Event::Warning {
            message: format!(
                "{seed}{}\n  repro ({} ops): {}\n  replay: {} fuzz --replay '{}'",
                f.failure,
                f.repro.len(),
                format_ops(&f.repro),
                options.command,
                format_ops(&f.repro)
            ),
        });
    }
    match found.len() {
        0 => Ok(()),
        n => Err(Error::other(format!("fuzz found {n} failure(s) in {name}"))),
    }
}

/// `frames 1200, seeks 80 (+40 rewinds), …`: the counters a run verified.
fn summary(stats: &Stats) -> String {
    let n = |key: &str| stats.get(key).copied().unwrap_or(0);
    format!(
        "{} frames recorded, {} seeks + {} rewinds ({} frames verified), {} replays \
({} live, {} frames; {} fork, {} frames), {} reloads ({} while paused), {} participant checks",
        n("frames recorded"),
        n("seeks"),
        n("rewinds"),
        n("frames verified"),
        n("replays") + n("fork replays"),
        n("replays"),
        n("replayed frames verified"),
        n("fork replays"),
        n("fork frames verified"),
        n("reloads") + n("reloads while paused"),
        n("reloads while paused"),
        n("participant checks"),
    )
}
