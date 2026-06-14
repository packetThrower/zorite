//! Zorite — a local-first outliner and daily-journal note app, built on
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
mod whiteboard;

use std::borrow::Cow;

use gpui::{
    App, AppContext, AssetSource, Bounds, Result, SharedString, TitlebarOptions, WindowBounds,
    WindowDecorations, WindowOptions, px, size,
};
use gpui_component::{Root, TitleBar};

/// The app's asset source: serves our bundled custom icons (Lucide faces not in
/// the gpui-component set) and delegates everything else to gpui-component's
/// embedded assets.
struct Assets;

// Lucide faces not packaged by gpui-component, served at the same
// `icons/<name>.svg` scheme so [`gpui_component::Icon::path`] can use them.
const CLIPBOARD_PLUS: &[u8] = include_bytes!("../assets/icons/clipboard-plus.svg");
const STICKY_NOTE_PLUS: &[u8] = include_bytes!("../assets/icons/sticky-note-plus.svg");

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // Icons compiled into the binary (the ones we actually ship).
        let custom = match path {
            "icons/clipboard-plus.svg" => Some(CLIPBOARD_PLUS),
            "icons/sticky-note-plus.svg" => Some(STICKY_NOTE_PLUS),
            _ => None,
        };
        if let Some(bytes) = custom {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        let delegated = gpui_component_assets::Assets.load(path);
        if matches!(delegated, Ok(Some(_))) {
            return delegated;
        }
        // Dev convenience: serve any Lucide icon from the on-disk set fetched by
        // `scripts/fetch-lucide.sh`, so new faces can be tried without bundling.
        // Debug builds only — release ships just the embedded + compiled-in icons.
        #[cfg(debug_assertions)]
        if let Some(name) = path.strip_prefix("icons/") {
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/icons/lucide")
                .join(name);
            if let Ok(bytes) = std::fs::read(p) {
                return Ok(Some(Cow::Owned(bytes)));
            }
        }
        delegated
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_component_assets::Assets.list(path)
    }
}

use app::{
    AppView, DocSignal, GlobalAppWindows, GlobalDocSignal, GlobalDraggingTab, GlobalDropTarget,
    TabKind,
};

fn main() {
    env_logger::init();

    let application = gpui_platform::application().with_assets(Assets);
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
                    title: Some("Zorite".into()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource;

    #[test]
    fn asset_source_serves_bundled_and_dev_icons() {
        // The icons compiled into the binary always resolve.
        assert!(Assets.load("icons/clipboard-plus.svg").unwrap().is_some());
        assert!(Assets.load("icons/sticky-note-plus.svg").unwrap().is_some());
        // The debug-only disk fallback serves any Lucide icon once
        // `scripts/fetch-lucide.sh` has populated the set (skipped if not, e.g. CI).
        // `map-pin` is in Lucide but not the embedded subset, so it exercises it.
        let fetched = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/icons/lucide/map-pin.svg")
            .exists();
        if fetched {
            assert!(Assets.load("icons/map-pin.svg").unwrap().is_some());
        }
    }
}
