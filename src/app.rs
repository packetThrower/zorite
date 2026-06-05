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

use std::collections::HashMap;

use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, TitlebarOptions, Window, WindowAppearance, WindowBounds, WindowDecorations,
    WindowHandle, WindowOptions, div, px, size,
};
use gpui_component::{
    RopeExt, Root, TitleBar, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    dialog::{DialogButtonProps, DialogFooter},
    input::{Input, InputEvent, InputState},
};

use crate::actions::{
    DeletePage, NewPage, OpenInNewTab, RenamePage, SlashCancel, SlashConfirm, SlashDown, SlashUp,
};
use crate::db::Db;
use crate::models::{Backlink, Page, SearchHit};
use crate::settings::SettingsView;
use crate::skins::{self, Skin};
use crate::slash::{self, ItemKind, Slash, SlashLevel, SlashTarget, Template};
use crate::theme;
use crate::ui;

/// How many days to add each time the feed grows.
const FEED_CHUNK: usize = 7;
/// Hard cap on how far back the feed loads (~10 years), a runaway guard.
const FEED_MAX_DAYS: usize = 3650;

/// What a tab shows. The Journal is the pinned tab 0; the rest are pages.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Journal,
    Page(i64),
}

/// An open tab: its content kind + a cached title for the tab strip.
pub struct OpenTab {
    pub kind: TabKind,
    pub title: SharedString,
}

/// A journal day's editor + the subscription saving its edits.
pub struct DayEditor {
    pub state: Entity<InputState>,
    _sub: Subscription,
}

/// The currently-open named/journal page in `View::Page`.
pub struct PageEditor {
    pub title: String,
    /// Inline-editable page title (named pages only); renames on Enter/blur.
    pub title_state: Entity<InputState>,
    pub is_journal: bool,
    pub state: Entity<InputState>,
    _sub: Subscription,
    _title_sub: Subscription,
    pub backlinks: Vec<Backlink>,
}

pub struct AppView {
    db: Db,
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
    /// In the feed, the date currently being edited (raw editor); all
    /// other days render as markdown. `None` = every day rendered.
    editing_day: Option<String>,
    /// Whether the single-page editor is in edit (raw) vs reading mode.
    page_editing: bool,

    // Journal feed.
    pub loaded_days: usize,
    pub day_editors: HashMap<String, DayEditor>,
    pub feed_scroll: ScrollHandle,

    // Single-page view.
    pub page_editor: Option<PageEditor>,

    // Sidebar.
    pub pages: Vec<Page>,
    pub new_page_input: Entity<InputState>,
    pub search_input: Entity<InputState>,
    pub search_results: Vec<SearchHit>,
    /// Open slash-command menu, if any.
    slash: Option<Slash>,
    /// Templates parsed from the reserved `Templates` page.
    templates: Vec<Template>,
    /// The page (id + title) targeted by an open right-click context menu,
    /// read by the `DeletePage` / `RenamePage` actions.
    context_page: Option<(i64, SharedString)>,
    /// The rename dialog's text field, and the page being renamed.
    rename_input: Entity<InputState>,
    rename_target: Option<i64>,

    _subs: Vec<Subscription>,
    pub focus_handle: FocusHandle,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let db = Db::open()
            .or_else(|e| {
                log::error!("open database on disk failed ({e}); using in-memory store");
                Db::open_in_memory()
            })
            .expect("initialize database");

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

        let mut this = Self {
            db,
            tabs: vec![OpenTab { kind: TabKind::Journal, title: "Journal".into() }],
            active: 0,
            searching: false,
            tab_scroll: ScrollHandle::new(),
            mode: theme::Mode::Dark,
            system_dark: true,
            settings_window: None,
            skins: skins::builtin_skins(),
            skin_id: String::new(),
            editing_day: None,
            page_editing: false,
            loaded_days: 0,
            day_editors: HashMap::new(),
            feed_scroll: ScrollHandle::new(),
            page_editor: None,
            pages: Vec::new(),
            new_page_input,
            search_input,
            search_results: Vec::new(),
            slash: None,
            templates: Vec::new(),
            context_page: None,
            rename_input: cx.new(|cx| InputState::new(window, cx)),
            rename_target: None,
            _subs: vec![search_sub],
            focus_handle: cx.focus_handle(),
        };

        this.loaded_days = 14;
        for i in 0..this.loaded_days {
            this.ensure_day_editor(date_for_offset(i), window, cx);
        }
        this.refresh_sidebar();
        // Load user themes on top of the built-ins, then apply the saved
        // (or default) skin + mode before the first paint.
        this.skins.extend(skins::load_user_skins());
        this.skin_id = this.db.get_setting("theme_skin").unwrap_or_else(|| "zorite".to_string());
        this.mode = this
            .db
            .get_setting("theme_mode")
            .map(|s| theme::Mode::from_str(&s))
            .unwrap_or_default();
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
        let state = make_editor(&content, window, cx);
        let key = date.clone();
        let sub = cx.subscribe_in(
            &state,
            window,
            move |this: &mut AppView, st, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => {
                    let value = st.read(cx).value().to_string();
                    this.save_journal(&key, &value);
                    this.update_slash(SlashTarget::Day(key.clone()), cx);
                }
                InputEvent::Focus => {
                    this.editing_day = Some(key.clone());
                    cx.notify();
                }
                InputEvent::Blur => {
                    if this.editing_day.as_deref() == Some(key.as_str()) {
                        this.editing_day = None;
                    }
                    this.slash = None;
                    let value = st.read(cx).value().to_string();
                    this.flush_journal(&key, &value);
                    cx.notify();
                }
                _ => {}
            },
        );
        self.day_editors.insert(date, DayEditor { state, _sub: sub });
    }

    /// Reload cached journal day editors from the DB. Called after an action
    /// that rewrites content across pages (e.g. a page rename that updated
    /// `[[links]]`) so the feed shows the new text instead of stale cache.
    /// Only days whose content actually changed are touched.
    fn reload_day_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dates: Vec<String> = self.day_editors.keys().cloned().collect();
        for date in dates {
            let content = self
                .db
                .get_journal_by_date(&date)
                .ok()
                .flatten()
                .map(|p| p.content)
                .unwrap_or_default();
            if let Some(de) = self.day_editors.get(&date) {
                if de.state.read(cx).value().to_string() != content {
                    de.state.update(cx, |s, cx| s.set_value(content, window, cx));
                }
            }
        }
    }

    /// Save a journal day's content on every keystroke — but NOT its
    /// links/tags. Link re-indexing (which creates target pages) happens
    /// on blur, so a half-typed `#tag` doesn't spawn a page per keystroke.
    fn save_journal(&mut self, date: &str, content: &str) {
        if let Ok(page) = self.db.get_or_create_journal(date) {
            if let Err(e) = self.db.set_page_content(page.id, content) {
                log::error!("save journal {date}: {e}");
            }
        }
    }

    /// On blur: persist the day and re-index its `[[links]]` / `#tags`.
    fn flush_journal(&mut self, date: &str, content: &str) {
        if let Ok(page) = self.db.get_or_create_journal(date) {
            self.persist(page.id, content);
        }
        self.refresh_sidebar();
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
        match self.db.get_page(id) {
            Ok(Some(page)) => self.open_page_foreground(page, window, cx),
            Ok(None) => log::warn!("page {id} not found"),
            Err(e) => log::error!("open page {id}: {e}"),
        }
    }

    pub fn open_page_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.get_or_create_page(title) {
            Ok(page) => {
                self.open_page_foreground(page, window, cx);
                // The page may be newly created (via the New-page dialog or a
                // [[link]]), so refresh the sidebar to show it.
                self.refresh_sidebar();
            }
            Err(e) => log::error!("open page '{title}': {e}"),
        }
    }

    fn open_page_foreground(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tab_index_for(page.id) {
            self.activate_tab(ix, window, cx);
        } else {
            self.tabs.push(OpenTab { kind: TabKind::Page(page.id), title: page.title.into() });
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
                self.tabs.push(OpenTab { kind: TabKind::Page(id), title: page.title.into() });
                cx.notify();
            }
            Ok(None) => log::warn!("page {id} not found"),
            Err(e) => log::error!("open page {id}: {e}"),
        }
    }

    fn tab_index_for(&self, id: i64) -> Option<usize> {
        self.tabs.iter().position(|t| matches!(t.kind, TabKind::Page(pid) if pid == id))
    }

    /// Switch to tab `ix` and (re)build its content. Tabs share one page
    /// editor, so activating a Page tab rebuilds the editor from the DB.
    pub fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        let kind = tab.kind.clone();
        self.active = ix;
        self.searching = false;
        match kind {
            TabKind::Journal => {
                self.page_editor = None;
                for i in 0..self.loaded_days {
                    self.ensure_day_editor(date_for_offset(i), window, cx);
                }
            }
            TabKind::Page(id) => self.load_page_editor(id, window, cx),
        }
        cx.notify();
    }

    /// Close tab `ix`. The Journal (index 0) is pinned and never closes.
    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix == 0 || ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        if self.active > ix {
            self.active -= 1;
        } else if self.active == ix {
            self.active = self.active.min(self.tabs.len() - 1);
        }
        self.activate_tab(self.active, window, cx);
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
        let state = make_editor(&page.content, window, cx);
        let sub = cx.subscribe_in(
            &state,
            window,
            move |this: &mut AppView, st, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => {
                    // Content only; link re-indexing happens on blur.
                    let value = st.read(cx).value().to_string();
                    if let Err(e) = this.db.set_page_content(pid, &value) {
                        log::error!("save page {pid}: {e}");
                    }
                    this.update_slash(SlashTarget::Page(pid), cx);
                }
                InputEvent::Focus => {
                    this.page_editing = true;
                    cx.notify();
                }
                InputEvent::Blur => {
                    this.page_editing = false;
                    this.slash = None;
                    let value = st.read(cx).value().to_string();
                    this.persist(pid, &value);
                    this.refresh_sidebar();
                    cx.notify();
                }
                _ => {}
            },
        );
        let backlinks = self.db.backlinks(pid).unwrap_or_default();

        // Inline-editable title: renames the page on Enter or blur.
        let title_state = cx.new(|cx| InputState::new(window, cx).default_value(page.title.clone()));
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

        self.page_editor = Some(PageEditor {
            title: page.title,
            title_state,
            is_journal: page.is_journal,
            state,
            _sub: sub,
            _title_sub: title_sub,
            backlinks,
        });
        self.page_editing = false;
    }

    // --- Persistence ---

    /// Save a page's content and re-index its outgoing `[[links]]`.
    fn persist(&mut self, page_id: i64, content: &str) {
        if let Err(e) = self.db.set_page_content(page_id, content) {
            log::error!("save page {page_id}: {e}");
        }
        let titles = ui::links::parse_links(content);
        if let Err(e) = self.db.rebuild_page_links(page_id, &titles) {
            log::error!("rebuild links for page {page_id}: {e}");
        }
    }

    fn refresh_sidebar(&mut self) {
        self.pages = self.db.list_pages().unwrap_or_default();
        self.templates = self
            .db
            .get_page_by_title(slash::TEMPLATES_PAGE)
            .ok()
            .flatten()
            .map(|p| slash::parse_templates(&p.content))
            .unwrap_or_default();
    }

    /// Run the sidebar search box live. Empty query returns to the feed.
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let q = self.search_input.read(cx).value().trim().to_string();
        if q.is_empty() {
            self.search_results.clear();
            self.searching = false;
        } else {
            self.search_results = self.db.search(&q, 50).unwrap_or_default();
            self.searching = true;
        }
        cx.notify();
    }

    // --- Slash-command menu ---

    /// Recompute the slash menu from the target editor's caret (called on
    /// every edit). Opens it at the caret when a `/token` is present.
    fn update_slash(&mut self, target: SlashTarget, cx: &mut Context<Self>) {
        let editor = self.editor_for(&target);
        let Some(editor) = editor else {
            self.slash = None;
            cx.notify();
            return;
        };
        let (value, cursor) = {
            let s = editor.read(cx);
            (s.value().to_string(), s.cursor())
        };
        let Some((start, query)) = slash::detect(&value, cursor) else {
            self.slash = None;
            cx.notify();
            return;
        };
        let Some(caret) = editor.read(cx).range_to_bounds(&(start..start)) else {
            self.slash = None;
            cx.notify();
            return;
        };
        let level = self.slash.as_ref().map_or(SlashLevel::Root, |s| s.level);
        let title = self.slash_title(&target);
        let items = slash::build_items(level, &query, &self.templates, &title);
        let selected = self.slash.as_ref().map_or(0, |s| s.selected);
        let selected = if items.is_empty() { 0 } else { selected.min(items.len() - 1) };
        self.slash = Some(Slash { target, query, start, caret, selected, level, items });
        cx.notify();
    }

    fn slash_title(&self, target: &SlashTarget) -> String {
        match target {
            SlashTarget::Day(d) => d.clone(),
            SlashTarget::Page(_) => {
                self.page_editor.as_ref().map(|pe| pe.title.clone()).unwrap_or_default()
            }
        }
    }

    /// Confirm the selected entry: open a category submenu, or insert.
    fn confirm_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        enum Act {
            Enter(SlashLevel),
            Insert(String, usize),
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
            }
        };
        match act {
            Act::Enter(level) => self.enter_slash_category(level, cx),
            Act::Insert(snippet, caret) => self.insert_slash(snippet, caret, window, cx),
        }
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
        let items = slash::build_items(level, &query, &self.templates, &title);
        self.slash = Some(Slash { target, query, start, caret, selected: 0, level, items });
        cx.notify();
    }

    /// Insert a snippet at the `/query`, then close the menu.
    fn insert_slash(&mut self, snippet: String, caret: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.slash.take() else { return };
        let Some(editor) = self.editor_for(&s.target) else {
            cx.notify();
            return;
        };
        let value = editor.read(cx).value().to_string();
        let cursor = editor.read(cx).cursor().min(value.len());
        let start = s.start.min(cursor);
        let new = format!("{}{}{}", &value[..start], snippet, &value[cursor..]);
        let caret_off = start + caret;
        editor.update(cx, |st, cx| {
            st.set_value(new.clone(), window, cx);
            let pos = st.text().offset_to_position(caret_off);
            st.set_cursor_position(pos, window, cx);
        });
        match &s.target {
            SlashTarget::Day(d) => self.save_journal(d, &new),
            SlashTarget::Page(pid) => {
                if let Err(e) = self.db.set_page_content(*pid, &new) {
                    log::error!("save page {pid}: {e}");
                }
            }
        }
        cx.notify();
    }

    fn editor_for(&self, target: &SlashTarget) -> Option<Entity<InputState>> {
        match target {
            SlashTarget::Day(d) => self.day_editors.get(d).map(|de| de.state.clone()),
            SlashTarget::Page(_) => self.page_editor.as_ref().map(|pe| pe.state.clone()),
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

    /// Enter edit mode for the open page (same not-yet-rendered caveat).
    pub fn edit_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page_editing = true;
        if let Some(pe) = self.page_editor.as_ref() {
            pe.state.clone().update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
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
        self.skins.iter().find(|s| s.id == self.skin_id).unwrap_or(&self.skins[0])
    }

    /// Resolve the active skin + mode (+ OS appearance for Auto) to a
    /// palette and push it live to every window.
    fn apply_theme(&self, window: &mut Window, cx: &mut Context<Self>) {
        let is_dark = match self.mode {
            theme::Mode::Light => false,
            theme::Mode::Dark => true,
            theme::Mode::Auto => self.system_dark,
        };
        let palette = {
            let skin = self.current_skin();
            if is_dark { skin.dark } else { skin.light }
        };
        theme::apply(palette, is_dark, window, cx);
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
    pub fn set_theme_mode(&mut self, mode: theme::Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        self.apply_theme(window, cx);
        let _ = self.db.set_setting("theme_mode", mode.as_str());
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
        if let Some(handle) = existing {
            if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
                return;
            }
        }
        let app = view.downgrade();
        let bounds = Bounds::centered(None, size(px(720.0), px(560.0)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Settings · zorite".into()),
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

    // --- Delete page (sidebar right-click → confirm) ---

    /// Remember which page a right-click context menu targets, so the
    /// `DeletePage` action knows what to delete. Called from the sidebar.
    pub fn set_context_page(&mut self, id: i64, title: SharedString) {
        self.context_page = Some((id, title));
    }

    /// `DeletePage` handler: confirm, then delete the remembered page.
    fn on_delete_page(&mut self, _: &DeletePage, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, title)) = self.context_page.take() else { return };
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
    fn on_open_in_new_tab(&mut self, _: &OpenInNewTab, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.context_page.take() {
            self.open_page_in_new_tab(id, cx);
        }
    }

    /// Delete a named page and refresh the UI. Journals are never deleted
    /// (the DB guards this too). Any tabs showing the page are closed.
    fn delete_page(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.delete_page(id) {
            Ok(true) => {
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
                self.activate_tab(self.active, window, cx);
            }
            Ok(false) => {}
            Err(e) => log::error!("delete page {id}: {e}"),
        }
    }

    /// `NewPage` handler: prompt for a title in a dialog, then create and
    /// open the page (dispatched from a pages-area right-click menu).
    fn on_new_page(&mut self, _: &NewPage, window: &mut Window, cx: &mut Context<Self>) {
        self.new_page_input.update(cx, |s, cx| s.set_value("", window, cx));
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
                        .child(Button::new("new-page-create").primary().label("Create").on_click(
                            move |_, window, cx| {
                                let title = input_btn.read(cx).value().trim().to_string();
                                if !title.is_empty() {
                                    let _ = weak_btn
                                        .update(cx, |this, cx| this.open_page_title(&title, window, cx));
                                }
                                window.close_dialog(cx);
                            },
                        )),
                )
                .on_ok(move |_, window, cx| {
                    let title = input_key.read(cx).value().trim().to_string();
                    if !title.is_empty() {
                        let _ =
                            weak_key.update(cx, |this, cx| this.open_page_title(&title, window, cx));
                    }
                    true
                })
                .on_cancel(|_, _window, _cx| true)
        });
        self.new_page_input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// `RenamePage` handler: open a dialog with a text field, pre-filled
    /// with the current title, to rename the right-clicked page.
    fn on_rename_page(&mut self, _: &RenamePage, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, title)) = self.context_page.take() else { return };
        self.rename_target = Some(id);
        self.rename_input.update(cx, |s, cx| s.set_value(title.to_string(), window, cx));

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
        let Some(id) = self.rename_target.take() else { return };
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
        let overlay = self.slash.as_ref().map(|s| {
            gpui::deferred(
                gpui::anchored()
                    .position(s.caret.bottom_left())
                    .snap_to_window()
                    .child(ui::slash_menu::render(s)),
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
            // Slash-menu keys (gated: act only while the menu is open, else
            // let the editor handle the key normally).
            .on_action(cx.listener(|this: &mut AppView, _: &SlashUp, _, cx| {
                if let Some(s) = this.slash.as_mut() {
                    let n = s.items.len().max(1);
                    s.selected = (s.selected + n - 1) % n;
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &SlashDown, _, cx| {
                if let Some(s) = this.slash.as_mut() {
                    let n = s.items.len().max(1);
                    s.selected = (s.selected + 1) % n;
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &SlashConfirm, window, cx| {
                if this.slash.is_some() {
                    this.confirm_slash(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &SlashCancel, _, cx| {
                // From a submenu, Esc backs out to the root categories;
                // from the root it closes the menu.
                match this.slash.as_ref().map(|s| s.level) {
                    Some(SlashLevel::Root) => {
                        this.slash = None;
                        cx.notify();
                    }
                    Some(_) => this.enter_slash_category(SlashLevel::Root, cx),
                    None => cx.propagate(),
                }
            }))
            // Sidebar right-click menu actions.
            .on_action(cx.listener(Self::on_delete_page))
            .on_action(cx.listener(Self::on_open_in_new_tab))
            .on_action(cx.listener(Self::on_rename_page))
            .on_action(cx.listener(Self::on_new_page))
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .w_full()
                        .child(
                            div()
                                .px_2()
                                .text_size(px(13.0))
                                .text_color(theme::text_secondary())
                                .child("zorite"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(2.0))
                                .mr_2()
                                .child(
                                    div()
                                        .id("settings-gear")
                                        .px_2()
                                        .py_1()
                                        .rounded(px(6.0))
                                        .text_size(px(14.0))
                                        .text_color(theme::text_secondary())
                                        .cursor_pointer()
                                        .hover(|h| {
                                            h.bg(theme::hover()).text_color(theme::text_primary())
                                        })
                                        .child("⚙")
                                        // Defer: opening a window from inside the
                                        // mouse-event callback aborts (no-unwind
                                        // boundary). Run it after the event cycle.
                                        .on_click(cx.listener(
                                            |_this: &mut AppView, _, window, cx| {
                                                let view = cx.entity();
                                                window.defer(cx, move |_, cx| {
                                                    AppView::open_settings(view, cx);
                                                });
                                            },
                                        )),
                                )
                                .child(
                                    div()
                                        .id("theme-toggle")
                                        .px_2()
                                        .py_1()
                                        .rounded(px(6.0))
                                        .text_size(px(12.0))
                                        .text_color(theme::text_secondary())
                                        .cursor_pointer()
                                        .hover(|h| {
                                            h.bg(theme::hover()).text_color(theme::text_primary())
                                        })
                                        .child(self.mode.label())
                                        .on_click(cx.listener(
                                            |this: &mut AppView, _, window, cx| {
                                                this.cycle_theme_mode(window, cx);
                                            },
                                        )),
                                ),
                        ),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(ui::sidebar::render(self, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .bg(theme::bg_content())
                            .child(ui::tab_bar::render(self, cx))
                            .child(div().flex_1().min_h_0().child(if self.searching {
                                ui::search::render(self, cx).into_any_element()
                            } else {
                                match self.tabs[self.active].kind {
                                    TabKind::Journal => {
                                        ui::journal::render(self, day_min, cx).into_any_element()
                                    }
                                    TabKind::Page(_) => {
                                        ui::page_view::render(self, cx).into_any_element()
                                    }
                                }
                            })),
                    ),
            )
            .children(overlay)
            // gpui-component's `Root` tracks dialog state but does NOT render
            // the dialog layer — the host view must, or dialogs (like the
            // delete-page confirm) stay invisible.
            .children(Root::render_dialog_layer(window, cx))
    }
}

/// A soft-wrapping, chrome-less editor seeded with `content`. Uses
/// `auto_grow` (not plain `multi_line`, which fills its container) so the
/// editor is one line when empty and grows line-by-line with content —
/// the outer feed scrolls, never the individual day. The high `max_rows`
/// effectively means "never scroll internally".
fn make_editor(content: &str, window: &mut Window, cx: &mut Context<AppView>) -> Entity<InputState> {
    cx.new(|cx| {
        let mut s = InputState::new(window, cx).auto_grow(1, 100_000);
        s.set_soft_wrap(true, window, cx);
        s.set_value(content, window, cx);
        s
    })
}

/// ISO `YYYY-MM-DD` for the day `i` days before today (local time).
pub(crate) fn date_for_offset(i: usize) -> String {
    let dt = now_local() - time::Duration::days(i as i64);
    format!("{:04}-{:02}-{:02}", dt.year(), u8::from(dt.month()), dt.day())
}

/// Human-friendly header for the day `i` days back, e.g.
/// "Today · Thursday, June 4, 2026".
pub(crate) fn date_label(i: usize) -> String {
    let dt = now_local() - time::Duration::days(i as i64);
    let label = format!(
        "{}, {} {}, {}",
        weekday_name(dt.weekday()),
        month_name(dt.month()),
        dt.day(),
        dt.year()
    );
    match i {
        0 => format!("Today · {label}"),
        1 => format!("Yesterday · {label}"),
        _ => label,
    }
}

fn now_local() -> time::OffsetDateTime {
    time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc())
}

fn weekday_name(w: time::Weekday) -> &'static str {
    use time::Weekday::*;
    match w {
        Monday => "Monday",
        Tuesday => "Tuesday",
        Wednesday => "Wednesday",
        Thursday => "Thursday",
        Friday => "Friday",
        Saturday => "Saturday",
        Sunday => "Sunday",
    }
}

fn month_name(m: time::Month) -> &'static str {
    use time::Month::*;
    match m {
        January => "January",
        February => "February",
        March => "March",
        April => "April",
        May => "May",
        June => "June",
        July => "July",
        August => "August",
        September => "September",
        October => "October",
        November => "November",
        December => "December",
    }
}
