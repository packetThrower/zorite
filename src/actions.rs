//! Keyboard actions for the slash-command menu.
//!
//! `up`/`down`/`enter`/`escape` are rebound in gpui-component's `"Input"`
//! key context (after `gpui_component::init`) to these actions. The
//! handlers on `AppView` act only while the slash menu is open; otherwise
//! they `cx.propagate()` so the editor handles the key normally (cursor
//! move, newline, etc.). Later bindings shadow earlier ones for the same
//! context + keystroke, so ours are tried first.

use gpui::{App, KeyBinding, actions};

actions!(rumin, [SlashUp, SlashDown, SlashConfirm, SlashCancel]);

const INPUT_CONTEXT: &str = "Input";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SlashUp, Some(INPUT_CONTEXT)),
        KeyBinding::new("down", SlashDown, Some(INPUT_CONTEXT)),
        KeyBinding::new("enter", SlashConfirm, Some(INPUT_CONTEXT)),
        KeyBinding::new("escape", SlashCancel, Some(INPUT_CONTEXT)),
    ]);
}
