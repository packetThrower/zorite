//! The **All pages** browser (sidebar → list icon; its own tab): every named
//! page, whiteboard, and stored PDF in one filterable index. An A–Z / 0–9 / `#` strip
//! narrows by first character, kind chips narrow by type, and both compose.
//! Journal days are deliberately excluded — the calendar is their browser,
//! and thousands of day rows would swamp the list.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};

use gpui_component::Icon;

use crate::app::AppView;
use crate::dates;
use crate::theme;

/// The kind filter chips.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum KindFilter {
    #[default]
    All,
    Pages,
    Whiteboards,
    Pdfs,
}

/// What a row is — and, for a PDF, where it lives.
#[derive(Clone)]
enum Row {
    Page,
    Board,
    Pdf(std::path::PathBuf),
}

/// The letter bucket a title sorts under: its first alphanumeric-ish char
/// uppercased (digits stay digits), or `#` for anything else.
fn bucket(title: &str) -> char {
    match title.trim().chars().next() {
        Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
        Some(c) if c.is_ascii_digit() => c,
        _ => '#',
    }
}

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let letter = app.all_pages_letter();
    let kind = app.all_pages_kind();

    // One merged, alphabetical index: (title, kind badge, created, updated).
    // DB timestamps are UTC; both kinds surface as local ISO dates.
    let local_date = |ts: &Option<String>| ts.as_deref().and_then(dates::db_timestamp_local_date);
    let mut rows: Vec<(String, Row, Option<String>, Option<String>)> = Vec::new();
    if matches!(kind, KindFilter::All | KindFilter::Pages) {
        rows.extend(app.pages().iter().map(|p| {
            (
                p.title.clone(),
                Row::Page,
                local_date(&p.created_at),
                local_date(&p.updated_at),
            )
        }));
    }
    if matches!(kind, KindFilter::All | KindFilter::Whiteboards) {
        rows.extend(app.whiteboards().iter().map(|w| {
            (
                w.title.clone(),
                Row::Board,
                local_date(&w.created_at),
                local_date(&w.updated_at),
            )
        }));
    }
    if matches!(kind, KindFilter::All | KindFilter::Pdfs) {
        rows.extend(app.all_pages_pdfs().iter().map(|(name, path, c, u)| {
            (name.clone(), Row::Pdf(path.clone()), c.clone(), u.clone())
        }));
    }
    rows.sort_by_key(|(t, ..)| t.to_lowercase());

    // Which buckets exist (pre-letter-filter), so empty chips render dimmed.
    let mut present = std::collections::HashSet::new();
    for (t, ..) in &rows {
        present.insert(bucket(t));
    }
    if let Some(l) = letter {
        rows.retain(|(t, ..)| bucket(t) == l);
    }
    let count = rows.len();

    // The A–Z / 0–9 / # strip. "All" clears.
    let mut strip = div().flex().flex_row().flex_wrap().gap(px(4.0)).child(chip(
        "All",
        letter.is_none(),
        true,
        cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
            this.set_all_pages_letter(None, cx);
        }),
    ));
    for c in ('A'..='Z').chain('0'..='9').chain(['#']) {
        strip = strip.child(chip(
            c,
            letter == Some(c),
            present.contains(&c),
            cx.listener(move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_letter(Some(c), cx);
            }),
        ));
    }

    // Kind chips.
    let kinds = div()
        .flex()
        .flex_row()
        .gap(px(6.0))
        .child(chip(
            "All types",
            kind == KindFilter::All,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::All, cx);
            }),
        ))
        .child(chip(
            "Pages",
            kind == KindFilter::Pages,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::Pages, cx);
            }),
        ))
        .child(chip(
            "Whiteboards",
            kind == KindFilter::Whiteboards,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::Whiteboards, cx);
            }),
        ))
        .child(chip(
            "PDFs",
            kind == KindFilter::Pdfs,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::Pdfs, cx);
            }),
        ));

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(theme::text_tertiary())
        .child(div().flex_1().child("Title"))
        .child(div().w(px(84.0)).flex_shrink_0().child("Created"))
        .child(div().w(px(84.0)).flex_shrink_0().child("Updated"))
        .child(div().w(px(92.0)).flex_shrink_0().child("Type"));

    let mut list = div().flex().flex_col();
    for (i, (title, row, created, updated)) in rows.into_iter().enumerate() {
        let open_title = title.clone();
        let open_row = row.clone();
        let badge = match &row {
            Row::Page => "Page",
            Row::Board => "Whiteboard",
            Row::Pdf(_) => "PDF",
        };
        list = list.child(
            div()
                .id(("all-pages-row", i))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme::hover()))
                .on_click(
                    cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                        // Pages/boards route like a wiki-link (boards open
                        // their canvas); a PDF opens its viewer directly.
                        match &open_row {
                            Row::Pdf(path) => this.open_pdf(path.clone(), window, cx),
                            _ => this.open_page_title(&open_title, window, cx),
                        }
                    }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(14.0))
                        .text_color(theme::text_primary())
                        .child(title),
                )
                .child(date_cell(created))
                .child(date_cell(updated))
                .child(
                    div().w(px(92.0)).flex_shrink_0().flex().flex_row().child(
                        div()
                            .px(px(8.0))
                            .py(px(1.0))
                            .rounded(px(10.0))
                            .bg(theme::glass())
                            .text_size(px(11.0))
                            .text_color(theme::text_secondary())
                            .child(badge),
                    ),
                ),
        );
    }
    if count == 0 {
        list = list.child(
            div()
                .px(px(10.0))
                .py(px(12.0))
                .text_size(px(13.0))
                .text_color(theme::text_tertiary())
                .child("Nothing matches this filter."),
        );
    }

    div()
        .id("all-pages")
        .size_full()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .px(px(28.0))
        .py(px(20.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_size(px(22.0))
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("All pages"),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::text_tertiary())
                                .child(format!("{count} shown")),
                        ),
                )
                .child(
                    div()
                        .id("open-graph")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .text_color(theme::text_secondary())
                        .hover(|s| s.bg(theme::hover()).text_color(theme::text_primary()))
                        .on_click(
                            cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
                                this.open_graph(window, cx);
                            }),
                        )
                        .child(Icon::empty().path("icons/waypoints.svg").size_4())
                        .child("Graph"),
                ),
        )
        .child(strip)
        .child(kinds)
        .child(header)
        .child(
            div()
                .id("all-pages-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .child(list),
        )
}

/// A fixed-width Created/Updated cell; an em dash when the date is unknown.
fn date_cell(date: Option<String>) -> impl IntoElement {
    div()
        .w(px(84.0))
        .flex_shrink_0()
        .text_size(px(12.0))
        .text_color(theme::text_secondary())
        .child(date.unwrap_or_else(|| "—".into()))
}

/// A small filter chip: accent-tinted when active, dimmed when it has nothing
/// to show (letters with no pages stay visible so the strip reads as one bar).
fn chip(
    label: impl Into<String>,
    active: bool,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let mut el = div()
        .id(gpui::SharedString::from(format!("chip-{label}")))
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(6.0))
        .text_size(px(12.0))
        .cursor_pointer();
    el = if active {
        el.bg(theme::accent_tint()).text_color(theme::accent())
    } else if enabled {
        el.text_color(theme::text_secondary())
            .hover(|s| s.bg(theme::hover()))
    } else {
        el.text_color(theme::text_tertiary()).opacity(0.4)
    };
    el.on_click(on_click).child(label)
}
