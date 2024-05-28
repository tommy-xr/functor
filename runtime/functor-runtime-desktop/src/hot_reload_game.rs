use std::env;
use std::path::Path;
use std::{fs, process};
use tempfile::tempdir;

use functor_runtime_common::Scene3D;
use libloading::{Library, Symbol};

use crate::game::Game;

pub struct HotReloadGame {
    // Utils for constructing the next lib path
    file_stem: String,
    extension: String,
    counter: u32,

    // Current hot reload state
    latest_lib_path: String,
    library: Option<Library>,
}

impl Game for HotReloadGame {
    fn render(&mut self) -> Scene3D {
        println!("Rendering");
        unsafe {
            let render_func: Symbol<fn() -> Scene3D> =
                self.library.as_ref().unwrap().get(b"test_render").unwrap();
            render_func()
        }
    }
}
impl HotReloadGame {
    pub fn create(path: String) -> HotReloadGame {
        let counter = 0;
        let lib_path = Path::new(&path);
        let file_stem = lib_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or("".to_string());

        let extension = lib_path
            .extension()
            .map(|s| {
                let mut dot = ".".to_owned();
                dot.push_str(&s.to_string_lossy());
                dot
            })
            .unwrap_or("".to_string());

        println!("File stem: {} extension: {}", file_stem, extension);

        let library = unsafe { Some(Library::new(&path).unwrap()) };
        HotReloadGame {
            file_stem,
            extension,
            counter,

            latest_lib_path: path,
            library,
        }
    }

    fn get_next_destination_name(&mut self) -> String {
        self.counter = self.counter + 1;
        format!(
            "{}_{}_{}{}",
            self.file_stem,
            process::id(),
            self.counter,
            self.extension
        )
    }

    pub fn reload(&mut self) {
        unsafe {
            let Self { library, .. } = self;

            if let Some(lib) = library.take() {
                lib.close().unwrap();
            }

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            let new_file_name = temp_dir.path().join(self.get_next_destination_name());
            // TODO: Better definition of path here - don't hardcode target/debug
            let source_path = &format!("target/debug/{}", self.latest_lib_path);
            fs::copy(source_path, &new_file_name).expect("Cmtpy should succeed");
            println!("Loading from: {:?}", &new_file_name);

            let lib = Library::new(&new_file_name).unwrap();

            let init_func: Symbol<fn()> = lib.get(b"init").unwrap(); // Get the function pointer
            println!("Running init after reload.");
            init_func();

            self.library = Some(lib); // Load the "hello_world" library

            // Re-init the game
        }
        // Get the function pointer
    }
}
