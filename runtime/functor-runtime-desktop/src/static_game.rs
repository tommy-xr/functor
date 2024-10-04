use functor_runtime_common::{FrameTime, Scene3D};
use libloading::{Library, Symbol};

use crate::game::Game;

pub struct StaticGame {
    library: Library,
}

impl Game for StaticGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {
        // Noop - nothing to do
    }

    fn render(&mut self, frame_time: FrameTime) -> Scene3D {
        // println!("Rendering");
        unsafe {
            let render_func: Symbol<fn(FrameTime) -> Scene3D> =
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
