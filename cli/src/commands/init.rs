//! Functor Lang-native project scaffolding for `functor init`.

use clap::ValueEnum;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MANIFEST: &str = include_str!("../../templates/functor.json");
const GAME_3D: &str = include_str!("../../templates/3d/game.fun");
const GAME_FPS: &str = include_str!("../../templates/fps/game.fun");

#[derive(Clone, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Template {
    /// A small lit 3D scene.
    #[default]
    #[value(name = "3d")]
    ThreeD,
    /// A first-person WASD + mouse-look scene.
    Fps,
}

impl Template {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ThreeD => "3d",
            Self::Fps => "fps",
        }
    }

    fn files(&self) -> [TemplateFile; 2] {
        let game = match self {
            Self::ThreeD => GAME_3D,
            Self::Fps => GAME_FPS,
        };
        [
            TemplateFile {
                name: "functor.json",
                contents: MANIFEST,
            },
            TemplateFile {
                name: "game.fun",
                contents: game,
            },
        ]
    }
}

struct TemplateFile {
    name: &'static str,
    contents: &'static str,
}

/// Create a project in `directory` without overwriting either scaffolded file.
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

    let files = template.files();
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
                template.as_str(),
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
        if let Err(error) = output.write_all(file.contents.as_bytes()) {
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
    use super::{execute, Template, GAME_3D, GAME_FPS, MANIFEST};
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
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

    fn assert_project_typechecks(directory: &Path) {
        let project = functor_lang::project::load_with_prelude(
            &directory.join("game.fun"),
            &HashMap::new(),
            &functor_prelude::modules(),
        )
        .unwrap_or_else(|error| panic!("template should load: {}", error.render()));
        let diagnostics = project.check();
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:#?}");
    }

    #[test]
    fn three_d_template_creates_a_typechecked_project() {
        let directory = TestDir::new("3d");
        execute(&directory.0, &Template::ThreeD).unwrap();

        assert_eq!(
            fs::read_to_string(directory.0.join("functor.json")).unwrap(),
            MANIFEST
        );
        assert_eq!(
            fs::read_to_string(directory.0.join("game.fun")).unwrap(),
            GAME_3D
        );
        assert_project_typechecks(&directory.0);
    }

    #[test]
    fn fps_template_creates_a_typechecked_project() {
        let directory = TestDir::new("fps");
        execute(&directory.0, &Template::Fps).unwrap();

        assert_eq!(
            fs::read_to_string(directory.0.join("game.fun")).unwrap(),
            GAME_FPS
        );
        assert_project_typechecks(&directory.0);
    }

    #[test]
    fn unrelated_files_are_preserved() {
        let directory = TestDir::new("existing-directory");
        fs::create_dir_all(&directory.0).unwrap();
        fs::write(directory.0.join("README.md"), "keep me\n").unwrap();

        execute(&directory.0, &Template::ThreeD).unwrap();

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

        let error = execute(&directory.0, &Template::Fps).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("game.fun"));
        assert_eq!(
            fs::read_to_string(directory.0.join("game.fun")).unwrap(),
            "user source\n"
        );
        assert!(!directory.0.join("functor.json").exists());
    }
}
