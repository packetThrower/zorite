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
//! `DeletePage` / `OpenInNewTab` / `OpenInNewWindow` / `RenamePage` /
//! `ToggleFavorite` have no keybinding — they're dispatched by right-click
//! context menus (sidebar pages and tabs) and handled on `AppView`.

use gpui::{App, KeyBinding, Menu, MenuItem, actions};

actions!(
    zorite,
    [
        SlashUp,
        SlashDown,
        SlashConfirm,
        SlashCancel,
        DeletePage,
        OpenInNewTab,
        OpenInNewWindow,
        RenamePage,
        // Sidebar right-click → pin/unpin a page to the "Favorites" group.
        ToggleFavorite,
        // Right-click → Edit on a rendered page/journal day → enter edit mode.
        EditNote,
        NewPage,
        // File menu: create + open a new whiteboard canvas (no keybinding).
        NewWhiteboard,
        InsertTab,
        Outdent,
        PasteImage,
        // App-wide shortcuts / menu commands (bound in `bind_keys`, surfaced in
        // `set_app_menu`). `NewPage` doubles as "New Tab".
        NewWindow,
        CloseTab,
        NextTab,
        PrevTab,
        OpenSettings,
        Quit,
        // Find: in the current page's rendered text, or the global note search.
        FindInPage,
        GlobalSearch,
        // File menu: import a Logseq graph folder (no keybinding).
        ImportLogseq,
        // Shrink any image wider than the content area back to fit the view.
        FitImages
    ]
);

const INPUT_CONTEXT: &str = "Input";
/// Key context of our gpui-editor body editors (matches `gpui_editor`'s own).
const EDITOR_CONTEXT: &str = "Editor";

pub fn bind_keys(cx: &mut App) {
    // Single-line gpui-component fields (title, alias, find bar, search, dialogs)
    // run in the "Input" context.
    cx.bind_keys([
        KeyBinding::new("up", SlashUp, Some(INPUT_CONTEXT)),
        KeyBinding::new("down", SlashDown, Some(INPUT_CONTEXT)),
        KeyBinding::new("enter", SlashConfirm, Some(INPUT_CONTEXT)),
        KeyBinding::new("escape", SlashCancel, Some(INPUT_CONTEXT)),
        KeyBinding::new("tab", InsertTab, Some(INPUT_CONTEXT)),
        // Shift+Tab outdents the caret's list line (no-op if nothing to remove).
        KeyBinding::new("shift-tab", Outdent, Some(INPUT_CONTEXT)),
    ]);
    // The note body editors run on gpui-editor (key context "Editor"), which
    // binds its own up/down/enter/escape. Rebind the same keys to the slash /
    // indent actions so the menu, Tab, and Esc work there too. `gpui_editor::
    // bind_keys` runs first (see `main`), so these are tried first and the
    // handlers `cx.propagate()` to fall through to the editor when not consumed.
    cx.bind_keys([
        KeyBinding::new("up", SlashUp, Some(EDITOR_CONTEXT)),
        KeyBinding::new("down", SlashDown, Some(EDITOR_CONTEXT)),
        KeyBinding::new("enter", SlashConfirm, Some(EDITOR_CONTEXT)),
        KeyBinding::new("escape", SlashCancel, Some(EDITOR_CONTEXT)),
        KeyBinding::new("tab", InsertTab, Some(EDITOR_CONTEXT)),
        KeyBinding::new("shift-tab", Outdent, Some(EDITOR_CONTEXT)),
    ]);
    // Paste-image: bind the platform's real paste chord — Cmd+V on macOS, Ctrl+V on
    // Windows/Linux. gpui treats `cmd-` and `ctrl-` as distinct chords, so a bare
    // `cmd-v` binding never fires off-Mac and image paste would be dead there. The
    // handler checks the clipboard for an image and otherwise propagates to
    // gpui-component's native text paste, so binding the real chord is safe.
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-v", PasteImage, Some(INPUT_CONTEXT)),
        KeyBinding::new("cmd-v", PasteImage, Some(EDITOR_CONTEXT)),
    ]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-v", PasteImage, Some(INPUT_CONTEXT)),
        KeyBinding::new("ctrl-v", PasteImage, Some(EDITOR_CONTEXT)),
    ]);

    // App-wide shortcuts. `secondary-` resolves to Cmd on macOS and Ctrl on
    // Windows/Linux, so one binding is correct on every OS. No key context →
    // they fire whether or not an editor is focused; every chord uses a modifier
    // so none collide with text input. Handlers: tab/settings actions on
    // `AppView`; `NewWindow` / `Quit` as global App actions (see `main`).
    cx.bind_keys([
        KeyBinding::new("secondary-t", NewPage, None), // New Tab == new page
        KeyBinding::new("secondary-n", NewWindow, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        // Find-in-page (a Page tab's rendered text) vs the global note search.
        // PDFs keep their own ⌘F (handled in the viewer); FindInPage no-ops there.
        KeyBinding::new("secondary-f", FindInPage, None),
        KeyBinding::new("secondary-shift-f", GlobalSearch, None),
        // Fit over-wide images back into the page / journal view.
        KeyBinding::new("secondary-shift-i", FitImages, None),
    ]);
}

/// Install the application menu bar. Native on macOS; on Windows/Linux the menus
/// are stored (no native bar yet) but the same `bind_keys` chords drive every
/// command, so shortcuts work regardless. Each item's accelerator is read from
/// the keymap, so this must run *after* [`bind_keys`]. The Edit items reuse
/// gpui-component's input actions, which it already binds in focused editors.
pub fn set_app_menu(cx: &mut App) {
    use gpui_component::input;
    cx.set_menus([
        Menu {
            name: "Zorite".into(),
            items: vec![
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit Zorite", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Tab", NewPage),
                MenuItem::action("New Whiteboard", NewWhiteboard),
                MenuItem::action("New Window", NewWindow),
                MenuItem::separator(),
                MenuItem::action("Import from Logseq…", ImportLogseq),
                MenuItem::separator(),
                MenuItem::action("Close Tab", CloseTab),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", input::Undo),
                MenuItem::action("Redo", input::Redo),
                MenuItem::separator(),
                MenuItem::action("Cut", input::Cut),
                MenuItem::action("Copy", input::Copy),
                MenuItem::action("Paste", input::Paste),
                MenuItem::action("Select All", input::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find in Page", FindInPage),
                MenuItem::action("Search All Notes", GlobalSearch),
                MenuItem::action("Fit Images to View", FitImages),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PrevTab),
            ],
            disabled: false,
        },
    ]);
}
