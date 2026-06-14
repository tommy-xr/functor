use functor_runtime_common::{Frame, FrameTime};
use libloading::{Library, Symbol};

use crate::game::Game;

pub struct StaticGame {
    library: Library,
}

impl Game for StaticGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {
        // Noop - nothing to do
    }

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        // println!("Rendering");
        unsafe {
            let render_func: Symbol<fn(FrameTime) -> Frame> =
                self.library.get(b"test_render").unwrap();
            render_func(frame_time)
        }
    }

    fn tick(&mut self, frame_time: FrameTime) {
        unsafe {
            let tick_func: Symbol<fn(FrameTime)> = self.library.get(b"tick").unwrap();
            tick_func(frame_time)
        }
    }

    fn key_event(&mut self, code: i32, is_down: bool) {
        unsafe {
            let func: Symbol<fn(i32, bool)> = self.library.get(b"key_event").unwrap();
            func(code, is_down)
        }
    }

    fn mouse_move(&mut self, x: i32, y: i32) {
        unsafe {
            let func: Symbol<fn(i32, i32)> = self.library.get(b"mouse_move").unwrap();
            func(x, y)
        }
    }

    fn mouse_wheel(&mut self, delta: i32) {
        unsafe {
            let func: Symbol<fn(i32)> = self.library.get(b"mouse_wheel").unwrap();
            func(delta)
        }
    }

    fn state_debug(&self) -> String {
        unsafe {
            let func: Symbol<fn() -> fable_library_rust::String_::LrcStr> =
                self.library.get(b"emit_state_debug").unwrap();
            func().to_string()
        }
    }

    fn quit(&mut self) {
        // Noop - nothing to do yet
    }
}
impl StaticGame {
    pub fn create(path: &str) -> StaticGame {
        let library = unsafe {
            println!("Running initial init.");
            let lib = Library::new(path).unwrap();
            let init_func: Symbol<fn()> = lib.get(b"init").unwrap(); // Get the function pointer
            init_func();
            lib
        };

        StaticGame { library }
    }
}
