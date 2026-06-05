//! `AppView` — the root view. Two surfaces:
//!
//! * `View::Journal` — an infinite, reverse-chronological feed of daily
//!   entries (today on top), each a single multi-line markdown editor.
//!   Older days are created lazily as you scroll down; a day's database
//!   row is created on first edit, so scrolling past empty days is free.
//! * `View::Page(id)` — a single named page (or a journal opened from the
//!   sidebar) in one editor, with a "Linked References" panel.
//!
//! Each editor is a gpui-component `InputState` in multi-line mode, which
//! gives a real Word-like typing experience (native Enter / selection /
//! undo / IME). Content saves on `Change` and re-indexes `[[links]]`.

use std::collections::HashMap;

use gpui::{
    AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, ScrollHandle, Styled, Subscription, Window, div, point, px,
};
use gpui_component::{
    TitleBar,
    input::{InputEvent, InputState},
};

use crate::db::Db;
use crate::models::{Backlink, Page};
use crate::theme;
use crate::ui;

/// How many days to add each time the feed grows.
const FEED_CHUNK: usize = 7;
/// Hard cap on how far back the feed loads (~10 years), a runaway guard.
const FEED_MAX_DAYS: usize = 3650;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Journal,
    Page(i64),
}

/// A journal day's editor + the subscription saving its edits.
pub struct DayEditor {
    pub state: Entity<InputState>,
    _sub: Subscription,
}

/// The currently-open named/journal page in `View::Page`.
pub struct PageEditor {
    pub title: String,
    pub state: Entity<InputState>,
    _sub: Subscription,
    pub backlinks: Vec<Backlink>,
}

pub struct AppView {
    db: Db,
    view: View,
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
    pub journals: Vec<Page>,
    pub pages: Vec<Page>,
    pub new_page_input: Entity<InputState>,

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

        let new_page_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find or create page…"));
        let np_sub = cx.subscribe_in(
            &new_page_input,
            window,
            |this: &mut AppView, state, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    let title = state.read(cx).value().trim().to_string();
                    if !title.is_empty() {
                        this.open_page_title(&title, window, cx);
                        state.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                }
            },
        );

        let mut this = Self {
            db,
            view: View::Journal,
            editing_day: None,
            page_editing: false,
            loaded_days: 0,
            day_editors: HashMap::new(),
            feed_scroll: ScrollHandle::new(),
            page_editor: None,
            journals: Vec::new(),
            pages: Vec::new(),
            new_page_input,
            _subs: vec![np_sub],
            focus_handle: cx.focus_handle(),
        };

        this.loaded_days = 14;
        for i in 0..this.loaded_days {
            this.ensure_day_editor(date_for_offset(i), window, cx);
        }
        this.refresh_sidebar();
        this.edit_day(&date_for_offset(0), window, cx);
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
                }
                InputEvent::Focus => {
                    this.editing_day = Some(key.clone());
                    cx.notify();
                }
                InputEvent::Blur => {
                    if this.editing_day.as_deref() == Some(key.as_str()) {
                        this.editing_day = None;
                    }
                    let value = st.read(cx).value().to_string();
                    this.flush_journal(&key, &value);
                    cx.notify();
                }
                _ => {}
            },
        );
        self.day_editors.insert(date, DayEditor { state, _sub: sub });
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
        self.view = View::Journal;
        self.page_editor = None;
        for i in 0..self.loaded_days {
            self.ensure_day_editor(date_for_offset(i), window, cx);
        }
        self.feed_scroll.set_offset(point(px(0.0), px(0.0)));
        self.refresh_sidebar();
        self.edit_day(&date_for_offset(0), window, cx);
        cx.notify();
    }

    pub fn open_page_id(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.get_page(id) {
            Ok(Some(page)) => self.open_page(page, window, cx),
            Ok(None) => log::warn!("page {id} not found"),
            Err(e) => log::error!("open page {id}: {e}"),
        }
    }

    pub fn open_page_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.get_or_create_page(title) {
            Ok(page) => self.open_page(page, window, cx),
            Err(e) => log::error!("open page '{title}': {e}"),
        }
    }

    fn open_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
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
                }
                InputEvent::Focus => {
                    this.page_editing = true;
                    cx.notify();
                }
                InputEvent::Blur => {
                    this.page_editing = false;
                    let value = st.read(cx).value().to_string();
                    this.persist(pid, &value);
                    this.refresh_sidebar();
                    cx.notify();
                }
                _ => {}
            },
        );
        let backlinks = self.db.backlinks(pid).unwrap_or_default();
        self.page_editor = Some(PageEditor { title: page.title, state, _sub: sub, backlinks });
        self.view = View::Page(pid);
        self.page_editing = false; // open in reading mode; click to edit
        self.refresh_sidebar();
        cx.notify();
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
        self.journals = self.db.list_journals(60).unwrap_or_default();
        self.pages = self.db.list_pages().unwrap_or_default();
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
        matches!(self.view, View::Journal)
    }

    pub fn is_page_active(&self, id: i64) -> bool {
        self.view == View::Page(id)
    }

    pub fn is_editing_day(&self, date: &str) -> bool {
        self.editing_day.as_deref() == Some(date)
    }

    pub fn is_page_editing(&self) -> bool {
        self.page_editing
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::bg_window())
            .text_color(theme::text_primary())
            .child(
                TitleBar::new().child(
                    div()
                        .px_2()
                        .text_size(px(13.0))
                        .text_color(theme::text_secondary())
                        .child("rumin"),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(ui::sidebar::render(self, cx))
                    .child(if self.is_journal_view() {
                        ui::journal::render(self, cx).into_any_element()
                    } else {
                        ui::page_view::render(self, cx).into_any_element()
                    }),
            )
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
