//! The **All pages** browser (sidebar → list icon; its own tab): every named
//! page, whiteboard, and stored PDF in one filterable index. An A–Z / 0–9 / `#` strip
//! narrows by first character, kind chips narrow by type, and both compose.
//! Journal days stay out of "All types" (thousands of day rows would swamp
//! it); the Journal chip lists them newest first, with year and month chips
//! in place of the A–Z strip.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};

use gpui_component::Icon;

use crate::app::AppView;
use crate::dates;
use crate::theme;
use rust_i18n::t;

/// The kind filter chips.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum KindFilter {
    #[default]
    All,
    Pages,
    Whiteboards,
    Pdfs,
    Journal,
}

/// What a row is — and, for a PDF, where it lives.
#[derive(Clone)]
enum Row {
    Page(i64),
    Board(i64),
    Pdf(std::path::PathBuf),
    /// A journal day: its ISO date (the title) and a one-line preview.
    Day(i64, String),
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
                Row::Page(p.id),
                local_date(&p.created_at),
                local_date(&p.updated_at),
            )
        }));
    }
    if matches!(kind, KindFilter::All | KindFilter::Whiteboards) {
        rows.extend(app.whiteboards().iter().map(|w| {
            (
                w.title.clone(),
                Row::Board(w.id),
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

    let strip = if kind == KindFilter::Journal {
        journal_rows(app, &mut rows, cx)
    } else {
        letter_strip(letter, &mut rows, cx)
    };
    let count = rows.len();

    // Kind chips.
    let kinds = div()
        .flex()
        .flex_row()
        .gap(px(6.0))
        .child(chip(
            t!("all_pages.filter_all").into_owned(),
            kind == KindFilter::All,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::All, cx);
            }),
        ))
        .child(chip(
            t!("all_pages.filter_pages").into_owned(),
            kind == KindFilter::Pages,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::Pages, cx);
            }),
        ))
        .child(chip(
            t!("all_pages.filter_whiteboards").into_owned(),
            kind == KindFilter::Whiteboards,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::Whiteboards, cx);
            }),
        ))
        .child(chip(
            t!("all_pages.filter_pdfs").into_owned(),
            kind == KindFilter::Pdfs,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::Pdfs, cx);
            }),
        ))
        .child(chip(
            t!("all_pages.filter_journal").into_owned(),
            kind == KindFilter::Journal,
            true,
            cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_kind(KindFilter::Journal, cx);
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
        .child(div().flex_1().child(t!("all_pages.col_title")))
        .child(
            div()
                .w(px(84.0))
                .flex_shrink_0()
                .child(t!("all_pages.col_created")),
        )
        .child(
            div()
                .w(px(84.0))
                .flex_shrink_0()
                .child(t!("all_pages.col_updated")),
        )
        .child(
            div()
                .w(px(92.0))
                .flex_shrink_0()
                .child(t!("all_pages.col_type")),
        );

    let mut list = div().flex().flex_col();
    for (i, (title, row, created, updated)) in rows.into_iter().enumerate() {
        let open_title = title.clone();
        let open_row = row.clone();
        let badge = match &row {
            Row::Page(_) => t!("all_pages.type_page").into_owned(),
            Row::Board(_) => t!("all_pages.type_whiteboard").into_owned(),
            Row::Pdf(_) => t!("all_pages.type_pdf").into_owned(),
            Row::Day(..) => t!("all_pages.type_day").into_owned(),
        };
        let menu_target = match &row {
            Row::Page(id) | Row::Board(id) | Row::Day(id, _) => Some(*id),
            Row::Pdf(_) => None,
        };
        let preview = match &row {
            Row::Day(_, p) => Some(p.clone()),
            _ => None,
        };
        let menu_title: SharedString = title.clone().into();
        let row_el = div()
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
                        Row::Day(..) => this.open_journal_day(&open_title, window, cx),
                        _ => this.open_page_title(&open_title, window, cx),
                    }
                }),
            )
            .child(match preview {
                // A day: its date, then the first line of what was written.
                Some(p) => div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .gap(px(10.0))
                    .text_size(px(14.0))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::text_primary())
                            .child(title),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::text_tertiary())
                            .child(p),
                    ),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.0))
                    .text_color(theme::text_primary())
                    .child(title),
            })
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
            );
        list = list.child(match menu_target {
            // Pages and boards carry the shared page menu; PDFs are files.
            Some(id) => super::with_page_menu(row_el, id, menu_title, app.is_favorite(id), cx),
            None => row_el.into_any_element(),
        });
    }
    if count == 0 {
        list = list.child(
            div()
                .px(px(10.0))
                .py(px(12.0))
                .text_size(px(13.0))
                .text_color(theme::text_tertiary())
                .child(t!("all_pages.empty")),
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
                                .child(t!("all_pages.title")),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::text_tertiary())
                                .child(t!("all_pages.shown", count = count)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .id("open-properties")
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
                                .on_click(cx.listener(
                                    |this: &mut AppView, _: &ClickEvent, window, cx| {
                                        this.open_properties(window, cx);
                                    },
                                ))
                                .child(Icon::empty().path("icons/list.svg").size_4())
                                .child(t!("all_pages.properties")),
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
                                .on_click(cx.listener(
                                    |this: &mut AppView, _: &ClickEvent, window, cx| {
                                        this.open_graph(window, cx);
                                    },
                                ))
                                .child(Icon::empty().path("icons/waypoints.svg").size_4())
                                .child(t!("all_pages.graph")),
                        ),
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
                .track_scroll(&app.all_pages_scroll)
                .flex()
                .flex_col()
                .child(list),
        )
}

/// The A–Z / 0–9 / `#` strip ("All" clears) — and the letter filter it applies.
fn letter_strip(
    letter: Option<char>,
    rows: &mut Vec<(String, Row, Option<String>, Option<String>)>,
    cx: &mut Context<AppView>,
) -> gpui::Div {
    // Which buckets exist (pre-letter-filter), so empty chips render dimmed.
    let present: std::collections::HashSet<char> = rows.iter().map(|(t, ..)| bucket(t)).collect();
    if let Some(l) = letter {
        rows.retain(|(t, ..)| bucket(t) == l);
    }
    let mut strip = div().flex().flex_row().flex_wrap().gap(px(4.0)).child(chip(
        t!("all_pages.letter_all").into_owned(),
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
    strip
}

/// The Journal filter: days with content, newest first, narrowed by a year
/// strip and (once a year is picked) a month strip. Fills `rows` and returns
/// the strips.
fn journal_rows(
    app: &AppView,
    rows: &mut Vec<(String, Row, Option<String>, Option<String>)>,
    cx: &mut Context<AppView>,
) -> gpui::Div {
    let (year, month) = app.all_pages_period();
    let local_date = |ts: &Option<String>| ts.as_deref().and_then(dates::db_timestamp_local_date);
    // `journal_date` is ISO `YYYY-MM-DD`, so year and month are fixed slices.
    let ym = |d: &str| -> (i32, u8) {
        (
            d.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0),
            d.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(0),
        )
    };
    let days = app.all_pages_journal();
    let mut years: Vec<i32> = days
        .iter()
        .filter_map(|d| d.journal_date.as_deref())
        .map(|d| ym(d).0)
        .collect();
    years.dedup(); // newest first already, so equal years are adjacent
    let months: std::collections::HashSet<u8> = days
        .iter()
        .filter_map(|d| d.journal_date.as_deref())
        .map(ym)
        .filter(|(y, _)| Some(*y) == year)
        .map(|(_, m)| m)
        .collect();

    rows.extend(days.iter().filter_map(|d| {
        let date = d.journal_date.clone()?;
        let (y, m) = ym(&date);
        if year.is_some_and(|yr| yr != y) || month.is_some_and(|mo| mo != m) {
            return None;
        }
        let preview = d.content.lines().find_map(plain_line).unwrap_or_default();
        Some((
            date.clone(),
            Row::Day(d.id, preview),
            local_date(&d.created_at),
            local_date(&d.updated_at),
        ))
    }));

    let mut strip = div().flex().flex_col().gap(px(6.0));
    let mut year_strip = div().flex().flex_row().flex_wrap().gap(px(4.0)).child(chip(
        t!("all_pages.year_all").into_owned(),
        year.is_none(),
        true,
        cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
            this.set_all_pages_year(None, cx);
        }),
    ));
    for y in years {
        year_strip = year_strip.child(chip(
            y.to_string(),
            year == Some(y),
            true,
            cx.listener(move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                this.set_all_pages_year(Some(y), cx);
            }),
        ));
    }
    strip = strip.child(year_strip);
    if year.is_some() {
        let mut month_strip = div().flex().flex_row().flex_wrap().gap(px(4.0));
        for m in 1..=12u8 {
            let Ok(name) = time::Month::try_from(m) else {
                continue;
            };
            month_strip = month_strip.child(chip(
                dates::month_name(name),
                month == Some(m),
                months.contains(&m),
                cx.listener(move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                    this.set_all_pages_month(Some(m), cx);
                }),
            ));
        }
        strip = strip.child(month_strip);
    }
    strip
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
        el.text_color(theme::text_tertiary())
    };
    el.on_click(on_click).child(label)
}

/// One line of Markdown as plain text for a preview: markers, link targets
/// and images dropped, `[[wiki links]]` shown by name. `None` for a line with
/// no text of its own (blank, an image, a `key:: value` property).
pub(crate) fn plain_line(line: &str) -> Option<String> {
    use markdown::mdast::Node;
    fn collect(n: &Node, out: &mut String) {
        match n {
            Node::Text(t) => out.push_str(&t.value),
            Node::InlineCode(c) => out.push_str(&c.value),
            Node::Image(_) => {}
            _ => n
                .children()
                .into_iter()
                .flatten()
                .for_each(|c| collect(c, out)),
        }
    }
    let line = line.trim();
    let is_property = line
        .split_once(":: ")
        .is_some_and(|(k, _)| !k.is_empty() && !k.contains(' '));
    if line.is_empty() || is_property {
        return None;
    }
    let mut text = String::new();
    match markdown::to_mdast(line, &markdown::ParseOptions::gfm()) {
        Ok(ast) => collect(&ast, &mut text),
        Err(_) => text.push_str(line),
    }
    // Wiki links and highlights aren't CommonMark, so they come through as
    // text: `[[Page|alias]]` reads as its alias, `[[Page]]` as the page.
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(i) = rest.find("[[") {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find("]]") else { break };
        let inner = &rest[i + 2..i + j];
        out.push_str(inner.rsplit('|').next().unwrap_or(inner));
        rest = &rest[i + j + 2..];
    }
    out.push_str(rest);
    let out = out.replace("==", "").trim().to_string();
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::plain_line;

    #[test]
    fn previews_drop_markdown() {
        let p = |s: &str| plain_line(s);
        assert_eq!(p("# Standup **notes**").as_deref(), Some("Standup notes"));
        assert_eq!(p("- [ ] call [[Dentist]]").as_deref(), Some("call Dentist"));
        assert_eq!(
            p("see [[Home network|the LAN]] and [docs](https://x.y)").as_deref(),
            Some("see the LAN and docs")
        );
        assert_eq!(p("==big== `code`").as_deref(), Some("big code"));
        assert_eq!(p("![](photo.png)"), None);
        assert_eq!(p("tags:: work"), None);
        assert_eq!(p("   "), None);
    }
}
