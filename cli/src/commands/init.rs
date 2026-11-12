//! Functor Lang-native project scaffolding for `functor init`.
//!
//! A template is a directory holding an ordinary project plus a
//! `template.json` manifest ([`Manifest`]). The built-in set is the
//! hand-written [`TEMPLATES`] table, embedded with `include_bytes!` so the CLI
//! needs no source tree. A test fails when a table entry and its directory
//! disagree; that is simpler than a build script and keeps the set readable.
//! A row lists the files a new project gets: every module and `functor.json`,
//! plus any assets, but not an example's repo-only extras (capture scripts).

use clap::builder::{PossibleValue, PossibleValuesParser, TypedValueParser};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The template `functor init` uses when none is named.
pub const DEFAULT: &str = "3d";

/// `template.json`. `--list --json` prints these objects as they are, so the
/// field names are a stable interface (docs/cli-output.md).
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// What `functor init <name>` takes; equal to the table's name.
    pub name: String,
    /// One line.
    pub description: String,
    pub tags: Vec<String>,
    pub kind: Kind,
    /// An image path relative to the template's directory. It is not a
    /// project file, so `init` does not write it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A small project to build on.
    Starter,
    /// A complete game from `examples/`.
    Example,
}

#[derive(Debug)]
pub struct Template {
    pub name: &'static str,
    /// The template's directory relative to `cli/`; examples live outside it.
    /// Only the drift tests read it: the CLI uses the embedded copy.
    #[cfg(test)]
    dir: &'static str,
    manifest: &'static str,
    files: &'static [TemplateFile],
}

#[derive(Debug)]
struct TemplateFile {
    name: &'static str,
    contents: &'static [u8],
}

/// One table row: `template!(name, dir relative to cli/, [project files])`.
macro_rules! template {
    ($name:literal, $dir:literal, [$($file:literal),+ $(,)?]) => {
        Template {
            name: $name,
            #[cfg(test)]
            dir: $dir,
            manifest: include_str!(concat!("../../", $dir, "/template.json")),
            files: &[$(TemplateFile {
                name: $file,
                contents: include_bytes!(concat!("../../", $dir, "/", $file)),
            }),+],
        }
    };
}

/// Every built-in template, in `--list` order: starters, then examples.
pub static TEMPLATES: &[Template] = &[
    template!("3d", "templates/3d", ["functor.json", "game.fun"]),
    template!("blank", "templates/blank", ["functor.json", "game.fun"]),
    template!("2d", "templates/2d", ["functor.json", "game.fun"]),
    template!("fps", "templates/fps", ["functor.json", "game.fun"]),
    template!(
        "multiplayer",
        "templates/multiplayer",
        ["functor.json", "game.fun"]
    ),
    template!("vr", "templates/vr", ["functor.json", "game.fun"]),
    template!(
        "netpong",
        "../examples/netpong",
        [
            "functor.json",
            "client.fun",
            "server.fun",
            "protocol.fun",
            "game.fun"
        ]
    ),
    template!("tetris", "../examples/tetris", ["functor.json", "game.fun"]),
];

pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|template| template.name == name)
}

/// The names `init` accepts, for error messages.
pub fn names() -> Vec<&'static str> {
    TEMPLATES.iter().map(|template| template.name).collect()
}

/// clap's parser for the template argument: the table's names, each with its
/// description as help.
pub fn parser() -> impl TypedValueParser<Value = &'static Template> {
    PossibleValuesParser::new(
        TEMPLATES.iter().map(|template| {
            PossibleValue::new(template.name).help(template.manifest().description)
        }),
    )
    .map(|name| find(&name).expect("clap only accepts table names"))
}

impl Template {
    pub fn manifest(&self) -> Manifest {
        serde_json::from_str(self.manifest).expect("template.json is valid (tested)")
    }

    /// The file names this template writes — the scaffolder's own answer, so
    /// callers that report them (the `init_game` MCP tool) cannot drift.
    pub fn file_names(&self) -> Vec<&'static str> {
        self.files.iter().map(|file| file.name).collect()
    }
}

/// `functor init --list`: a JSON array of [`Manifest`]s, or a text table.
pub fn list(json: bool) -> String {
    let manifests: Vec<Manifest> = TEMPLATES.iter().map(Template::manifest).collect();
    if json {
        return serde_json::to_string(&manifests).expect("manifests serialize") + "\n";
    }
    let rows: Vec<[String; 4]> = manifests
        .into_iter()
        .map(|manifest| {
            let kind = match manifest.kind {
                Kind::Starter => "starter",
                Kind::Example => "example",
            };
            [
                manifest.name,
                kind.to_string(),
                manifest.tags.join(","),
                manifest.description,
            ]
        })
        .collect();
    let header = ["NAME", "KIND", "TAGS", "DESCRIPTION"].map(String::from);
    let width = |column: usize| {
        std::iter::once(&header)
            .chain(&rows)
            .map(|row| row[column].len())
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2)];
    std::iter::once(&header)
        .chain(&rows)
        .map(|row| {
            format!(
                "{:w0$}  {:w1$}  {:w2$}  {}\n",
                row[0],
                row[1],
                row[2],
                row[3],
                w0 = widths[0],
                w1 = widths[1],
                w2 = widths[2]
            )
        })
        .collect()
}

/// Create a project in `directory` without overwriting any scaffolded file: if
/// even one of the template's files already exists the whole call aborts before
/// writing anything (no per-file skipping — a partially scaffolded project is
/// worse than a refusal).
/// Unrelated existing files are left alone. If a write fails, files created by
/// this call are removed so the directory is not left half-initialized.
pub fn execute(directory: &Path, template: &Template) -> io::Result<()> {
    if directory.exists() && !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "cannot initialize a project at {}: path is not a directory",
                directory.display()
            ),
        ));
    }

    let files = template.files;
    let conflicts: Vec<_> = files
        .iter()
        .filter(|file| directory.join(file.name).exists())
        .map(|file| file.name)
        .collect();
    if !conflicts.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "cannot initialize the {} project in {}: refusing to overwrite {}",
                template.name,
                directory.display(),
                conflicts.join(", ")
            ),
        ));
    }

    let created_directory = !directory.exists();
    fs::create_dir_all(directory)?;

    let mut created_files: Vec<PathBuf> = Vec::new();
    for file in files {
        let path = directory.join(file.name);
        // `create_new` also refuses a symlink (even a dangling one), so a
        // planted link cannot redirect a write outside `directory`.
        let mut output = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(output) => output,
            Err(error) => {
                rollback(directory, created_directory, &created_files);
                return Err(io::Error::new(
                    error.kind(),
                    format!("failed to create {}: {error}", path.display()),
                ));
            }
        };
        if let Err(error) = output.write_all(file.contents) {
            // This call created `path`, so it is ours to remove. In contrast,
            // an `open` failure above may be a racing user-created file and
            // must never be deleted.
            drop(output);
            let _ = fs::remove_file(&path);
            rollback(directory, created_directory, &created_files);
            return Err(io::Error::new(
                error.kind(),
                format!("failed to write {}: {error}", path.display()),
            ));
        }
        created_files.push(path);
    }

    Ok(())
}

fn rollback(directory: &Path, created_directory: bool, created_files: &[PathBuf]) {
    for created in created_files.iter().rev() {
        let _ = fs::remove_file(created);
    }
    if created_directory {
        let _ = fs::remove_dir(directory);
    }
}

#[cfg(test)]
mod tests {
    use super::{execute, find, list, Kind, Template, TEMPLATES};
    use crate::commands::functor_lang_project;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Component, Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let suffix = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            Self(std::env::temp_dir().join(format!(
                "functor-init-{name}-{}-{suffix}",
                std::process::id()
            )))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn template_dir(template: &Template) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(template.dir)
    }

    /// `functor build`'s gate (every role typechecks, asset locators verify,
    /// a named role's contract validates), then `functor test`'s expects.
    fn assert_project_builds_and_tests(directory: &Path) {
        let dir = directory.to_string_lossy();
        let config = functor_lang_project::detect(&dir).expect("a Functor Lang project");
        let roles = config.all().expect("every role resolves");
        for (i, role) in roles.iter().enumerate() {
            let project = role
                .build_without_refresh(&dir, i == 0)
                .unwrap_or_else(|error| panic!("{dir} role {:?} should build: {error}", role.role));
            if config.is_named() {
                role.check_contract(&project)
                    .unwrap_or_else(|error| panic!("{dir} role contract: {error}"));
            }
        }
        let selected = config.select(None).unwrap();
        let project = selected.build_without_refresh(&dir, false).unwrap();
        selected
            .test(&project)
            .unwrap_or_else(|error| panic!("{dir} expects should pass: {error}"));
    }

    /// Every template writes exactly its files, byte for byte, and the result
    /// builds and passes its own `expect`s.
    #[test]
    fn every_template_creates_a_project_that_builds_and_passes_its_tests() {
        for template in TEMPLATES {
            let directory = TestDir::new(template.name);
            execute(&directory.0, template).unwrap();

            let written: BTreeSet<String> = fs::read_dir(&directory.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            let expected: BTreeSet<String> = template
                .file_names()
                .into_iter()
                .map(String::from)
                .collect();
            assert_eq!(written, expected, "{}", template.name);
            for file in template.files {
                assert_eq!(
                    fs::read(directory.0.join(file.name)).unwrap(),
                    file.contents,
                    "{}/{} should be written verbatim",
                    template.name,
                    file.name
                );
            }
            assert_project_builds_and_tests(&directory.0);
        }
    }

    /// The multiplayer template's role NAMES are part of its contract: the
    /// header tells the user to run `--entry server`, and a bare `run` must
    /// pick the client.
    #[test]
    fn multiplayer_template_declares_client_and_server_roles() {
        let directory = TestDir::new("multiplayer");
        execute(&directory.0, find("multiplayer").unwrap()).unwrap();
        let config = functor_lang_project::detect(&directory.0.to_string_lossy()).unwrap();
        let roles = config.all().unwrap();
        let names: Vec<&str> = roles.iter().map(|role| role.role.as_str()).collect();
        assert_eq!(names, ["client", "server"]);
        assert_eq!(config.select(None).unwrap().role, "client");
    }

    /// The table and the directories agree: every listed file exists and is
    /// embedded byte for byte (so an example template is the example itself),
    /// every module and `functor.json` in the directory is listed (a missing
    /// sibling module would break the project; missing assets fail the build
    /// test above), and every directory under `cli/templates` is listed.
    #[test]
    fn the_table_matches_the_template_directories() {
        let mut names = BTreeSet::new();
        for template in TEMPLATES {
            assert!(
                names.insert(template.name),
                "duplicate template {}",
                template.name
            );
            let manifest = template.manifest();
            assert_eq!(manifest.name, template.name, "template.json name");
            assert!(
                !manifest.description.contains('\n'),
                "{}: one line",
                template.name
            );

            let dir = template_dir(template);
            let required: BTreeSet<String> = fs::read_dir(&dir)
                .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| {
                    name == "functor.json" || name.ends_with(".fun") || name.ends_with(".funi")
                })
                .collect();
            let listed: BTreeSet<String> = template
                .file_names()
                .into_iter()
                .map(String::from)
                .collect();
            assert!(
                required.is_subset(&listed),
                "{} must list every module in {}: {:?}",
                template.name,
                dir.display(),
                required.difference(&listed).collect::<Vec<_>>()
            );

            for file in template.files {
                let mut components = Path::new(file.name).components();
                assert!(
                    matches!(
                        (components.next(), components.next()),
                        (Some(Component::Normal(_)), None)
                    ),
                    "{}: {:?} must be a plain file name",
                    template.name,
                    file.name
                );
                assert_eq!(
                    fs::read(dir.join(file.name)).unwrap(),
                    file.contents,
                    "{}/{} is embedded from disk",
                    template.name,
                    file.name
                );
            }
        }

        let template_dirs: BTreeSet<String> =
            fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("templates"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
        let starters: BTreeSet<String> = TEMPLATES
            .iter()
            .filter(|template| template.manifest().kind == Kind::Starter)
            .map(|template| template.dir.trim_start_matches("templates/").to_string())
            .collect();
        assert_eq!(
            template_dirs, starters,
            "every cli/templates directory is a starter in the table"
        );
    }

    /// `--list`'s text and JSON, pinned. Regenerate the snapshots with
    /// `UPDATE_GOLDENS=1 cargo test -p functor-cli --bin functor init::`.
    #[test]
    fn list_matches_its_snapshots() {
        let snapshots = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
        for (file, actual) in [
            ("init-list.txt", list(false)),
            ("init-list.json", list(true)),
        ] {
            let path = snapshots.join(file);
            if std::env::var_os("UPDATE_GOLDENS").is_some() {
                fs::create_dir_all(&snapshots).unwrap();
                fs::write(&path, &actual).unwrap();
            }
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                actual,
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn list_json_has_the_documented_fields() {
        let listed: Vec<serde_json::Value> = serde_json::from_str(&list(true)).unwrap();
        assert_eq!(listed.len(), TEMPLATES.len());
        for entry in listed {
            let keys: BTreeSet<&str> = entry
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                BTreeSet::from(["description", "kind", "name", "tags"])
            );
        }
    }

    #[test]
    fn unrelated_files_are_preserved() {
        let directory = TestDir::new("existing-directory");
        fs::create_dir_all(&directory.0).unwrap();
        fs::write(directory.0.join("README.md"), "keep me\n").unwrap();

        execute(&directory.0, find("3d").unwrap()).unwrap();

        assert_eq!(
            fs::read_to_string(directory.0.join("README.md")).unwrap(),
            "keep me\n"
        );
        assert!(directory.0.join("functor.json").is_file());
        assert!(directory.0.join("game.fun").is_file());
    }

    #[test]
    fn a_conflict_does_not_overwrite_or_partially_initialize() {
        let directory = TestDir::new("conflict");
        fs::create_dir_all(&directory.0).unwrap();
        fs::write(directory.0.join("game.fun"), "user source\n").unwrap();

        let error = execute(&directory.0, find("fps").unwrap()).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("game.fun"));
        assert_eq!(
            fs::read_to_string(directory.0.join("game.fun")).unwrap(),
            "user source\n"
        );
        assert!(!directory.0.join("functor.json").exists());
    }

    /// A multi-file template refuses as a whole: one existing sibling module
    /// stops it before any file is written.
    #[test]
    fn a_multi_file_conflict_writes_nothing() {
        let directory = TestDir::new("multi-conflict");
        fs::create_dir_all(&directory.0).unwrap();
        fs::write(directory.0.join("protocol.fun"), "mine\n").unwrap();

        let error = execute(&directory.0, find("netpong").unwrap()).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("protocol.fun"));
        let left: Vec<_> = fs::read_dir(&directory.0).unwrap().collect();
        assert_eq!(left.len(), 1, "only the user's file remains");
        assert_eq!(
            fs::read_to_string(directory.0.join("protocol.fun")).unwrap(),
            "mine\n"
        );
    }

    /// A link planted under one of the template's names cannot carry a write
    /// outside the target, and the files written before it are rolled back.
    #[cfg(unix)]
    #[test]
    fn a_planted_link_cannot_write_outside_the_target() {
        let root = TestDir::new("outside");
        let target = root.0.join("game");
        fs::create_dir_all(&target).unwrap();
        let outside = root.0.join("stolen.fun");
        std::os::unix::fs::symlink(&outside, target.join("server.fun")).unwrap();

        let error = execute(&target, find("netpong").unwrap()).unwrap_err();

        assert!(error.to_string().contains("server.fun"), "{error}");
        assert!(!outside.exists(), "nothing is written through the link");
        let left: Vec<String> = fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            left,
            ["server.fun"],
            "files written before the link are rolled back"
        );
    }
}
