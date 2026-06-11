//! zorite — a local-first outliner and daily-journal note app, built on
//! GPUI + gpui-component with a SQLite backend. Bootstrap mirrors
//! `~/git/etch341`'s `gui::run`: register icon assets, init
//! gpui-component, pin a dark theme, rebind the outliner keys, then open
//! a single window wrapped in gpui-component's `Root`.

// On Windows release builds, don't pop a console window behind the GUI.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod actions;
mod app;
mod dates;
mod db;
mod hierarchy;
mod images;
mod import;
mod mermaid;
mod models;
mod paths;
mod pdf;
mod search;
mod settings;
mod skins;
mod slash;
mod theme;
mod ui;

use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions, px,
    size,
};
use gpui_component::{Root, TitleBar};

use app::{
    AppView, DocSignal, GlobalAppWindows, GlobalDocSignal, GlobalDraggingTab, GlobalDropTarget,
    TabKind,
};

fn main() {
    env_logger::init();

    let application = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    application.run(|cx: &mut App| {
        gpui_component::init(cx);
        // Slash-menu keys (up/down/enter/escape, gated on the menu being open)
        // plus the app-wide shortcuts (new tab/window, close tab, quit, …).
        actions::bind_keys(cx);
        // View-independent commands, handled at the App level so they work from
        // any focused window. Tab/settings commands are handled per-window on
        // `AppView`.
        cx.on_action(|_: &actions::Quit, cx: &mut App| cx.quit());
        cx.on_action(|_: &actions::NewWindow, cx: &mut App| {
            AppView::open_in_new_window(TabKind::Journal, cx);
        });
        // Native menu bar (macOS); shortcuts above also work on Windows/Linux.
        actions::set_app_menu(cx);
        // Shared cross-window save signal — every window's AppView subscribes for
        // live multi-window sync.
        let doc_signal = cx.new(|_| DocSignal);
        cx.set_global(GlobalDocSignal(doc_signal));
        // Cross-window tab dragging: the in-flight tab, and the registry of open
        // windows a tab can be dropped onto.
        cx.set_global(GlobalDraggingTab::default());
        cx.set_global(GlobalDropTarget::default());
        cx.set_global(GlobalAppWindows::default());

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        if let Err(err) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("zorite".into()),
                    ..TitleBar::title_bar_options()
                }),
                app_id: Some("zorite".into()),
                // Force client-side decorations so KWin doesn't stack a
                // server titlebar over gpui-component's. No-op on
                // macOS / Windows.
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            |window, cx| {
                // Wider resize hit-test margin for Wayland CSD.
                window.set_client_inset(px(10.0));
                let view = cx.new(|cx| AppView::new(window, cx));
                view.update(cx, |this, cx| this.attach_appearance_observer(window, cx));
                AppView::register_window(&view, window, cx);
                cx.new(|cx| Root::new(view, window, cx))
            },
        ) {
            eprintln!("zorite: failed to open window: {err}");
        }
        cx.activate(true);
    });
}
