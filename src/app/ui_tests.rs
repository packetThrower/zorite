//! Headless UI tests of chrome flows that used to be verified by hand: a real
//! `AppView` in gpui's test window, actions dispatched and keys simulated, the
//! outcome read back from the view. No pixels are inspected.
//!
//! `paths::data_dir` is a throwaway directory under `cfg(test)`, so the
//! database these open is never a real notebook.

use super::*;
use gpui::{TestAppContext, VisualTestContext};

/// Run the UI tests one at a time. They share this process's one throwaway
/// data dir (`paths::data_dir` resolves once), so two opening its database at
/// once can hit "database is locked" — and the app's DB-error dialog then
/// swallows the clicks and keys the test sends.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// What `main` sets up before the window opens, minus the platform bits
/// (assets, cursors, fonts): the widget library, the keymap, and the globals
/// `AppView::new` reads.
fn boot(cx: &mut TestAppContext) -> (Entity<AppView>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::actions::bind_keys(cx);
        let doc_signal = cx.new(|_| DocSignal);
        cx.set_global(GlobalDocSignal(doc_signal));
        cx.set_global(GlobalDraggingTab::default());
        cx.set_global(GlobalDropTarget::default());
        cx.set_global(GlobalAppWindows::default());
        cx.set_global(crate::updater::UpdateState::default());
    });
    let mut app = None;
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| AppView::new(window, cx));
        app = Some(view.clone());
        Root::new(view, window, cx)
    });
    let app = app.expect("the window builder ran");
    // Actions dispatch along the focused element's path; the app's root div
    // (where the handlers live) is focused once the user interacts, so do it.
    cx.update(|window, cx| {
        let handle = app.read(cx).focus_handle.clone();
        window.focus(&handle, cx);
    });
    cx.run_until_parked();
    (app, cx)
}

fn dialog_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| window.has_active_dialog(cx))
}

#[gpui::test]
fn command_palette_opens_closes_and_runs_a_command(cx: &mut TestAppContext) {
    let _serial = serial();
    let (app, cx) = boot(cx);

    // ⌘⇧P's action opens the palette as a dialog…
    cx.dispatch_action(OpenCommandPalette);
    cx.run_until_parked();
    assert!(dialog_open(cx), "the palette should be open");

    // …and Esc on an empty query closes it (the app's own Esc binding must
    // propagate past the note editor while a dialog is up).
    cx.simulate_keystrokes("escape");
    assert!(!dialog_open(cx), "escape should close the palette");

    // Type to filter, Enter to confirm: the dialog closes first, then the
    // command runs — here the Navigate group's "Hide sidebar".
    assert!(!cx.update(|_, cx| app.read(cx).sidebar_collapsed));
    cx.dispatch_action(OpenCommandPalette);
    cx.run_until_parked();
    cx.simulate_input("Hide sidebar");
    cx.simulate_keystrokes("enter");
    assert!(!dialog_open(cx), "confirm should close the palette");
    assert!(
        cx.update(|_, cx| app.read(cx).sidebar_collapsed),
        "the confirmed command should have run"
    );
}

#[gpui::test]
fn five_clicks_on_the_journal_tab_open_the_game(cx: &mut TestAppContext) {
    let _serial = serial();
    let (app, cx) = boot(cx);
    let tab = cx.debug_bounds("tab-0").expect("the Journal tab paints");
    // Four clicks just (re)select the journal…
    for _ in 0..4 {
        cx.simulate_click(tab.center(), gpui::Modifiers::default());
    }
    assert!(cx.update(|_, cx| app.read(cx).game.is_none()));
    // …the fifth opens the arcade. Through a real click, since the counter
    // once sat in a handler gpui-component silently ignored.
    cx.simulate_click(tab.center(), gpui::Modifiers::default());
    assert!(cx.update(|_, cx| app.read(cx).game.is_some()));
}
