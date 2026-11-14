//! A hidden run on macOS is a background process: no Dock tile, no menu
//! bar, never the active app, so Quit reaches the app that started it. Also
//! when it is a `.app`'s own executable, whose Info.plist would make it a
//! regular app (D40).

/// Before GLFW starts, on the main thread. GLFW turns the process into a
/// regular app unless its menu bar is off.
pub fn become_background() {
    glfw::init_hint(glfw::InitHint::CocoaMenubar(false));
    let Some(main) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(main);
    app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Prohibited);
}
