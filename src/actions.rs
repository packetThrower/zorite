//! Outliner key actions and the rebinding that lets them fire while a
//! block's text input is focused.
//!
//! gpui-component's `Input` widget claims `tab` / `shift-tab` (inline
//! indent) and `up` / `down` (cursor moves) in its own `"Input"` key
//! context. A single-line outliner block has no use for those, so we
//! rebind those keystrokes — in the *same* `"Input"` context, *after*
//! `gpui_component::init` — to our block-level actions. Later bindings
//! shadow earlier ones for the same context + keystroke, and these
//! actions are handled on the `AppView` root (an ancestor of every
//! block input), so they win whenever a block editor is focused.

use gpui::{App, KeyBinding, actions};

actions!(rumin, [Indent, Outdent, FocusUp, FocusDown]);

/// gpui-component's input key context. Must match the string the
/// widget sets internally (`input::state::CONTEXT`).
const INPUT_CONTEXT: &str = "Input";

/// Rebind the outliner keys. Call once, after `gpui_component::init`.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", Indent, Some(INPUT_CONTEXT)),
        KeyBinding::new("shift-tab", Outdent, Some(INPUT_CONTEXT)),
        KeyBinding::new("up", FocusUp, Some(INPUT_CONTEXT)),
        KeyBinding::new("down", FocusDown, Some(INPUT_CONTEXT)),
    ]);
}
