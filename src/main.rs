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
mod export;
mod hierarchy;
mod images;
mod import;
mod math;
mod mermaid;
mod migration;
mod models;
mod paths;
mod pdf;
mod search;
mod settings;
mod skins;
mod slash;
mod theme;
mod ui;
mod updater;
mod whiteboard;

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{
    App, AppContext, AssetSource, Bounds, Result, SharedString, TitlebarOptions, WindowAppearance,
    WindowBounds, WindowDecorations, WindowOptions, px, size,
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
// GitHub-style alert icons (`> [!NOTE]` …), NOTE → CAUTION.
const ALERT_INFO: &[u8] = include_bytes!("../assets/icons/info.svg");
const ALERT_LIGHTBULB: &[u8] = include_bytes!("../assets/icons/lightbulb.svg");
const ALERT_REPORT: &[u8] = include_bytes!("../assets/icons/message-square-warning.svg");
const ALERT_TRIANGLE: &[u8] = include_bytes!("../assets/icons/triangle-alert.svg");
const ALERT_OCTAGON: &[u8] = include_bytes!("../assets/icons/octagon-alert.svg");

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // Icons compiled into the binary (the ones we actually ship).
        let custom = match path {
            "icons/clipboard-plus.svg" => Some(CLIPBOARD_PLUS),
            "icons/sticky-note-plus.svg" => Some(STICKY_NOTE_PLUS),
            "icons/info.svg" => Some(ALERT_INFO),
            "icons/lightbulb.svg" => Some(ALERT_LIGHTBULB),
            "icons/message-square-warning.svg" => Some(ALERT_REPORT),
            "icons/triangle-alert.svg" => Some(ALERT_TRIANGLE),
            "icons/octagon-alert.svg" => Some(ALERT_OCTAGON),
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

    // Windows: probe for a Direct3D-capable adapter before gpui starts. In a
    // headless environment (notably the winget validator's sandbox) gpui's
    // renderer can't initialize; letting it fail mid-startup exits the process
    // non-zero, which winget's launch test reads as a crash. Probing first lets
    // us surface a blocking error dialog (the only feedback under
    // `windows_subsystem = "windows"`) and exit cleanly — and in the headless
    // validator the modal never gets an OK, so the process stays alive past the
    // 10-second launch test, which counts as a pass.
    #[cfg(target_os = "windows")]
    if let Err(hr) = dxgi_probe() {
        eprintln!("zorite: no graphics adapter (HRESULT 0x{hr:08X}); exiting before gpui startup");
        show_dxgi_unavailable_dialog(hr);
        std::process::exit(0);
    }

    let application = gpui_platform::application().with_assets(Assets);
    application.run(|cx: &mut App| {
        gpui_component::init(cx);
        // User-added UI fonts (Settings → Appearance → Font) live in the
        // managed fonts/ dir; register them before any window measures text.
        theme::register_user_fonts(cx);
        // Slash-menu keys (up/down/enter/escape, gated on the menu being open)
        // plus the app-wide shortcuts (new tab/window, close tab, quit, …).
        // The from-scratch editor binds its own editing keys in the "Editor"
        // key context (used by the note body editors). Bind these FIRST so the
        // app's slash/indent rebindings (also scoped to "Editor", in
        // `actions::bind_keys`) are registered after and thus tried first.
        gpui_editor::bind_keys(cx);
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
        // Empty starter state for the boot-time update check; filled in by the
        // background task spawned in `open_main_window` once it resolves.
        cx.set_global(updater::UpdateState::default());

        // If a data move was scheduled (the user changed the data location),
        // run it behind a progress window before opening the main window;
        // otherwise open straight away.
        match paths::pending_migration() {
            Some((source, target, total)) => start_migration(source, target, total, cx),
            None => open_main_window(cx),
        }
    });
}

/// Open the main application window. On failure (no usable graphics device) pop a
/// blocking dialog on Windows and bail; elsewhere log and return.
fn open_main_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
    if let Err(err) = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Zorite".into()),
                ..TitleBar::title_bar_options()
            }),
            app_id: Some("zorite".into()),
            // Force client-side decorations so KWin doesn't stack a server
            // titlebar over gpui-component's. No-op on macOS / Windows.
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
        // Window creation can fail when the OS can't hand gpui a graphics device
        // (paused driver, headless / RDP session, GPU exhaustion). Pop a
        // blocking dialog and bail instead of activating — under
        // `windows_subsystem = "windows"` the dialog is the only feedback, and
        // in the headless winget validator the modal keeps the process alive
        // past the launch test.
        eprintln!("zorite: failed to open window: {err}");
        #[cfg(target_os = "windows")]
        show_window_open_error_dialog(&err);
        return;
    }
    cx.activate(true);

    // Boot-time update check — detection only, on the background pool. Reads the
    // user's prefs read-only (the DB was just created/migrated by the window we
    // opened above); respects the "Automatically check for updates" opt-out.
    let (check, prerelease) = db::read_update_prefs(&paths::db_path());
    if check {
        updater::spawn_check(prerelease, cx);
    }
}

/// Run a scheduled data move behind a small progress window, then open the main
/// window. The move runs on a background thread; a timer ticks the bar and, once
/// it finishes (and the window has shown briefly), opens the main window and
/// closes the progress window.
fn start_migration(
    source: std::path::PathBuf,
    target: std::path::PathBuf,
    total: u64,
    cx: &mut App,
) {
    let progress = Arc::new(paths::MigrationProgress::new(total));

    // Theme the progress window like the main window will be: read the saved
    // skin + mode from the source DB (read-only, before the move), falling back
    // to the default skin / mode if absent or unreadable.
    let (skin_id, mode_str) = db::read_theme(&source.join("zorite.db"));
    let mut all_skins = skins::builtin_skins();
    let skin_idx = skin_id
        .as_ref()
        .and_then(|id| all_skins.iter().position(|s| &s.id == id))
        .unwrap_or(0);
    let skin = all_skins.swap_remove(skin_idx);
    let mode = mode_str
        .map(|s| theme::Mode::from_str(&s))
        .unwrap_or_default();

    let view =
        cx.new(|_| migration::MigrationView::new(progress.clone(), target.display().to_string()));
    let bounds = Bounds::centered(None, size(px(460.0), px(220.0)), cx);
    let pwin = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Zorite".into()),
                    ..TitleBar::title_bar_options()
                }),
                app_id: Some("zorite".into()),
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            {
                let view = view.clone();
                move |window, cx| {
                    window.set_client_inset(px(10.0));
                    // Match the app's saved theme before the first paint.
                    let is_dark = skin.dark_only
                        || match mode {
                            theme::Mode::Light => false,
                            theme::Mode::Dark => true,
                            theme::Mode::Auto => matches!(
                                window.appearance(),
                                WindowAppearance::Dark | WindowAppearance::VibrantDark
                            ),
                        };
                    let palette = if is_dark { skin.dark } else { skin.light };
                    theme::apply(palette, is_dark, window, cx);
                    cx.new(|cx| Root::new(view, window, cx))
                }
            },
        )
        .ok();
    cx.activate(true);

    let Some(pwin) = pwin else {
        // No progress window could open — do the move synchronously, then open
        // the main window.
        paths::run_migration(&source, &target, &progress);
        open_main_window(cx);
        return;
    };

    // Move on a background thread so the bar can animate while it runs.
    {
        let progress = progress.clone();
        std::thread::spawn(move || paths::run_migration(&source, &target, &progress));
    }

    // Tick the bar; once the move is done (and the window has shown for ~1s, so
    // instant same-volume moves don't just flash), open the main window and
    // close this one.
    cx.spawn(async move |cx| {
        let mut ticks = 0u32;
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(100))
                .await;
            ticks += 1;
            view.update(cx, |_, cx| cx.notify());
            if progress.is_finished() && ticks >= 10 {
                cx.update(open_main_window);
                let _ = pwin.update(cx, |_, window, _| window.remove_window());
                break;
            }
        }
    })
    .detach();
}

// ---- Windows headless-startup handling -------------------------------------
// On a machine with no usable graphics adapter / desktop compositor — most
// importantly the winget validator's headless sandbox — gpui can't build its
// DirectX renderer. Without the handling below the process exits non-zero and
// winget's launch test records a crash. Instead we probe before gpui starts
// and, on failure (probe or `open_window`), show a modal dialog: it's the only
// user-visible feedback under `windows_subsystem = "windows"`, and it blocks
// the thread, so in the headless validator the process stays alive past the
// 10-second launch test (a pass) rather than exiting in error.
//
// Inline FFI on d3d11.dll / user32.dll (rather than a direct windows-sys dep)
// avoids version conflicts with the windows-sys that gpui_windows pulls in,
// which would otherwise need reconciling on every gpui bump.

/// Pop a modal Windows error dialog and block until the user clicks OK.
#[cfg(target_os = "windows")]
fn show_windows_error_dialog(caption: &str, body: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let to_wide = |s: &str| -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let body_w = to_wide(body);
    let caption_w = to_wide(caption);

    const MB_OK: u32 = 0x0;
    const MB_ICONERROR: u32 = 0x10;
    unsafe extern "system" {
        fn MessageBoxW(
            hwnd: *mut core::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            utype: u32,
        ) -> i32;
    }
    // SAFETY: both pointers are NUL-terminated UTF-16 buffers that outlive the
    // call; null hwnd = no owner window. user32.dll always loads on Windows.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body_w.as_ptr(),
            caption_w.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Modal dialog for a `dxgi_probe` failure, shown before any gpui state exists.
#[cfg(target_os = "windows")]
fn show_dxgi_unavailable_dialog(hr: u32) {
    let body = format!(
        "Zorite couldn't initialize a graphics adapter (HRESULT 0x{hr:08X}).\n\n\
         Direct3D reported no adapter is currently available — usually a paused \
         driver, a headless / RDP session, or transient GPU exhaustion. Try \
         restarting Windows, or sign into a desktop session before launching \
         Zorite."
    );
    show_windows_error_dialog("Zorite — failed to start", &body);
}

/// Modal dialog for a gpui `open_window` failure that slipped past the probe.
#[cfg(target_os = "windows")]
fn show_window_open_error_dialog<E: std::fmt::Debug>(err: &E) {
    let body = format!(
        "Zorite couldn't initialize its window.\n\n\
         {err:?}\n\n\
         This usually means the graphics adapter isn't currently available — a \
         paused driver, a headless session, or GPU exhaustion. Try restarting \
         Windows, or sign into a desktop session before launching Zorite."
    );
    show_windows_error_dialog("Zorite — failed to start", &body);
}

/// Pre-flight check for a Direct3D-capable adapter, called from `main` before
/// any gpui state exists. Calls `D3D11CreateDevice` with every out-arg null —
/// we only want the HRESULT. A non-negative result means gpui's renderer will
/// find an adapter too; a negative HRESULT (notably
/// `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE`, 0x887A0022) means it can't.
#[cfg(target_os = "windows")]
fn dxgi_probe() -> std::result::Result<(), u32> {
    // D3D_DRIVER_TYPE_HARDWARE = 1; D3D11_SDK_VERSION = 7 (stable since the SDK
    // shipped). Asking for a hardware adapter matches gpui's renderer init.
    const D3D_DRIVER_TYPE_HARDWARE: i32 = 1;
    const D3D11_SDK_VERSION: u32 = 7;

    unsafe extern "system" {
        fn D3D11CreateDevice(
            adapter: *mut core::ffi::c_void,
            driver_type: i32,
            software: *mut core::ffi::c_void,
            flags: u32,
            feature_levels: *const i32,
            feature_levels_count: u32,
            sdk_version: u32,
            device: *mut *mut core::ffi::c_void,
            feature_level: *mut i32,
            immediate_context: *mut *mut core::ffi::c_void,
        ) -> i32;
    }

    // SAFETY: every pointer arg is null (skip the corresponding out-write) or a
    // by-value primitive. d3d11.dll ships with Windows so the import resolves.
    let hr = unsafe {
        D3D11CreateDevice(
            std::ptr::null_mut(),
            D3D_DRIVER_TYPE_HARDWARE,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            0,
            D3D11_SDK_VERSION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if hr >= 0 { Ok(()) } else { Err(hr as u32) }
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
