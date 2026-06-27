//! `AppView` — the root view. The content area is a set of **tabs**: a
//! pinned **Journal** tab (an infinite, reverse-chronological feed of daily
//! entries, today on top, older days lazy-loaded) plus a tab per opened
//! **page** (one editor + a "Linked References" panel). Left-click a sidebar
//! page to open/focus its tab; right-click → "Open in new tab" opens it in
//! the background. The sidebar search box shows results over the active tab
//! while it has text.
//!
//! Each editor is a gpui-component `InputState` in multi-line mode, which
//! gives a real Word-like typing experience (native Enter / selection /
//! undo / IME). Content saves on `Change` and re-indexes `[[links]]`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, App, AppContext, Bounds, ClipboardEntry, ClipboardItem, Context, CursorStyle,
    Entity, EventEmitter, FocusHandle, Focusable, Global, ImageFormat, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Task, TitlebarOptions, WeakEntity, Window, WindowAppearance, WindowBounds, WindowDecorations,
    WindowHandle, WindowOptions, div, point, px, size,
};
use gpui_component::{
    Root, TitleBar, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    calendar::{Calendar, CalendarEvent, CalendarState},
    dialog::{DialogButtonProps, DialogFooter},
    input::{Input, InputEvent, InputState},
};
use gpui_editor::{Diagnostic, EditorEvent, EditorState};

use crate::actions::{
    CloseTab, DeletePage, FindInPage, FitImages, GlobalSearch, ImportLogseq, InsertTab, NewPage,
    NewWhiteboard, NextTab, OpenInNewTab, OpenInNewWindow, OpenSettings, Outdent, PasteImage,
    PrevTab, RenamePage, SlashCancel, SlashConfirm, SlashDown, SlashUp, ToggleFavorite,
};
use crate::db::Db;
use crate::images::ImageSeed;
use crate::models::{Backlink, Page};
use crate::settings::SettingsView;
use crate::skins::{self, Skin};
use crate::slash::{self, ItemKind, Slash, SlashLevel, SlashTarget, Template, Trigger};
use crate::theme;
use crate::ui;

/// How many days to add each time the feed grows.
const FEED_CHUNK: usize = 7;
/// Hard cap on how far back the feed loads (~10 years), a runaway guard.
const FEED_MAX_DAYS: usize = 3650;
/// Default PDF render-quality multiplier (fraction of native DPI) for a fresh
/// install. 0.75 trades a little sharpness for noticeably faster rendering,
/// especially on slower (non-ARM) machines; users can raise it in Settings.
const DEFAULT_PDF_QUALITY: f32 = 0.75;
/// Default list-indent width in spaces (Tab / nesting). Configurable in Settings.
const DEFAULT_LIST_INDENT: usize = 4;

/// What a tab shows. The Journal is the pinned tab 0; the rest are pages or PDFs.
#[derive(Clone, PartialEq, Eq)]
pub enum TabKind {
    Journal,
    Page(i64),
    /// A PDF viewer for the file at this path.
    Pdf(PathBuf),
    /// A whiteboard canvas (the `kind = 'whiteboard'` page id).
    Whiteboard(i64),
}

/// An open tab: its content kind + a cached title for the tab strip.
pub struct OpenTab {
    pub kind: TabKind,
    pub title: SharedString,
}

/// A process-wide signal that note content was saved to the database. Every
/// window's `AppView` subscribes; when one window saves, the others reload the
/// now-stale journal days / active page from the shared DB, giving live
/// cross-window updates. Held in a gpui global so windows opened later share the
/// same instance.
pub struct DocSignal;

/// Emitted by [`DocSignal`] after a content save.
pub struct DocChanged;

impl EventEmitter<DocChanged> for DocSignal {}

/// Global wrapper holding the shared [`DocSignal`] entity (set once at startup).
pub struct GlobalDocSignal(pub Entity<DocSignal>);

impl Global for GlobalDocSignal {}

/// The payload + floating preview for a tab being dragged in the strip. Dropping
/// it on another tab reorders (`reorder_tab`); releasing it anywhere off the
/// strip hands it to whichever window sits under the cursor, or — over no window
/// — tears it into a fresh one (`on_tab_drag_release`). Browser-style.
#[derive(Clone)]
pub struct TabDrag {
    pub ix: usize,
    pub kind: TabKind,
    pub title: SharedString,
}

/// The tab currently being dragged, shared across windows. The strip drag is a
/// gpui-internal drag (never a native OS file drag), so releasing on the desktop
/// only ever opens a new window — it can't drop a file there. Set when a drag
/// starts; read by the source window on the terminating mouse-up.
#[derive(Clone)]
pub struct DraggingTab {
    /// The window the tab was dragged from (only it acts on release — it owns the
    /// tab and, via OS mouse capture, the release event).
    pub source: AnyWindowHandle,
    pub kind: TabKind,
    /// The tab's label, shown as a "ghost tab" in whichever window it's over.
    pub title: SharedString,
}

/// What a moving tab hands its destination window so the content shows up there
/// immediately (see `take_tab_seed`): the source window's decoded image bitmaps
/// for a page, or — for a PDF — the live viewer entity itself, preserving its
/// scroll, zoom, unlocked state, parsed document, and rendered pages.
#[derive(Default)]
pub struct TabSeed {
    images: ImageSeed,
    pdf: Option<(PathBuf, Entity<crate::pdf::PdfView>)>,
}

/// Global slot holding the in-flight [`DraggingTab`] (set once at startup).
#[derive(Default)]
pub struct GlobalDraggingTab(pub Option<DraggingTab>);

impl Global for GlobalDraggingTab {}

/// The window the dragged tab is currently hovering over (another window, never
/// the source), so that window can show a ghost tab where the tab would land.
/// Driven by the source window's `on_drag_move`; cleared on release.
#[derive(Default)]
pub struct GlobalDropTarget(pub Option<AnyWindowHandle>);

impl Global for GlobalDropTarget {}

/// Every live main window + a weak handle to its `AppView`, so a tab released
/// over another window can be handed to it. Registered on window creation, pruned
/// lazily (closed windows drop out). Settings windows aren't registered.
#[derive(Default)]
pub struct GlobalAppWindows(pub Vec<(AnyWindowHandle, WeakEntity<AppView>)>);

impl Global for GlobalAppWindows {}

impl Render for TabDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1()
            .rounded(px(6.0))
            .bg(theme::glass_strong())
            .border_1()
            .border_color(theme::border_subtle())
            .text_size(px(13.0))
            .text_color(theme::text_primary())
            .child(self.title.clone())
    }
}

/// A journal day's editor + the subscriptions saving its edits.
pub struct DayEditor {
    pub state: Entity<EditorState>,
    /// Tracks the day's rendered-markdown root bounds (the slot the editor
    /// takes over in edit mode) — the anchor for click-to-caret's scroll math.
    pub md_scroll: ScrollHandle,
    /// The editor's text as of the last change, used to detect single-char
    /// bracket/quote insertions for auto-pairing.
    prev: String,
    _sub: Subscription,
    /// gpui-editor has no Focus/Blur events, so we listen on its focus handle.
    _focus_sub: Subscription,
    _blur_sub: Subscription,
}

/// The currently-open named/journal page in `View::Page`.
pub struct PageEditor {
    /// The page's id, so the editor can be flushed without consulting the
    /// active tab (used before the editor is dropped).
    pub id: i64,
    pub title: String,
    /// Inline-editable page title (named pages only); renames on Enter/blur.
    pub title_state: Entity<InputState>,
    /// The page's aliases as a comma-separated list (named pages); commits on
    /// Enter/blur. Replaces typing an `alias::` property in the body.
    pub alias_state: Entity<InputState>,
    pub is_journal: bool,
    pub state: Entity<EditorState>,
    /// Last-change text snapshot for auto-pair detection (see `DayEditor::prev`).
    prev: String,
    _sub: Subscription,
    /// gpui-editor has no Focus/Blur events, so we listen on its focus handle.
    _focus_sub: Subscription,
    _blur_sub: Subscription,
    _title_sub: Subscription,
    _alias_sub: Subscription,
    pub backlinks: Vec<Backlink>,
}

/// An in-progress image resize drag (dragging the corner handle of a rendered
/// image). Tracked on `AppView` because the markdown renderer is stateless.
pub struct ImageDrag {
    /// Which editor's source holds the image being resized.
    target: SlashTarget,
    /// Byte range in that source to overwrite with `{width=N}`.
    attr_target: Range<usize>,
    /// Mouse x when the drag began, and the image's width then.
    start_x: Pixels,
    start_width: f32,
    /// The live width as the mouse moves (px).
    width: f32,
}

/// How many recently-viewed pages the sidebar's page tree is capped to.
const RECENT_PAGES_LIMIT: usize = 10;

/// How many image decodes may run concurrently. JPEGs now decode at a reduced
/// size (DCT scaling — see `images::decode_jpeg_reduced`), so their transient
/// buffer is small; only a non-JPEG fallback holds a full-resolution buffer
/// (~35 MB for a 12 MP photo). With that, a typical photo page decodes in a
/// single wave on any multi-core machine.
const MAX_IMAGE_DECODES: usize = 6;

/// Open rows×cols table-size picker (from the `/table` command). Hovering its
/// grid sets `rows`/`cols` (1-based; 0 = nothing hovered yet); a click inserts a
/// table of that size at `start`, replacing the `/table` query.
/// A table visual design offered by the `/table` picker. Maps to the
/// `<!-- table:STYLE -->` marker the renderers honor; `Grid` is the default and
/// writes no marker (a plain table).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum TableDesign {
    #[default]
    Grid,
    Striped,
    Header,
    Minimal,
}

impl TableDesign {
    pub const ALL: [TableDesign; 4] = [
        TableDesign::Grid,
        TableDesign::Striped,
        TableDesign::Header,
        TableDesign::Minimal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TableDesign::Grid => "Grid",
            TableDesign::Striped => "Striped",
            TableDesign::Header => "Header",
            TableDesign::Minimal => "Minimal",
        }
    }

    /// The hidden `<!-- table:NAME -->` marker line, or `None` for the default
    /// `Grid` (written without a marker). The names match the editor's parser.
    fn marker(self) -> Option<&'static str> {
        match self {
            TableDesign::Grid => None,
            TableDesign::Striped => Some("<!-- table:striped -->"),
            TableDesign::Header => Some("<!-- table:header -->"),
            TableDesign::Minimal => Some("<!-- table:minimal -->"),
        }
    }
}

pub struct TablePicker {
    target: SlashTarget,
    /// Byte offset of the `/` to replace in the target editor.
    start: usize,
    /// Caret bounds (window space) to anchor the popup.
    caret: gpui::Bounds<gpui::Pixels>,
    pub rows: usize,
    pub cols: usize,
    /// The chosen visual design (its marker is prepended on insert).
    pub style: TableDesign,
    /// Typed custom dimensions, for tables larger than the hover grid.
    pub rows_input: Entity<InputState>,
    pub cols_input: Entity<InputState>,
}

/// State for an open structural-math edit (double-clicking a `$$` block): the 2D editor, the
/// note editor + document it came from, and the block's byte range to overwrite on commit.
/// A right-click context menu in the content area, rendered as an anchored overlay. Built
/// element-side (not gpui-component's window-level `.context_menu()`, which a child's
/// `stop_propagation` can't suppress) so a formula's menu cleanly overrides the day/page one.
struct CtxMenu {
    anchor: Point<Pixels>,
    kind: CtxKind,
}

enum CtxKind {
    /// A rendered `$$…$$` formula was right-clicked: copy its LaTeX or export it (the LaTeX
    /// source). `alignable` adds the Align items — only while editing the formula in-line,
    /// where the host can re-justify it and persist the marker on commit.
    Formula {
        source: SharedString,
        alignable: bool,
    },
    /// A reader-view day/page was right-clicked away from a formula: a single "Edit" entry
    /// into edit mode for that target.
    Edit(SlashTarget),
}

/// Map the editor's `<!-- math:ALIGN -->` marker alignment to the in-line editor's, and back.
fn to_ratex_align(a: gpui_editor::MathAlign) -> ratex_gpui::MathAlign {
    match a {
        gpui_editor::MathAlign::Left => ratex_gpui::MathAlign::Left,
        gpui_editor::MathAlign::Center => ratex_gpui::MathAlign::Center,
        gpui_editor::MathAlign::Right => ratex_gpui::MathAlign::Right,
    }
}
fn to_editor_align(a: ratex_gpui::MathAlign) -> gpui_editor::MathAlign {
    match a {
        ratex_gpui::MathAlign::Left => gpui_editor::MathAlign::Left,
        ratex_gpui::MathAlign::Center => gpui_editor::MathAlign::Center,
        ratex_gpui::MathAlign::Right => gpui_editor::MathAlign::Right,
    }
}

struct MathEdit {
    editor: Entity<ratex_gpui::MathEditor>,
    source: Entity<EditorState>,
    target: SlashTarget,
    /// Commits the edit when the math editor loses focus (click-away). Kept alive here.
    _blur_sub: gpui::Subscription,
    /// Flows the caret back to the text when an arrow hits a formula boundary. Kept alive.
    _nav_sub: gpui::Subscription,
}

pub struct AppView {
    db: Db,
    /// This view's window, so it can tell whether a cross-window tab drag is
    /// hovering it (see [`GlobalDropTarget`]).
    window_handle: AnyWindowHandle,
    /// The tab strip's window-relative rect, captured each paint. A drag from
    /// another window only treats this window as a move target when the cursor is
    /// over *this rect* — so a window hidden behind the source (whose full bounds
    /// overlap) is never picked; you must drop on a visible tab bar.
    pub tab_strip_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Open tabs (index 0 is the pinned Journal) and the active index.
    pub tabs: Vec<OpenTab>,
    pub active: usize,
    /// When the sidebar search box has text, the content area shows search
    /// results instead of the active tab's content.
    searching: bool,
    /// Horizontal scroll handle for the tab strip.
    pub tab_scroll: ScrollHandle,
    /// Active theme mode (Light / Dark / Auto) + last-known OS appearance
    /// (used to resolve Auto).
    mode: theme::Mode,
    system_dark: bool,
    /// The open Settings window, if any (focused instead of duplicated).
    settings_window: Option<WindowHandle<gpui_component::Root>>,
    /// Available themes (built-ins + user) and the active one's id.
    skins: Vec<Skin>,
    skin_id: String,
    /// PDF render-quality multiplier (1.0 = native DPI), persisted; mirrored into the
    /// `crate::pdf` global that each `PdfView`'s quality closure reads.
    pdf_quality: f32,
    /// List-indent width in spaces (2 / 4 / 8), persisted. Drives both the editor's
    /// Tab/nesting unit and the markdown render's per-level indent, so they line up.
    list_indent: usize,
    /// Check GitHub Releases for a newer version at startup, persisted.
    check_updates: bool,
    /// Whether the update check considers pre-releases (betas), persisted.
    include_prerelease: bool,
    /// WYSIWYG live-preview editing, persisted (default on). On = the editor
    /// shows inline markdown formatting as you type (W1+); off = "editor mode",
    /// plain raw markdown in edit + the rendered page on Esc (the classic flow).
    wysiwyg: bool,
    /// In the feed, the date currently being edited (raw editor); all
    /// other days render as markdown. `None` = every day rendered.
    editing_day: Option<String>,
    /// Whether the single-page editor is in edit (raw) vs reading mode.
    page_editing: bool,

    // Journal feed.
    pub loaded_days: usize,
    pub day_editors: HashMap<String, DayEditor>,
    pub feed_scroll: ScrollHandle,
    /// Scroll offset of the open completion menu — persists across the per-keystroke rebuild
    /// of `Slash`, so the list doesn't snap back to the top as the user types or arrows.
    pub slash_scroll: ScrollHandle,

    /// The Windows/Linux in-titlebar menu bar (File/Edit/View). macOS shows the
    /// native menu bar instead; this gives the other OSes visual parity.
    app_menu_bar: Entity<gpui_component::menu::AppMenuBar>,

    // Single-page view.
    pub page_editor: Option<PageEditor>,

    // Image resize: live drag state, plus rendered image widths captured during
    // paint (keyed by the image's source attr offset) so a drag knows its
    // starting size. The map is shared into the renderer's measure callbacks.
    image_drag: Option<ImageDrag>,
    image_widths: Rc<RefCell<HashMap<usize, f32>>>,
    // Decodes note images at display resolution and holds the GPU-ready bitmaps,
    // freed on view change. Shared into the markdown image renderer. `Rc<RefCell>`
    // so the renderer (no `cx`) can read it during paint while methods here drive
    // loads and eviction. See `images::ImageStore`.
    image_store: Rc<RefCell<crate::images::ImageStore>>,
    // Whiteboard image elements pre-rotated to a quarter turn (gpui can't
    // transform a raster sprite, so we rotate the pixels). Keyed by (src, degrees)
    // so two elements sharing a file at different angles each get their own
    // bitmap instead of evicting each other every frame; freed on view close.
    // Bounded: at most the 90/180/270 turns actually shown per rotated src.
    rotated_images:
        std::collections::HashMap<(SharedString, i32), std::sync::Arc<gpui::RenderImage>>,
    // Rendered `mermaid` diagrams, cached by source text. Shared into the markdown
    // mermaid renderer (no `cx`) so it can read a ready diagram during paint while
    // `ensure_mermaid_loaded` drives the off-thread render. See `mermaid::MermaidStore`.
    mermaid_store: Rc<RefCell<crate::mermaid::MermaidStore>>,
    // Typeset `$$…$$` math, cached by LaTeX. Shared into the markdown math renderer (no
    // `cx`) so it can read a ready formula during paint; `ensure_math_loaded` drives the
    // off-thread render. See `math::MathStore`.
    math_store: Rc<RefCell<crate::math::MathStore>>,
    // The source of the mermaid diagram currently expanded in the lightbox overlay
    // (click a diagram to open it large + scrollable). `None` = closed.
    mermaid_lightbox: Option<SharedString>,
    // Focus for the open lightbox so it can capture Esc-to-close without a global
    // key binding (which would clash with the editor's Escape → slash-cancel).
    lightbox_focus: FocusHandle,
    // An open structural-math edit (a double-clicked `$$` block), or `None`.
    math_edit: Option<MathEdit>,
    // Right-click context menu on a rendered formula (Copy LaTeX / Export SVG / Export PNG).
    ctx_menu: Option<CtxMenu>,
    // Pending image decodes, run a bounded few at a time (`image_decodes` counts
    // what's in flight, capped at `MAX_IMAGE_DECODES`). The bound keeps the
    // transient full-resolution buffers in check — decoding a 12 MP photo briefly
    // needs tens of MB, which would otherwise multiply unbounded across a
    // photo-heavy page and spike RSS — while still loading a page of photos
    // several times faster than one-at-a-time.
    image_queue: std::collections::VecDeque<(SharedString, PathBuf)>,
    image_decodes: usize,

    // Open PDF viewers, keyed by resolved path. Each is an independent,
    // page-virtualized `gpui_pdf::PdfView` (own scroll handle + bounded memory),
    // removed (and its GPU textures released) when the tab closes.
    pub pdf_views: HashMap<PathBuf, Entity<crate::pdf::PdfView>>,

    // Open whiteboard canvases, keyed by board (page) id. Each is an independent
    // `gpui_whiteboard::WhiteboardView`; dropped when its tab closes. Reloaded
    // from the DB on a cross-window move (no live hand-off needed yet).
    pub whiteboard_views: HashMap<i64, Entity<crate::whiteboard::WhiteboardView>>,

    // Sidebar.
    pub pages: Vec<Page>,
    /// Whiteboards for the sidebar's "Whiteboards" section (titles only; content
    /// not loaded). Refreshed alongside `pages`.
    pub whiteboards: Vec<Page>,
    pub new_page_input: Entity<InputState>,
    pub search_input: Entity<InputState>,
    /// Jump-to-date calendar (opened from the sidebar calendar icon); picking
    /// a date opens that journal day.
    pub calendar: Entity<CalendarState>,
    show_calendar: bool,
    /// When collapsed, the sidebar shrinks to a thin icon rail (expand caret +
    /// the calendar/settings icons); the page list and search box hide.
    pub sidebar_collapsed: bool,
    /// Ids of recently-viewed named pages, most-recent first (capped). The
    /// sidebar page tree is filtered to these; persisted across launches.
    pub recent_pages: Vec<i64>,
    /// Ids of pages the user pinned to the sidebar's "Favorites" group, in the
    /// order added; persisted across launches.
    pub favorites: Vec<i64>,
    /// Namespace nodes (by full path) collapsed in the sidebar tree — their
    /// descendants are hidden. Persisted across launches.
    pub collapsed_nodes: HashSet<String>,
    /// Sidebar sections (by key — `favorites` / `whiteboards` / `recent`) collapsed
    /// to just their header. Persisted across launches.
    pub collapsed_sections: HashSet<String>,
    /// The current global-search results (pages + referenced PDF/image files),
    /// kind-filtered, with per-kind counts for the results-pane chips.
    pub search: crate::search::Results,
    /// Open slash-command menu, if any.
    slash: Option<Slash>,
    /// Open `/table` rows×cols picker, if any.
    table_picker: Option<TablePicker>,
    /// Debounced spell-check for the focused body editor; replacing it cancels
    /// the prior pending run so we don't hit the OS spell service per keystroke.
    spell_task: Option<Task<()>>,
    /// Debounced cross-window "document changed" signal, so the feed-reloading
    /// `apply_external_edit` doesn't run on every keystroke (only after idle).
    signal_task: Option<Task<()>>,
    /// Templates parsed from the reserved `Templates` page.
    templates: Vec<Template>,
    /// The page (id + title) targeted by an open right-click context menu,
    /// read by the `DeletePage` / `RenamePage` actions.
    context_page: Option<(i64, SharedString)>,
    /// The target of a right-click "Open in new window" — a page (sidebar or
    /// tab) or a PDF/journal tab. Set on right-click, taken by the handler.
    context_target: Option<TabKind>,
    /// Shared cross-window save signal (see [`DocSignal`]): this window emits on
    /// save and reloads stale content on other windows' saves (live multi-window).
    doc_signal: Entity<DocSignal>,
    /// The rename dialog's text field, and the page being renamed.
    rename_input: Entity<InputState>,
    rename_target: Option<i64>,
    /// The text field for the password prompt shown when an encrypted PDF tab is
    /// locked (one field shared across PDF tabs — only the active one prompts).
    pdf_password_input: Entity<InputState>,

    /// Set when the on-disk database couldn't be opened/migrated and we fell back
    /// to an empty in-memory store. Drives a one-time startup dialog (see
    /// `show_db_error_dialog`) so the user isn't silently shown blank notes — their
    /// data is preserved in the pre-migration backup.
    db_error: Option<DbError>,
    db_error_shown: bool,
    /// In-page find bar (⌘F) shown above a named page; `None` = closed.
    pub page_find: Option<PageFind>,
    /// Find's scroll-to-match handles: `page_scroll` drives the page's scroll
    /// offset; `md_block_scroll` tracks the rendered markdown blocks' bounds (via
    /// `MarkdownView::track_blocks`) so the active match's block can be located.
    pub page_scroll: ScrollHandle,
    pub md_block_scroll: ScrollHandle,

    _subs: Vec<Subscription>,
    pub focus_handle: FocusHandle,
}

/// In-page find state. The query field's Change events recompute `count` against
/// the active page; `current` + `count` size the bar's "n of m" and pick which
/// match [`gpui_markdown::MarkdownView::search`] emphasizes.
pub struct PageFind {
    pub input: Entity<InputState>,
    pub query: String,
    pub current: usize,
    pub count: usize,
    /// Block index (per `gpui_markdown::find_matches`) of each match, used to scroll
    /// the active match's block into view.
    match_blocks: Vec<usize>,
    _sub: Subscription,
}

/// Details of a failed on-disk database open, surfaced once at startup.
struct DbError {
    /// The underlying error text.
    message: String,
    /// The pre-migration backup (`<db>.bak-v<N>`), if one was taken.
    backup: Option<PathBuf>,
    /// The folder holding the database + its backups (for the "Reveal" button).
    folder: PathBuf,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (db, db_error) = match Db::open() {
            Ok(db) => (db, None),
            Err(e) => {
                log::error!(
                    "open database on disk failed: {}; using a temporary in-memory store",
                    e.source
                );
                // Where the user's data (and any pre-migration backup) lives.
                let folder = e
                    .backup
                    .as_deref()
                    .and_then(Path::parent)
                    .map(Path::to_path_buf)
                    .or_else(|| crate::paths::db_path().parent().map(Path::to_path_buf))
                    .unwrap_or_default();
                let db = Db::open_in_memory().expect("initialize in-memory database");
                let err = DbError {
                    message: e.source.to_string(),
                    backup: e.backup,
                    folder,
                };
                (db, Some(err))
            }
        };

        // The page-name field shown in the "New page" dialog (opened from the
        // pages-area right-click menu).
        let new_page_input = cx.new(|cx| InputState::new(window, cx).placeholder("Page name…"));

        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search…"));
        let search_sub = cx.subscribe_in(
            &search_input,
            window,
            |this: &mut AppView, _state, ev: &InputEvent, _window, cx| {
                if let InputEvent::Change = ev {
                    this.run_search(cx);
                }
            },
        );

        // Jump-to-date: the sidebar calendar icon opens this calendar; picking
        // a date closes it and opens that journal day as a tab.
        let calendar = cx.new(|cx| CalendarState::new(window, cx));
        let calendar_sub = cx.subscribe_in(
            &calendar,
            window,
            |this: &mut AppView, _state, ev: &CalendarEvent, window, cx| {
                let CalendarEvent::Selected(date) = ev;
                if let Some(day) = date.start() {
                    this.show_calendar = false;
                    this.open_journal_day(&day.to_string(), window, cx);
                }
            },
        );

        // Live multi-window sync: share one save-signal across all windows.
        let doc_signal = cx.global::<GlobalDocSignal>().0.clone();
        let doc_sub = cx.subscribe_in(
            &doc_signal,
            window,
            |this: &mut AppView, _sig, _ev: &DocChanged, window, cx| {
                this.apply_external_edit(window, cx);
            },
        );

        // The encrypted-PDF password field; Enter submits like the Unlock button.
        let pdf_password_input = cx.new(|cx| InputState::new(window, cx));
        let pdf_password_sub = cx.subscribe_in(
            &pdf_password_input,
            window,
            |this: &mut AppView, _st, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev
                    && let Some(TabKind::Pdf(path)) =
                        this.tabs.get(this.active).map(|t| t.kind.clone())
                {
                    this.unlock_pdf(&path, window, cx);
                }
            },
        );

        let mut this = Self {
            db,
            window_handle: window.window_handle(),
            tab_strip_bounds: Rc::new(Cell::new(Bounds::default())),
            tabs: vec![OpenTab {
                kind: TabKind::Journal,
                title: "Journal".into(),
            }],
            active: 0,
            searching: false,
            tab_scroll: ScrollHandle::new(),
            mode: theme::Mode::Dark,
            system_dark: true,
            settings_window: None,
            skins: skins::builtin_skins(),
            skin_id: String::new(),
            pdf_quality: DEFAULT_PDF_QUALITY,
            list_indent: DEFAULT_LIST_INDENT,
            check_updates: true,
            include_prerelease: false,
            wysiwyg: true,
            editing_day: None,
            page_editing: false,
            loaded_days: 0,
            day_editors: HashMap::new(),
            image_drag: None,
            image_widths: Rc::new(RefCell::new(HashMap::new())),
            image_store: Rc::new(RefCell::new(crate::images::ImageStore::default())),
            rotated_images: std::collections::HashMap::new(),
            mermaid_store: Rc::new(RefCell::new(crate::mermaid::MermaidStore::default())),
            math_store: Rc::new(RefCell::new(crate::math::MathStore::default())),
            mermaid_lightbox: None,
            lightbox_focus: cx.focus_handle(),
            math_edit: None,
            ctx_menu: None,
            image_queue: std::collections::VecDeque::new(),
            image_decodes: 0,
            pdf_views: HashMap::new(),
            whiteboard_views: HashMap::new(),
            feed_scroll: ScrollHandle::new(),
            slash_scroll: ScrollHandle::new(),
            app_menu_bar: gpui_component::menu::AppMenuBar::new(cx),
            page_editor: None,
            pages: Vec::new(),
            whiteboards: Vec::new(),
            new_page_input,
            search_input,
            calendar,
            show_calendar: false,
            sidebar_collapsed: false,
            recent_pages: Vec::new(),
            favorites: Vec::new(),
            collapsed_nodes: HashSet::new(),
            collapsed_sections: HashSet::new(),
            search: crate::search::Results::default(),
            slash: None,
            table_picker: None,
            spell_task: None,
            signal_task: None,
            templates: Vec::new(),
            context_page: None,
            context_target: None,
            doc_signal,
            rename_input: cx.new(|cx| InputState::new(window, cx)),
            rename_target: None,
            pdf_password_input,
            db_error,
            db_error_shown: false,
            page_find: None,
            page_scroll: ScrollHandle::new(),
            md_block_scroll: ScrollHandle::new(),
            _subs: vec![search_sub, calendar_sub, doc_sub, pdf_password_sub],
            focus_handle: cx.focus_handle(),
        };

        // The journal feed loads lazily, on the first frame that actually shows
        // it (see `ensure_feed_loaded` in `render`) — so a window opened on a
        // torn-off page/PDF never builds a feed, and never pays to keep it
        // in sync with other windows' edits.
        this.refresh_sidebar();
        this.recent_pages = this.load_recent_pages();
        this.favorites = this.load_favorites();
        this.collapsed_nodes = this.load_collapsed();
        this.collapsed_sections = this.load_collapsed_sections();
        // Load user themes on top of the built-ins, then apply the saved
        // (or default) skin + mode before the first paint.
        this.skins.extend(skins::load_user_skins());
        this.skin_id = this
            .db
            .get_setting("theme_skin")
            .unwrap_or_else(|| "zorite".to_string());
        this.mode = this
            .db
            .get_setting("theme_mode")
            .map(|s| theme::Mode::from_str(&s))
            .unwrap_or_default();
        this.pdf_quality = this
            .db
            .get_setting("pdf_quality")
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_PDF_QUALITY);
        crate::pdf::set_quality(this.pdf_quality);
        this.list_indent = this
            .db
            .get_setting("list_indent")
            .and_then(|s| s.parse().ok())
            .filter(|n| matches!(n, 2 | 4 | 8))
            .unwrap_or(DEFAULT_LIST_INDENT);
        this.check_updates = this
            .db
            .get_setting("check_updates")
            .map(|v| v != "0")
            .unwrap_or(true);
        this.include_prerelease = this
            .db
            .get_setting("include_prerelease")
            .map(|v| v == "1")
            .unwrap_or(false);
        this.wysiwyg = this
            .db
            .get_setting("wysiwyg")
            .map(|v| v != "0")
            .unwrap_or(true);
        // Date/time display formats for /date, /time, and {{date}}/{{time}} —
        // applied to the thread-local in `crate::dates`; validated against the
        // known ids so a stale persisted value can't stick.
        if let Some(id) = this
            .db
            .get_setting("date_format")
            .filter(|s| crate::dates::DATE_FORMATS.contains(&s.as_str()))
        {
            crate::dates::set_date_format(&id);
        }
        if let Some(id) = this
            .db
            .get_setting("time_format")
            .filter(|s| crate::dates::TIME_FORMATS.contains(&s.as_str()))
        {
            crate::dates::set_time_format(&id);
        }
        this.system_dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        this.apply_theme(window, cx);
        // Start with today rendered (like every other day); click to edit.
        this
    }

    // --- Journal feed ---

    fn ensure_day_editor(&mut self, date: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.day_editors.contains_key(&date) {
            return;
        }
        let content = self
            .db
            .get_journal_by_date(&date)
            .ok()
            .flatten()
            .map(|p| p.content)
            .unwrap_or_default();
        let state = make_editor(
            &content,
            self.wysiwyg,
            self.list_indent,
            self.image_store(),
            self.mermaid_store(),
            self.math_store(),
            window,
            cx,
        );
        self.ensure_content_images(&content, cx);
        self.ensure_content_mermaid(&content, cx);
        self.ensure_content_math(&content, cx);
        let key = date.clone();
        let sub = cx.subscribe_in(
            &state,
            window,
            move |this: &mut AppView, st, ev: &EditorEvent, window, cx| match ev {
                EditorEvent::Changed => {
                    // Auto-pair may rewrite the text and save directly; only save
                    // here if it didn't. Always refresh the slash menu.
                    if !this.maybe_autopair(&SlashTarget::Day(key.clone()), window, cx) {
                        let value = st.read(cx).text().to_string();
                        this.save_journal(&key, &value, cx);
                    }
                    this.update_slash(SlashTarget::Day(key.clone()), cx);
                    this.schedule_spellcheck(st.clone(), cx);
                }
                EditorEvent::OpenLink(src) => {
                    if let Some(path) = crate::pdf::resolve_path(src) {
                        this.open_pdf(path, window, cx);
                    }
                }
                EditorEvent::SelectionChanged => {
                    this.scroll_caret_into_view(st, &this.feed_scroll, cx)
                }
                EditorEvent::EditMath {
                    range,
                    source,
                    at_end,
                } => {
                    this.open_math_edit(
                        st.clone(),
                        SlashTarget::Day(key.clone()),
                        range.clone(),
                        source.clone(),
                        *at_end,
                        window,
                        cx,
                    );
                }
                EditorEvent::MathMenu { source, position } => {
                    // Not editing → no Align items (nothing to re-justify live + persist).
                    this.open_math_menu(source.clone(), *position, false, cx);
                }
            },
        );
        // gpui-editor has no Focus/Blur events; listen on its focus handle.
        let handle = state.read(cx).focus_handle(cx);
        let weak = cx.entity().downgrade();
        let fkey = date.clone();
        let fstate = state.clone();
        let focus_sub = window.on_focus_in(&handle, cx, move |_window, cx| {
            weak.update(cx, |this: &mut AppView, cx| {
                this.editing_day = Some(fkey.clone());
                // Spell-check on entering edit mode so existing misspellings show.
                let diags = spell_diagnostics(fstate.read(cx).text());
                fstate.update(cx, |ed, cx| ed.set_diagnostics(diags, cx));
                cx.notify();
            })
            .ok();
        });
        let weak = cx.entity().downgrade();
        let bkey = date.clone();
        let bstate = state.clone();
        let blur_sub = window.on_focus_out(&handle, cx, move |_ev, _window, cx| {
            weak.update(cx, |this: &mut AppView, cx| {
                if this.editing_day.as_deref() == Some(bkey.as_str()) {
                    this.editing_day = None;
                }
                this.slash = None;
                let value = bstate.read(cx).text().to_string();
                this.flush_journal(&bkey, &value);
                // Link re-index changed backlinks elsewhere — sync windows.
                this.signal_doc_changed(cx);
                cx.notify();
            })
            .ok();
        });
        self.day_editors.insert(
            date,
            DayEditor {
                prev: content,
                state,
                md_scroll: ScrollHandle::new(),
                _sub: sub,
                _focus_sub: focus_sub,
                _blur_sub: blur_sub,
            },
        );
    }

    /// Reload cached journal day editors from the DB. Called after an action
    /// that rewrites content across pages (e.g. a page rename that updated
    /// `[[links]]`) so the feed shows the new text instead of stale cache.
    /// Only days whose content actually changed are touched.
    fn reload_day_editors(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let dates: Vec<String> = self.day_editors.keys().cloned().collect();
        for date in dates {
            // Never reload the day being edited here — that would clobber the
            // in-progress edit with the DB copy.
            if self.editing_day.as_deref() == Some(date.as_str()) {
                continue;
            }
            let content = self
                .db
                .get_journal_by_date(&date)
                .ok()
                .flatten()
                .map(|p| p.content)
                .unwrap_or_default();
            if let Some(de) = self.day_editors.get(&date)
                && de.state.read(cx).value() != content
            {
                de.state.update(cx, |s, cx| s.set_text(content, cx));
            }
        }
    }

    /// Save a journal day's content on every keystroke — but NOT its
    /// links/tags. Link re-indexing (which creates target pages) happens
    /// on blur, so a half-typed `#tag` doesn't spawn a page per keystroke.
    pub(crate) fn save_journal(&mut self, date: &str, content: &str, cx: &mut Context<Self>) {
        if let Ok(page) = self.db.get_or_create_journal(date) {
            self.save_page_content(page.id, content, cx);
        }
    }

    /// Save a page's content to the DB and signal other windows to refresh. The
    /// single choke point for content writes, so every save reaches other windows.
    pub(crate) fn save_page_content(&mut self, id: i64, content: &str, cx: &mut Context<Self>) {
        if let Err(e) = self.db.set_page_content(id, content) {
            log::error!("save page {id}: {e}");
        }
        self.schedule_doc_signal(cx);
    }

    /// Notify every window (including this one) that content changed, so each
    /// reloads any now-stale journal days / active page from the shared database.
    pub(crate) fn signal_doc_changed(&self, cx: &mut Context<Self>) {
        self.doc_signal.update(cx, |_, cx| cx.emit(DocChanged));
    }

    /// Debounced [`Self::signal_doc_changed`]: per-keystroke saves still hit the
    /// DB immediately, but the cross-window refresh (which reloads the feed and
    /// re-renders it via `apply_external_edit`) coalesces to one run after a
    /// short idle — so typing doesn't re-render the whole journal each key. Blur
    /// signals immediately for a prompt final sync.
    fn schedule_doc_signal(&mut self, cx: &mut Context<Self>) {
        self.signal_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            let _ = this.update(cx, |this, cx| this.signal_doc_changed(cx));
        }));
    }

    /// Reload stale content after another window saved: refresh changed journal
    /// days and the active page editor from the DB. Value-comparison means we only
    /// touch what actually changed — and never clobber what we're editing here
    /// (our own just-saved content already matches the DB).
    fn apply_external_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_day_editors(window, cx);
        if !self.page_editing {
            let stale = match self.page_editor.as_ref() {
                Some(pe) => {
                    let id = pe.id;
                    let current = pe.state.read(cx).value().to_string();
                    self.db
                        .get_page(id)
                        .ok()
                        .flatten()
                        .filter(|p| p.content != current)
                        .map(|_| id)
                }
                None => None,
            };
            if let Some(id) = stale {
                self.load_page_editor(id, window, cx);
            }
        }
        // Refresh the active page's backlinks (another window may have edited a
        // page that links here) and the sidebar list (a page may have been
        // created / renamed / deleted elsewhere).
        if let Some(id) = self.page_editor.as_ref().map(|pe| pe.id)
            && let Ok(bl) = self.db.backlinks(id)
            && let Some(pe) = self.page_editor.as_mut()
            && pe.id == id
        {
            pe.backlinks = bl;
        }
        self.refresh_sidebar();
        cx.notify();
    }

    /// On blur: persist the day and re-index its `[[links]]` / `#tags`.
    fn flush_journal(&mut self, date: &str, content: &str) {
        if let Ok(page) = self.db.get_or_create_journal(date) {
            self.persist(page.id, content);
        }
        self.refresh_sidebar();
    }

    /// Build the initial feed the first time this window shows the journal.
    /// Runs from `render` (gated to the Journal tab being active), so the cost —
    /// editors, day text, and the cross-window reload work that comes with them —
    /// is only ever paid by windows that actually display the feed.
    fn ensure_feed_loaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded_days > 0 {
            return;
        }
        self.loaded_days = 14;
        for i in 0..self.loaded_days {
            self.ensure_day_editor(date_for_offset(i), window, cx);
        }
    }

    /// Grow the feed if the user has scrolled near the bottom.
    pub fn maybe_extend_feed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let off = f32::from(self.feed_scroll.offset().y).abs();
        let max = f32::from(self.feed_scroll.max_offset().y).abs();
        if max > 1.0 && off >= max - 600.0 {
            self.extend_feed(window, cx);
        }
    }

    /// Load the next chunk of older days.
    pub fn extend_feed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded_days >= FEED_MAX_DAYS {
            return;
        }
        let start = self.loaded_days;
        self.loaded_days = (self.loaded_days + FEED_CHUNK).min(FEED_MAX_DAYS);
        for i in start..self.loaded_days {
            self.ensure_day_editor(date_for_offset(i), window, cx);
        }
        cx.notify();
    }

    // --- Navigation ---

    pub fn show_journal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The journal is the pinned first tab.
        self.activate_tab(0, window, cx);
    }

    /// Open a page in the **foreground** (left-click): focus its tab if it's
    /// already open, else open a new tab for it and switch to it.
    pub fn open_page_id(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // A whiteboard is a page row too, so route it to the canvas viewer rather
        // than the markdown editor (e.g. opening a board from a page's backlinks).
        if self.db.is_whiteboard(id) {
            self.open_whiteboard(id, window, cx);
            return;
        }
        match self.db.get_page(id) {
            Ok(Some(page)) => self.open_page_foreground(page, window, cx),
            Ok(None) => log::warn!("page {id} not found"),
            Err(e) => log::error!("open page {id}: {e}"),
        }
    }

    pub fn open_page_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        // A `[[file.pdf]]` link opens the PDF viewer instead of a page; a `#pN`
        // fragment (`[[file.pdf#p12]]`) also jumps to page N when it's already loaded.
        let (base, target_page) = match title.split_once('#') {
            Some((b, frag)) => (b, frag.trim_start_matches(['p', 'P']).parse::<usize>().ok()),
            None => (title, None),
        };
        if crate::pdf::is_pdf(base)
            && let Some(path) = crate::pdf::resolve_path(base)
        {
            self.open_pdf(path.clone(), window, cx);
            if let Some(n) = target_page
                && n > 0
                && let Some(v) = self.pdf_views.get(&path)
            {
                v.update(cx, |v, cx| v.reveal_highlight(n - 1, cx));
            }
            return;
        }
        match self.db.get_or_create_page(title) {
            Ok(page) => {
                self.open_page_foreground(page, window, cx);
                // The page may be newly created (via the New-page dialog or a
                // [[link]]), so refresh the sidebar to show it — and tell other
                // windows so their sidebars pick up the new page too.
                self.refresh_sidebar();
                self.signal_doc_changed(cx);
            }
            Err(e) => log::error!("open page '{title}': {e}"),
        }
    }

    /// Toggle the jump-to-date calendar overlay (the sidebar calendar icon).
    pub fn toggle_calendar(&mut self, cx: &mut Context<Self>) {
        self.show_calendar = !self.show_calendar;
        cx.notify();
    }

    /// Collapse the sidebar to a thin icon rail, or expand it back. Driven by
    /// the caret at the top of the sidebar (`<` to collapse, `>` to expand).
    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    /// Load the persisted recent-pages list, falling back to the most-recently
    /// edited pages so the sidebar isn't empty before anything's been viewed.
    fn load_recent_pages(&self) -> Vec<i64> {
        let stored: Vec<i64> = self
            .db
            .get_setting("recent_pages")
            .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default();
        if stored.is_empty() {
            self.db
                .recent_page_ids(RECENT_PAGES_LIMIT)
                .unwrap_or_default()
        } else {
            stored
        }
    }

    /// Mark a named page as most-recently-viewed (front of the list, capped)
    /// and persist it. The sidebar page tree is filtered to this list.
    fn record_recent(&mut self, page_id: i64) {
        self.recent_pages.retain(|&id| id != page_id);
        self.recent_pages.insert(0, page_id);
        self.recent_pages.truncate(RECENT_PAGES_LIMIT);
        let csv = self
            .recent_pages
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if let Err(e) = self.db.set_setting("recent_pages", &csv) {
            log::error!("save recent pages: {e}");
        }
    }

    /// Load the persisted favorites (a comma-separated id list; empty if none).
    fn load_favorites(&self) -> Vec<i64> {
        self.db
            .get_setting("favorites")
            .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_default()
    }

    /// Whether `id` is pinned to the sidebar's Favorites group.
    pub fn is_favorite(&self, id: i64) -> bool {
        self.favorites.contains(&id)
    }

    /// Pin / unpin a page (sidebar right-click → Favorite) and persist. The
    /// sidebar reads `favorites` at render, so a notify is all that's needed.
    fn toggle_favorite(&mut self, id: i64, cx: &mut Context<Self>) {
        match self.favorites.iter().position(|&x| x == id) {
            Some(pos) => {
                self.favorites.remove(pos);
            }
            None => self.favorites.push(id),
        }
        self.persist_favorites();
        cx.notify();
    }

    /// Persist the favorites as a comma-separated id list (mirrors recent pages).
    fn persist_favorites(&self) {
        let csv = self
            .favorites
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if let Err(e) = self.db.set_setting("favorites", &csv) {
            log::error!("save favorites: {e}");
        }
    }

    /// Load the persisted collapsed sidebar nodes (newline-separated paths —
    /// titles are single-line, so a newline can't appear inside one).
    fn load_collapsed(&self) -> HashSet<String> {
        self.db
            .get_setting("collapsed_nodes")
            .map(|s| {
                s.split('\n')
                    .filter(|x| !x.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the sidebar tree node at `path` is collapsed (descendants hidden).
    pub fn is_collapsed(&self, path: &str) -> bool {
        self.collapsed_nodes.contains(path)
    }

    /// Collapse / expand a sidebar namespace node (its disclosure chevron) and
    /// persist. The sidebar reads `collapsed_nodes` at render, so just notify.
    pub fn toggle_collapsed(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.collapsed_nodes.remove(path) {
            self.collapsed_nodes.insert(path.to_string());
        }
        let data = self
            .collapsed_nodes
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = self.db.set_setting("collapsed_nodes", &data) {
            log::error!("save collapsed nodes: {e}");
        }
        cx.notify();
    }

    /// Load the persisted collapsed sidebar sections (newline-separated keys).
    fn load_collapsed_sections(&self) -> HashSet<String> {
        self.db
            .get_setting("collapsed_sections")
            .map(|s| {
                s.split('\n')
                    .filter(|x| !x.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the sidebar section `key` is collapsed (its rows hidden).
    pub fn is_section_collapsed(&self, key: &str) -> bool {
        self.collapsed_sections.contains(key)
    }

    /// Collapse / expand a sidebar section (its header chevron) and persist.
    pub fn toggle_section(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.collapsed_sections.remove(key) {
            self.collapsed_sections.insert(key.to_string());
        }
        let data = self
            .collapsed_sections
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = self.db.set_setting("collapsed_sections", &data) {
            log::error!("save collapsed sections: {e}");
        }
        cx.notify();
    }

    /// Open a specific journal day (by ISO `YYYY-MM-DD`) as a focused tab,
    /// creating the day if it doesn't exist yet. Used by the date picker.
    pub fn open_journal_day(&mut self, date: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.get_or_create_journal(date) {
            Ok(page) => self.open_page_foreground(page, window, cx),
            Err(e) => log::error!("open journal {date}: {e}"),
        }
    }

    fn open_page_foreground(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        // Viewing a named page bumps it to the top of the sidebar's recent list.
        if !page.is_journal {
            self.record_recent(page.id);
        }
        if let Some(ix) = self.tab_index_for(page.id) {
            self.activate_tab(ix, window, cx);
        } else {
            self.tabs.push(OpenTab {
                kind: TabKind::Page(page.id),
                title: page.title.into(),
            });
            self.activate_tab(self.tabs.len() - 1, window, cx);
        }
    }

    /// Open a page in a **background** tab without leaving the current one
    /// (right-click → "Open in new tab"). No-op if it's already open.
    pub fn open_page_in_new_tab(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.tab_index_for(id).is_some() {
            return;
        }
        match self.db.get_page(id) {
            Ok(Some(page)) => {
                self.tabs.push(OpenTab {
                    kind: TabKind::Page(id),
                    title: page.title.into(),
                });
                cx.notify();
            }
            Ok(None) => log::warn!("page {id} not found"),
            Err(e) => log::error!("open page {id}: {e}"),
        }
    }

    fn tab_index_for(&self, id: i64) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| matches!(t.kind, TabKind::Page(pid) if pid == id))
    }

    /// Switch to tab `ix` and (re)build its content. Tabs share one page
    /// editor, so activating a Page tab rebuilds the editor from the DB.
    /// Persist the open page editor before it's dropped/replaced. The
    /// per-keystroke save misses undo/redo (they don't emit `Change`), and the
    /// editor's `Blur` doesn't fire once it's dropped (switching/closing tabs),
    /// so flush here to avoid losing those edits.
    fn flush_page_editor(&mut self, cx: &mut Context<Self>) {
        let Some((id, content, aliases)) = self.page_editor.as_ref().map(|pe| {
            (
                pe.id,
                pe.state.read(cx).value().to_string(),
                pe.alias_state.read(cx).value().to_string(),
            )
        }) else {
            return;
        };
        // Re-index content and save aliases, not just save the body — edits made
        // right before switching/closing a tab don't fire the editors' `Blur`
        // once they're dropped.
        self.persist(id, &content);
        self.commit_aliases(id, &aliases);
        self.signal_doc_changed(cx);
    }

    pub fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Save the page we're leaving before its editor is dropped/replaced.
        self.flush_page_editor(cx);
        // The find bar is per-page; drop it when switching tabs.
        self.page_find = None;
        let Some(tab) = self.tabs.get(ix) else { return };
        let kind = tab.kind.clone();
        // Leaving a view: free its decoded note images (CPU + GPU); they re-decode,
        // downscaled and cheap, when painted again. Only on a real switch, so a
        // same-tab re-activation (e.g. a settings re-render) doesn't churn them.
        if self.active != ix {
            self.release_images(window, cx);
        }
        self.active = ix;
        self.searching = false;
        let is_pdf = matches!(kind, TabKind::Pdf(_));
        match kind {
            TabKind::Journal => {
                self.page_editor = None;
                for i in 0..self.loaded_days {
                    self.ensure_day_editor(date_for_offset(i), window, cx);
                }
            }
            TabKind::Page(id) => self.load_page_editor(id, window, cx),
            TabKind::Pdf(_) => self.page_editor = None,
            TabKind::Whiteboard(_) => self.page_editor = None,
        }
        // Focus the AppView so the window's key dispatch reaches its global shortcuts
        // (⌘F, ⌘W, …) right after a tab click — without having to click into the
        // content first. PDFs manage their own focus (the viewer grabs it on click).
        if !is_pdf {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    /// Close tab `ix`. The Journal (index 0) is pinned and never closes.
    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix == 0 || ix >= self.tabs.len() {
            return;
        }
        // Free a PDF's rasterized pages when its viewer closes — both the CPU-side
        // pixel buffers (by dropping the `Arc`s) AND the GPU atlas textures. gpui
        // caches one atlas texture per `RenderImage` on paint and only frees it via
        // `drop_image`; a raw `ImageSource::Render` is never auto-evicted, so without
        // this the textures leak and accumulate across open/close cycles.
        let evict = match &self.tabs[ix].kind {
            TabKind::Pdf(path) => Some(path.clone()),
            _ => None,
        };
        if let Some(path) = evict
            && let Some(view) = self.pdf_views.remove(&path)
        {
            view.update(cx, |v, cx| v.release(window, cx));
        }
        // Drop a closing board's view entity (no GPU textures to release yet).
        if let TabKind::Whiteboard(id) = self.tabs[ix].kind {
            self.whiteboard_views.remove(&id);
        }
        self.tabs.remove(ix);
        if self.active > ix {
            self.active -= 1;
        } else if self.active == ix {
            self.active = self.active.min(self.tabs.len() - 1);
        }
        self.activate_tab(self.active, window, cx);
    }

    /// Switch to the next (`delta = 1`) or previous (`delta = -1`) tab, wrapping
    /// around the ends. No-op with a single tab. Drives Ctrl+Tab / Ctrl+Shift+Tab.
    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() as isize;
        if n <= 1 {
            return;
        }
        let next = (self.active as isize + delta).rem_euclid(n) as usize;
        self.activate_tab(next, window, cx);
    }

    // --- In-page find (⌘F) ---

    /// Open (or refocus) the in-page find bar for the active named page. No-op
    /// unless a Page tab is showing — PDFs have their own find, and the journal
    /// feed uses the global search (⌘⇧F).
    pub fn open_page_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(
            self.tabs.get(self.active).map(|t| &t.kind),
            Some(TabKind::Page(_))
        ) {
            return;
        }
        if let Some(pf) = self.page_find.as_ref() {
            pf.input.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in page…"));
        let sub = cx.subscribe_in(
            &input,
            window,
            |this: &mut AppView, _st, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => this.recompute_page_find(cx),
                // Enter steps to the next match, Shift+Enter to the previous.
                InputEvent::PressEnter { shift, .. } => {
                    this.page_find_step(if *shift { -1 } else { 1 }, cx)
                }
                _ => {}
            },
        );
        input.update(cx, |s, cx| s.focus(window, cx));
        self.page_find = Some(PageFind {
            input,
            query: String::new(),
            current: 0,
            count: 0,
            match_blocks: Vec::new(),
            _sub: sub,
        });
        cx.notify();
    }

    /// Recompute the match count against the active page after the query changed,
    /// resetting to the first match.
    fn recompute_page_find(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.page_find.as_ref().map(|pf| pf.input.clone()) else {
            return;
        };
        let query = input.read(cx).value().to_string();
        let content = self
            .page_editor
            .as_ref()
            .map(|pe| pe.state.read(cx).value().to_string())
            .unwrap_or_default();
        let blocks = gpui_markdown::find_matches(&content, &query);
        if let Some(pf) = self.page_find.as_mut() {
            pf.query = query;
            pf.count = blocks.len();
            pf.current = 0;
            pf.match_blocks = blocks;
        }
        self.scroll_to_current_match();
        cx.notify();
    }

    /// Step the active find match (`delta`: +1 next, -1 prev), wrapping.
    pub fn page_find_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(pf) = self.page_find.as_mut()
            && pf.count > 0
        {
            let n = pf.count as isize;
            pf.current = (pf.current as isize + delta).rem_euclid(n) as usize;
        }
        self.scroll_to_current_match();
        cx.notify();
    }

    /// Scroll the page so the active find match's block is comfortably visible (a
    /// little below the viewport top). No-op if the block isn't laid out yet or is
    /// already in view, so starting a find on text you're reading doesn't yank it.
    fn scroll_to_current_match(&self) {
        let Some(pf) = self.page_find.as_ref() else {
            return;
        };
        let Some(&block) = pf.match_blocks.get(pf.current) else {
            return;
        };
        let Some(b) = self.md_block_scroll.bounds_for_item(block) else {
            return;
        };
        let viewport = self.page_scroll.bounds();
        if viewport.size.height <= px(0.0) {
            return;
        }
        let margin = px(48.0);
        let (block_top, block_bottom) = (b.origin.y, b.origin.y + b.size.height);
        let (v_top, v_bottom) = (viewport.origin.y, viewport.origin.y + viewport.size.height);
        // Already comfortably visible — leave the view put.
        if block_top >= v_top + margin && block_bottom <= v_bottom - margin {
            return;
        }
        // Bring the block to `margin` below the viewport top. Clamp only at the top
        // (offset 0); the target is always a real, laid-out block, so it can't
        // over-scroll past the content. (Not clamping at `max_offset`, which this
        // plain scroll element doesn't populate — clamping there pinned the offset
        // to 0 and blocked all downward scrolling.)
        let new_y = (self.page_scroll.offset().y - (block_top - (v_top + margin))).min(px(0.0));
        self.page_scroll.set_offset(gpui::point(px(0.0), new_y));
    }

    /// Scroll `scroll` so the editor's caret stays comfortably in view — used on
    /// caret moves (arrow keys) so it doesn't slip off-screen as it crosses the
    /// viewport edge. Mirrors `scroll_to_current_match`'s clamp-at-top behavior.
    fn scroll_caret_into_view(
        &self,
        editor: &Entity<EditorState>,
        scroll: &ScrollHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(cb) = editor.read(cx).caret_screen_bounds() else {
            return;
        };
        let viewport = scroll.bounds();
        if viewport.size.height <= px(0.0) {
            return;
        }
        let margin = px(24.0);
        let (c_top, c_bottom) = (cb.origin.y, cb.origin.y + cb.size.height);
        let (v_top, v_bottom) = (viewport.origin.y, viewport.origin.y + viewport.size.height);
        if c_top >= v_top + margin && c_bottom <= v_bottom - margin {
            return; // already comfortably visible
        }
        let new_y = if c_top < v_top + margin {
            scroll.offset().y + (v_top + margin - c_top)
        } else {
            scroll.offset().y - (c_bottom - (v_bottom - margin))
        };
        scroll.set_offset(gpui::point(px(0.0), new_y.min(px(0.0))));
    }

    /// Close the in-page find bar.
    pub fn close_page_find(&mut self, cx: &mut Context<Self>) {
        if self.page_find.take().is_some() {
            cx.notify();
        }
    }

    /// Focus the sidebar's global search field (expanding the rail if collapsed).
    /// Drives ⌘⇧F — the journal feed's "find", and a quick jump from anywhere.
    pub fn focus_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = false;
        self.search_input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Build the single page editor for page `id` (the active Page tab).
    fn load_page_editor(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let page = match self.db.get_page(id) {
            Ok(Some(p)) => p,
            Ok(None) => {
                log::warn!("page {id} not found");
                self.page_editor = None;
                return;
            }
            Err(e) => {
                log::error!("load page {id}: {e}");
                return;
            }
        };
        let pid = page.id;
        let state = make_editor(
            &page.content,
            self.wysiwyg,
            self.list_indent,
            self.image_store(),
            self.mermaid_store(),
            self.math_store(),
            window,
            cx,
        );
        self.ensure_content_images(&page.content, cx);
        self.ensure_content_mermaid(&page.content, cx);
        self.ensure_content_math(&page.content, cx);
        let sub = cx.subscribe_in(
            &state,
            window,
            move |this: &mut AppView, st, ev: &EditorEvent, window, cx| match ev {
                EditorEvent::Changed => {
                    // Auto-pair may rewrite + save directly; otherwise save here.
                    // Content only; link re-indexing happens on blur.
                    if !this.maybe_autopair(&SlashTarget::Page(pid), window, cx) {
                        let value = st.read(cx).text().to_string();
                        this.save_page_content(pid, &value, cx);
                    }
                    this.update_slash(SlashTarget::Page(pid), cx);
                    this.schedule_spellcheck(st.clone(), cx);
                }
                EditorEvent::OpenLink(src) => {
                    if let Some(path) = crate::pdf::resolve_path(src) {
                        this.open_pdf(path, window, cx);
                    }
                }
                EditorEvent::SelectionChanged => {
                    this.scroll_caret_into_view(st, &this.page_scroll, cx)
                }
                EditorEvent::EditMath {
                    range,
                    source,
                    at_end,
                } => {
                    this.open_math_edit(
                        st.clone(),
                        SlashTarget::Page(pid),
                        range.clone(),
                        source.clone(),
                        *at_end,
                        window,
                        cx,
                    );
                }
                EditorEvent::MathMenu { source, position } => {
                    // Not editing → no Align items (nothing to re-justify live + persist).
                    this.open_math_menu(source.clone(), *position, false, cx);
                }
            },
        );
        // gpui-editor has no Focus/Blur events; listen on its focus handle.
        let handle = state.read(cx).focus_handle(cx);
        let weak = cx.entity().downgrade();
        let fstate = state.clone();
        let focus_sub = window.on_focus_in(&handle, cx, move |_window, cx| {
            weak.update(cx, |this: &mut AppView, cx| {
                this.page_editing = true;
                // Editing replaces the rendered view (where matches highlight),
                // so the find bar no longer applies.
                this.page_find = None;
                // Spell-check on entering edit mode so existing misspellings show.
                let diags = spell_diagnostics(fstate.read(cx).text());
                fstate.update(cx, |ed, cx| ed.set_diagnostics(diags, cx));
                cx.notify();
            })
            .ok();
        });
        let weak = cx.entity().downgrade();
        let bstate = state.clone();
        let blur_sub = window.on_focus_out(&handle, cx, move |_ev, _window, cx| {
            weak.update(cx, |this: &mut AppView, cx| {
                this.page_editing = false;
                this.slash = None;
                let value = bstate.read(cx).text().to_string();
                this.persist(pid, &value);
                this.refresh_sidebar();
                this.signal_doc_changed(cx);
                cx.notify();
            })
            .ok();
        });
        let backlinks = self.db.backlinks(pid).unwrap_or_default();

        // Inline-editable title: renames the page on Enter or blur.
        let title_state =
            cx.new(|cx| InputState::new(window, cx).default_value(page.title.clone()));
        let title_sub = cx.subscribe_in(
            &title_state,
            window,
            move |this: &mut AppView, st, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    let new = st.read(cx).value().trim().to_string();
                    this.commit_title_rename(pid, new, window, cx);
                }
                _ => {}
            },
        );

        // Alias field: a comma-separated list, committed on Enter/blur.
        let aliases = self.db.get_page_aliases(pid).unwrap_or_default().join(", ");
        let alias_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("alias1, alias2, …")
                .default_value(aliases)
        });
        let alias_sub = cx.subscribe_in(
            &alias_state,
            window,
            move |this: &mut AppView, st, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let value = st.read(cx).value().to_string();
                    this.commit_aliases(pid, &value);
                }
            },
        );

        self.page_editor = Some(PageEditor {
            id: pid,
            title: page.title,
            title_state,
            alias_state,
            is_journal: page.is_journal,
            state,
            prev: page.content,
            _sub: sub,
            _focus_sub: focus_sub,
            _blur_sub: blur_sub,
            _title_sub: title_sub,
            _alias_sub: alias_sub,
            backlinks,
        });
        self.page_editing = false;
    }

    // --- Persistence ---

    /// Save a page's content and re-index its outgoing `[[links]]`. Aliases are
    /// edited via the alias field (see `commit_aliases`), not parsed from the body.
    fn persist(&mut self, page_id: i64, content: &str) {
        if let Err(e) = self.db.set_page_content(page_id, content) {
            log::error!("save page {page_id}: {e}");
        }
        let titles = ui::links::parse_links(content);
        if let Err(e) = self.db.rebuild_page_links(page_id, &titles) {
            log::error!("rebuild links for page {page_id}: {e}");
        }
    }

    /// Save the alias field's comma-separated list as the page's aliases.
    fn commit_aliases(&mut self, page_id: i64, value: &str) {
        let aliases = ui::links::parse_alias_list(value);
        if let Err(e) = self.db.rebuild_page_aliases(page_id, &aliases) {
            log::error!("save aliases for page {page_id}: {e}");
        }
    }

    fn refresh_sidebar(&mut self) {
        self.pages = self.db.list_pages().unwrap_or_default();
        self.whiteboards = self.db.list_whiteboards().unwrap_or_default();
        // An import can add favorites; pick them up so they show without a relaunch.
        self.favorites = self.load_favorites();
        self.templates = self
            .db
            .get_page_by_title(slash::TEMPLATES_PAGE)
            .ok()
            .flatten()
            .map(|p| slash::parse_templates(&p.content))
            .unwrap_or_default();
    }

    /// Run the sidebar search box live. A `pdf:` / `img:` / `page:` prefix filters
    /// by kind. An empty query (no prefix) returns to the feed.
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let q = self.search_input.read(cx).value().to_string();
        self.search = crate::search::run(&self.db, &q);
        // Searching whenever the box has any prefix or term to act on.
        self.searching = !q.trim().is_empty();
        cx.notify();
    }

    /// Apply a results-pane chip: rewrite the search box to that filter's prefix
    /// (keeping the current term) and re-run, so the box and chips stay in sync.
    pub fn set_search_filter(
        &mut self,
        filter: crate::search::Filter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let term = self.search.term.clone();
        let value = format!("{}{}", filter.prefix(), term);
        self.search_input
            .update(cx, |s, cx| s.set_value(value, window, cx));
        self.run_search(cx);
        self.search_input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Open a clicked search hit: a page, the PDF viewer, or the page showing an
    /// image (looked up by reference when not already known).
    pub fn open_search_hit(
        &mut self,
        target: crate::search::Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            crate::search::Target::Page(id) => self.open_page_id(id, window, cx),
            crate::search::Target::Pdf(path) => self.open_pdf(path, window, cx),
            crate::search::Target::Image { src, in_page } => {
                let page = in_page.or_else(|| self.db.page_referencing(&src).ok().flatten());
                if let Some(id) = page {
                    self.open_page_id(id, window, cx);
                }
            }
        }
    }

    // --- Slash-command menu ---

    /// Recompute the slash menu from the target editor's caret (called on
    /// every edit). Opens it at the caret when a `/token` is present.
    fn update_slash(&mut self, target: SlashTarget, cx: &mut Context<Self>) {
        let editor = self.editor_for(&target);
        let Some(editor) = editor else {
            if self.slash.take().is_some() {
                cx.notify();
            }
            return;
        };
        let (value, cursor) = {
            let s = editor.read(cx);
            (s.value().to_string(), s.cursor())
        };
        let Some((trigger, start, query)) = slash::detect(&value, cursor) else {
            if self.slash.take().is_some() {
                cx.notify();
            }
            return;
        };
        let Some(caret) = editor.read(cx).bounds_for_offset(start) else {
            if self.slash.take().is_some() {
                cx.notify();
            }
            return;
        };
        // Only the slash menu has submenu levels; carry the level forward only
        // while the completion stays a slash one.
        let level = self
            .slash
            .as_ref()
            .filter(|s| s.trigger == Trigger::Slash)
            .map_or(SlashLevel::Root, |s| s.level);
        let title = self.slash_title(&target);
        let items = match trigger {
            Trigger::Slash => slash::build_slash_items(level, &query, &self.templates, &title),
            Trigger::Link => slash::build_link_items(&query, &self.pages),
            Trigger::Tag => slash::build_tag_items(&query, &self.pages),
            Trigger::Placeholder => slash::build_placeholder_items(&query),
            Trigger::Math => slash::build_math_items(&query),
        };
        let selected = self.slash.as_ref().map_or(0, |s| s.selected);
        let selected = if items.is_empty() {
            0
        } else {
            selected.min(items.len() - 1)
        };
        self.slash = Some(Slash {
            target,
            trigger,
            query,
            start,
            caret,
            selected,
            level,
            items,
        });
        // Keep the highlighted row visible as the list is filtered/rebuilt.
        self.scroll_slash_into_view();
        cx.notify();
    }

    /// Scroll the open completion menu so the selected row sits inside the viewport.
    /// Keyboard nav (and filtering) can move the selection past the height cap on long
    /// lists — e.g. the ~75-entry `\` LaTeX menu. Geometry mirrors `ui::slash_menu`.
    fn scroll_slash_into_view(&self) {
        let Some(s) = self.slash.as_ref() else {
            return;
        };
        let top = s.selected as f32 * ui::slash_menu::ITEM_H;
        let bot = top + ui::slash_menu::ITEM_H;
        let cur = -f32::from(self.slash_scroll.offset().y);
        let new = if top < cur {
            top
        } else if bot > cur + ui::slash_menu::VIEW_H {
            bot - ui::slash_menu::VIEW_H
        } else {
            return;
        };
        self.slash_scroll.set_offset(gpui::point(px(0.0), px(-new)));
    }

    /// Debounced spell-check for a body editor: re-run after a short idle so we
    /// don't do an OS spell-service round-trip on every keystroke. Replacing
    /// `spell_task` cancels any still-pending run.
    fn schedule_spellcheck(&mut self, editor: Entity<EditorState>, cx: &mut Context<Self>) {
        self.spell_task = Some(cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            editor.update(cx, |editor, cx| {
                let text = editor.text().to_string();
                let diags = spell_diagnostics(&text);
                editor.set_diagnostics(diags, cx);
            });
        }));
    }

    fn slash_title(&self, target: &SlashTarget) -> String {
        match target {
            SlashTarget::Day(d) => d.clone(),
            SlashTarget::Page(_) => self
                .page_editor
                .as_ref()
                .map(|pe| pe.title.clone())
                .unwrap_or_default(),
        }
    }

    /// Confirm the selected entry: open a category submenu, or insert.
    fn confirm_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        enum Act {
            Enter(SlashLevel),
            Insert(String, usize),
            OpenPicker(SlashTarget, usize, gpui::Bounds<gpui::Pixels>),
        }
        let act = {
            let Some(s) = self.slash.as_ref() else { return };
            let Some(item) = s.items.get(s.selected) else {
                cx.notify();
                return;
            };
            match &item.kind {
                ItemKind::Category(level) => Act::Enter(*level),
                ItemKind::Insert { snippet, caret } => Act::Insert(snippet.clone(), *caret),
                ItemKind::TablePicker => Act::OpenPicker(s.target.clone(), s.start, s.caret),
            }
        };
        match act {
            Act::Enter(level) => self.enter_slash_category(level, cx),
            Act::Insert(snippet, caret) => self.insert_slash(snippet, caret, window, cx),
            Act::OpenPicker(target, start, caret) => {
                self.slash = None;
                let rows_input = cx.new(|cx| InputState::new(window, cx).placeholder("rows"));
                let cols_input = cx.new(|cx| InputState::new(window, cx).placeholder("cols"));
                self.table_picker = Some(TablePicker {
                    target,
                    start,
                    caret,
                    rows: 0,
                    cols: 0,
                    style: TableDesign::Grid,
                    rows_input,
                    cols_input,
                });
                cx.notify();
            }
        }
    }

    /// Hovering a slash-menu item moves the selection to it, so the highlighted
    /// row is the one both a click and Enter accept.
    pub fn slash_hover(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(s) = self.slash.as_mut()
            && i < s.items.len()
            && s.selected != i
        {
            s.selected = i;
            cx.notify();
        }
    }

    /// Click a slash-menu item: select it, then accept like Enter. Driven from the
    /// menu's `on_mouse_down` (which stops propagation) so it fires before the press
    /// can blur the editor — the insertion lands and focus stays put.
    pub fn click_slash(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.slash.as_mut() {
            Some(s) if i < s.items.len() => s.selected = i,
            _ => return,
        }
        self.confirm_slash(window, cx);
    }

    /// Switch the open menu to a level (root or a submenu) and rebuild it.
    fn enter_slash_category(&mut self, level: SlashLevel, cx: &mut Context<Self>) {
        let Some((query, target, start, caret)) = self
            .slash
            .as_ref()
            .map(|s| (s.query.clone(), s.target.clone(), s.start, s.caret))
        else {
            return;
        };
        let title = self.slash_title(&target);
        let items = slash::build_slash_items(level, &query, &self.templates, &title);
        self.slash = Some(Slash {
            target,
            trigger: Trigger::Slash,
            query,
            start,
            caret,
            selected: 0,
            level,
            items,
        });
        cx.notify();
    }

    /// `InsertTab` handler: insert two spaces at the cursor of the focused
    /// day/page editor (auto-grow editors aren't gpui-component-indentable, so
    /// Tab is handled here). Propagates when no editor is focused so Tab works
    /// normally elsewhere (search box, dialogs).
    fn on_insert_tab(&mut self, _: &InsertTab, window: &mut Window, cx: &mut Context<Self>) {
        // If a completion menu is open, Tab accepts the selection (like Enter).
        if self.slash.is_some() {
            self.confirm_slash(window, cx);
            return;
        }
        let Some(target) = self.focused_editor_target() else {
            cx.propagate();
            return;
        };
        let Some(editor) = self.editor_for(&target) else {
            cx.propagate();
            return;
        };
        let value = editor.read(cx).value().to_string();
        let cursor = editor.read(cx).cursor().min(value.len());
        // On a list/quote line, Tab indents the whole item; elsewhere it inserts the
        // configured indent (default four spaces) at the caret.
        let indent = self.list_indent_str();
        let (new, caret) =
            gpui_markdown::indent_list_line(&value, cursor, &indent).unwrap_or_else(|| {
                (
                    format!("{}{indent}{}", &value[..cursor], &value[cursor..]),
                    cursor + indent.len(),
                )
            });
        self.apply_editor_edit(&target, &editor, new, caret, window, cx);
    }

    /// `Outdent` (Shift+Tab): remove one indent level from the caret's line.
    /// No-op when there's nothing to remove (so it doesn't shift focus).
    fn on_outdent(&mut self, _: &Outdent, window: &mut Window, cx: &mut Context<Self>) {
        if self.slash.is_some() {
            return;
        }
        let Some(target) = self.focused_editor_target() else {
            cx.propagate();
            return;
        };
        let Some(editor) = self.editor_for(&target) else {
            cx.propagate();
            return;
        };
        let value = editor.read(cx).value().to_string();
        let cursor = editor.read(cx).cursor().min(value.len());
        let indent = self.list_indent_str();
        if let Some((new, caret)) = gpui_markdown::outdent_line(&value, cursor, &indent) {
            self.apply_editor_edit(&target, &editor, new, caret, window, cx);
        }
    }

    /// Replace a focused editor's text and place the caret, then persist + signal.
    /// Shared by the Tab/Shift+Tab handlers.
    fn apply_editor_edit(
        &mut self,
        target: &SlashTarget,
        editor: &Entity<EditorState>,
        new: String,
        caret: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        editor.update(cx, |st, cx| {
            st.set_text(new.clone(), cx);
            st.set_cursor(caret, cx);
        });
        match target {
            SlashTarget::Day(d) => self.save_journal(d, &new, cx),
            SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
        }
        cx.notify();
    }

    /// Insert a snippet at the `/query`, then close the menu.
    fn insert_slash(
        &mut self,
        snippet: String,
        caret: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.slash.take() else { return };
        let Some(editor) = self.editor_for(&s.target) else {
            cx.notify();
            return;
        };
        let value = editor.read(cx).value().to_string();
        let cursor = editor.read(cx).cursor().min(value.len());
        let start = s.start.min(cursor);
        // If auto-pairing already placed this snippet's own closing delimiter
        // right after the caret (e.g. the `]]` from `[[`), absorb it so the
        // completion doesn't double up (`[[Title]]]]`).
        let mut tail = cursor;
        for closer in ["]]", "}}"] {
            if snippet.ends_with(closer) && value[tail..].starts_with(closer) {
                tail += closer.len();
                break;
            }
        }
        let new = format!("{}{}{}", &value[..start], snippet, &value[tail..]);
        let caret_off = start + caret;
        editor.update(cx, |st, cx| {
            st.set_text(new.clone(), cx);
            st.set_cursor(caret_off, cx);
            // If the snippet dropped the caret into a $$…$$ block (i.e. `/math`), open the
            // structural editor on it rather than leaving the user in raw source. A no-op for
            // every other snippet.
            st.edit_math_at_caret(cx);
        });
        match &s.target {
            SlashTarget::Day(d) => self.save_journal(d, &new, cx),
            SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
        }
        cx.notify();
    }

    fn editor_for(&self, target: &SlashTarget) -> Option<Entity<EditorState>> {
        match target {
            SlashTarget::Day(d) => self.day_editors.get(d).map(|de| de.state.clone()),
            SlashTarget::Page(_) => self.page_editor.as_ref().map(|pe| pe.state.clone()),
        }
    }

    /// Hovering the `/table` picker grid previews a size (1-based; 0 = none).
    pub fn table_picker_hover(&mut self, rows: usize, cols: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.table_picker.as_mut()
            && (p.rows != rows || p.cols != cols)
        {
            p.rows = rows;
            p.cols = cols;
            cx.notify();
        }
    }

    /// Select a table design in the open picker (highlighted; its marker is
    /// prepended when a size is then chosen).
    pub fn table_picker_set_style(&mut self, style: TableDesign, cx: &mut Context<Self>) {
        if let Some(p) = self.table_picker.as_mut() {
            p.style = style;
            cx.notify();
        }
    }

    /// Recompute the alignment toolbar from `editor`'s caret: show it (anchored at
    /// the caret) while the caret is in a table cell, hide it otherwise. Called on
    /// the editor's `SelectionChanged` / `Changed`; only notifies when it changes.
    /// Close the `/table` picker without inserting.
    pub fn cancel_table_picker(&mut self, cx: &mut Context<Self>) {
        if self.table_picker.take().is_some() {
            cx.notify();
        }
    }

    /// Insert a `rows`×`cols` Markdown table (header + separator + body, empty
    /// cells) at the picker's start, replacing the `/table` query, caret in the
    /// first cell.
    pub fn table_picker_pick(&mut self, rows: usize, cols: usize, cx: &mut Context<Self>) {
        let Some(p) = self.table_picker.take() else {
            return;
        };
        let (rows, cols) = (rows.max(1), cols.max(1));
        let Some(editor) = self.editor_for(&p.target) else {
            cx.notify();
            return;
        };
        let row = format!("|{}", "  |".repeat(cols));
        let sep = format!("|{}", " --- |".repeat(cols));
        // The design's hidden marker (if any) goes on the line directly above the
        // header, so the editor associates it with this table.
        let mut lines = Vec::new();
        if let Some(marker) = p.style.marker() {
            lines.push(marker.to_string());
        }
        // Byte length of the marker line(s) + their newline, before the header.
        let header_off: usize = lines.iter().map(|l| l.len() + 1).sum();
        lines.push(row.clone());
        lines.push(sep);
        for _ in 1..rows {
            lines.push(row.clone());
        }
        let snippet = format!("{}\n", lines.join("\n"));
        let value = editor.read(cx).value().to_string();
        let cursor = editor.read(cx).cursor().min(value.len());
        let start = p.start.min(cursor);
        let new = format!("{}{}{}", &value[..start], snippet, &value[cursor..]);
        let caret_off = start + header_off + 2; // first header cell, just after "| "
        editor.update(cx, |st, cx| {
            st.set_text(new.clone(), cx);
            st.set_cursor(caret_off, cx);
        });
        match &p.target {
            SlashTarget::Day(d) => self.save_journal(d, &new, cx),
            SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
        }
        cx.notify();
    }

    /// Insert a table from the picker's typed custom dimensions (for sizes beyond
    /// the hover grid). No-op unless both fields are positive numbers.
    pub fn table_picker_insert_custom(&mut self, cx: &mut Context<Self>) {
        let (rows, cols) = match self.table_picker.as_ref() {
            Some(p) => (
                p.rows_input
                    .read(cx)
                    .value()
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(0),
                p.cols_input
                    .read(cx)
                    .value()
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(0),
            ),
            None => return,
        };
        if rows == 0 || cols == 0 {
            return;
        }
        self.table_picker_pick(rows.min(100), cols.min(100), cx);
    }

    /// On Enter with the slash menu closed: continue a markdown list / blockquote
    /// onto the next line (indent preserved, ordered numbers incremented), or
    /// remove the marker when the current item is empty. Returns whether it
    /// handled the Enter (so the caller skips inserting a plain newline).
    fn continue_list(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.focused_editor_target() else {
            return false;
        };
        let Some(editor) = self.editor_for(&target) else {
            return false;
        };
        let value = editor.read(cx).value().to_string();
        let cursor = editor.read(cx).cursor().min(value.len());
        let Some(edit) = gpui_markdown::list_continuation(&value, cursor) else {
            return false;
        };
        let (new, caret) = match edit {
            gpui_markdown::ListEdit::Continue(insert) => (
                format!("{}{}{}", &value[..cursor], insert, &value[cursor..]),
                cursor + insert.len(),
            ),
            gpui_markdown::ListEdit::Exit { start, end } => {
                (format!("{}{}", &value[..start], &value[end..]), start)
            }
        };
        editor.update(cx, |st, cx| {
            st.set_text(new.clone(), cx);
            st.set_cursor(caret, cx);
        });
        match &target {
            SlashTarget::Day(d) => self.save_journal(d, &new, cx),
            SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
        }
        cx.notify();
        true
    }

    /// Shared map of rendered image widths (keyed by source attr offset),
    /// handed to the renderer so its measure callbacks can record sizes.
    pub fn image_widths(&self) -> Rc<RefCell<HashMap<usize, f32>>> {
        self.image_widths.clone()
    }

    /// The downscaling image cache, shared into the markdown image renderer so it
    /// can read ready bitmaps during paint.
    pub fn image_store(&self) -> Rc<RefCell<crate::images::ImageStore>> {
        self.image_store.clone()
    }

    /// The rendered-diagram cache, shared into the markdown mermaid renderer.
    pub fn mermaid_store(&self) -> Rc<RefCell<crate::mermaid::MermaidStore>> {
        self.mermaid_store.clone()
    }

    /// The typeset-formula cache, shared into the markdown math renderer.
    pub fn math_store(&self) -> Rc<RefCell<crate::math::MathStore>> {
        self.math_store.clone()
    }

    /// Ensure the ```mermaid block `source` is rendering/rendered (idempotent).
    /// Called from a not-yet-rendered diagram's placeholder the first time it
    /// paints: claims the slot, then renders mermaid → SVG → bitmap off-thread
    /// (it's a layout-heavy parse) and repaints when it lands.
    pub fn ensure_mermaid_loaded(&mut self, source: SharedString, cx: &mut Context<Self>) {
        {
            let mut store = self.mermaid_store.borrow_mut();
            if store.started(&source) {
                return; // already rendering / ready / failed
            }
            store.begin(source.clone());
        }
        // Build the diagram theme from Zorite's current palette now (it's a
        // thread-local read on this main thread); the result is `Send`.
        let theme = crate::mermaid::current_theme();
        let svg = cx.svg_renderer();
        let store = self.mermaid_store.clone();
        cx.spawn(async move |this, cx| {
            let src = source.to_string();
            let result = cx
                .background_executor()
                .spawn(async move { crate::mermaid::render_to_image(&src, theme, &svg, 1.0) })
                .await;
            store.borrow_mut().finish(source, result);
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// Ensure the `$$…$$` block `source` is typesetting/typeset (idempotent). Called from
    /// a not-yet-rendered formula's placeholder the first time it paints: claims the slot,
    /// then typesets the LaTeX via RaTeX off-thread and repaints when it lands.
    pub fn ensure_math_loaded(&mut self, source: SharedString, cx: &mut Context<Self>) {
        // Tint formulas in the current theme's text color; set_color drops the cached rasters
        // if the theme changed, so a light/dark switch re-renders them.
        let color = theme::text_primary();
        {
            let mut store = self.math_store.borrow_mut();
            store.set_color(color);
            if store.started(&source) {
                return; // already rendering / ready / failed
            }
            store.begin(source.clone());
        }
        let store = self.math_store.clone();
        cx.spawn(async move |this, cx| {
            let src = source.to_string();
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::math::render_to_image(
                        &src,
                        crate::math::FONT_SIZE,
                        crate::math::DPR,
                        color,
                    )
                })
                .await;
            store.borrow_mut().finish(source, result);
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// Open a rendered mermaid diagram in the full-window lightbox overlay (large +
    /// scrollable). Clicking the inline diagram calls this; focusing the overlay lets
    /// it capture Esc.
    pub fn open_mermaid_lightbox(
        &mut self,
        source: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mermaid_lightbox = Some(source);
        window.focus(&self.lightbox_focus, cx);
        cx.notify();
    }

    /// Close the lightbox and hand focus back to the app root (so keyboard shortcuts
    /// keep working).
    pub fn close_mermaid_lightbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mermaid_lightbox = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub fn open_math_menu(
        &mut self,
        source: SharedString,
        anchor: Point<Pixels>,
        alignable: bool,
        cx: &mut Context<Self>,
    ) {
        self.ctx_menu = Some(CtxMenu {
            anchor,
            kind: CtxKind::Formula { source, alignable },
        });
        cx.notify();
    }

    /// Re-justify the formula being edited (the right-click "Align" items). Live feedback;
    /// the marker persists on commit.
    fn ctx_menu_align(&mut self, align: ratex_gpui::MathAlign, cx: &mut Context<Self>) {
        self.ctx_menu = None;
        if let Some(me) = &self.math_edit {
            me.editor.update(cx, |ed, cx| ed.set_align(align, cx));
        }
        cx.notify();
    }

    /// Open the reader-view right-click menu for a day/page (a single "Edit" entry). Built as
    /// our own overlay so a formula's `stop_propagation` suppresses it over the formula.
    pub fn open_edit_menu(
        &mut self,
        target: SlashTarget,
        anchor: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.ctx_menu = Some(CtxMenu {
            anchor,
            kind: CtxKind::Edit(target),
        });
        cx.notify();
    }

    /// The LaTeX source of the open formula menu, taken (closing the menu), or `None` if the
    /// open menu isn't a formula one.
    fn take_ctx_formula(&mut self) -> Option<String> {
        match self.ctx_menu.take()? {
            CtxMenu {
                kind: CtxKind::Formula { source, .. },
                ..
            } => Some(source.to_string()),
            _ => None,
        }
    }

    fn math_menu_copy_latex(&mut self, cx: &mut Context<Self>) {
        if let Some(source) = self.take_ctx_formula() {
            cx.write_to_clipboard(ClipboardItem::new_string(source));
        }
        cx.notify();
    }

    fn math_menu_export_png(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.take_ctx_formula() else {
            return;
        };
        cx.notify();
        let home = std::env::var("HOME").unwrap_or_default();
        let rx = cx.prompt_for_new_path(
            Path::new(&home).join("Desktop").as_path(),
            Some("formula.png"),
        );
        cx.spawn_in(window, async move |_this, _cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let Some(png) = ratex_gpui::render::render_latex_to_png(&source, 48.0, 4.0) else {
                return;
            };
            let _ = std::fs::write(path, png);
        })
        .detach();
    }

    fn math_menu_export_svg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.take_ctx_formula() else {
            return;
        };
        cx.notify();
        let home = std::env::var("HOME").unwrap_or_default();
        let rx = cx.prompt_for_new_path(
            Path::new(&home).join("Desktop").as_path(),
            Some("formula.svg"),
        );
        cx.spawn_in(window, async move |_this, _cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let Some(svg) = ratex_gpui::render::render_latex_to_svg(&source, 48.0) else {
                return;
            };
            let _ = std::fs::write(path, svg);
        })
        .detach();
    }

    /// Run the "Edit" entry of the reader-view menu: enter edit mode for its day/page.
    fn ctx_menu_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(CtxMenu {
            kind: CtxKind::Edit(target),
            ..
        }) = self.ctx_menu.take()
        {
            self.edit_from_reader(&target, window, cx);
        }
        cx.notify();
    }

    /// Open the structural editor for a `$$` block (clicked or arrowed into): seed it from
    /// `latex`, remember the note editor + document + byte range to write back to, and focus
    /// it with the caret at the formula's end (`at_end`) or its start.
    #[allow(clippy::too_many_arguments)]
    fn open_math_edit(
        &mut self,
        source: Entity<EditorState>,
        target: SlashTarget,
        range: std::ops::Range<usize>,
        latex: SharedString,
        at_end: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Clicking another formula opens its editor; commit the one we were editing first, so
        // its edits (incl. justification) persist instead of being dropped when math_edit is
        // replaced. Always re-find this block's range against the current content afterward: a
        // just-committed formula (here, or via its deferred blur) may have shifted byte offsets
        // by adding/removing an alignment marker.
        let mut range = range;
        if self.math_edit.is_some() {
            self.commit_math_edit(cx);
        }
        // Re-find THIS block by its exact LaTeX against the current content (the commit above,
        // or this block's own deferred blur, may have shifted offsets). Matching the source —
        // not a guessed-nearest range — keeps us on the right block; bail if it's gone.
        match source.read(cx).find_math_block(&latex, range.start) {
            Some(r) => range = r,
            None => return,
        }
        // Render the editor at the formula's displayed font + reserve only its height, so the
        // formula stays put (no jump); the height comes from the cached static render.
        let height = self
            .math_store
            .borrow()
            .get(&latex)
            .map_or(px(56.0), |(_, _, h)| px(h + 16.0));
        // Open at the block's saved alignment so the in-line editor matches the display block.
        let align = to_ratex_align(source.read(cx).math_align(range.start));
        let editor = cx.new(|cx| {
            ratex_gpui::MathEditor::from_latex(
                &latex,
                crate::math::FONT_SIZE,
                at_end,
                align,
                ratex_gpui::MathTheme {
                    fg: theme::text_primary(),
                    muted: theme::text_secondary(),
                    panel: theme::elevated(),
                    border: theme::divider(),
                    accent: theme::accent(),
                    accent_bg: theme::accent_tint(),
                },
                cx,
            )
        });
        let focus = editor.read(cx).focus_handle();
        // Reserve the gap at the block + host the structural editor there, in the text flow.
        source.update(cx, |e, cx| {
            e.set_editing_block(range, editor.clone().into(), height, cx)
        });
        // Commit when the math editor loses focus (the user clicks away). Guard on identity:
        // if a click on another formula already committed + replaced us, this stale blur must
        // not commit the NEW edit. (Compare the active edit's editor to ours.)
        let weak = cx.entity().downgrade();
        let editor_id = editor.entity_id();
        let blur_sub = window.on_focus_out(&focus, cx, move |_ev, _window, cx| {
            weak.update(cx, |this: &mut AppView, cx| {
                if this
                    .math_edit
                    .as_ref()
                    .is_some_and(|m| m.editor.entity_id() == editor_id)
                {
                    this.commit_math_edit(cx);
                }
            })
            .ok();
        });
        // Arrowing past a formula boundary flows the caret back into the surrounding text;
        // a right-click while editing opens the formula menu (copy LaTeX / export).
        let nav_sub = cx.subscribe_in(
            &editor,
            window,
            |this, editor, ev: &ratex_gpui::MathNav, window, cx| match ev {
                ratex_gpui::MathNav::Exit { after } => this.exit_math_edit(*after, window, cx),
                ratex_gpui::MathNav::ContextMenu { position } => {
                    let latex = editor.read(cx).to_latex();
                    // Editing → offer Align (the in-line editor can re-justify live).
                    this.open_math_menu(latex.into(), *position, true, cx);
                }
            },
        );
        self.math_edit = Some(MathEdit {
            editor,
            source,
            target,
            _blur_sub: blur_sub,
            _nav_sub: nav_sub,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Commit the structural edit: serialize the formula to LaTeX, splice it back into the
    /// `$$…$$` block, persist, and return the note editor + the block's new byte range (so the
    /// caret can flow out to it). No-op (→ `None`) if the source range shifted out of bounds.
    fn commit_math_edit(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(Entity<EditorState>, std::ops::Range<usize>)> {
        let edit = self.math_edit.take()?;
        let latex = edit.editor.read(cx).to_latex();
        let align = to_editor_align(edit.editor.read(cx).align());
        let block = format!("$$\n{latex}\n$$");
        // End the in-line edit (closes the gap, re-renders the formula) + get the range.
        let range = edit.source.update(cx, |e, cx| e.end_editing_block(cx))?;
        // Safety: only splice if the range still starts a `$$` block. A stale/shifted range
        // would otherwise insert the block at the wrong offset and corrupt the document — drop
        // the (rare) edit instead.
        if !edit.source.read(cx).is_math_block_range(&range) {
            cx.notify();
            return None;
        }
        // Fold the alignment marker into the same recorded edit: replace the block (and any
        // existing `<!-- math:ALIGN -->` line above it) with `<marker?>` + the new block.
        let (full_range, prefix) = edit.source.read(cx).math_marker_edit(range, align);
        let replacement = format!("{prefix}{block}");
        // The new block sits after the (possibly empty) marker prefix.
        let block_start = full_range.start + prefix.len();
        let new_range = block_start..block_start + block.len();
        // Recorded (undoable) splice — NOT `set_text`, which would wipe the document's undo
        // history. `replace_range` snaps to char boundaries, so a shifted/stale range can't
        // panic; read the result back rather than splicing the string ourselves.
        edit.source
            .update(cx, |e, cx| e.replace_range(full_range, &replacement, cx));
        let new = edit.source.read(cx).text().to_string();
        // Rasterize the edited formula into the shared store, or the block-math provider
        // can't find the (now-changed) LaTeX and the block shows raw `$$…$$`.
        self.ensure_content_math(&new, cx);
        match &edit.target {
            SlashTarget::Day(key) => self.save_journal(key, &new, cx),
            SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
        }
        cx.notify();
        Some((edit.source, new_range))
    }

    /// The caret arrowed past a formula's edge: commit the edit, then seat the text caret on
    /// the line just before/after the block (and re-focus the note editor) — the keyboard path
    /// out of the structural editor, as opposed to clicking away.
    fn exit_math_edit(&mut self, after: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((source, block)) = self.commit_math_edit(cx) {
            source.update(cx, |e, cx| e.exit_math(block, after, window, cx));
        }
    }

    /// Kick off decoding for every standalone image in `content`, so an editor in
    /// WYSIWYG mode can render them inline (W4) rather than as raw `![](src)`.
    /// `ensure_image_loaded` dedupes, so re-scanning is cheap; a finished decode
    /// notifies → repaint → the editor's block-image provider finds the bitmap.
    fn ensure_content_images(&mut self, content: &str, cx: &mut Context<Self>) {
        for info in gpui_markdown::images(content) {
            self.ensure_image_loaded(info.src, cx);
        }
    }

    /// Kick off the off-thread render of every ```mermaid block in `content`, so
    /// an editor in WYSIWYG mode can render them as diagrams. Idempotent (the
    /// store dedupes); a finished render notifies → repaint → the editor's mermaid
    /// provider finds the bitmap. Uses the editor's extraction so the cache key
    /// matches what the editor looks up.
    fn ensure_content_mermaid(&mut self, content: &str, cx: &mut Context<Self>) {
        for source in gpui_editor::mermaid_sources(content) {
            self.ensure_mermaid_loaded(source, cx);
        }
    }

    /// Kick off the off-thread typeset of every `$$…$$` block in `content`, so an editor
    /// in WYSIWYG mode can render them as equations. Idempotent; a finished render
    /// notifies → repaint → the editor's math provider finds the bitmap.
    fn ensure_content_math(&mut self, content: &str, cx: &mut Context<Self>) {
        for source in gpui_editor::math_sources(content) {
            self.ensure_math_loaded(source, cx);
        }
        // Inline `$…$` formulas typeset into the same store (keyed by LaTeX); the editor reuses
        // the raster scaled to text size.
        for source in gpui_editor::inline_math_sources(content) {
            self.ensure_math_loaded(source, cx);
        }
    }

    /// Ensure the image at `src` is decoding/decoded (idempotent). Called from a
    /// not-yet-loaded image's placeholder the first time it paints: claims the
    /// slot and queues a downscaled decode (run a bounded few at a time by
    /// [`Self::pump_image_decodes`]).
    pub fn ensure_image_loaded(&mut self, src: SharedString, cx: &mut Context<Self>) {
        if !self.image_store.borrow_mut().begin(src.clone()) {
            return; // already loading / ready / failed
        }
        match crate::paths::resolve_local(&src).filter(|p| p.exists()) {
            Some(path) => self.image_queue.push_back((src, path)),
            None => {
                self.image_store.borrow_mut().finish(src, None);
                cx.notify();
                return;
            }
        }
        self.pump_image_decodes(cx);
    }

    /// Decode queued images off-thread, up to [`MAX_IMAGE_DECODES`] at a time,
    /// each storing its bitmap, repainting, and pumping the next on completion.
    /// The cap bounds the transient full-resolution decode buffers (a page of
    /// 12 MP photos would otherwise hold one ~35 MB buffer per image at once).
    fn pump_image_decodes(&mut self, cx: &mut Context<Self>) {
        while self.image_decodes < MAX_IMAGE_DECODES {
            let Some((src, path)) = self.image_queue.pop_front() else {
                return;
            };
            self.image_decodes += 1;
            let store = self.image_store.clone();
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move { crate::images::decode_scaled(&path) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    store.borrow_mut().finish(src, decoded);
                    this.image_decodes -= 1;
                    this.pump_image_decodes(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Free every decoded note image (CPU + GPU) and drop any pending decodes.
    /// Called when the visible view changes — switching tabs or closing one — so
    /// images don't accumulate for the life of the window; they re-decode
    /// (downscaled, cheap) on return.
    fn release_images(&mut self, window: &mut Window, cx: &mut App) {
        self.image_queue.clear();
        self.image_store.borrow_mut().release(window, cx);
        // Free the pre-rotated whiteboard bitmaps' GPU textures too.
        for (_, arc) in self.rotated_images.drain() {
            cx.drop_image(arc, Some(window));
        }
        // Mermaid bitmaps are per-window and can be several MB each; drop them on
        // view change like images (they re-render off-thread when shown again).
        // Without this, a window that showed the journal once kept its diagrams
        // in memory while displaying an unrelated page.
        self.mermaid_store.borrow_mut().clear();
    }

    /// The image currently being resized, as `(attr offset, live width)`, so
    /// the renderer can preview that width while dragging.
    pub fn image_drag_snapshot(&self) -> Option<(usize, f32)> {
        self.image_drag
            .as_ref()
            .map(|d| (d.attr_target.start, d.width))
    }

    /// Open a PDF in its own viewer tab (focusing it if already open). Reads the
    /// file + page sizes off-thread for instant layout; the pages themselves are
    /// rasterized lazily by `ensure_pdf_window` as they scroll into view.
    pub fn open_pdf(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| matches!(&t.kind, TabKind::Pdf(p) if *p == path))
        {
            self.activate_tab(ix, window, cx);
            return;
        }
        let title: SharedString = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "PDF".to_string())
            .into();
        self.tabs.push(OpenTab {
            kind: TabKind::Pdf(path.clone()),
            title,
        });
        self.activate_tab(self.tabs.len() - 1, window, cx);

        if self.pdf_views.contains_key(&path) {
            return; // viewer already open
        }
        // Each viewer is an independent, page-virtualized component: it loads and
        // measures the file off-thread and rasterizes only the on-screen pages. It
        // reads its chrome colors from the theme at paint time, so it follows live
        // theme changes (and can differ per window) on its own.
        let view = cx.new(|cx| {
            crate::pdf::PdfView::new(
                path.clone(),
                Rc::new(|| crate::pdf::PdfStyle {
                    bg: theme::bg_content(),
                    border: theme::border_subtle(),
                    placeholder_bg: theme::glass(),
                    placeholder_fg: theme::text_tertiary(),
                    header_fg: theme::text_secondary(),
                    header_muted: theme::text_tertiary(),
                }),
                Rc::new(crate::pdf::quality),
                cx,
            )
        });
        // Markup: load this PDF's saved highlights from its per-PDF "(highlights)"
        // page and render them; clicking one opens that notes page.
        let notes_title = crate::pdf::highlights_title(&path);
        let highlights = crate::pdf::parse_highlights(
            &self
                .db
                .get_page_by_title(&notes_title)
                .ok()
                .flatten()
                .map(|p| p.content)
                .unwrap_or_default(),
        );
        let weak = cx.entity().downgrade();
        let create_weak = weak.clone();
        let create_path = path.clone();
        view.update(cx, move |v, cx| {
            v.set_highlights(highlights, cx);
            v.set_highlight_palette(crate::pdf::highlight_palette(), cx);
            v.set_on_highlight(Rc::new(move |_id, window, cx| {
                if let Some(app) = weak.upgrade() {
                    app.update(cx, |a, cx| a.open_page_title(&notes_title, window, cx));
                }
            }));
            // Drag-select in the viewer → append a highlight block to the notes page.
            v.set_on_create_highlight(Rc::new(move |page, quote, occ, color, window, cx| {
                if let Some(app) = create_weak.upgrade() {
                    app.update(cx, |a, cx| {
                        a.add_pdf_highlight(
                            &create_path,
                            page,
                            &quote,
                            occ,
                            color.as_ref(),
                            window,
                            cx,
                        )
                    });
                }
            }));
        });
        // Re-render the surrounding UI on lock/unlock, so the password prompt
        // appears when an encrypted PDF loads and is replaced by the viewer once
        // it's unlocked; clear the password field once it's no longer needed.
        cx.subscribe_in(
            &view,
            window,
            |this, view, _ev: &crate::pdf::PdfEvent, window, cx| {
                if view.read(cx).is_locked() {
                    // Prompt just appeared (or a wrong password) — focus the field.
                    this.pdf_password_input
                        .update(cx, |s, cx| s.focus(window, cx));
                } else {
                    // Unlocked — clear the field so the secret isn't kept around.
                    this.pdf_password_input
                        .update(cx, |s, cx| s.set_value("", window, cx));
                }
                cx.notify();
            },
        )
        .detach();
        self.pdf_views.insert(path, view);
    }

    /// Open a whiteboard in its own canvas tab (focusing it if already open). A
    /// board is a `kind = 'whiteboard'` page; its canvas JSON lives in the page
    /// `content`, deserialized into a `Scene` the view renders.
    pub fn open_whiteboard(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| matches!(t.kind, TabKind::Whiteboard(bid) if bid == id))
        {
            self.activate_tab(ix, window, cx);
            return;
        }
        let (title, content) = match self.db.get_page(id) {
            Ok(Some(p)) => (p.title, p.content),
            Ok(None) => {
                log::warn!("whiteboard {id} not found");
                return;
            }
            Err(e) => {
                log::error!("open whiteboard {id}: {e}");
                return;
            }
        };
        self.tabs.push(OpenTab {
            kind: TabKind::Whiteboard(id),
            title: title.into(),
        });
        self.activate_tab(self.tabs.len() - 1, window, cx);
        if self.whiteboard_views.contains_key(&id) {
            return; // view already built (e.g. re-open of a background tab)
        }
        let scene = crate::whiteboard::Scene::from_json(&content);
        let view = cx.new(|cx| {
            crate::whiteboard::WhiteboardView::new(scene, crate::whiteboard::style(), cx)
        });
        // Persist edits (strokes, camera) back to the board's page row; pick a
        // page for a placed card; open a page when a card is double-clicked.
        let weak = cx.entity().downgrade();
        let saved_colors = self.saved_colors_list();
        let board_font = self.board_font(id);
        let toolbar_pos = self.saved_toolbar_pos();
        let toolbar_vertical = self.saved_toolbar_vertical();
        view.update(cx, |v, cx| {
            let w = weak.clone();
            v.set_on_change(Rc::new(move |json, _window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, _| a.save_board(id, &json));
                }
            }));
            let w = weak.clone();
            v.set_on_place_embed(Rc::new(move |x, y, window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.place_embed_dialog(id, x, y, window, cx));
                }
            }));
            let w = weak.clone();
            v.set_on_open(Rc::new(move |page_id, window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.open_page_id(page_id, window, cx));
                }
            }));
            let w = weak.clone();
            v.set_on_save_template(Rc::new(move |json, window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.save_template_dialog(json, window, cx));
                }
            }));
            let w = weak.clone();
            v.set_on_delete_template(Rc::new(move |tid, window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.confirm_delete_template(tid, window, cx));
                }
            }));
            // Serve a decoded bitmap for an image element from the shared store
            // (decoding off-thread on the first ask; the decode notifies the
            // AppView, which re-renders the board).
            let w = weak.clone();
            v.set_on_image(Rc::new(move |src, rotation, _window, cx| {
                let app = w.upgrade()?;
                app.update(cx, |a, cx| a.board_image(src, rotation, cx))
            }));
            // Image tool click → file picker → place at the click point.
            let w = weak.clone();
            v.set_on_place_image(Rc::new(move |x, y, window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.pick_image_for_board(id, x, y, window, cx));
                }
            }));
            // Files dropped on the canvas → import the images, place at the drop.
            let w = weak.clone();
            v.set_on_drop_files(Rc::new(move |paths, x, y, _window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.drop_files_on_board(id, paths, x, y, cx));
                }
            }));
            // ⌘C / ⌘X → put the serialized selection on the system clipboard,
            // tagged so paste can tell it from arbitrary text (see `on_paste_image`).
            v.set_on_copy(Rc::new(move |json, _window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(format!("{WB_CLIP_PREFIX}{json}")));
            }));
            // Context-menu Paste → hand back copied board elements from the clipboard
            // (keyboard ⌘V is routed through `on_paste_image`).
            v.set_on_paste(Rc::new(|_window, cx| clipboard_board_json(cx)));
            // Saved-color palette: seed from settings, persist + sync on change.
            v.set_saved_colors(saved_colors, cx);
            let w = weak.clone();
            v.set_on_save_colors(Rc::new(move |colors, _window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.persist_saved_colors(colors, cx));
                }
            }));
            // Per-board font: apply the persisted face (if any) and wire the
            // Font flyout (upload / revert to default).
            if let Some(font) = board_font {
                v.set_font(font, cx);
            }
            let w = weak.clone();
            v.set_on_pick_font(Rc::new(move |pick, window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.choose_board_font(id, pick, window, cx));
                }
            }));
            // Movable toolbar: apply the persisted position + orientation, and
            // persist on change (drag, reset, or R-flip).
            v.set_toolbar_pos(toolbar_pos, cx);
            v.set_toolbar_vertical(toolbar_vertical, cx);
            let w = weak.clone();
            v.set_on_move_toolbar(Rc::new(move |pos, vertical, _window, cx| {
                if let Some(app) = w.upgrade() {
                    app.update(cx, |a, cx| a.persist_toolbar(pos, vertical, cx));
                }
            }));
        });
        self.whiteboard_views.insert(id, view);
        // Seed the new view with the current template list.
        self.refresh_templates(cx);
    }

    /// The user's saved whiteboard colors (packed `0xRRGGBBAA`), from settings.
    fn saved_colors_list(&self) -> Vec<u32> {
        self.db
            .get_setting("whiteboard_swatches")
            .map(|s| s.split(',').filter_map(|t| t.parse::<u32>().ok()).collect())
            .unwrap_or_default()
    }

    /// Persist the saved-color palette and push it to every open board view.
    fn persist_saved_colors(&mut self, colors: Vec<u32>, cx: &mut Context<Self>) {
        let s = colors
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(",");
        if let Err(e) = self.db.set_setting("whiteboard_swatches", &s) {
            log::error!("save whiteboard colors: {e}");
        }
        // Defer the view sync: this runs from inside a board view's own update
        // (the picker's `+` / a removed swatch), so pushing back into that same
        // view now would re-enter it and panic.
        let views: Vec<_> = self.whiteboard_views.values().cloned().collect();
        cx.defer(move |cx| {
            for view in &views {
                let colors = colors.clone();
                view.update(cx, |v, cx| v.set_saved_colors(colors, cx));
            }
        });
    }

    /// The persisted whiteboard toolbar position (`"x,y"` in settings), or `None`
    /// for the default top-center. Global (the same position for every board).
    fn saved_toolbar_pos(&self) -> Option<(f32, f32)> {
        let s = self.db.get_setting("whiteboard_toolbar_pos")?;
        let (a, b) = s.split_once(',')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    }

    /// The persisted toolbar orientation (`"1"` = vertical).
    fn saved_toolbar_vertical(&self) -> bool {
        self.db
            .get_setting("whiteboard_toolbar_vertical")
            .as_deref()
            == Some("1")
    }

    /// Persist the toolbar position and push it to every open board.
    fn persist_toolbar(&mut self, pos: Option<(f32, f32)>, vertical: bool, cx: &mut Context<Self>) {
        let s = pos.map_or(String::new(), |(x, y)| format!("{x},{y}"));
        if let Err(e) = self.db.set_setting("whiteboard_toolbar_pos", &s) {
            log::error!("save whiteboard toolbar pos: {e}");
        }
        let v = if vertical { "1" } else { "0" };
        if let Err(e) = self.db.set_setting("whiteboard_toolbar_vertical", v) {
            log::error!("save whiteboard toolbar orientation: {e}");
        }
        // Defer the view sync (this runs inside the dragging view's own update).
        let views: Vec<_> = self.whiteboard_views.values().cloned().collect();
        cx.defer(move |cx| {
            for view in &views {
                view.update(cx, |v, cx| {
                    v.set_toolbar_pos(pos, cx);
                    v.set_toolbar_vertical(vertical, cx);
                });
            }
        });
    }

    /// Load templates from the DB and push them into every open board view.
    fn refresh_templates(&mut self, cx: &mut Context<Self>) {
        let templates: Vec<crate::whiteboard::Template> = self
            .db
            .list_templates()
            .unwrap_or_default()
            .into_iter()
            .map(|(tid, name, content)| crate::whiteboard::Template::from_json(tid, name, &content))
            .collect();
        for view in self.whiteboard_views.values().cloned().collect::<Vec<_>>() {
            let templates = templates.clone();
            view.update(cx, |v, cx| v.set_templates(templates, cx));
        }
    }

    /// Image tool click on board `board_id` at world `(x, y)`: pick a file, then
    /// import + place it.
    fn pick_image_for_board(
        &mut self,
        board_id: i64,
        x: f32,
        y: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Place".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.place_image_file_on_board(board_id, path, x, y, cx);
            });
        })
        .detach();
    }

    /// Files dropped on board `board_id` at world `(x, y)`: import each supported
    /// image and place it, nudging successive drops so they don't fully overlap.
    fn drop_files_on_board(
        &mut self,
        board_id: i64,
        paths: Vec<PathBuf>,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) {
        let mut n = 0.0;
        for path in paths {
            if crate::images::is_supported(&path) {
                self.place_image_file_on_board(board_id, path, x + n * 16.0, y + n * 16.0, cx);
                n += 1.0;
            }
        }
    }

    /// Copy an image file into the managed images dir, then place it on the board.
    fn place_image_file_on_board(
        &mut self,
        board_id: i64,
        path: PathBuf,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) {
        match crate::images::import_file(&path) {
            Ok(rel) => self.add_image_to_board(board_id, rel.into(), x, y, cx),
            Err(e) => log::error!("import image {}: {e}", path.display()),
        }
    }

    /// Decode `src` (off-thread) to learn its pixel size, cache the bitmap, then
    /// add an image element centered at world `(x, y)` and persist the board.
    fn add_image_to_board(
        &mut self,
        board_id: i64,
        src: SharedString,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = crate::paths::resolve_local(&src) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move { crate::images::decode_scaled(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(img) = decoded else {
                    log::error!("decode image {src}");
                    return;
                };
                let size = img.size(0);
                let (pw, ph) = (size.width.0 as f32, size.height.0 as f32);
                this.image_store.borrow_mut().finish(src.clone(), Some(img));
                if let Some(view) = this.whiteboard_views.get(&board_id).cloned() {
                    let json = view.update(cx, |v, cx| {
                        v.add_image_at(src.to_string(), pw, ph, x, y, cx);
                        v.scene().to_json()
                    });
                    this.save_board(board_id, &json);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The Font flyout fired for a board: upload a face from disk, or revert to
    /// the bundled default. Each board keeps its own face (see [`board_font_key`]).
    fn choose_board_font(
        &mut self,
        board_id: i64,
        pick: crate::whiteboard::FontPick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match pick {
            crate::whiteboard::FontPick::Default => {
                let _ = self.db.set_setting(&board_font_key(board_id), "");
                if let Some(view) = self.whiteboard_views.get(&board_id).cloned() {
                    view.update(cx, |v, cx| {
                        v.set_font(crate::whiteboard::Font::default(), cx)
                    });
                }
            }
            crate::whiteboard::FontPick::Upload => {
                let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Use font".into()),
                });
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(Ok(Some(paths))) = rx.await else {
                        return;
                    };
                    let Some(path) = paths.into_iter().next() else {
                        return;
                    };
                    let _ = this.update(cx, |this, cx| {
                        this.apply_board_font_file(board_id, path, cx);
                    });
                })
                .detach();
            }
        }
    }

    /// Validate a picked font file, copy it into the managed `fonts/` dir, persist
    /// the per-board choice, and apply it to the live view. A file that isn't a
    /// usable face is rejected (and logged) before anything is stored.
    fn apply_board_font_file(&mut self, board_id: i64, path: PathBuf, cx: &mut Context<Self>) {
        let Ok(bytes) = std::fs::read(&path) else {
            log::error!("read font {}", path.display());
            return;
        };
        if crate::whiteboard::Font::from_bytes(bytes, 0).is_none() {
            log::warn!("not a usable font: {}", path.display());
            return;
        }
        let rel = match crate::images::import_font(&path) {
            Ok(rel) => rel,
            Err(e) => {
                log::error!("import font {}: {e}", path.display());
                return;
            }
        };
        let _ = self.db.set_setting(&board_font_key(board_id), &rel);
        self.apply_board_font(board_id, cx);
    }

    /// Load a board's persisted face (if any) and push it to the live view. A
    /// missing/empty ref or an unreadable/invalid file leaves the default in place.
    fn apply_board_font(&self, board_id: i64, cx: &mut Context<Self>) {
        if let (Some(font), Some(view)) = (
            self.board_font(board_id),
            self.whiteboard_views.get(&board_id).cloned(),
        ) {
            view.update(cx, |v, cx| v.set_font(font, cx));
        }
    }

    /// Build a board's persisted face from settings, or `None` for the default.
    fn board_font(&self, board_id: i64) -> Option<crate::whiteboard::Font> {
        let rel = self.db.get_setting(&board_font_key(board_id))?;
        if rel.is_empty() {
            return None;
        }
        let abs = crate::paths::resolve_local(&rel)?;
        let bytes = std::fs::read(&abs).ok()?;
        crate::whiteboard::Font::from_bytes(bytes, 0)
    }

    /// Serve an image element's bitmap for the board, rotated to `rotation`
    /// radians. Upright images come straight from the store; a rotated one is
    /// pre-rotated once per (src, quarter-turn) and cached (gpui can't transform a
    /// raster sprite), since a steady angle re-renders every frame.
    fn board_image(
        &mut self,
        src: &str,
        rotation: f32,
        cx: &mut Context<Self>,
    ) -> Option<gpui::ImageSource> {
        let key: SharedString = src.to_string().into();
        self.ensure_image_loaded(key.clone(), cx);
        let base = self.image_store.borrow().get(&key)?;
        // Snap to a quarter turn (0 / 90 / 180 / 270); upright uses the original.
        let qdeg = ((rotation.to_degrees().round() as i32).rem_euclid(360) + 45) / 90 % 4 * 90;
        if qdeg == 0 {
            return Some(gpui::ImageSource::from(base));
        }
        if let Some(arc) = self.rotated_images.get(&(key.clone(), qdeg)) {
            return Some(gpui::ImageSource::from(arc.clone()));
        }
        let rotated = crate::images::rotate_render_image(&base, qdeg)?;
        self.rotated_images.insert((key, qdeg), rotated.clone());
        Some(gpui::ImageSource::from(rotated))
    }

    /// Persist a whiteboard's canvas JSON and index its page-card embeds as
    /// links (so each referenced page's backlinks show the board).
    fn save_board(&self, id: i64, json: &str) {
        if let Err(e) = self.db.set_page_content(id, json) {
            log::error!("save whiteboard {id}: {e}");
            return;
        }
        let scene = crate::whiteboard::Scene::from_json(json);
        let targets: Vec<i64> = scene
            .elements
            .iter()
            .filter_map(|e| match &e.kind {
                crate::whiteboard::ElementKind::Embed(em) => Some(em.page_id),
                _ => None,
            })
            .collect();
        if let Err(e) = self.db.set_page_links(id, &targets) {
            log::error!("link whiteboard {id}: {e}");
        }
    }

    /// Open the "insert page card" dialog, then place the chosen page as a card
    /// at world `(x, y)` on board `board_id`.
    fn place_embed_dialog(
        &mut self,
        board_id: i64,
        x: f32,
        y: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_page_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let input = self.new_page_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let weak_ok = weak.clone();
            let weak_btn = weak.clone();
            dialog
                .title("Insert page card")
                .w(px(420.0))
                // Enter inserts (the dialog binds enter → ConfirmDialog → on_ok).
                .on_ok(move |_, _window, cx| {
                    let _ = weak_ok.update(cx, |this, cx| {
                        this.insert_embed_from_input(board_id, x, y, cx)
                    });
                    true
                })
                .child(Input::new(&input))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("embed-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("embed-insert")
                                .primary()
                                .label("Insert")
                                .on_click(move |_, window, cx| {
                                    let _ = weak_btn.update(cx, |this, cx| {
                                        this.insert_embed_from_input(board_id, x, y, cx)
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    /// Resolve `title` to a page (creating it) and add it as a card on the board.
    fn insert_embed(&mut self, board_id: i64, title: &str, x: f32, y: f32, cx: &mut Context<Self>) {
        let page = match self.db.get_or_create_page(title) {
            Ok(p) => p,
            Err(e) => {
                log::error!("embed page {title:?}: {e}");
                return;
            }
        };
        if let Some(view) = self.whiteboard_views.get(&board_id).cloned() {
            // Persist here (not via the view's on_change) — we're already inside
            // an AppView update, so a re-entrant save would panic.
            let json = view.update(cx, |v, cx| {
                v.add_embed(page.id, page.title.clone(), x, y, cx);
                v.scene().to_json()
            });
            self.save_board(board_id, &json);
        }
        self.record_recent(page.id);
        self.refresh_sidebar();
        cx.notify();
    }

    /// Insert the page named in the shared input as a card (no-op if blank).
    /// Shared by the Insert button and Enter (`on_ok`).
    fn insert_embed_from_input(&mut self, board_id: i64, x: f32, y: f32, cx: &mut Context<Self>) {
        let title = self.new_page_input.read(cx).value().trim().to_string();
        if !title.is_empty() {
            self.insert_embed(board_id, &title, x, y, cx);
        }
    }

    /// Prompt for a name, then persist the selection JSON as a whiteboard
    /// template (invoked from a board's right-click "Save as template").
    fn save_template_dialog(&mut self, json: String, window: &mut Window, cx: &mut Context<Self>) {
        self.new_page_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let input = self.new_page_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let weak_ok = weak.clone();
            let json_ok = json.clone();
            let weak_btn = weak.clone();
            let json_btn = json.clone();
            dialog
                .title("Save as template")
                .w(px(420.0))
                // The dialog binds `enter` → ConfirmDialog → `on_ok`; without
                // this, Enter closes the dialog without saving (looks like
                // Cancel). Save here, same as the button.
                .on_ok(move |_, _window, cx| {
                    let _ = weak_ok.update(cx, |this, cx| this.save_template_named(&json_ok, cx));
                    true
                })
                .child(Input::new(&input))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("tmpl-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("tmpl-save").primary().label("Save").on_click(
                            move |_, window, cx| {
                                let _ = weak_btn
                                    .update(cx, |this, cx| this.save_template_named(&json_btn, cx));
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    /// Read the template name from the shared page-name input (blank → a default
    /// title) and store it. Shared by the Save button and Enter (`on_ok`).
    fn save_template_named(&mut self, json: &str, cx: &mut Context<Self>) {
        let name = self.new_page_input.read(cx).value().trim().to_string();
        let name = if name.is_empty() {
            "Untitled template".to_string()
        } else {
            name
        };
        self.save_template(&name, json, cx);
    }

    /// Store a template and push the refreshed list to every open board.
    fn save_template(&mut self, name: &str, json: &str, cx: &mut Context<Self>) {
        match self.db.create_template(name, json) {
            Ok(_) => self.refresh_templates(cx),
            Err(e) => log::error!("save template {name:?}: {e}"),
        }
    }

    /// Confirm, then delete a template (invoked from a right-click on its card).
    fn confirm_delete_template(&mut self, tid: i64, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .db
            .list_templates()
            .unwrap_or_default()
            .into_iter()
            .find(|(id, ..)| *id == tid)
            .map(|(_, name, _)| name)
            .unwrap_or_else(|| "this template".to_string());
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let weak = weak.clone();
            dialog
                .title("Delete template?")
                .description(SharedString::from(format!(
                    "“{name}” will be permanently deleted. This can't be undone."
                )))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("Cancel")
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if let Err(e) = this.db.delete_template(tid) {
                            log::error!("delete template {tid}: {e}");
                        } else {
                            this.refresh_templates(cx);
                        }
                    });
                    true
                })
        });
    }

    /// Create a new, distinct whiteboard ("Untitled Whiteboard", suffixed if
    /// taken) and open it. Refreshes the sidebar so the new board shows in the
    /// "Whiteboards" section right away.
    pub fn new_whiteboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.create_whiteboard() {
            Ok(page) => {
                self.open_whiteboard(page.id, window, cx);
                self.refresh_sidebar();
                cx.notify();
            }
            Err(e) => log::error!("new whiteboard: {e}"),
        }
    }

    /// `NewWhiteboard` handler (File menu): create + open a whiteboard canvas.
    fn on_new_whiteboard(
        &mut self,
        _: &NewWhiteboard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_whiteboard(window, cx);
    }

    /// Read the password field and try to unlock the encrypted PDF at `path`.
    fn unlock_pdf(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.pdf_password_input.read(cx).value().to_string();
        if let Some(view) = self.pdf_views.get(path).cloned() {
            view.update(cx, |v, cx| v.unlock(password, cx));
        }
        // Keep focus in the field so a wrong password can be retyped immediately.
        self.pdf_password_input
            .update(cx, |s, cx| s.focus(window, cx));
    }

    /// The card shown in place of an encrypted PDF's viewer until it's unlocked: a
    /// masked password field + Unlock button. `failed` adds an error line after a
    /// wrong attempt. Replaced by the viewer once [`PdfView::unlock`] succeeds.
    fn pdf_password_prompt(
        &self,
        path: PathBuf,
        failed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::bg_content())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w(px(360.0))
                    .p_5()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(theme::border_subtle())
                    .bg(theme::glass())
                    .child(
                        div()
                            .text_size(px(15.0))
                            .text_color(theme::text_primary())
                            .child("🔒 This PDF is password protected"),
                    )
                    .child(Input::new(&self.pdf_password_input).mask_toggle())
                    .children(failed.then(|| {
                        div()
                            .text_size(px(12.0))
                            .text_color(gpui::rgb(0xE5484D))
                            .child("Incorrect password — try again.")
                    }))
                    .child(
                        div().flex().justify_end().child(
                            Button::new("pdf-unlock")
                                .label("Unlock")
                                .primary()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.unlock_pdf(&path, window, cx);
                                })),
                        ),
                    ),
            )
    }

    /// Append a drag-selected highlight to the PDF's per-PDF notes page, then
    /// re-render the open viewer so it shows up immediately.
    // Args mirror the viewer's create-highlight callback (page, quote, occurrence,
    // color) plus the PDF path; bundling them wouldn't read more clearly.
    #[allow(clippy::too_many_arguments)]
    fn add_pdf_highlight(
        &mut self,
        pdf_path: &Path,
        page: usize,
        quote: &str,
        _occurrence: usize,
        color: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let q: String = quote.split_whitespace().collect::<Vec<_>>().join(" ");
        if q.is_empty() {
            return;
        }
        let title = crate::pdf::highlights_title(pdf_path);
        let Ok(p) = self.db.get_or_create_page(&title) else {
            return;
        };
        // `- p{N}: {quote}` + an optional `{color}` (omitted for the default yellow, to
        // keep notes clean) + a reverse link `[[<ref>#pN|↗]]` that opens the PDF and
        // flashes the highlight. The ref is data-dir-relative so it's portable.
        //
        // A selection spanning PDF bullets becomes a *group*: a `- pN:` header (page +
        // color + jump link) with the bullet items as an indented markdown sub-list, so
        // it reads as a list rather than a run-on of `●` glyphs. Each item still
        // re-locates (its text stays a substring of the page line, sans bullet). A
        // single (non-bulleted) selection stays a flat one-line highlight.
        let mut meta = String::new();
        if !color.is_empty() && !color.eq_ignore_ascii_case("yellow") {
            meta.push_str(&format!(" {{{color}}}"));
        }
        meta.push_str(&format!(" [[{}#p{}|↗]]", self.pdf_ref(pdf_path), page + 1));
        let items = crate::pdf::split_bullets(&q);
        let block = if items.len() > 1 {
            let mut b = format!("- p{}:{}", page + 1, meta);
            for item in &items {
                b.push_str(&format!("\n    - {item}"));
            }
            b
        } else {
            format!("- p{}: {}{}", page + 1, items[0], meta)
        };
        let content = if p.content.trim().is_empty() {
            block
        } else {
            format!("{}\n{}", p.content.trim_end(), block)
        };
        self.save_page_content(p.id, &content, cx);
        // The highlights page may have just been created. The sidebar's page tree is
        // filtered to recently-viewed pages, so mark it recent + refresh so it shows up
        // (and signal other windows to pick up the new page).
        self.record_recent(p.id);
        self.refresh_sidebar();
        self.signal_doc_changed(cx);
        cx.notify();
        // Refresh the open viewer's highlights — but *deferred*. We're called from
        // inside that viewer's own mouse handler (its entity is leased), so updating
        // it synchronously would be a reentrant entity update and panic. Run it after
        // the lease ends.
        let highlights = crate::pdf::parse_highlights(&content);
        let path = pdf_path.to_path_buf();
        let view = cx.entity();
        cx.defer(move |cx| {
            view.update(cx, |this, cx| {
                if let Some(v) = this.pdf_views.get(&path) {
                    v.update(cx, |v, cx| v.set_highlights(highlights, cx));
                }
            });
        });
    }

    /// A portable reference string for a PDF, for storing in a `[[…]]` link: relative
    /// to the data dir when possible (e.g. `pdf/file.pdf`, which survives moving the
    /// notes between machines), falling back to the managed `pdf/<name>` location.
    fn pdf_ref(&self, pdf_path: &Path) -> String {
        let data = crate::paths::data_dir();
        pdf_path
            .strip_prefix(&data)
            .ok()
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| {
                format!(
                    "pdf/{}",
                    pdf_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                )
            })
    }

    /// Begin resizing an image: capture the start position and its current
    /// rendered width (measured during paint).
    pub fn start_image_drag(
        &mut self,
        target: SlashTarget,
        attr_target: Range<usize>,
        start_x: Pixels,
        cx: &mut Context<Self>,
    ) {
        let start_width = self
            .image_widths
            .borrow()
            .get(&attr_target.start)
            .copied()
            .unwrap_or(320.0);
        self.image_drag = Some(ImageDrag {
            target,
            attr_target,
            start_x,
            start_width,
            width: start_width,
        });
        cx.notify();
    }

    /// Update the live width as the mouse moves during a resize drag.
    fn update_image_drag(&mut self, x: Pixels, cx: &mut Context<Self>) {
        if let Some(d) = self.image_drag.as_mut() {
            let delta = f32::from(x) - f32::from(d.start_x);
            d.width = (d.start_width + delta).clamp(40.0, 2000.0);
            cx.notify();
        }
    }

    /// Finish a resize drag: write `{width=N}` into the source and persist.
    fn finish_image_drag(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.image_drag.take() else {
            return;
        };
        let width = d.width.round() as i64;
        if let Some(editor) = self.editor_for(&d.target) {
            let value = editor.read(cx).value().to_string();
            let start = d.attr_target.start.min(value.len());
            let end = d.attr_target.end.min(value.len());
            let new = format!("{}{{width={width}}}{}", &value[..start], &value[end..]);
            editor.update(cx, |st, cx| {
                st.set_text(new.clone(), cx);
            });
            match &d.target {
                SlashTarget::Day(day) => self.save_journal(day, &new, cx),
                SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
            }
        }
        cx.notify();
    }

    /// The day/page editor that currently has focus, if any (for paste).
    fn focused_editor_target(&self) -> Option<SlashTarget> {
        if let Some(d) = self.editing_day.clone() {
            Some(SlashTarget::Day(d))
        } else if self.page_editing {
            match self.tabs.get(self.active).map(|t| t.kind.clone()) {
                Some(TabKind::Page(id)) => Some(SlashTarget::Page(id)),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Insert `![](rel)` into `target`'s source as its own block — at the caret
    /// when `at_cursor`, else appended — then persist.
    fn insert_image_markdown(
        &mut self,
        target: &SlashTarget,
        rel: &str,
        at_cursor: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.editor_for(target) else {
            return;
        };
        let value = editor.read(cx).value().to_string();
        let pos = if at_cursor {
            editor.read(cx).cursor().min(value.len())
        } else {
            value.len()
        };
        let (before, after) = value.split_at(pos);
        // Keep the image on its own line (a blank line before unless we're
        // already at a block boundary, and a newline after).
        let lead = if before.is_empty() || before.ends_with("\n\n") {
            ""
        } else if before.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let trail = if after.starts_with('\n') { "" } else { "\n" };
        let snippet = format!("{lead}![]({rel}){trail}");
        let caret = pos + snippet.len();
        let new = format!("{before}{snippet}{after}");
        editor.update(cx, |st, cx| {
            st.set_text(new.clone(), cx);
            st.set_cursor(caret, cx);
        });
        match target {
            SlashTarget::Day(d) => self.save_journal(d, &new, cx),
            SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
        }
        cx.notify();
    }

    /// `Cmd+V`: if the clipboard holds an image and a day/page editor is
    /// focused, save it and insert a reference. Otherwise propagate so
    /// gpui-component's normal text paste runs.
    fn on_paste_image(&mut self, _: &PasteImage, window: &mut Window, cx: &mut Context<Self>) {
        let clip_image = |cx: &mut Context<Self>| {
            cx.read_from_clipboard()?
                .entries()
                .iter()
                .find_map(|e| match e {
                    ClipboardEntry::Image(img) => {
                        Some((img.bytes().to_vec(), clipboard_ext(img.format())))
                    }
                    _ => None,
                })
        };
        // On a whiteboard, paste a clipboard image at the viewport center. (Copied
        // whiteboard *elements* are pasted in the crate's ⌘V handler via `on_paste`,
        // which only consumes the key when the clipboard actually holds elements —
        // otherwise it falls through here.)
        if let TabKind::Whiteboard(board_id) = self.tabs[self.active].kind {
            let Some((bytes, ext)) = clip_image(cx) else {
                cx.propagate();
                return;
            };
            match crate::images::import_bytes(&bytes, ext) {
                Ok(rel) => {
                    if let Some(view) = self.whiteboard_views.get(&board_id).cloned() {
                        let c = view.read(cx).viewport_center();
                        self.add_image_to_board(board_id, rel.into(), c[0], c[1], cx);
                    }
                }
                Err(e) => log::error!("save pasted image: {e}"),
            }
            return;
        }
        // Otherwise: a focused day/page editor inserts an inline markdown image.
        let Some(target) = self.focused_editor_target() else {
            cx.propagate();
            return;
        };
        let Some((bytes, ext)) = clip_image(cx) else {
            cx.propagate();
            return;
        };
        match crate::images::import_bytes(&bytes, ext) {
            Ok(rel) => self.insert_image_markdown(&target, &rel, true, window, cx),
            Err(e) => log::error!("save pasted image: {e}"),
        }
    }

    /// Import dropped files into `target` (appended as blocks): images render
    /// inline, PDFs are copied into the `pdf/` folder and become a viewer chip.
    /// Other file types are ignored.
    pub fn insert_dropped_files(
        &mut self,
        target: SlashTarget,
        paths: &[std::path::PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            let imported = if crate::images::is_supported(path) {
                crate::images::import_file(path)
            } else if crate::pdf::is_pdf(&path.to_string_lossy()) {
                crate::images::import_pdf(path)
            } else {
                continue;
            };
            match imported {
                Ok(rel) => self.insert_image_markdown(&target, &rel, false, window, cx),
                Err(e) => log::error!("import dropped file {}: {e}", path.display()),
            }
        }
    }

    /// Auto-pair brackets/quotes in the target editor. Compares the editor's
    /// text to its `prev` snapshot; if a single opener was just typed it inserts
    /// the matching closer (caret stays between), and if a closer was typed in
    /// front of its twin it steps over instead of duplicating. Returns whether
    /// it changed the text (the caller then skips its own save/refresh, since
    /// our edit re-enters the change handler). Always refreshes `prev`.
    fn maybe_autopair(
        &mut self,
        target: &SlashTarget,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editor) = self.editor_for(target) else {
            return false;
        };
        let value = editor.read(cx).value().to_string();
        // Only typed characters / single-char backspaces auto-pair. A programmatic
        // or multi-char edit (table row/column ops, paste, …) just refreshes the
        // baseline so the next keystroke compares correctly — without rewriting the
        // text or moving the caret.
        if !editor.read(cx).last_edit_was_keystroke() {
            self.set_autopair_prev(target, value);
            return false;
        }
        let cursor = editor.read(cx).cursor().min(value.len());
        let prev = self.autopair_prev(target);
        // Each arm yields the rewritten text and where the caret should land.
        let (new, caret) = match slash::autopair_action(&prev, &value, cursor) {
            Some(slash::AutoPair::Close(close)) => (
                format!("{}{close}{}", &value[..cursor], &value[cursor..]),
                cursor,
            ),
            Some(slash::AutoPair::TypeOver(skip)) => (
                format!("{}{}", &value[..cursor], &value[cursor + skip..]),
                cursor,
            ),
            Some(slash::AutoPair::Wrap { close, inner }) => {
                // `value` is already `…opener|suffix`; splice the selection back
                // in plus its closer, caret left just inside the closer.
                let caret = cursor + inner.len();
                (
                    format!("{}{inner}{close}{}", &value[..cursor], &value[cursor..]),
                    caret,
                )
            }
            None => match slash::autopair_backspace(&prev, &value, cursor) {
                Some(skip) => (
                    format!("{}{}", &value[..cursor], &value[cursor + skip..]),
                    cursor,
                ),
                None => {
                    self.set_autopair_prev(target, value);
                    return false;
                }
            },
        };
        editor.update(cx, |st, cx| {
            st.set_text(new.clone(), cx);
            st.set_cursor(caret, cx);
        });
        self.set_autopair_prev(target, new.clone());
        match target {
            SlashTarget::Day(d) => self.save_journal(d, &new, cx),
            SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
        }
        cx.notify();
        true
    }

    fn autopair_prev(&self, target: &SlashTarget) -> String {
        match target {
            SlashTarget::Day(d) => self
                .day_editors
                .get(d)
                .map(|de| de.prev.clone())
                .unwrap_or_default(),
            SlashTarget::Page(_) => self
                .page_editor
                .as_ref()
                .map(|pe| pe.prev.clone())
                .unwrap_or_default(),
        }
    }

    fn set_autopair_prev(&mut self, target: &SlashTarget, value: String) {
        match target {
            SlashTarget::Day(d) => {
                if let Some(de) = self.day_editors.get_mut(d) {
                    de.prev = value;
                }
            }
            SlashTarget::Page(_) => {
                if let Some(pe) = self.page_editor.as_mut() {
                    pe.prev = value;
                }
            }
        }
    }

    /// Enter edit mode for a feed day: flip it to the raw editor *now*
    /// (so the `Input` mounts this frame), then focus it. Setting the
    /// state explicitly — rather than waiting on the editor's Focus event
    /// — is required because focusing a not-yet-rendered editor doesn't
    /// reliably emit Focus.
    pub fn edit_day(&mut self, date: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_day = Some(date.to_string());
        if let Some(de) = self.day_editors.get(date) {
            de.state.clone().update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    /// Enter edit mode for a reader-view `target` (a day or a page). Used when a rendered
    /// formula is clicked: the formula `occlude`s the reader view's own click-to-edit, so it
    /// re-dispatches here.
    pub fn edit_from_reader(
        &mut self,
        target: &SlashTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            SlashTarget::Day(date) => self.edit_day(date, window, cx),
            SlashTarget::Page(_) => self.edit_page(window, cx),
        }
    }

    /// [`Self::edit_day`] variant for clicking a day's rendered text: enter edit mode
    /// with the caret at source byte `offset` and keep the clicked line under the cursor
    /// (gpui-markdown maps the click to a source offset and reports the click's `click_y`).
    pub fn edit_day_at_offset(
        &mut self,
        date: &str,
        offset: usize,
        click_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_day = Some(date.to_string());
        let Some(de) = self.day_editors.get(date) else {
            cx.notify();
            return;
        };
        let editor = de.state.clone();
        let source = editor.read(cx).value().to_string();
        let off = clamp_to_boundary(&source, offset);
        editor.update(cx, |s, cx| {
            s.set_cursor(off, cx);
            s.focus(window, cx);
        });
        // Same predict-then-jump as `edit_page_at_offset`, anchored on this day's
        // markdown root (the editor takes over its slot in the day section).
        let slot = de.md_scroll.bounds();
        if slot.size.width > px(0.0) {
            let (rows, line_height) = predict_caret_row(&source, off, slot.size.width, window, cx);
            let caret_y = slot.origin.y + INPUT_PY + line_height * rows as f32;
            let new_y = (self.feed_scroll.offset().y + (click_y - caret_y)).min(px(0.0));
            self.feed_scroll.set_offset(gpui::point(px(0.0), new_y));
        }
        align_caret_to_click(
            CaretAlign::new(editor, self.feed_scroll.clone(), cx.entity(), off, click_y),
            window,
        );
        cx.notify();
    }

    /// Entering edit mode focuses the full-height editor, which makes gpui autoscroll
    /// the page to the editor's top. Capture the current scroll offset and restore it
    /// after the next frame (once that autoscroll has run), so the view stays where it
    /// was instead of jumping to the top.
    fn keep_page_scroll(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.page_scroll.clone();
        let saved = handle.offset();
        let entity = cx.entity();
        window.on_next_frame(move |_window, cx| {
            handle.set_offset(saved);
            entity.update(cx, |_, cx| cx.notify());
        });
    }

    /// Enter edit mode for the open page (same not-yet-rendered caveat).
    pub fn edit_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep_page_scroll(window, cx);
        self.page_editing = true;
        if let Some(pe) = self.page_editor.as_ref() {
            pe.state.clone().update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    /// Enter edit mode with the caret at source byte `offset` — used when clicking
    /// the rendered page (gpui-markdown maps the click to a source offset and reports
    /// the click's window `click_y`), so the cursor lands where you clicked.
    /// `set_cursor_position` also focuses the editor.
    pub fn edit_page_at_offset(
        &mut self,
        offset: usize,
        click_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page_editing = true;
        let Some(pe) = self.page_editor.as_ref() else {
            cx.notify();
            return;
        };
        let editor = pe.state.clone();
        let source = editor.read(cx).value().to_string();
        let off = clamp_to_boundary(&source, offset);
        editor.update(cx, |s, cx| {
            s.set_cursor(off, cx);
            s.focus(window, cx);
        });
        // The source layout is more compact than the rendered one, so keeping the
        // scroll offset would let the clicked line slide away from the cursor.
        // Predict the caret's position from the still-painted rendered frame (the
        // editor takes over the markdown root's slot) and jump the scroll *now*,
        // so the editor's first paint already has the line under the cursor.
        // Clamping at the top keeps near-top lines put instead of force-centering.
        let slot = self.md_block_scroll.bounds();
        if slot.size.width > px(0.0) {
            let (rows, line_height) = predict_caret_row(&source, off, slot.size.width, window, cx);
            let caret_y = slot.origin.y + INPUT_PY + line_height * rows as f32;
            let new_y = (self.page_scroll.offset().y + (click_y - caret_y)).min(px(0.0));
            self.page_scroll.set_offset(gpui::point(px(0.0), new_y));
        }
        // Mop up any prediction drift once the editor reports real caret bounds.
        align_caret_to_click(
            CaretAlign::new(editor, self.page_scroll.clone(), cx.entity(), off, click_y),
            window,
        );
        cx.notify();
    }

    /// Like [`Self::edit_day`], but for clicking the empty area below a day:
    /// drop the caret on a trailing blank line so you can start writing at the
    /// bottom right away.
    pub fn edit_day_at_end(&mut self, date: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_day = Some(date.to_string());
        if let Some(de) = self.day_editors.get(date) {
            let editor = de.state.clone();
            Self::focus_editor_at_end(&editor, window, cx);
        }
        cx.notify();
    }

    /// [`Self::edit_page`] variant for clicking the page's open area.
    pub fn edit_page_at_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page_editing = true;
        if let Some(pe) = self.page_editor.as_ref() {
            let editor = pe.state.clone();
            Self::focus_editor_at_end(&editor, window, cx);
        }
        cx.notify();
    }

    /// Focus `editor` with the caret on a trailing blank line, appending a
    /// newline first when the content doesn't already end with one. The appended
    /// newline persists on the next edit or on blur.
    fn focus_editor_at_end(
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        editor.update(cx, |st, cx| {
            let value = st.value().to_string();
            if !value.is_empty() && !value.ends_with('\n') {
                st.set_text(format!("{value}\n"), cx);
            }
            let end = st.text().len();
            st.set_cursor(end, cx);
            st.focus(window, cx);
        });
    }

    // --- Read accessors for the UI ---

    pub fn is_journal_view(&self) -> bool {
        !self.searching && matches!(self.tabs[self.active].kind, TabKind::Journal)
    }

    pub fn is_page_active(&self, id: i64) -> bool {
        !self.searching && matches!(self.tabs[self.active].kind, TabKind::Page(pid) if pid == id)
    }

    pub fn is_editing_day(&self, date: &str) -> bool {
        self.editing_day.as_deref() == Some(date)
    }

    pub fn is_page_editing(&self) -> bool {
        self.page_editing
    }

    pub fn theme_mode(&self) -> theme::Mode {
        self.mode
    }

    /// The available themes (for the Settings picker).
    pub fn skins(&self) -> &[Skin] {
        &self.skins
    }

    /// The active theme's id.
    pub fn active_skin_id(&self) -> &str {
        &self.skin_id
    }

    // --- Theme / appearance ---

    fn current_skin(&self) -> &Skin {
        self.skins
            .iter()
            .find(|s| s.id == self.skin_id)
            .unwrap_or(&self.skins[0])
    }

    /// Resolve the active skin + mode (+ OS appearance for Auto) to a
    /// palette and push it live to every window.
    fn apply_theme(&self, window: &mut Window, cx: &mut Context<Self>) {
        let skin = self.current_skin();
        // A dark-only theme ignores the Light/Dark/Auto setting and forces dark,
        // so the window chrome / titlebar matches its always-dark content.
        let is_dark = skin.dark_only
            || match self.mode {
                theme::Mode::Light => false,
                theme::Mode::Dark => true,
                theme::Mode::Auto => self.system_dark,
            };
        let palette = if is_dark { skin.dark } else { skin.light };
        theme::apply(palette, is_dark, window, cx);
        // Diagrams are themed at render time — drop the cache so they re-render
        // with the new palette (Rc<RefCell>, so this is fine from `&self`).
        self.mermaid_store.borrow_mut().clear();
    }

    /// Switch to theme `id`, apply it live, and persist.
    pub fn set_skin(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.skin_id = id;
        self.apply_theme(window, cx);
        let _ = self.db.set_setting("theme_skin", &self.skin_id);
    }

    /// Re-scan the themes folder (built-ins + user) and re-apply, so edits
    /// to a JSON theme appear without a restart.
    pub fn reload_skins(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.skins = skins::builtin_skins();
        self.skins.extend(skins::load_user_skins());
        self.apply_theme(window, cx);
        cx.notify();
    }

    /// Open the user themes folder in the OS file manager.
    pub fn reveal_themes_folder(&self) {
        let dir = crate::paths::themes_dir();
        let _ = std::fs::create_dir_all(&dir);
        #[cfg(target_os = "macos")]
        let cmd = "open";
        #[cfg(target_os = "windows")]
        let cmd = "explorer";
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        let cmd = "xdg-open";
        let _ = std::process::Command::new(cmd).arg(&dir).spawn();
    }

    /// Open a folder in the OS file manager (Finder / Explorer / file manager).
    pub fn reveal_folder(folder: &Path) {
        #[cfg(target_os = "macos")]
        let cmd = "open";
        #[cfg(target_os = "windows")]
        let cmd = "explorer";
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        let cmd = "xdg-open";
        let _ = std::process::Command::new(cmd).arg(folder).spawn();
    }

    /// Surface a failed database open as a one-time modal, so the user learns why
    /// their notes look empty and where the pre-migration backup is — rather than
    /// silently landing in a blank workspace. Changes made here aren't persisted.
    fn show_db_error_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(err) = self.db_error.as_ref() else {
            return;
        };
        let folder = err.folder.clone();
        // SQLite reports batch failures as "<reason> in <whole SQL> at offset N";
        // keep just the reason (the full text is in the log) so the dialog stays
        // readable, and cap length defensively.
        let detail: String = err
            .message
            .split(" in ")
            .next()
            .unwrap_or(&err.message)
            .chars()
            .take(200)
            .collect();
        let recovery = match &err.backup {
            Some(b) => format!(
                "Your notes were backed up before the update and are safe — restore them from {}",
                b.display()
            ),
            None => format!("Your notes on disk are unchanged, in {}", folder.display()),
        };
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let folder = folder.clone();
            dialog
                .title("Couldn't open your notes database")
                .w(px(480.0))
                // Enter triggers the primary action (Quit); the temporary
                // workspace isn't saved, so there's nothing to lose.
                .on_ok(|_, _window, cx| {
                    cx.quit();
                    true
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(div().text_color(theme::text_secondary()).child(
                            "Zorite opened a temporary, empty workspace because the database \
                                 couldn't be opened or upgraded. Changes here won't be saved.",
                        ))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::text_secondary())
                                .child(detail.clone()),
                        )
                        .child(div().child(recovery.clone())),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("db-error-reveal")
                                .label("Reveal Backup")
                                .on_click(move |_, _window, _cx| AppView::reveal_folder(&folder)),
                        )
                        .child(
                            Button::new("db-error-quit")
                                .primary()
                                .label("Quit")
                                .on_click(|_, _window, cx| cx.quit()),
                        ),
                )
        });
    }

    /// Watch OS appearance so `Auto` mode tracks light/dark. Called once
    /// after the view entity exists (from `main`).
    pub fn attach_appearance_observer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let sub = window.observe_window_appearance(move |window, cx| {
            let dark = matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            );
            if let Some(view) = weak.upgrade() {
                view.update(cx, |this, cx| this.on_system_appearance(dark, window, cx));
            }
        });
        self._subs.push(sub);
    }

    fn on_system_appearance(&mut self, dark: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.system_dark = dark;
        if self.mode == theme::Mode::Auto {
            self.apply_theme(window, cx);
        }
    }

    /// Set the theme mode, apply it live, and persist the choice.
    pub fn set_theme_mode(
        &mut self,
        mode: theme::Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = mode;
        self.apply_theme(window, cx);
        let _ = self.db.set_setting("theme_mode", mode.as_str());
    }

    /// The current PDF render-quality multiplier (1.0 = native DPI).
    pub fn pdf_quality(&self) -> f32 {
        self.pdf_quality
    }

    /// Set the PDF render-quality multiplier, persist it, and re-render open PDFs so
    /// they pick up the new scale. Each viewer keeps its current bitmap on screen
    /// (rescaled) until the crisp re-render lands, so nothing blanks.
    pub fn set_pdf_quality(&mut self, quality: f32, cx: &mut Context<Self>) {
        let q = quality.clamp(0.25, 3.0);
        if (q - self.pdf_quality).abs() < 0.001 {
            return;
        }
        self.pdf_quality = q;
        crate::pdf::set_quality(q);
        let _ = self.db.set_setting("pdf_quality", &q.to_string());
        for view in self.pdf_views.values() {
            view.update(cx, |_view, cx| cx.notify());
        }
    }

    /// The list-indent width in spaces (the Tab / nesting unit).
    pub fn list_indent(&self) -> usize {
        self.list_indent
    }

    /// The list-indent as a run of spaces, for inserting in the editor.
    pub fn list_indent_str(&self) -> String {
        " ".repeat(self.list_indent)
    }

    /// Set the list-indent width (2 / 4 / 8 spaces). Persists and re-renders this
    /// window so the editor's Tab unit and the render indent stay in step. No-op if
    /// unchanged or invalid.
    pub fn set_list_indent(&mut self, spaces: usize, cx: &mut Context<Self>) {
        if !matches!(spaces, 2 | 4 | 8) || spaces == self.list_indent {
            return;
        }
        let old = self.list_indent;
        self.list_indent = spaces;
        let _ = self.db.set_setting("list_indent", &spaces.to_string());
        // For every open editor: update its Tab unit, then re-indent its existing
        // list items from the old width to the new one and persist, so the change
        // re-flows live. Collect (target, state) first to not borrow `self` across
        // the updates.
        let mut targets: Vec<(SlashTarget, Entity<EditorState>)> = self
            .day_editors
            .iter()
            .map(|(d, de)| (SlashTarget::Day(d.clone()), de.state.clone()))
            .collect();
        if let Some(pe) = self.page_editor.as_ref() {
            targets.push((SlashTarget::Page(pe.id), pe.state.clone()));
        }
        for (target, state) in targets {
            state.update(cx, |editor, _| editor.set_tab_indent(spaces));
            let content = state.read(cx).value().to_string();
            if let Some(new) = gpui_markdown::reindent(&content, old, spaces) {
                state.update(cx, |editor, cx| editor.set_text(new.clone(), cx));
                match &target {
                    SlashTarget::Day(d) => self.save_journal(d, &new, cx),
                    SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
                }
            }
        }
        cx.notify();
    }

    /// Whether startup checks GitHub Releases for a newer version.
    pub fn check_updates(&self) -> bool {
        self.check_updates
    }

    /// Whether the update check considers pre-releases (betas).
    pub fn include_prerelease(&self) -> bool {
        self.include_prerelease
    }

    /// Enable / disable the startup update check; persists.
    pub fn set_check_updates(&mut self, on: bool) {
        self.check_updates = on;
        let _ = self
            .db
            .set_setting("check_updates", if on { "1" } else { "0" });
    }

    /// Whether WYSIWYG live-preview editing is on (default). Off = "editor mode":
    /// raw markdown while editing, rendered page on Esc.
    pub fn wysiwyg(&self) -> bool {
        self.wysiwyg
    }

    /// Toggle WYSIWYG live-preview editing; persists, then re-applies to every
    /// open editor so the change takes effect without reopening notes.
    pub fn set_wysiwyg(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.wysiwyg == on {
            return;
        }
        self.wysiwyg = on;
        let _ = self.db.set_setting("wysiwyg", if on { "1" } else { "0" });
        // Set or clear live-preview styling on each open editor. Collect the
        // handles first so we don't hold a borrow of `self` across `update`.
        let states: Vec<Entity<EditorState>> = self
            .day_editors
            .values()
            .map(|de| de.state.clone())
            .chain(self.page_editor.as_ref().map(|pe| pe.state.clone()))
            .collect();
        for state in states {
            state.update(cx, |editor, cx| {
                if on {
                    editor.set_markdown_style(theme::editor_syntax_style(), cx);
                } else {
                    editor.clear_markdown_style(cx);
                }
            });
        }
        cx.notify();
    }

    /// Include / exclude pre-releases in the update check; persists, then re-runs
    /// the check so the indicator reflects the new preference right away.
    pub fn set_include_prerelease(&mut self, on: bool, cx: &mut Context<Self>) {
        self.include_prerelease = on;
        let _ = self
            .db
            .set_setting("include_prerelease", if on { "1" } else { "0" });
        crate::updater::spawn_check(on, cx);
    }

    /// Re-run the update check now (Settings → Updates → "Check now").
    pub fn check_for_updates_now(&self, cx: &mut Context<Self>) {
        crate::updater::spawn_check(self.include_prerelease, cx);
    }

    /// Set the date format used by `/date` and `{{date}}` (a [`crate::dates`] id):
    /// applies it to the shared thread-local and persists. Only affects future
    /// insertions (existing content + journal headers are untouched), so there's
    /// nothing to re-render. No-op for an unknown id.
    pub fn set_date_format(&mut self, id: &str) {
        if !crate::dates::DATE_FORMATS.contains(&id) {
            return;
        }
        crate::dates::set_date_format(id);
        let _ = self.db.set_setting("date_format", id);
    }

    /// Set the time format used by `/time` and `{{time}}` (a [`crate::dates`] id).
    pub fn set_time_format(&mut self, id: &str) {
        if !crate::dates::TIME_FORMATS.contains(&id) {
            return;
        }
        crate::dates::set_time_format(id);
        let _ = self.db.set_setting("time_format", id);
    }

    /// Quick cycle for the title-bar toggle: Light → Dark → Auto → Light.
    fn cycle_theme_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = match self.mode {
            theme::Mode::Light => theme::Mode::Dark,
            theme::Mode::Dark => theme::Mode::Auto,
            theme::Mode::Auto => theme::Mode::Light,
        };
        self.set_theme_mode(next, window, cx);
    }

    /// Open the Settings window, or focus it if already open. An associated
    /// function (not `&mut self`) run at the App level: `open_window`
    /// renders `SettingsView` synchronously, and `SettingsView` *reads*
    /// `AppView`, so `AppView` must NOT be mid-update while we open. Call
    /// this from a deferred closure (e.g. the gear's click handler).
    pub fn open_settings(view: Entity<AppView>, cx: &mut App) {
        // Focus an existing settings window instead of duplicating it.
        let existing = view.read(cx).settings_window;
        if let Some(handle) = existing
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let app = view.downgrade();
        let bounds = Bounds::centered(None, size(px(720.0), px(560.0)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Settings · Zorite".into()),
                    ..TitleBar::title_bar_options()
                }),
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            move |window, cx| {
                window.set_client_inset(px(10.0));
                let v = cx.new(|cx| SettingsView::new(app.clone(), window, cx));
                cx.new(|cx| gpui_component::Root::new(v, window, cx))
            },
        );
        if let Ok(handle) = opened {
            view.update(cx, |this, _| this.settings_window = Some(handle));
        }
    }

    /// Open `target` in a new top-level window — a full, independent `AppView`
    /// (its own SQLite connection to the same file) focused on the given page /
    /// PDF / journal, like a new browser window. Run at the App level from a
    /// deferred closure (`open_window` must not run mid-`AppView` update). Each
    /// window is independent; they share the database file, so edits are visible
    /// across windows on the next read (same-page concurrent edits = last write
    /// wins — there's no live in-memory sync yet).
    pub fn open_in_new_window(target: TabKind, cx: &mut App) {
        Self::open_in_new_window_at(target, None, TabSeed::default(), cx);
    }

    /// Open a window showing `target`. With `at` set (a tear-off drop point in
    /// global coords), the window opens under the cursor; otherwise it's centered.
    /// `seed` carries the source window's hand-off (decoded image bitmaps / the
    /// live PDF viewer), so the moved content appears immediately.
    pub fn open_in_new_window_at(
        target: TabKind,
        at: Option<Point<Pixels>>,
        mut seed: TabSeed,
        cx: &mut App,
    ) {
        let win_size = size(px(1100.0), px(800.0));
        let bounds = match at {
            // Drop the window so the cursor lands near where the tab strip will be
            // (roughly under the grabbed tab), clamped onto the visible area.
            Some(p) => Bounds {
                origin: point(
                    (p.x - px(160.0)).max(px(0.0)),
                    (p.y - px(12.0)).max(px(0.0)),
                ),
                size: win_size,
            },
            None => Bounds::centered(None, win_size, cx),
        };
        let opened = cx.open_window(
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
            move |window, cx| {
                window.set_client_inset(px(10.0));
                let view = cx.new(|cx| AppView::new(window, cx));
                view.update(cx, |this, cx| this.attach_appearance_observer(window, cx));
                AppView::register_window(&view, window, cx);
                // A moved PDF viewer must be in place before `open_pdf` looks
                // for one; images are adopted after the open wipes the store.
                view.update(cx, |this, _| this.adopt_pdf_seed(&mut seed));
                match target {
                    TabKind::Page(id) => {
                        view.update(cx, |this, cx| this.open_page_id(id, window, cx));
                    }
                    TabKind::Pdf(path) => {
                        view.update(cx, |this, cx| this.open_pdf(path, window, cx));
                    }
                    TabKind::Whiteboard(id) => {
                        view.update(cx, |this, cx| this.open_whiteboard(id, window, cx));
                    }
                    TabKind::Journal => {}
                }
                view.update(cx, |this, _| {
                    this.image_store.borrow_mut().adopt(seed.images)
                });
                cx.new(|cx| gpui_component::Root::new(view, window, cx))
            },
        );
        if let Err(err) = opened {
            log::error!("open new window: {err}");
        }
    }

    /// Record a freshly-created main window in [`GlobalAppWindows`] so dragged
    /// tabs can find it, pruning any windows that have since closed.
    pub fn register_window(view: &Entity<AppView>, window: &Window, cx: &mut App) {
        let entry = (window.window_handle(), view.downgrade());
        let reg = &mut cx.global_mut::<GlobalAppWindows>().0;
        reg.retain(|(_, w)| w.upgrade().is_some());
        reg.push(entry);
    }

    /// Each move of a tab strip drag (fired on the source window, which keeps mouse
    /// capture even off-window): light up whichever *other* window sits under the
    /// cursor so it can show a ghost tab, repainting the windows whose hover state
    /// just changed.
    fn on_tab_drag_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pos = window.bounds().origin + window.mouse_position();
        let new = Self::window_under(pos, self.window_handle, cx).map(|(h, _)| h);
        let cur = cx.global::<GlobalDropTarget>().0;
        if new != cur {
            cx.global_mut::<GlobalDropTarget>().0 = new;
            if let Some(old) = cur {
                Self::notify_window(old, cx);
            }
            if let Some(h) = new {
                Self::notify_window(h, cx);
            }
        }
    }

    /// Terminating mouse-up of a tab strip drag (released off the strip, anywhere
    /// on screen). Only the source window runs this. Removes the tab here, then —
    /// after the event settles — hands it to the window under the cursor, or opens
    /// a new one if there's none. Reorders within the strip are handled separately
    /// by the tab's own `on_drop`, which consumes the drag before this fires.
    fn on_tab_drag_release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.has_active_drag() {
            return;
        }
        let Some(drag) = cx.global::<GlobalDraggingTab>().0.clone() else {
            return;
        };
        if drag.source != window.window_handle() {
            return;
        }
        cx.global_mut::<GlobalDraggingTab>().0 = None;
        // Drop the hover ghost and repaint that window.
        if let Some(prev) = cx.global_mut::<GlobalDropTarget>().0.take() {
            Self::notify_window(prev, cx);
        }
        // The release point in global (screen) coordinates: the window's on-screen
        // origin plus the window-relative cursor.
        let origin = window.bounds().origin;
        let pos = origin + window.mouse_position();
        // Released over our *own* tab strip → keep the tab here. A drop onto a tab
        // already reordered (its `on_drop` consumed the drag before this ran); a
        // drop on the empty strip is just a no-op. Either way, never tear off — so
        // you can always drag back to the strip to cancel.
        let own_strip = self.tab_strip_bounds.get();
        let own_strip = Bounds {
            origin: origin + own_strip.origin,
            size: own_strip.size,
        };
        if own_strip.contains(&pos) {
            return;
        }
        // Drop our copy of the tab (found by content, not the stale drag index).
        let Some(ix) = self.tabs.iter().position(|t| t.kind == drag.kind) else {
            return;
        };
        if ix == 0 {
            return;
        }
        // Hand over the page's already-decoded bitmaps — snapshot before
        // `close_tab`, whose tab switch frees this window's image store.
        let seed = self.take_tab_seed(&drag.kind, window, cx);
        self.close_tab(ix, window, cx);
        let source = window.window_handle();
        // Defer so cross-window updates don't re-enter mid-event.
        window.defer(cx, move |_, cx| {
            AppView::resolve_tab_drop(drag.kind, pos, source, seed, cx);
        });
    }

    /// Everything a moving tab hands its destination window so the content
    /// appears there immediately. Destructive for a PDF tab — the live viewer
    /// entity is *taken* from this window.
    fn take_tab_seed(
        &mut self,
        kind: &TabKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TabSeed {
        match kind {
            // A page carries the already-decoded bitmaps for its images. The
            // store holds the *active* view's images, so filtering by the moved
            // page's content keeps a background-tab drag from carrying
            // unrelated bitmaps.
            TabKind::Page(id) => {
                let Ok(Some(page)) = self.db.get_page(*id) else {
                    return TabSeed::default();
                };
                let mut images = self.image_store.borrow().snapshot();
                images.retain(|(src, _)| page.content.contains(src.as_ref()));
                TabSeed { images, pdf: None }
            }
            // A PDF carries its whole viewer — scroll, zoom, parsed document,
            // unlocked state, rendered pages. Taken out of `pdf_views` here so
            // the upcoming `close_tab` won't release it; this window's GPU
            // textures are dropped now, and the kept bitmaps re-upload where
            // the viewer next paints.
            TabKind::Pdf(path) => {
                let pdf = self.pdf_views.remove(path);
                if let Some(view) = &pdf {
                    view.update(cx, |v, cx| v.detach_textures(window, cx));
                }
                TabSeed {
                    images: Vec::new(),
                    pdf: pdf.map(|v| (path.clone(), v)),
                }
            }
            // A board reloads cheaply from the DB on the destination window, so
            // it carries no live entity (no unsaved in-memory edits in Phase 0).
            TabKind::Whiteboard(_) => TabSeed::default(),
            TabKind::Journal => TabSeed::default(),
        }
    }

    /// Hand a seed's PDF viewer to this window — the receiving half of
    /// [`Self::take_tab_seed`]. Must run *before* `open_pdf` so its
    /// "viewer already open" check adopts the moved entity instead of building
    /// a fresh one (which would lose scroll/zoom/unlock and re-parse the file).
    fn adopt_pdf_seed(&mut self, seed: &mut TabSeed) {
        if let Some((path, view)) = seed.pdf.take()
            // A viewer this window already has wins — don't clobber a live one.
            && !self.pdf_views.contains_key(&path)
        {
            self.pdf_views.insert(path, view);
        }
    }

    /// The registered window other than `source` whose **tab strip** is under
    /// `pos` (a global point). Hit-testing the strip — not the whole window —
    /// means a window hidden behind the source is never picked: to move a tab you
    /// drop it on a visible tab bar, leaving the rest of a window free for "drag
    /// back to cancel". Used to route a release and to drive the hover ghost.
    fn window_under(
        pos: Point<Pixels>,
        source: AnyWindowHandle,
        cx: &mut App,
    ) -> Option<(AnyWindowHandle, WeakEntity<AppView>)> {
        cx.global::<GlobalAppWindows>()
            .0
            .clone()
            .into_iter()
            .find(|(handle, weak)| {
                if *handle == source {
                    return false;
                }
                let Some(view) = weak.upgrade() else {
                    return false;
                };
                // The strip rect is window-relative; offset by the window's
                // on-screen origin to compare against the global cursor.
                let strip = view.read(cx).tab_strip_bounds.get();
                handle
                    .update(cx, |_, w, _| {
                        Bounds {
                            origin: w.bounds().origin + strip.origin,
                            size: strip.size,
                        }
                        .contains(&pos)
                    })
                    .unwrap_or(false)
            })
    }

    /// Repaint the `AppView` in `handle`'s window — e.g. to add or drop its ghost tab.
    fn notify_window(handle: AnyWindowHandle, cx: &mut App) {
        let weak = cx
            .global::<GlobalAppWindows>()
            .0
            .iter()
            .find(|(h, _)| *h == handle)
            .map(|(_, w)| w.clone());
        if let Some(weak) = weak {
            let _ = weak.update(cx, |_, cx| cx.notify());
        }
    }

    /// The label to show as a ghost tab in this window's strip — `Some` only while
    /// a tab dragged from another window is hovering over this one.
    pub fn drop_ghost_title(&self, cx: &App) -> Option<SharedString> {
        if cx.global::<GlobalDropTarget>().0 != Some(self.window_handle) {
            return None;
        }
        cx.global::<GlobalDraggingTab>()
            .0
            .as_ref()
            .map(|d| d.title.clone())
    }

    /// Hand a torn-off tab to the window under `pos`, or open a fresh window when
    /// the cursor is over none. Runs at the App level (deferred), where re-entering
    /// other windows is safe. `seed`: the source window's hand-off for the moved
    /// tab (see [`Self::take_tab_seed`]).
    fn resolve_tab_drop(
        kind: TabKind,
        pos: Point<Pixels>,
        source: AnyWindowHandle,
        seed: TabSeed,
        cx: &mut App,
    ) {
        match Self::window_under(pos, source, cx) {
            Some((handle, weak)) => {
                let _ = handle.update(cx, |_, w, cx| {
                    if let Some(view) = weak.upgrade() {
                        view.update(cx, |this, cx| this.receive_tab(kind, seed, w, cx));
                    }
                    w.activate_window();
                });
            }
            None => AppView::open_in_new_window_at(kind, Some(pos), seed, cx),
        }
    }

    /// Open (and focus) a tab for `kind` in this window — the receiving end of a
    /// cross-window tab drag.
    fn receive_tab(
        &mut self,
        kind: TabKind,
        mut seed: TabSeed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A moved PDF viewer must be in place before `open_pdf` looks for one.
        self.adopt_pdf_seed(&mut seed);
        match kind {
            TabKind::Page(id) => self.open_page_id(id, window, cx),
            TabKind::Pdf(path) => self.open_pdf(path, window, cx),
            TabKind::Whiteboard(id) => self.open_whiteboard(id, window, cx),
            TabKind::Journal => {}
        }
        // After the open (whose tab switch wiped this window's store), adopt the
        // source window's bitmaps so the moved page paints without re-decoding.
        self.image_store.borrow_mut().adopt(seed.images);
    }

    /// Drag-reorder: move tab `from` to where tab `to` sits. `to == tabs.len()`
    /// appends to the very end (the drop zone past the last tab). The pinned
    /// Journal (index 0) never moves, and nothing moves before it.
    pub fn reorder_tab(
        &mut self,
        from: usize,
        to: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A reorder ends the drag inside the strip — clear the shared slot so a
        // later release elsewhere can't read a stale tab.
        cx.global_mut::<GlobalDraggingTab>().0 = None;
        let n = self.tabs.len();
        if from == 0 || to == 0 || from >= n || to > n || from == to {
            return;
        }
        // Track the active tab by identity so it stays selected after the move.
        let active_kind = self.tabs[self.active].kind.clone();
        let tab = self.tabs.remove(from);
        let dest = if from < to { to - 1 } else { to };
        self.tabs.insert(dest.clamp(1, self.tabs.len()), tab);
        self.active = self
            .tabs
            .iter()
            .position(|t| t.kind == active_kind)
            .unwrap_or(self.active.min(self.tabs.len() - 1));
        cx.notify();
    }

    /// Tear a tab off into its own new window (drag it off the strip into the
    /// content area). Removes it from this window and reopens its content in a
    /// fresh window. The pinned Journal isn't torn off.
    fn tear_off_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix == 0 || ix >= self.tabs.len() {
            return;
        }
        let target = self.tabs[ix].kind.clone();
        // Snapshot before `close_tab` — its tab switch frees this window's images.
        let seed = self.take_tab_seed(&target, window, cx);
        self.close_tab(ix, window, cx);
        window.defer(cx, move |_, cx| {
            AppView::open_in_new_window_at(target, None, seed, cx)
        });
    }

    // --- Delete page (sidebar right-click → confirm) ---

    /// Remember which page a right-click context menu targets, so the
    /// `DeletePage` action knows what to delete. Called from the sidebar.
    pub fn set_context_page(&mut self, id: i64, title: SharedString) {
        self.context_page = Some((id, title));
        self.context_target = Some(TabKind::Page(id));
    }

    /// Remember a tab's content as the "Open in new window" target (called from
    /// the tab strip's right-click, where there's no page id — e.g. a PDF tab).
    pub fn set_context_target(&mut self, target: TabKind) {
        self.context_target = Some(target);
    }

    /// `DeletePage` handler: confirm, then delete the remembered page.
    fn on_delete_page(&mut self, _: &DeletePage, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, title)) = self.context_page.take() else {
            return;
        };
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let weak = weak.clone();
            dialog
                .title("Delete page?")
                .description(SharedString::from(format!(
                    "“{title}” will be permanently deleted. This can't be undone."
                )))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("Cancel")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.delete_page(id, window, cx));
                    true
                })
        });
    }

    /// `OpenInNewTab` handler: open the right-clicked page in a background tab.
    fn on_open_in_new_tab(
        &mut self,
        _: &OpenInNewTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((id, _)) = self.context_page.take() {
            self.open_page_in_new_tab(id, cx);
        }
    }

    /// `ToggleFavorite` handler: pin / unpin the right-clicked page.
    fn on_toggle_favorite(
        &mut self,
        _: &ToggleFavorite,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((id, _)) = self.context_page.take() {
            self.toggle_favorite(id, cx);
        }
    }

    /// `OpenInNewWindow` handler (sidebar page or tab right-click): the remembered
    /// target *moves* to a fresh window rather than duplicating — if it's already open
    /// as a (non-Journal) tab here, tear it off (close here + open there); otherwise
    /// just open it there. Deferred to the App level because `open_window` must not run
    /// while this `AppView` is mid-update.
    fn on_open_in_new_window(
        &mut self,
        _: &OpenInNewWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.context_target.take() else {
            return;
        };
        let open_ix = self.tabs.iter().position(|t| t.kind == target);
        if let Some(ix) = open_ix
            && ix != 0
        {
            self.tear_off_tab(ix, window, cx);
            return;
        }
        let seed = self.take_tab_seed(&target, window, cx);
        window.defer(cx, move |_, cx| {
            AppView::open_in_new_window_at(target, None, seed, cx)
        });
    }

    /// Delete a named page and refresh the UI. Journals are never deleted
    /// (the DB guards this too). Any tabs showing the page are closed.
    fn delete_page(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.delete_page(id) {
            Ok(true) => {
                // Drop a deleted page from favorites so the dead id doesn't linger.
                if let Some(pos) = self.favorites.iter().position(|&x| x == id) {
                    self.favorites.remove(pos);
                    self.persist_favorites();
                }
                // Close any tabs showing the deleted page (journal at 0 is safe).
                let mut i = self.tabs.len();
                while i > 1 {
                    i -= 1;
                    if matches!(self.tabs[i].kind, TabKind::Page(pid) if pid == id) {
                        self.tabs.remove(i);
                        if self.active > i {
                            self.active -= 1;
                        } else if self.active == i {
                            self.active = self.active.min(self.tabs.len() - 1);
                        }
                    }
                }
                self.refresh_sidebar();
                self.signal_doc_changed(cx);
                self.activate_tab(self.active, window, cx);
            }
            Ok(false) => {}
            Err(e) => log::error!("delete page {id}: {e}"),
        }
    }

    /// `NewPage` handler: prompt for a title in a dialog, then create and
    /// open the page (dispatched from a pages-area right-click menu).
    fn on_new_page(&mut self, _: &NewPage, window: &mut Window, cx: &mut Context<Self>) {
        self.new_page_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let input = self.new_page_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_body = input.clone();
            let input_btn = input.clone();
            let input_key = input.clone();
            let weak_btn = weak.clone();
            let weak_key = weak.clone();
            dialog
                .title("New page")
                .w(px(420.0))
                .child(Input::new(&input_body))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("new-page-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("new-page-create")
                                .primary()
                                .label("Create")
                                .on_click(move |_, window, cx| {
                                    let title = input_btn.read(cx).value().trim().to_string();
                                    if !title.is_empty() {
                                        let _ = weak_btn.update(cx, |this, cx| {
                                            this.open_page_title(&title, window, cx)
                                        });
                                    }
                                    window.close_dialog(cx);
                                }),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let title = input_key.read(cx).value().trim().to_string();
                    if !title.is_empty() {
                        let _ = weak_key
                            .update(cx, |this, cx| this.open_page_title(&title, window, cx));
                    }
                    true
                })
                .on_cancel(|_, _window, _cx| true)
        });
        self.new_page_input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// `ImportLogseq` handler: pick a Logseq graph folder, then choose how
    /// the outline converts before importing.
    fn on_import_logseq(&mut self, _: &ImportLogseq, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(root) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.show_logseq_options(root, window, cx);
            });
        })
        .detach();
    }

    /// Ask how Logseq's all-bullets outline should convert, then run the import.
    fn show_logseq_options(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let (root_flat, root_list, root_ok) = (root.clone(), root.clone(), root.clone());
            let (weak_flat, weak_list, weak_ok) = (weak.clone(), weak.clone(), weak.clone());
            dialog
                .title("Import from Logseq")
                .w(px(500.0))
                // Enter runs the primary action (Flatten outline), like the button.
                .on_ok(move |_, window, cx| {
                    window.close_dialog(cx);
                    let root = root_ok.clone();
                    let _ = weak_ok.update(cx, |this, cx| {
                        this.run_logseq_import(root, true, window, cx)
                    });
                    false
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_color(theme::text_secondary())
                                .child(format!("Importing “{}”.", root.display())),
                        )
                        .child(div().text_color(theme::text_secondary()).child(
                            "Logseq makes every line a bullet. “Flatten outline” turns each \
                             top-level bullet into a paragraph or heading (nested bullets stay \
                             lists) so pages read like Zorite pages; “Keep bullets” preserves \
                             the outline exactly. Existing pages keep their content — imported \
                             text is appended below it.",
                        )),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("ls-import-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("ls-import-bullets")
                                .label("Keep bullets")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let root = root_list.clone();
                                    let _ = weak_list.update(cx, |this, cx| {
                                        this.run_logseq_import(root, false, window, cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new("ls-import-flatten")
                                .primary()
                                .label("Flatten outline")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let root = root_flat.clone();
                                    let _ = weak_flat.update(cx, |this, cx| {
                                        this.run_logseq_import(root, true, window, cx)
                                    });
                                }),
                        ),
                )
                .on_cancel(|_, _window, _cx| true)
        });
    }

    /// Import `root` on a background thread (its own DB connection — WAL keeps
    /// it concurrent with this one), then show the summary and refresh.
    fn run_logseq_import(
        &mut self,
        root: PathBuf,
        flatten: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.open_dialog(cx, |dialog, _window, _cx| {
            dialog
                .title("Importing from Logseq…")
                .w(px(400.0))
                .child(
                    div()
                        .text_color(theme::text_secondary())
                        .child("Copying notes and assets — this may take a minute."),
                )
                // A progress indicator has no confirm action — Enter shouldn't
                // dismiss it (Escape still cancels).
                .on_ok(|_, _window, _cx| false)
                .on_cancel(|_, _window, _cx| true)
        });
        let data_dir = crate::paths::data_dir();
        let task = cx.background_executor().spawn(async move {
            let db = Db::open().map_err(|e| format!("open database: {}", e.source))?;
            let opts = crate::import::logseq::Options { flatten };
            let bundle = crate::import::logseq::read_graph(&root, &opts)?;
            crate::import::write_bundle(&db, &data_dir, bundle, |_, _| {})
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                window.close_dialog(cx);
                this.refresh_sidebar();
                // Reload journal days / the open page from the DB everywhere.
                this.signal_doc_changed(cx);
                this.show_logseq_summary(result, window, cx);
            });
        })
        .detach();
    }

    /// Post-import summary (or failure) dialog.
    fn show_logseq_summary(
        &mut self,
        result: Result<crate::import::Summary, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        /// At most `n` names, with a `+ N more` tail when the list is long.
        fn sample(list: &[String], n: usize) -> String {
            let mut s = list.iter().take(n).cloned().collect::<Vec<_>>().join(", ");
            if list.len() > n {
                s.push_str(&format!(" — and {} more", list.len() - n));
            }
            s
        }
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let (title, lines) = match &result {
                Ok(s) => {
                    let mut lines = vec![format!(
                        "{} pages, {} journal days, {} PDF-highlight pages, \
                         {} whiteboards; {} assets copied; {} favorites.",
                        s.pages,
                        s.journals,
                        s.highlight_pages,
                        s.whiteboards,
                        s.assets_copied,
                        s.favorites
                    )];
                    if !s.appended.is_empty() {
                        lines.push(format!(
                            "Appended below existing content: {}.",
                            sample(&s.appended, 6)
                        ));
                    }
                    if !s.warnings.is_empty() {
                        lines.push(format!("Warnings: {}", sample(&s.warnings, 6)));
                    }
                    ("Logseq import complete", lines)
                }
                Err(e) => ("Logseq import failed", vec![e.clone()]),
            };
            dialog
                .title(title)
                .w(px(520.0))
                .child(
                    div().flex().flex_col().gap(px(8.0)).children(
                        lines
                            .into_iter()
                            .map(|l| div().text_color(theme::text_secondary()).child(l)),
                    ),
                )
                .footer(
                    DialogFooter::new().child(
                        Button::new("ls-import-done")
                            .primary()
                            .label("Done")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
                )
                .on_cancel(|_, _window, _cx| true)
        });
    }

    /// `FitImages` (`⌘⇧I`) handler: shrink *every* image in the active view that
    /// renders wider than ~half the content column down to that comfortable size,
    /// so over-wide images (dragged, pasted, or imported with no `{width}`) stop
    /// dominating the page. Works on a page or the whole journal feed; images
    /// already at or under the target are untouched, so it's a no-op the second
    /// time. No image to select first — one keystroke fits them all.
    fn on_fit_images(&mut self, _: &FitImages, window: &mut Window, cx: &mut Context<Self>) {
        // Collect (target, editor, max-width) up front so the write loop doesn't
        // borrow `self` while it also mutates it.
        let mut targets: Vec<(SlashTarget, Entity<EditorState>, i64)> = Vec::new();
        match self.tabs.get(self.active).map(|t| t.kind.clone()) {
            Some(TabKind::Page(id)) => {
                if let Some(pe) = self.page_editor.as_ref() {
                    let max_w = content_width(self.page_scroll.bounds());
                    targets.push((SlashTarget::Page(id), pe.state.clone(), max_w));
                }
            }
            Some(TabKind::Journal) => {
                let max_w = content_width(self.feed_scroll.bounds());
                for (date, de) in &self.day_editors {
                    targets.push((SlashTarget::Day(date.clone()), de.state.clone(), max_w));
                }
            }
            _ => {} // PDF tab: no markdown images
        }
        let mut fitted = false;
        for (slash_target, editor, max_w) in targets {
            // Skip a viewport that hasn't been measured yet (width ~0).
            if max_w < 80 {
                continue;
            }
            // Shrink anything rendered wider than ~half the column down to it.
            let comfortable = max_w / 2;
            let value = editor.read(cx).value().to_string();
            // Pair each image with its rendered width: an explicit `{width=N}`,
            // else the width measured during paint. Images never measured (e.g.
            // still off-screen and unpainted) are skipped — size unknown.
            let imgs: Vec<(Range<usize>, f32)> = {
                let widths = self.image_widths.borrow();
                gpui_markdown::images(&value)
                    .into_iter()
                    .filter_map(|img| {
                        let w = img
                            .width
                            .or_else(|| widths.get(&img.attr_target.start).copied())?;
                        Some((img.attr_target, w))
                    })
                    .collect()
            };
            let Some(new) = apply_fit(&value, &imgs, comfortable) else {
                continue;
            };
            editor.update(cx, |st, cx| st.set_text(new.clone(), cx));
            match &slash_target {
                SlashTarget::Day(d) => self.save_journal(d, &new, cx),
                SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
            }
            fitted = true;
        }
        if fitted {
            cx.notify();
            // Force a full redraw: `cx.notify()` alone can reuse cached child
            // elements, leaving the resized image painted at its old width.
            window.refresh();
        }
    }

    /// `RenamePage` handler: open a dialog with a text field, pre-filled
    /// with the current title, to rename the right-clicked page.
    fn on_rename_page(&mut self, _: &RenamePage, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, title)) = self.context_page.take() else {
            return;
        };
        self.rename_target = Some(id);
        self.rename_input
            .update(cx, |s, cx| s.set_value(title.to_string(), window, cx));

        // `AlertDialog` is title/description-only; a text field needs the
        // generic `Dialog` (it impls `ParentElement`, so the Input goes in as
        // a child) with a footer we build ourselves. Enter/Escape are wired
        // via on_ok/on_cancel.
        let input = self.rename_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_body = input.clone();
            let input_btn = input.clone();
            let input_key = input.clone();
            let weak_btn = weak.clone();
            let weak_key = weak.clone();
            dialog
                .title("Rename page")
                .w(px(420.0))
                .child(Input::new(&input_body))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("rename-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("rename-ok").primary().label("Rename").on_click(
                            move |_, window, cx| {
                                let title = input_btn.read(cx).value().to_string();
                                let _ = weak_btn
                                    .update(cx, |this, cx| this.commit_rename(title, window, cx));
                                window.close_dialog(cx);
                            },
                        )),
                )
                .on_ok(move |_, window, cx| {
                    let title = input_key.read(cx).value().to_string();
                    let _ = weak_key.update(cx, |this, cx| this.commit_rename(title, window, cx));
                    true
                })
                .on_cancel(|_, _window, _cx| true)
        });
        self.rename_input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Apply a confirmed rename: rewrite `[[links]]`, refresh the sidebar,
    /// and update any open tab titles for the page.
    fn commit_rename(&mut self, new_title: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.rename_target.take() else {
            return;
        };
        match self.db.rename_page(id, &new_title) {
            Ok(true) => {
                let title: SharedString = new_title.trim().to_string().into();
                for tab in &mut self.tabs {
                    if matches!(tab.kind, TabKind::Page(pid) if pid == id) {
                        tab.title = title.clone();
                    }
                }
                self.refresh_sidebar();
                self.reload_day_editors(window, cx);
                self.signal_doc_changed(cx);
                self.activate_tab(self.active, window, cx);
            }
            Ok(false) => {}
            Err(e) => log::error!("rename page {id}: {e}"),
        }
    }

    /// Rename the open page from its inline title field. Updates state in
    /// place (no tab reload) so the title field keeps focus; reverts the
    /// field if the new name is empty, a duplicate, or a journal.
    fn commit_title_rename(
        &mut self,
        id: i64,
        new_title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((current, title_state)) = self
            .page_editor
            .as_ref()
            .map(|pe| (pe.title.clone(), pe.title_state.clone()))
        else {
            return;
        };
        if new_title == current {
            return;
        }
        match self.db.rename_page(id, &new_title) {
            Ok(true) => {
                // Backlink snippets now show the rewritten `[[new]]` text.
                let backlinks = self.db.backlinks(id).unwrap_or_default();
                if let Some(pe) = self.page_editor.as_mut() {
                    pe.title = new_title.clone();
                    pe.backlinks = backlinks;
                }
                let title: SharedString = new_title.into();
                for tab in &mut self.tabs {
                    if matches!(tab.kind, TabKind::Page(pid) if pid == id) {
                        tab.title = title.clone();
                    }
                }
                self.refresh_sidebar();
                self.reload_day_editors(window, cx);
                self.signal_doc_changed(cx);
                cx.notify();
            }
            Ok(false) => {
                // Empty, duplicate, or journal — revert the field.
                title_state.update(cx, |s, cx| s.set_value(current, window, cx));
                cx.notify();
            }
            Err(e) => log::error!("rename page {id} (inline): {e}"),
        }
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Lazy journal feed: build it on the first frame that shows the Journal
        // tab (covers startup, ⌘N windows, and a later tab click — the editors
        // are created just above the feed render in this same pass).
        if self.loaded_days == 0 && matches!(self.tabs[self.active].kind, TabKind::Journal) {
            self.ensure_feed_loaded(window, cx);
        }
        // First paint after a failed DB open: surface it once (deferred so we
        // don't open a dialog mid-layout).
        if self.db_error.is_some() && !self.db_error_shown {
            self.db_error_shown = true;
            let this = cx.entity();
            window.defer(cx, move |window, cx| {
                this.update(cx, move |this, cx| this.show_db_error_dialog(window, cx));
            });
        }

        let slash_scroll = self.slash_scroll.clone();
        let overlay = self.slash.as_ref().map(|s| {
            gpui::deferred(
                gpui::anchored()
                    .position(s.caret.bottom_left())
                    .snap_to_window()
                    .child(ui::slash_menu::render(s, &slash_scroll, cx)),
            )
            .into_any_element()
        });

        // The `/table` size picker: a full-window backdrop (click outside to
        // cancel) with the hover-grid anchored at the caret.
        let table_picker_overlay = self.table_picker.as_ref().map(|p| {
            gpui::deferred(
                div()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this: &mut AppView, _: &MouseDownEvent, _, cx| {
                            this.cancel_table_picker(cx);
                        }),
                    )
                    .child(
                        gpui::anchored()
                            .position(p.caret.bottom_left())
                            .snap_to_window()
                            .child(ui::table_picker::render(p, cx)),
                    ),
            )
            .into_any_element()
        });

        // While resizing an image, a transparent full-window layer captures the
        // mouse so the drag continues even as the pointer leaves the handle.
        let drag_overlay = self.image_drag.as_ref().map(|_| {
            gpui::deferred(
                div()
                    .occlude()
                    .absolute()
                    .inset_0()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_move(cx.listener(
                        |this: &mut AppView, ev: &MouseMoveEvent, _window, cx| {
                            this.update_image_drag(ev.position.x, cx);
                        },
                    ))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this: &mut AppView, _ev: &MouseUpEvent, window, cx| {
                            this.finish_image_drag(window, cx);
                        }),
                    ),
            )
            .into_any_element()
        });

        // Jump-to-date calendar: a full-window layer (click-away to close) with
        // the calendar anchored under the sidebar icon. Selecting a date closes
        // it via the calendar subscription.
        let calendar_overlay = self.show_calendar.then(|| {
            gpui::deferred(
                div()
                    .occlude()
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this: &mut AppView, _, _window, cx| {
                            this.show_calendar = false;
                            cx.notify();
                        }),
                    )
                    .child(
                        gpui::anchored()
                            .position(gpui::point(px(8.0), px(86.0)))
                            .snap_to_window_with_margin(px(8.0))
                            .child(
                                div()
                                    // Clicks inside the calendar must not reach
                                    // the click-away backdrop.
                                    .occlude()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .bg(theme::bg_sidebar())
                                    .border_1()
                                    .border_color(theme::border_subtle())
                                    .rounded(px(8.0))
                                    .shadow_lg()
                                    .child(Calendar::new(&self.calendar)),
                            ),
                    ),
            )
            .into_any_element()
        });

        // A clicked mermaid diagram, expanded full-window: the cached image at full
        // resolution in a scrollable box, on a dimmed backdrop that dismisses on click.
        let mermaid_lightbox = self
            .mermaid_lightbox
            .clone()
            .and_then(|source| self.mermaid_store.borrow().get(&source))
            .map(|image| {
                gpui::deferred(
                    div()
                        .occlude()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::hsla(0., 0., 0., 0.72))
                        // Focused on open, so Esc dismisses (no global binding to
                        // clash with the editor's Escape → slash-cancel).
                        .track_focus(&self.lightbox_focus)
                        .on_key_down(cx.listener(
                            |this: &mut AppView, ev: &gpui::KeyDownEvent, window, cx| {
                                if ev.keystroke.key == "escape" {
                                    this.close_mermaid_lightbox(window, cx);
                                }
                            },
                        ))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this: &mut AppView, _, window, cx| {
                                this.close_mermaid_lightbox(window, cx);
                            }),
                        )
                        .child(
                            // The diagram itself: full size + scrollable; clicks here
                            // pan/scroll rather than dismissing.
                            div()
                                .id("mermaid-lightbox")
                                .occlude()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .max_w(gpui::relative(0.95))
                                .max_h(gpui::relative(0.95))
                                .overflow_scroll()
                                .rounded(px(8.0))
                                .bg(theme::bg_content())
                                .border_1()
                                .border_color(theme::border_subtle())
                                .shadow_lg()
                                .child(gpui::img(gpui::ImageSource::from(image))),
                        )
                        .child(
                            // A clear close affordance (the backdrop handler does the work).
                            div()
                                .absolute()
                                .top(px(14.0))
                                .right(px(18.0))
                                .text_size(px(22.0))
                                .text_color(gpui::white())
                                .cursor_pointer()
                                .child("✕"),
                        ),
                )
                .into_any_element()
            });

        let ctx_menu_overlay = self.ctx_menu.as_ref().map(|menu| {
            // Action ids: 0..=2 formula copy/export, 3 day/page Edit, 4..=6 align L/C/R (only
            // while editing the formula, where the in-line editor can re-justify it live).
            let items: Vec<(&str, usize)> = match &menu.kind {
                CtxKind::Formula { alignable, .. } => {
                    let mut v = vec![("Copy LaTeX", 0), ("Export PNG…", 1), ("Export SVG…", 2)];
                    if *alignable {
                        v.extend([("Align left", 4), ("Align center", 5), ("Align right", 6)]);
                    }
                    v
                }
                CtxKind::Edit(_) => vec![("Edit", 3)],
            };
            let mut rows = div().flex().flex_col().py(px(4.0));
            for (label, action_id) in items {
                rows = rows.child(
                    div()
                        .id(("ctx-menu-row", action_id))
                        .px(px(12.0))
                        .py(px(5.0))
                        .text_size(px(14.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::accent_tint()))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                match action_id {
                                    0 => this.math_menu_copy_latex(cx),
                                    1 => this.math_menu_export_png(window, cx),
                                    2 => this.math_menu_export_svg(window, cx),
                                    3 => this.ctx_menu_edit(window, cx),
                                    4 => this.ctx_menu_align(ratex_gpui::MathAlign::Left, cx),
                                    5 => this.ctx_menu_align(ratex_gpui::MathAlign::Center, cx),
                                    _ => this.ctx_menu_align(ratex_gpui::MathAlign::Right, cx),
                                }
                            }),
                        )
                        .child(label),
                );
            }
            gpui::deferred(
                gpui::anchored()
                    .position(menu.anchor)
                    .snap_to_window()
                    .child(
                        div()
                            .occlude()
                            .min_w(px(140.0))
                            .bg(theme::bg_sidebar())
                            .border_1()
                            .border_color(theme::border_subtle())
                            .rounded(px(8.0))
                            .overflow_hidden()
                            .text_color(theme::text_primary())
                            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                this.ctx_menu = None;
                                cx.notify();
                            }))
                            .child(rows),
                    ),
            )
            .into_any_element()
        });

        // Each journal day fills most of the window height so days read as
        // distinct "pages" instead of a continuous wall of text.
        let day_min = px(f32::from(window.viewport_size().height) * 0.75);

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::bg_window())
            .text_color(theme::text_primary())
            // A tab strip drag released anywhere off the strip ends here — in-window
            // (`on_mouse_up`) or past the window edge (`on_mouse_up_out`, which fires
            // because the drag began inside, so the OS keeps delivering the release).
            // The handler hands the tab to the window under the cursor, or tears it
            // into a new one. Strip reorders are consumed earlier by the tab's own
            // `on_drop`, so they never reach here.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this: &mut AppView, _: &MouseUpEvent, window, cx| {
                    this.on_tab_drag_release(window, cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this: &mut AppView, _: &MouseUpEvent, window, cx| {
                    this.on_tab_drag_release(window, cx);
                }),
            )
            // While a tab is dragged, track which other window it's over so that
            // window can show a ghost tab where it would land.
            .on_drag_move(cx.listener(
                |this: &mut AppView, _: &gpui::DragMoveEvent<TabDrag>, window, cx| {
                    this.on_tab_drag_move(window, cx);
                },
            ))
            // Slash-menu keys (gated: act only while the menu is open, else
            // let the editor handle the key normally).
            .on_action(cx.listener(|this: &mut AppView, _: &SlashUp, _, cx| {
                let moved = if let Some(s) = this.slash.as_mut() {
                    let n = s.items.len().max(1);
                    s.selected = (s.selected + n - 1) % n;
                    true
                } else {
                    false
                };
                if moved {
                    this.scroll_slash_into_view();
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &SlashDown, _, cx| {
                let moved = if let Some(s) = this.slash.as_mut() {
                    let n = s.items.len().max(1);
                    s.selected = (s.selected + 1) % n;
                    true
                } else {
                    false
                };
                if moved {
                    this.scroll_slash_into_view();
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(
                cx.listener(|this: &mut AppView, _: &SlashConfirm, window, cx| {
                    if this.slash.is_some() {
                        this.confirm_slash(window, cx);
                    } else if !this.continue_list(window, cx) {
                        cx.propagate();
                    }
                }),
            )
            .on_action(
                cx.listener(|this: &mut AppView, _: &SlashCancel, window, cx| {
                    // From a submenu, Esc backs out to the root categories; from the root
                    // it closes the menu. With no menu open, Esc leaves edit mode: blurring
                    // the focused editor swaps the page/day back to its rendered view via
                    // the editor's Blur handler (same path as clicking away). Otherwise it
                    // propagates (so dialogs etc. still get Esc).
                    match this.slash.as_ref().map(|s| s.level) {
                        Some(SlashLevel::Root) => {
                            this.slash = None;
                            cx.notify();
                        }
                        Some(_) => this.enter_slash_category(SlashLevel::Root, cx),
                        // An open find bar takes Esc first (closes it).
                        None if this.page_find.is_some() => this.close_page_find(cx),
                        None if this.page_editing || this.editing_day.is_some() => window.blur(),
                        None => cx.propagate(),
                    }
                }),
            )
            // Sidebar right-click menu actions.
            .on_action(cx.listener(Self::on_delete_page))
            .on_action(cx.listener(Self::on_open_in_new_tab))
            .on_action(cx.listener(Self::on_open_in_new_window))
            .on_action(cx.listener(Self::on_rename_page))
            .on_action(cx.listener(Self::on_toggle_favorite))
            .on_action(cx.listener(Self::on_new_page))
            .on_action(cx.listener(Self::on_new_whiteboard))
            .on_action(cx.listener(Self::on_import_logseq))
            .on_action(cx.listener(Self::on_fit_images))
            .on_action(cx.listener(Self::on_insert_tab))
            .on_action(cx.listener(Self::on_outdent))
            .on_action(cx.listener(Self::on_paste_image))
            // App-wide shortcuts (Cmd/Ctrl): tab + settings commands handled here
            // per-window; NewWindow / Quit are global App actions (see `main`).
            .on_action(cx.listener(|this: &mut AppView, _: &CloseTab, window, cx| {
                let ix = this.active;
                this.close_tab(ix, window, cx);
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &NextTab, window, cx| {
                this.cycle_tab(1, window, cx)
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &PrevTab, window, cx| {
                this.cycle_tab(-1, window, cx)
            }))
            .on_action(
                cx.listener(|_this: &mut AppView, _: &OpenSettings, window, cx| {
                    // Defer: `open_settings` opens a window and reads `AppView`, which
                    // must not be mid-update (same reason the gear defers).
                    let view = cx.entity();
                    window.defer(cx, move |_, cx| AppView::open_settings(view, cx));
                }),
            )
            .on_action(
                cx.listener(|this: &mut AppView, _: &FindInPage, window, cx| {
                    // ⌘F: find in the active page's rendered text. Only on a Page tab —
                    // PDFs handle ⌘F in the viewer; the journal feed uses ⌘⇧F.
                    if matches!(
                        this.tabs.get(this.active).map(|t| &t.kind),
                        Some(TabKind::Page(_))
                    ) {
                        this.open_page_find(window, cx);
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .on_action(
                cx.listener(|this: &mut AppView, _: &GlobalSearch, window, cx| {
                    this.focus_global_search(window, cx)
                }),
            )
            .child(TitleBar::new().child({
                // The settings gear lives in the sidebar (next to search); the
                // title bar keeps just the theme toggle.
                //
                // `.occlude()` is load-bearing on Windows: the title bar's content
                // is one big window "Drag" region, so a plain button there reads as
                // the OS caption and the click becomes a window-drag (the toggle
                // appeared dead, stuck on Auto). Occluding the toggle removes the
                // drag hitbox under it, so the OS hit-test returns client area and
                // the click lands. Harmless on macOS.
                let toggle = div()
                    .id("theme-toggle")
                    .occlude()
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(theme::text_secondary())
                    .cursor_pointer()
                    .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
                    .child(self.mode.label())
                    .on_click(cx.listener(|this: &mut AppView, _, window, cx| {
                        this.cycle_theme_mode(window, cx);
                    }));
                let label = div()
                    .px_2()
                    .text_size(px(13.0))
                    .text_color(theme::text_secondary())
                    .child("Zorite");
                let row = div().flex().flex_row().items_center().w_full();
                // macOS: the native menu bar carries File/Edit/View, so the titlebar
                // keeps just the title + theme toggle. Windows/Linux: the AppMenuBar
                // leads (it already includes the "Zorite" app menu), then the toggle —
                // and the redundant title label is dropped. `.occlude()` keeps clicks
                // off the titlebar's drag region.
                if cfg!(target_os = "macos") {
                    row.justify_between()
                        .child(label)
                        .child(div().mr_2().child(toggle))
                } else {
                    row.child(
                        div()
                            .occlude()
                            .flex()
                            .items_center()
                            .child(self.app_menu_bar.clone()),
                    )
                    .child(div().mr_2().child(toggle))
                    .child(div().flex_1())
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(ui::sidebar::render(self, window, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .bg(theme::bg_content())
                            .child(ui::tab_bar::render(self, cx))
                            // A dragged tab released here (or anywhere off the strip)
                            // is handled by the root `on_mouse_up` / `on_mouse_up_out`
                            // above — it tears off into a new window or moves to the
                            // window under the cursor.
                            .child(div().flex_1().min_h_0().child(if self.searching {
                                ui::search::render(self, cx).into_any_element()
                            } else {
                                match self.tabs[self.active].kind.clone() {
                                    TabKind::Journal => {
                                        ui::journal::render(self, day_min, cx).into_any_element()
                                    }
                                    TabKind::Page(_) => {
                                        ui::page_view::render(self, cx).into_any_element()
                                    }
                                    TabKind::Pdf(path) => {
                                        match self.pdf_views.get(&path).cloned() {
                                            // Encrypted + not yet unlocked:
                                            // show the password prompt instead
                                            // of the (blank) viewer.
                                            Some(v) if v.read(cx).is_locked() => self
                                                .pdf_password_prompt(
                                                    path.clone(),
                                                    v.read(cx).unlock_failed(),
                                                    cx,
                                                )
                                                .into_any_element(),
                                            Some(v) => v.into_any_element(),
                                            None => gpui::div().into_any_element(),
                                        }
                                    }
                                    TabKind::Whiteboard(id) => {
                                        match self.whiteboard_views.get(&id).cloned() {
                                            Some(v) => v.into_any_element(),
                                            None => gpui::div().into_any_element(),
                                        }
                                    }
                                }
                            })),
                    ),
            )
            .children(overlay)
            .children(table_picker_overlay)
            .children(drag_overlay)
            .children(calendar_overlay)
            .children(mermaid_lightbox)
            .children(ctx_menu_overlay)
            // gpui-component's `Root` tracks dialog state but does NOT render
            // the dialog layer — the host view must, or dialogs (like the
            // delete-page confirm) stay invisible.
            .children(Root::render_dialog_layer(window, cx))
    }
}

/// Tag prefixing whiteboard elements written to the system clipboard, so a ⌘V on
/// a board can tell a copied selection from arbitrary text (and prefer it over a
/// clipboard image). The remainder is the JSON from `WhiteboardView::selection_json`.
const WB_CLIP_PREFIX: &str = "zorite-whiteboard-v1\n";

/// Settings key holding a board's chosen text face (a `fonts/<name>` ref, or empty
/// for the bundled default). Per-board, so each whiteboard keeps its own font.
fn board_font_key(board_id: i64) -> String {
    format!("whiteboard_font_{board_id}")
}

/// Copied whiteboard elements from the clipboard (the JSON after [`WB_CLIP_PREFIX`]),
/// or `None` if the clipboard holds no board elements. Shared by keyboard paste
/// ([`AppView::on_paste_image`]) and the context-menu Paste hook.
fn clipboard_board_json(cx: &App) -> Option<String> {
    cx.read_from_clipboard()?
        .text()?
        .strip_prefix(WB_CLIP_PREFIX)
        .map(str::to_owned)
}

/// Map a clipboard image format to a file extension for the saved file.
fn clipboard_ext(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
        ImageFormat::Svg => "svg",
        _ => "png",
    }
}

/// The image content-column width for a page/feed `scroll` viewport: its width
/// minus the body's 28px padding on each side.
fn content_width(bounds: Bounds<Pixels>) -> i64 {
    (f32::from(bounds.size.width) - 56.0) as i64
}

/// Shrink every image rendered wider than `target` px down to `target` — a
/// comfortable size (about half the column) so images dragged or imported wider
/// than that don't dominate the page. `images` pairs each image's `attr_target`
/// byte range with its current rendered width (an explicit `{width=N}`, else the
/// width measured during paint); the range is overwritten with `{width=target}`
/// (an empty range inserts on a width-less image, a `{width=N}` range replaces).
/// Edits apply right-to-left so byte offsets stay valid. Idempotent: an image
/// already at or under `target` is left alone. Returns new content if anything
/// changed.
fn apply_fit(content: &str, images: &[(Range<usize>, f32)], target: i64) -> Option<String> {
    let mut edits: Vec<&Range<usize>> = images
        .iter()
        .filter(|(_, w)| *w as i64 > target)
        .map(|(r, _)| r)
        .collect();
    if edits.is_empty() {
        return None;
    }
    // Apply later (higher-offset) edits first so earlier offsets don't shift.
    edits.sort_by_key(|r| std::cmp::Reverse(r.start));
    let repl = format!("{{width={target}}}");
    let mut out = content.to_string();
    for range in edits {
        out.replace_range(range.clone(), &repl);
    }
    Some(out)
}

/// Clamp `offset` to `source`'s length and snap it down to a char boundary.
fn clamp_to_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Layout constants for our chrome-less gpui-editor body editors, used by
/// [`predict_caret_row`] to position the caret *before* the editor first paints.
/// gpui-editor draws no internal padding and soft-wraps at its full width, so
/// the padding / wrap-margin are zero (kept named so the click-to-edit math
/// reads clearly). Its line height is `EDITOR_TEXT_SIZE * 1.25`, which matches
/// the `rems(1.25)` used below.
const INPUT_PY: Pixels = px(0.0);
const INPUT_PX: Pixels = px(0.0);
const INPUT_WRAP_RIGHT_MARGIN: Pixels = px(0.0);
const EDITOR_TEXT_SIZE: Pixels = px(16.0);

/// Predict where the caret at byte `off` will land inside one of our editors:
/// `(wrap rows above the caret, line height)`. `slot_width` is the width of the
/// element the editor will occupy. Counts soft-wrap rows with the same
/// `LineWrapper` machinery (same font, size, and wrap width) gpui-component
/// wraps with, so the prediction matches the editor's own layout.
fn predict_caret_row(
    source: &str,
    off: usize,
    slot_width: Pixels,
    window: &Window,
    cx: &App,
) -> (usize, Pixels) {
    use gpui_component::ActiveTheme as _;
    // The editor inherits the root text style with the theme font family
    // (applied by gpui-component's Root) and the host's text size, while the
    // Input element pins line height to 1.25 rems (its `LINE_HEIGHT`). The
    // inheritance stack isn't populated during event dispatch, so mirror it.
    let mut style = window.text_style();
    style.font_size = EDITOR_TEXT_SIZE.into();
    style.font_family = cx.theme().font_family.clone();
    style.line_height = gpui::rems(1.25).into();
    let line_height = style.line_height_in_pixels(window.rem_size());
    let wrap_width = slot_width - INPUT_PX * 2.0 - INPUT_WRAP_RIGHT_MARGIN;
    let mut wrapper = cx
        .text_system()
        .line_wrapper(style.font(), EDITOR_TEXT_SIZE);
    let off = clamp_to_boundary(source, off);
    let mut rows = 0usize;
    let mut line_start = 0usize;
    for line in source.split('\n') {
        let line_end = line_start + line.len();
        let fragments = [gpui::LineFragment::text(line)];
        let boundaries = wrapper.wrap_line(&fragments, wrap_width);
        if off <= line_end {
            // The caret's line: wraps at or before the caret's column push it down.
            let col = off - line_start;
            rows += boundaries.filter(|b| b.ix <= col).count();
            break;
        }
        rows += 1 + boundaries.count();
        line_start = line_end + 1;
    }
    (rows, line_height)
}

/// Frame-loop state for [`align_caret_to_click`].
struct CaretAlign {
    editor: Entity<EditorState>,
    scroll: ScrollHandle,
    view: Entity<AppView>,
    /// Caret byte offset and the window y it should sit at.
    off: usize,
    click_y: Pixels,
    /// Last frame's `(caret y, scroll offset)` — a correction only applies once
    /// two consecutive frames agree, because the editor's reported caret bounds
    /// can be stale right after the mode switch (layout from a previous paint).
    last: Option<(Pixels, Pixels)>,
    tries: u32,
    applies: u32,
}

impl CaretAlign {
    fn new(
        editor: Entity<EditorState>,
        scroll: ScrollHandle,
        view: Entity<AppView>,
        off: usize,
        click_y: Pixels,
    ) -> Self {
        Self {
            editor,
            scroll,
            view,
            off,
            click_y,
            last: None,
            tries: 20,
            applies: 2,
        }
    }
}

/// After entering edit mode, fine-tune the scroll so the caret sits at the
/// click's y — a mop-up pass for any drift in [`predict_caret_row`]'s estimate.
/// Samples are gated on two-frame agreement (see [`CaretAlign::last`]); the
/// correction is skipped inside a small epsilon and capped to a couple of
/// nudges so it can never fight the user.
fn align_caret_to_click(mut state: CaretAlign, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        if state.tries == 0 || state.applies == 0 {
            return;
        }
        state.tries -= 1;
        // Keep frames coming while we sample: layout only refreshes on paint.
        state.view.update(cx, |_, cx| cx.notify());
        let caret = state
            .editor
            .read(cx)
            .bounds_for_offset(state.off)
            .map(|b| b.origin.y);
        let offset = state.scroll.offset().y;
        let Some(caret_y) = caret else {
            state.last = None;
            align_caret_to_click(state, window);
            return;
        };
        let sample = (caret_y, offset);
        if state.last != Some(sample) {
            state.last = Some(sample);
            align_caret_to_click(state, window);
            return;
        }
        let new_y = (offset + (state.click_y - caret_y)).min(px(0.0));
        if (new_y - offset).abs() <= px(2.0) {
            return;
        }
        state.scroll.set_offset(gpui::point(px(0.0), new_y));
        state.last = None;
        state.applies -= 1;
        align_caret_to_click(state, window);
    });
}

/// A soft-wrapping, chrome-less editor seeded with `content`. Uses
/// `auto_grow` (not plain `multi_line`, which fills its container) so the
/// editor is one line when empty and grows line-by-line with content —
/// the outer feed scrolls, never the individual day. The high `max_rows`
/// effectively means "never scroll internally".
#[allow(clippy::too_many_arguments)]
fn make_editor(
    content: &str,
    wysiwyg: bool,
    list_indent: usize,
    image_store: Rc<RefCell<crate::images::ImageStore>>,
    mermaid_store: Rc<RefCell<crate::mermaid::MermaidStore>>,
    math_store: Rc<RefCell<crate::math::MathStore>>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> Entity<EditorState> {
    // Our gpui-editor auto-grows to its content height and soft-wraps by design,
    // so the feed/page scrolls and the editor never does — the behavior the old
    // `auto_grow(1, 100_000)` InputState approximated.
    let editor = cx.new(|cx| {
        let mut editor = EditorState::new(window, cx).with_text(content);
        // Right-click a flagged word → the OS's suggestions, fetched lazily.
        editor.on_suggest(|word| os_spellcheck::SpellChecker::new().suggestions(word));
        // Inline images (W4): resolve a standalone image's src to its decoded
        // bitmap from the shared store (None until decoding finishes / on fail).
        editor.set_block_image_provider(move |src| image_store.borrow().get(src));
        // A `![](file.pdf)` renders as a clickable chip (label = file name) that
        // opens the PDF viewer on click — matching the reading view.
        editor.set_block_chip_provider(|src| {
            crate::pdf::is_pdf(src).then(|| {
                crate::pdf::resolve_path(src)
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| src.to_string())
                    .into()
            })
        });
        // A ```mermaid block renders as its diagram from the shared store (None
        // until the off-thread render finishes — the block shows raw code then).
        editor.set_block_mermaid_provider(move |source| {
            mermaid_store
                .borrow()
                .get(&gpui::SharedString::from(source))
        });
        // A $$…$$ block renders as its typeset equation from the shared store (None
        // until the off-thread render finishes — the block shows raw source then). The
        // store keeps a logical size for the reading view; the editor sizes the bitmap
        // itself, so hand it just the image.
        editor.set_block_math_provider(move |source| {
            math_store
                .borrow()
                .get(&gpui::SharedString::from(source))
                .map(|(img, _, _)| img)
        });
        // Inline `$…$` formulas reuse those rasters (typeset at this em) scaled to text size.
        editor.set_block_math_em(crate::math::FONT_SIZE);
        // Tab / Shift+Tab indent by the configured number of spaces per level.
        editor.set_tab_indent(list_indent);
        editor
    });
    // Live-preview markdown styling when WYSIWYG is on — mirrors the rendered
    // view's colors so formatting (bold/italic/code/links/tags) shows inline as
    // you type (W1). Off = plain raw markdown ("editor mode").
    if wysiwyg {
        editor.update(cx, |editor, cx| {
            editor.set_markdown_style(theme::editor_syntax_style(), cx)
        });
    }
    editor
}

/// Run the OS spell checker over `text`, mapping each misspelling to an editor
/// diagnostic (a red wavy underline). Detection only — suggestions are fetched
/// lazily on right-click via the editor's `on_suggest` provider.
fn spell_diagnostics(text: &str) -> Vec<Diagnostic> {
    os_spellcheck::SpellChecker::new()
        .check(text)
        .into_iter()
        .map(|range| Diagnostic { range })
        .collect()
}

/// ISO `YYYY-MM-DD` for the day `i` days before today (local time). This is the
/// stable storage key for a journal day, so it stays ISO regardless of the
/// user's display date-format preference.
pub(crate) fn date_for_offset(i: usize) -> String {
    let dt = crate::dates::now_local() - time::Duration::days(i as i64);
    format!(
        "{:04}-{:02}-{:02}",
        dt.year(),
        u8::from(dt.month()),
        dt.day()
    )
}

/// Human-friendly header for the day `i` days back, e.g.
/// "Today · Thursday, June 4, 2026".
pub(crate) fn date_label(i: usize) -> String {
    let dt = crate::dates::now_local() - time::Duration::days(i as i64);
    let label = format!(
        "{}, {} {}, {}",
        crate::dates::weekday_name(dt.weekday()),
        crate::dates::month_name(dt.month()),
        dt.day(),
        dt.year()
    );
    match i {
        0 => format!("Today · {label}"),
        1 => format!("Yesterday · {label}"),
        _ => label,
    }
}

#[cfg(test)]
mod tests {
    use super::apply_fit;

    #[test]
    fn apply_fit_shrinks_only_wide_images() {
        // `a` has an explicit {width=2000} at bytes 6..18; `b` is width-less, so
        // its attr_target is the empty insertion point 25..25 (measured at 900).
        let src = "![](a){width=2000} ![](b)";
        let imgs = vec![(6..18, 2000.0), (25..25, 900.0)];
        assert_eq!(
            apply_fit(src, &imgs, 400).as_deref(),
            Some("![](a){width=400} ![](b){width=400}")
        );
    }

    #[test]
    fn apply_fit_leaves_images_at_or_under_target() {
        // A width-less image measured under the target stays untouched.
        assert_eq!(apply_fit("![](a)", &[(6..6, 300.0)], 400), None);
        // An explicit width already at the target is a no-op (idempotent).
        assert_eq!(apply_fit("![](a){width=400}", &[(6..17, 400.0)], 400), None);
    }

    #[test]
    fn apply_fit_is_idempotent() {
        let once = apply_fit("![](a){width=2000}", &[(6..18, 2000.0)], 400).unwrap();
        assert_eq!(once, "![](a){width=400}");
        // Re-running with the now-comfortable width changes nothing.
        assert_eq!(apply_fit(&once, &[(6..17, 400.0)], 400), None);
    }
}
