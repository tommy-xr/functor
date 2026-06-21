namespace Functor

open Fable.Core
open Functor.Math

/// A declarative 2D UI tree — the value returned by a game's `ui : 'model -> View`
/// projection (the 2D sibling of `draw3d`). It is a thin shim over the Rust
/// `functor_runtime_common::ui::View`; the runtime lowers it to a text overlay
/// drawn on top of the 3D frame. egui lives entirely in the runtime shell — game
/// code only ever builds this declarative tree.
[<Erase; Emit("functor_runtime_common::ui::View")>]
type View = | Noop

/// Screen corner a `Ui.panel` pins its subtree to.
[<Erase; Emit("functor_runtime_common::ui::Anchor")>]
type Anchor = | Noop

module Ui =

    /// An empty view — renders nothing. The default `ui` for games with no HUD.
    [<Emit("functor_runtime_common::ui::View::empty()")>]
    let none (): View = nativeOnly

    /// A line of white text.
    [<Emit("functor_runtime_common::ui::View::text($0)")>]
    let text (s: string): View = nativeOnly

    [<Emit("functor_runtime_common::ui::View::text_color($0, $1, $2, $3)")>]
    let private textColorRaw (r: float32) (g: float32) (b: float32) (s: string): View = nativeOnly

    /// A line of text in `color`.
    let textColor (color: Color) (s: string): View = textColorRaw color.r color.g color.b s

    /// A line of text in a named font `family` at `size` points. Forward-looking:
    /// only the default font renders today, but the family is recorded so games
    /// can author against it before custom fonts are wired into the runtime.
    [<Emit("functor_runtime_common::ui::View::text_font($0, $1, $2)")>]
    let textFont (family: string) (size: float32) (s: string): View = nativeOnly

    /// Stack children top-to-bottom.
    [<Emit("functor_runtime_common::ui::View::column($0)")>]
    let column (items: View[]): View = nativeOnly

    /// Lay children out left-to-right.
    [<Emit("functor_runtime_common::ui::View::row($0)")>]
    let row (items: View[]): View = nativeOnly

    /// Pin a subtree to a screen corner (see `Ui.topLeft` etc.).
    [<Emit("functor_runtime_common::ui::View::panel($0, $1)")>]
    let panel (anchor: Anchor) (child: View): View = nativeOnly

    [<Emit("functor_runtime_common::ui::Anchor::TopLeft")>]
    let topLeft (): Anchor = nativeOnly

    [<Emit("functor_runtime_common::ui::Anchor::TopRight")>]
    let topRight (): Anchor = nativeOnly

    [<Emit("functor_runtime_common::ui::Anchor::BottomLeft")>]
    let bottomLeft (): Anchor = nativeOnly

    [<Emit("functor_runtime_common::ui::Anchor::BottomRight")>]
    let bottomRight (): Anchor = nativeOnly
