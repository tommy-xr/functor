//! The loading screen: what a producer draws while the load job evaluates a
//! project's top-level values off the frame thread ([`crate::jobs::load`]).
//! The game's `loading(progress, tts)` hook draws it when the game has one;
//! the engine's view (`loading.fun`) otherwise.

use std::rc::Rc;

use functor_lang::ir::Module;
use functor_lang::reuse::CodeIndex;
use functor_lang::{Host, HostReturn, RunError, Session, Span, Value};

use crate::functor_lang_prelude::{frame_value, FunctorHost};
use crate::ui::View;
use crate::Frame;

/// The engine's loading view, as a game: also the stand-in a desktop shell
/// runs until its game has loaded.
pub const ENGINE_VIEW: &str = include_str!("loading.fun");
pub const ENGINE_VIEW_PATH: &str = "loading.fun";

/// A loading view: a function of `(progress, tts)` returning a frame.
#[derive(Clone)]
pub struct LoadingView {
    session: Rc<Session>,
    name: String,
}

thread_local! {
    static ENGINE: Rc<Session> = {
        let project = functor_lang::project::load_sources_with_bundled_modules(
            vec![(ENGINE_VIEW_PATH.into(), ENGINE_VIEW.to_string())],
            &functor_prelude::bundled_modules(),
        )
        .unwrap_or_else(|e| panic!("the engine's loading view compiles: {}", e.render()));
        let session = Session::load(&project.module, &mut PureHost);
        Rc::new(session.unwrap_or_else(|f| panic!("the engine's loading view loads: {}", f.error.message)))
    };
}

impl LoadingView {
    /// The engine's view.
    pub fn engine() -> LoadingView {
        LoadingView { session: ENGINE.with(Rc::clone), name: "view".to_string() }
    }

    /// The game's `name` hook when `module` has one, with only what it uses
    /// evaluated; else the engine's. The hook must read nothing (files, the
    /// physics world, UI slots): it draws before the load has finished.
    pub fn of_game(module: &Module, name: &str, render: impl Fn(Span, &str) -> String) -> LoadingView {
        let Some(deps) = CodeIndex::new(module).deps_of(name) else {
            return LoadingView::engine();
        };
        match Session::load_only(module, &|def| deps.contains(def), &mut PureHost) {
            Ok(session) => LoadingView { session: Rc::new(session), name: name.to_string() },
            Err(failure) => {
                let message = render(failure.error.span, &failure.error.message);
                log::warn!("[functor-lang] `{name}` can't draw the loading screen: {message}");
                LoadingView::engine()
            }
        }
    }

    fn frame(&self, progress: f64, tts: f64) -> Result<Frame, String> {
        let args = vec![Value::Number(progress), Value::Number(tts)];
        let value = self.session.call(&self.name, args, &mut PureHost).map_err(|e| e.message)?;
        frame_value(&value)
            .cloned()
            .ok_or_else(|| format!("`{}` must return a Frame, got {}", self.name, value.kind_name()))
    }
}

/// A load's screen: its view, the progress it has shown (never less than
/// before), and the error once the load failed.
pub struct LoadScreen {
    view: LoadingView,
    shown: f64,
    failed: Option<String>,
}

impl LoadScreen {
    pub fn new(view: LoadingView) -> LoadScreen {
        LoadScreen { view, shown: 0.0, failed: None }
    }

    /// Show `latest` (the load's progress), unless it went backwards: a
    /// `Job.report` inside a def can run ahead of the defs count.
    pub fn advance(&mut self, latest: Option<f64>) {
        self.shown = self.shown.max(latest.unwrap_or(0.0));
    }

    /// What the screen shows, in 0..1.
    pub fn shown(&self) -> f64 {
        self.shown
    }

    pub fn fail(&mut self, message: String) {
        self.failed = Some(message);
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// This frame of the screen. A failed game view falls back to the
    /// engine's, once, with a warning.
    pub fn frame(&mut self, tts: f64) -> Frame {
        let progress = if self.failed.is_some() { -1.0 } else { self.shown };
        match self.view.frame(progress, tts) {
            Ok(frame) => frame,
            Err(message) => {
                log::warn!("[functor-lang] the loading screen failed, so the engine's draws it: {message}");
                self.view = LoadingView::engine();
                self.view.frame(progress, tts).expect("the engine's loading view draws")
            }
        }
    }

    /// The HUD line: the percentage, or the error.
    pub fn ui(&self) -> View {
        let (text, color) = match &self.failed {
            Some(message) => (format!("Load failed: {message}"), [235, 90, 80]),
            None => (format!("Loading {:.0}%", self.shown * 100.0), [230, 230, 235]),
        };
        View::Panel {
            anchor: crate::ui::Anchor::TopLeft,
            child: Box::new(View::Text { text, color, font: None }),
        }
    }
}

/// The host a loading view runs on: a call that reads anything is an error.
struct PureHost;

impl Host for PureHost {
    fn provides(&self, path: &str) -> bool {
        FunctorHost.provides(path)
    }

    fn call(&mut self, path: &str, args: Vec<Value>, span: Span) -> Result<HostReturn, RunError> {
        let before = crate::functor_lang_prelude::all_reads();
        let result = FunctorHost.call(path, args, span);
        if crate::functor_lang_prelude::all_reads() != before {
            return Err(RunError {
                message: format!(
                    "`{path}` reads host state, which a loading view can't: it draws before the \
load has finished — draw from `progress`, `tts` and plain values"
                ),
                span,
            });
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(src: &str) -> functor_lang::project::Project {
        functor_lang::project::load_sources_with_bundled_modules(
            vec![("game.fun".into(), src.to_string())],
            &functor_prelude::bundled_modules(),
        )
        .unwrap_or_else(|e| panic!("{}", e.render()))
    }

    #[test]
    fn the_engine_view_draws_and_its_progress_never_goes_back() {
        let mut screen = LoadScreen::new(LoadingView::engine());
        screen.advance(Some(0.5));
        screen.advance(Some(0.2));
        assert_eq!(screen.shown(), 0.5);
        let _ = screen.frame(1.0);
        screen.fail("boom".to_string());
        assert!(matches!(screen.ui(), View::Panel { child, .. } if matches!(&*child, View::Text { text, .. } if text == "Load failed: boom")));
    }

    #[test]
    fn a_game_view_that_reads_a_file_falls_back_to_the_engines() {
        let project = module(
            "let level = File.exists(Path.of(\"x.bin\"))\n\
             let loading = (p, tts) => if level then Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, 1.0), Vec3.make(0.0, 0.0, 0.0)), Scene.group([])) else Frame.create(Camera3D.lookAt(Vec3.make(0.0, 0.0, 1.0), Vec3.make(0.0, 0.0, 0.0)), Scene.group([]))\n",
        );
        let view = LoadingView::of_game(&project.module, "loading", |_, m| m.to_string());
        assert_eq!(view.name, "view", "the engine's");
        let pure = module(
            "let camera = Camera3D.lookAt(Vec3.make(0.0, 0.0, 1.0), Vec3.make(0.0, 0.0, 0.0))\n\
             let loading = (p, tts) => Frame.create(camera, Scene.group([Scene.cube() |> Scene.scale(p)]))\n",
        );
        let view = LoadingView::of_game(&pure.module, "loading", |_, m| m.to_string());
        assert_eq!(view.name, "loading");
        assert!(view.frame(0.5, 0.0).is_ok());
    }
}
