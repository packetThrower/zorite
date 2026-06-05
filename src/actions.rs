//! App actions.
//!
//! `up`/`down`/`enter`/`escape` are rebound in gpui-component's `"Input"`
//! key context (after `gpui_component::init`) to the `Slash*` actions. The
//! handlers on `AppView` act only while the slash menu is open; otherwise
//! they `cx.propagate()` so the editor handles the key normally (cursor
//! move, newline, etc.). Later bindings shadow earlier ones for the same
//! context + keystroke, so ours are tried first. `tab` is likewise rebound
//! to `InsertTab` (insert two spaces in the focused editor; propagates when
//! no editor is focused) — auto-grow editors aren't gpui-component-indentable.
//!
//! `DeletePage` / `OpenInNewTab` / `RenamePage` have no keybinding —
//! they're dispatched by the sidebar's right-click context menu and
//! handled on `AppView`.

use gpui::{App, KeyBinding, actions};

actions!(
    zorite,
    [
        SlashUp, SlashDown, SlashConfirm, SlashCancel, DeletePage, OpenInNewTab, RenamePage,
        NewPage, InsertTab
    ]
);

const INPUT_CONTEXT: &str = "Input";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SlashUp, Some(INPUT_CONTEXT)),
        KeyBinding::new("down", SlashDown, Some(INPUT_CONTEXT)),
        KeyBinding::new("enter", SlashConfirm, Some(INPUT_CONTEXT)),
        KeyBinding::new("escape", SlashCancel, Some(INPUT_CONTEXT)),
        KeyBinding::new("tab", InsertTab, Some(INPUT_CONTEXT)),
    ]);
}
