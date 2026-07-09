//! Search results in the main pane, driven by the sidebar search box. A row of
//! type-filter chips (mirroring the `pdf:` / `img:` / `page:` prefixes) sits above
//! the results; each hit is a page, a PDF file, or an image file.

use gpui::{
    ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};

use crate::app::AppView;
use crate::search::{Filter, Hit, Kind, Target};
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let hits = &app.search.hits;
    let rows: Vec<_> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| hit_row(i, hit, app, cx))
        .collect();

    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(theme::bg_content())
        .child(
            div()
                .id("search-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .max_w(px(760.0))
                        .mx_auto()
                        .px(px(48.0))
                        .py(px(28.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(filter_chips(app, cx))
                        .when(hits.is_empty(), |d| {
                            d.child(
                                div()
                                    .pt_2()
                                    .text_color(theme::text_tertiary())
                                    .child("No matches"),
                            )
                        })
                        .children(rows),
                ),
        )
}

/// The type-filter chip row: All · Pages · PDFs · Images, each with a live count,
/// the active one highlighted. Clicking a chip sets the search box's prefix.
fn filter_chips(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let c = &app.search.counts;
    let active = app.search.filter;
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .pb_2()
        .child(chip("All", Filter::All, c.total(), active, cx))
        .child(chip("Pages", Filter::Page, c.page, active, cx))
        .child(chip(
            "▦ Whiteboards",
            Filter::Whiteboard,
            c.whiteboard,
            active,
            cx,
        ))
        .child(chip("PDFs", Filter::Pdf, c.pdf, active, cx))
        .child(chip("Images", Filter::Image, c.image, active, cx))
}

fn chip(
    label: &str,
    filter: Filter,
    count: usize,
    active: Filter,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let is_active = filter == active;
    div()
        .id(chip_id(filter))
        .px_3()
        .py_1()
        .rounded_full()
        .cursor_pointer()
        .text_size(px(13.0))
        .bg(if is_active {
            theme::accent_tint()
        } else {
            theme::glass()
        })
        .text_color(if is_active {
            theme::accent()
        } else {
            theme::text_secondary()
        })
        .hover(|h| h.bg(theme::glass_strong()))
        .child(format!("{label} · {count}"))
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                this.set_search_filter(filter, window, cx);
            }),
        )
}

/// A stable element id per filter chip.
fn chip_id(filter: Filter) -> &'static str {
    match filter {
        Filter::All => "chip-all",
        Filter::Page => "chip-page",
        Filter::Whiteboard => "chip-whiteboard",
        Filter::Pdf => "chip-pdf",
        Filter::Image => "chip-image",
    }
}

fn hit_row(i: usize, hit: &Hit, app: &AppView, cx: &mut Context<AppView>) -> gpui::AnyElement {
    let target = hit.target.clone();
    // Flat marker for boards (matches the chip); the colored file/image emoji
    // are dropped — the subtitle already names the kind ("PDF · in …").
    let icon = match hit.kind {
        Kind::Whiteboard => "▦",
        Kind::Page | Kind::Pdf | Kind::Image => "",
    };
    let row = div()
        .id(("hit", i))
        .px_3()
        .py_2()
        .rounded(px(6.0))
        .bg(theme::glass())
        .cursor_pointer()
        .hover(|h| h.bg(theme::glass_strong()))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .when(!icon.is_empty(), |d| d.child(div().child(icon)))
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text_primary())
                        .child(hit.title.clone()),
                ),
        )
        .when(!hit.subtitle.trim().is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme::text_secondary())
                    .child(hit.subtitle.clone()),
            )
        })
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                this.open_search_hit(target.clone(), window, cx);
            }),
        );
    // Page-like hits carry the shared page menu; file hits are files.
    match (&hit.kind, &hit.target) {
        (Kind::Page | Kind::Whiteboard, Target::Page(id)) => {
            super::with_page_menu(row, *id, hit.title.clone().into(), app.is_favorite(*id), cx)
        }
        _ => row.into_any_element(),
    }
}
