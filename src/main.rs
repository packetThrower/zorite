//! rumin — a local-first outliner and daily-journal note app, built on
//! GPUI + gpui-component with a SQLite backend. Bootstrap mirrors
//! `~/git/etch341`'s `gui::run`: register icon assets, init
//! gpui-component, pin a dark theme, rebind the outliner keys, then open
//! a single window wrapped in gpui-component's `Root`.

// On Windows release builds, don't pop a console window behind the GUI.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod db;
mod models;
mod paths;
mod theme;
mod ui;

use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions, px,
    size,
};
use gpui_component::{Root, Theme, ThemeMode, TitleBar};

use app::AppView;

fn main() {
    env_logger::init();

    let application = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    application.run(|cx: &mut App| {
        gpui_component::init(cx);
        // Dark-only chrome, like etch341 — pin it regardless of OS
        // appearance so embedded gpui-component widgets don't paint light.
        Theme::change(ThemeMode::Dark, None, cx);
        theme::apply_accent_to_component_theme(cx);

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        if let Err(err) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("rumin".into()),
                    ..TitleBar::title_bar_options()
                }),
                app_id: Some("rumin".into()),
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
                cx.new(|cx| Root::new(view, window, cx))
            },
        ) {
            eprintln!("rumin: failed to open window: {err}");
        }
        cx.activate(true);
    });
}
