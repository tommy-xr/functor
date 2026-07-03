use notify::{event, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::{fs, process};
use tempfile::tempdir;

use functor_runtime_common::{Frame, FrameTime, OpaqueState};
use libloading::{Library, Symbol};

use crate::game::Game;

pub struct HotReloadGame {
    // Utils for constructing the next lib path
    file_stem: String,
    extension: String,
    counter: u32,

    // Current hot reload state
    latest_lib_path: Arc<String>,
    library: Option<Library>,

    // Hot reload state
    file_changed: Arc<AtomicBool>,
    watcher_thread: Option<JoinHandle<()>>,
}

impl Game for HotReloadGame {
    fn check_hot_reload(&mut self, frame_time: FrameTime) {
        if self.file_changed.load(Ordering::SeqCst) {
            println!("Reloading!");
            self.file_changed.store(false, Ordering::SeqCst);
            self.reload();
            println!("Rendering: {:?}", self.render(frame_time.clone()));
        }
    }

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        unsafe {
            let render_func: Symbol<fn(FrameTime) -> Frame> =
                self.library.as_ref().unwrap().get(b"test_render").unwrap();
            render_func(frame_time)
        }
    }

    fn ui(&self) -> functor_runtime_common::ui::View {
        unsafe {
            let ui_func: Symbol<fn() -> functor_runtime_common::ui::View> =
                self.library.as_ref().unwrap().get(b"emit_ui").unwrap();
            ui_func()
        }
    }

    fn tick(&mut self, frame_time: FrameTime) {
        unsafe {
            let tick_func: Symbol<fn(FrameTime)> =
                self.library.as_ref().unwrap().get(b"tick").unwrap();
            tick_func(frame_time)
        }
    }

    fn key_event(&mut self, code: i32, is_down: bool) {
        unsafe {
            let func: Symbol<fn(i32, bool)> =
                self.library.as_ref().unwrap().get(b"key_event").unwrap();
            func(code, is_down)
        }
    }

    fn mouse_move(&mut self, x: i32, y: i32) {
        unsafe {
            let func: Symbol<fn(i32, i32)> =
                self.library.as_ref().unwrap().get(b"mouse_move").unwrap();
            func(x, y)
        }
    }

    fn mouse_wheel(&mut self, delta: i32) {
        unsafe {
            let func: Symbol<fn(i32)> = self.library.as_ref().unwrap().get(b"mouse_wheel").unwrap();
            func(delta)
        }
    }

    fn state_debug(&self) -> String {
        unsafe {
            let func: Symbol<fn() -> fable_library_rust::String_::LrcStr> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"emit_state_debug")
                .unwrap();
            func().to_string()
        }
    }

    fn net_drain_commands(&self) -> String {
        unsafe {
            let func: Symbol<fn() -> fable_library_rust::String_::LrcStr> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"net_drain_commands_json")
                .unwrap();
            func().to_string()
        }
    }

    fn net_push_http_response(&mut self, token: i32, status: i32, body: String) {
        unsafe {
            let func: Symbol<fn(i32, i32, fable_library_rust::String_::LrcStr)> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"net_push_http_response")
                .unwrap();
            func(token, status, fable_library_rust::String_::fromString(body))
        }
    }

    fn net_push_http_error(&mut self, token: i32, message: String) {
        unsafe {
            let func: Symbol<fn(i32, fable_library_rust::String_::LrcStr)> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"net_push_http_error")
                .unwrap();
            func(token, fable_library_rust::String_::fromString(message))
        }
    }

    fn audio_drain_commands(&self) -> String {
        unsafe {
            let func: Symbol<fn() -> fable_library_rust::String_::LrcStr> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"audio_drain_commands_json")
                .unwrap();
            func().to_string()
        }
    }

    fn audio_scene_json(&self) -> String {
        unsafe {
            let func: Symbol<fn() -> fable_library_rust::String_::LrcStr> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"audio_scene_json")
                .unwrap();
            func().to_string()
        }
    }

    fn net_drain_conn_commands(&self) -> String {
        unsafe {
            let func: Symbol<fn() -> fable_library_rust::String_::LrcStr> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"net_drain_conn_commands_json")
                .unwrap();
            func().to_string()
        }
    }

    fn net_push_connected(&mut self, key: String, conn: i32) {
        unsafe {
            let func: Symbol<fn(fable_library_rust::String_::LrcStr, i32)> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"net_push_connected")
                .unwrap();
            func(fable_library_rust::String_::fromString(key), conn)
        }
    }

    fn net_push_conn_message(&mut self, key: String, conn: i32, text: String) {
        unsafe {
            let func: Symbol<
                fn(fable_library_rust::String_::LrcStr, i32, fable_library_rust::String_::LrcStr),
            > = self
                .library
                .as_ref()
                .unwrap()
                .get(b"net_push_conn_message")
                .unwrap();
            func(
                fable_library_rust::String_::fromString(key),
                conn,
                fable_library_rust::String_::fromString(text),
            )
        }
    }

    fn net_push_disconnected(&mut self, key: String, conn: i32) {
        unsafe {
            let func: Symbol<fn(fable_library_rust::String_::LrcStr, i32)> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"net_push_disconnected")
                .unwrap();
            func(fable_library_rust::String_::fromString(key), conn)
        }
    }

    fn net_push_conn_error(&mut self, key: String, conn: i32, message: String) {
        unsafe {
            let func: Symbol<
                fn(fable_library_rust::String_::LrcStr, i32, fable_library_rust::String_::LrcStr),
            > = self
                .library
                .as_ref()
                .unwrap()
                .get(b"net_push_conn_error")
                .unwrap();
            func(
                fable_library_rust::String_::fromString(key),
                conn,
                fable_library_rust::String_::fromString(message),
            )
        }
    }

    fn audio_push_finished(&mut self, token: i32) {
        unsafe {
            let func: Symbol<fn(i32)> = self
                .library
                .as_ref()
                .unwrap()
                .get(b"audio_push_finished")
                .unwrap();
            func(token)
        }
    }

    fn quit(&mut self) {
        if let Some(handle) = self.watcher_thread.take() {
            handle.join().expect("Failed to join watcher thread");
        }
    }
}
impl HotReloadGame {
    pub fn create(path: &str) -> HotReloadGame {
        let path = Arc::new(path.to_string());
        let counter = 0;
        let lib_path = Path::new(path.as_str());
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

        let library = unsafe {
            println!("Running initial init.");
            let lib = Library::new(path.as_str()).unwrap();
            let init_func: Symbol<fn()> = lib.get(b"init").unwrap(); // Get the function pointer
            init_func();
            Some(lib)
        };

        let file_changed = Arc::new(AtomicBool::new(false));
        let file_changed_watcher = Arc::clone(&file_changed);
        let owned_game_path = path.clone();

        let watcher_thread = std::thread::spawn(move || {
            // Select recommended watcher for debouncer.
            // Using a callback here, could also be a channel.

            let (tx, rx) = std::sync::mpsc::channel();
            let mut watcher = notify::recommended_watcher(tx).unwrap();

            let mut had_remove_event = false;

            let path = Path::new(owned_game_path.as_str());
            watcher.watch(&path, RecursiveMode::Recursive).unwrap();

            println!("watcher created!");
            loop {
                match rx.recv() {
                    Ok(event) => {
                        match event {
                            Ok(event) => {
                                match event.kind {
                                    event::EventKind::Remove(_) => had_remove_event = true,
                                    event::EventKind::Create(_) => {
                                        if had_remove_event {
                                            had_remove_event = false;
                                            println!("Pushing hot reload event from thread...");
                                            file_changed_watcher.store(true, Ordering::SeqCst);
                                        } else {
                                            println!("ignoring event");
                                        }
                                    }
                                    _ => (),
                                };
                                // TODO: Can we parse events here to handle create -> restore loop?
                                println!("event: {:?}", event);
                                //file_changed_watcher.store(true, Ordering::SeqCst);
                            }
                            Err(e) => println!("watch error: {:?}", e),
                        }
                    }
                    Err(e) => println!("watch error: {:?}", e),
                }
            }
        });

        HotReloadGame {
            file_stem,
            extension,
            counter,

            latest_lib_path: path,
            library,

            watcher_thread: Some(watcher_thread),
            file_changed,
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

            let mut maybe_previous_state: Option<OpaqueState> = None;

            if let Some(lib) = library.take() {
                println!("Saving previous state...");
                let get_state_fn: Symbol<fn() -> OpaqueState> = lib.get(b"emit_state").unwrap(); // Get the function pointer
                maybe_previous_state = Some(get_state_fn());
                lib.close().unwrap();
            }

            let temp_dir = tempdir().expect("Failed to create temporary directory");
            println!("Copying to: {}", &temp_dir.path().to_str().unwrap());
            let new_file_name = temp_dir.path().join(self.get_next_destination_name());
            // TODO: Better definition of path here - don't hardcode target/debug
            let source_path = &self.latest_lib_path;
            fs::copy(source_path.as_str(), &new_file_name).expect("Cmtpy should succeed");
            println!("Loading from: {:?}", &new_file_name);

            let lib = Library::new(&new_file_name).unwrap();

            let init_func: Symbol<fn()> = lib.get(b"init").unwrap(); // Get the function pointer
            println!("Running init after reload.");
            init_func();

            if let Some(previous_state) = maybe_previous_state {
                println!("Rehydrating state");
                let set_state_fn: Symbol<fn(OpaqueState) -> ()> = lib.get(b"set_state").unwrap(); // Get the function pointer
                set_state_fn(previous_state);

                // Run a tick
                let tick_fn: Symbol<fn() -> ()> = lib.get(b"tick").unwrap();
                tick_fn();
            }

            // Get state and set state

            self.library = Some(lib); // Load the "hello_world" library

            // Re-init the game
        }
        // Get the function pointer
    }
}
