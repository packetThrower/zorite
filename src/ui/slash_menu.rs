//! The slash-command popup list, rendered as an anchored overlay by `AppView`.
//! Keyboard-driven (arrows + Enter) and mouse-driven (hover highlights a row,
//! click accepts it).

use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px, relative,
};

use crate::app::AppView;
use crate::slash::{ItemKind, PaletteItem, Slash, Trigger};
use crate::theme;

/// Fixed row heights (px) — rows are exactly this tall (`.h(item_h(..))`), so
/// the keyboard scroll-into-view (`AppView::scroll_slash_into_view`) and the
/// scrollbar thumb below always agree with the painted list. (They drifted
/// ~4px/row when rows sized themselves from text + padding.) The `/` palette
/// uses tall Notion-style rows (icon box + title + description,
/// Cditor-inspired); the other triggers (`[[`, `#`, `\`, `{{`) are plain
/// autocompletes and keep the compact row.
const SLASH_ITEM_H: f32 = 44.0;
const COMPACT_ITEM_H: f32 = 28.0;
const PAD: f32 = 4.0;

/// Row height for `trigger`'s menu.
pub fn item_h(trigger: Trigger) -> f32 {
    if trigger == Trigger::Slash {
        SLASH_ITEM_H
    } else {
        COMPACT_ITEM_H
    }
}

/// Height cap = an exact number of rows, so the bottom row is never
/// half-clipped and arrow-key scrolling advances by whole rows.
pub fn max_h(trigger: Trigger) -> f32 {
    let rows = if trigger == Trigger::Slash { 7.0 } else { 10.0 };
    2.0 * PAD + rows * item_h(trigger)
}

/// Scrollable viewport height (the cap minus top/bottom padding).
pub fn view_h(trigger: Trigger) -> f32 {
    max_h(trigger) - 2.0 * PAD
}

/// The category flyout panel's width.
const FLYOUT_WIDTH: f32 = 240.0;

/// The rendered height of a panel showing `n_items` (capped at the visible-row
/// cap, plus padding). Used by the caller to place the main panel and to size
/// the flyout for its overflow check.
pub fn panel_height(trigger: Trigger, n_items: usize) -> f32 {
    let rows = if trigger == Trigger::Slash { 7 } else { 10 };
    let shown = n_items.clamp(1, rows) as f32;
    2.0 * PAD + shown * item_h(trigger)
}

/// Icon-box glyph + one-line description for the `/` palette's known entries.
/// Dynamic rows (dates, templates, pages) match on their label's stable
/// prefix; anything unknown falls back to a bare "+" box, no description.
fn meta(label: &str) -> (&'static str, Option<&'static str>) {
    match label {
        "Heading 1" => ("H1", Some("Big section heading.")),
        "Heading 2" => ("H2", Some("Medium section heading.")),
        "Heading 3" => ("H3", Some("Small section heading.")),
        "Bullet list" => ("•", Some("A simple bulleted list.")),
        "Numbered list" => ("1.", Some("A list with numbering.")),
        "To-do" => ("☐", Some("Track a task with a checkbox.")),
        "Quote" => ("❝", Some("Capture a quote.")),
        "Note alert" => ("!", Some("Callout — something worth noting.")),
        "Tip alert" => ("!", Some("Callout — a helpful tip.")),
        "Important alert" => ("!", Some("Callout — key information.")),
        "Warning alert" => ("!", Some("Callout — needs attention.")),
        "Caution alert" => ("!", Some("Callout — risky consequences.")),
        "Code block" => ("</>", Some("Capture a code snippet.")),
        "Mermaid diagram" => ("◇", Some("Draw a diagram from text.")),
        "Math" => ("Σ", Some("Typeset a display formula.")),
        "Table" => ("⊞", Some("Pick a rows × columns size.")),
        "Divider" => ("—", Some("Visually divide the page.")),
        "Bold" => ("B", Some("Bold text.")),
        "Italic" => ("I", Some("Italic text.")),
        "Strikethrough" => ("S", Some("Struck-through text.")),
        "Inline code" => ("<>", Some("Code within a sentence.")),
        "Inline math" => ("$", Some("A formula within a sentence.")),
        "Highlight" => ("==", Some("Highlighted text.")),
        "Underline" => ("U", Some("Underlined text.")),
        "Link" => ("↗", Some("A web link.")),
        "Property" => ("::", Some("Add a key:: value property.")),
        "Markdown" => ("≡", Some("All markdown blocks.")),
        "Templates" => ("⧉", Some("Insert one of your templates.")),
        _ if label.starts_with("Date (") => ("@", Some("Insert today's date.")),
        _ if label.starts_with("Time (") => ("@", Some("Insert the current time.")),
        _ => ("+", None),
    }
}

pub fn render(
    slash: &Slash,
    scroll: &gpui::ScrollHandle,
    flyout: (&[PaletteItem], &gpui::ScrollHandle),
    // `flyout_top_offset`: vertical offset for the flyout from the main panel's
    // top (0 = aligned; negative shifts it up so it clears the window bottom).
    // The caller sizes this since only it knows the menu's final window position.
    flyout_top_offset: f32,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let items = &slash.items;

    // The `\` LaTeX menu has short entries — keep it narrow like the structural editor's
    // dropdown; other completions (pages, templates) want the wider column.
    let min_w = if slash.trigger == Trigger::Math {
        120.0
    } else {
        220.0
    };

    // Inner scroll viewport: caps the height + scrolls the overflow. The chrome (bg/border)
    // lives on the outer `relative` box below so the scrollbar thumb can position against it.
    let rich = slash.trigger == Trigger::Slash;
    let row_h = item_h(slash.trigger);
    let mut col = div()
        .id("completion-menu")
        .max_h(px(max_h(slash.trigger)))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .py(px(PAD));

    if items.is_empty() {
        col = col.child(
            div()
                .px_3()
                .py_1()
                .text_size(px(13.0))
                .text_color(theme::text_tertiary())
                .child("No commands"),
        );
    } else {
        for (i, item) in items.iter().enumerate() {
            let selected = i == slash.selected;
            let is_category = matches!(item.kind, ItemKind::Category(_));
            let row = div()
                .px_3()
                .h(px(row_h))
                .flex_none()
                .text_size(px(13.0))
                .flex()
                .flex_row()
                .items_center()
                .gap_4()
                .cursor_pointer()
                .when(selected, |d| {
                    d.bg(theme::accent_tint()).text_color(theme::text_primary())
                })
                .when(!selected, |d| d.text_color(theme::text_secondary()))
                // Hover moves the keyboard selection to this row, so the one
                // highlight is what both a click and Enter accept.
                .on_mouse_move(cx.listener(move |this, _, _window, cx| {
                    this.slash_hover(i, cx);
                }))
                // Mouse-DOWN (not click) + stop_propagation: accept before the press
                // can blur the editor, so the insertion lands and focus stays put.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.click_slash(i, window, cx);
                    }),
                );
            col = col.child(if rich {
                // Notion-style row (Cditor-inspired): a boxed glyph, the title,
                // and a muted one-line description under it.
                let (icon, desc) = meta(&item.label);
                row.child(
                    div()
                        .flex_none()
                        .w(px(30.0))
                        .h(px(30.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(theme::border_subtle())
                        .bg(theme::bg_content())
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.0))
                        .text_color(theme::text_secondary())
                        .child(icon),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().child(item.label.clone()))
                        .children(desc.map(|d| {
                            div()
                                .text_size(px(11.0))
                                .text_color(theme::text_tertiary())
                                .child(d)
                        })),
                )
                .when(is_category, |d| {
                    d.child(div().text_color(theme::text_tertiary()).child("›"))
                })
            } else {
                row.justify_between()
                    .child(item.label.clone())
                    .when(is_category, |d| {
                        d.child(div().text_color(theme::text_tertiary()).child("›"))
                    })
            });
        }
    }

    // Scrollbar thumb — only when the rows overflow the cap; sized from the content height
    // and positioned from the live scroll offset (mirrors the gpui-editor table/suggestion
    // menus). Wheel + keyboard scroll both re-render, so the offset read here stays fresh.
    let vh = view_h(slash.trigger);
    let rows_h = items.len().max(1) as f32 * row_h;
    let thumb = (rows_h > vh).then(|| {
        let scrolled = (-f32::from(scroll.offset().y)).clamp(0.0, rows_h - vh);
        let thumb_h = (vh * vh / rows_h).max(24.0);
        let thumb_top = PAD + scrolled / (rows_h - vh) * (vh - thumb_h);
        let mut thumb_c = theme::text_tertiary();
        thumb_c.a = 0.5;
        div()
            .absolute()
            .top(px(thumb_top))
            .right(px(2.0))
            .w(px(6.0))
            .h(px(thumb_h))
            .rounded(px(3.0))
            .bg(thumb_c)
    });

    // Outer chrome: `relative` so the absolute thumb anchors to it, `occlude` so the wheel
    // scrolls the menu rather than bleeding through to the page beneath.
    let main_panel = div()
        .relative()
        .occlude()
        // Swallow clicks on panel dead-space so they don't bubble to the
        // backdrop and close the menu (row clicks already stop_propagation).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .min_w(px(min_w))
        .bg(theme::bg_sidebar())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(8.0))
        .shadow_md()
        .overflow_hidden()
        .child(col)
        .children(thumb);

    // A selected category's submenu flies out beside the main panel
    // (Cditor-style) instead of replacing the list. The caller passes its rows
    // (empty = no category selected → no flyout).
    let (fly_items, fly_scroll) = flyout;
    let fly_panel = (!fly_items.is_empty()).then(|| {
        let mut fcol = div()
            .id("completion-flyout")
            .max_h(px(max_h(Trigger::Slash)))
            .overflow_y_scroll()
            .track_scroll(fly_scroll)
            .flex()
            .flex_col()
            .py(px(PAD));
        for (i, item) in fly_items.iter().enumerate() {
            let selected = slash.flyout == Some(i);
            let (icon, desc) = meta(&item.label);
            fcol = fcol.child(
                div()
                    .px_3()
                    .h(px(SLASH_ITEM_H))
                    .flex_none()
                    .text_size(px(13.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_4()
                    .cursor_pointer()
                    .when(selected, |d| {
                        d.bg(theme::accent_tint()).text_color(theme::text_primary())
                    })
                    .when(!selected, |d| d.text_color(theme::text_secondary()))
                    .on_mouse_move(cx.listener(move |this, _, _window, cx| {
                        this.slash_flyout_hover(i, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.click_slash_flyout(i, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(30.0))
                            .h(px(30.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(theme::border_subtle())
                            .bg(theme::bg_content())
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(theme::text_secondary())
                            .child(icon),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().child(item.label.clone()))
                            .children(desc.map(|d| {
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::text_tertiary())
                                    .child(d)
                            })),
                    ),
            );
        }
        // Its own thumb, against its own scroll offset.
        let fly_view_h = view_h(Trigger::Slash);
        let fly_rows_h = fly_items.len() as f32 * SLASH_ITEM_H;
        let fthumb = (fly_rows_h > fly_view_h).then(|| {
            let scrolled = (-f32::from(fly_scroll.offset().y)).clamp(0.0, fly_rows_h - fly_view_h);
            let thumb_h = (fly_view_h * fly_view_h / fly_rows_h).max(24.0);
            let thumb_top = PAD + scrolled / (fly_rows_h - fly_view_h) * (fly_view_h - thumb_h);
            let mut thumb_c = theme::text_tertiary();
            thumb_c.a = 0.5;
            div()
                .absolute()
                .top(px(thumb_top))
                .right(px(2.0))
                .w(px(6.0))
                .h(px(thumb_h))
                .rounded(px(3.0))
                .bg(thumb_c)
        });
        div()
            // Absolutely positioned beside the main panel — so it floats out
            // and does NOT enlarge the anchored element's bounding box (which
            // would make gpui snap the whole menu up near the window bottom).
            // Anchored to the main panel's right edge (`left: 100%`) + a gap.
            .absolute()
            .top(px(flyout_top_offset))
            .left(relative(1.0))
            .ml(px(4.0))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(FLYOUT_WIDTH))
            .bg(theme::bg_sidebar())
            .border_1()
            .border_color(theme::border_subtle())
            .rounded(px(8.0))
            .shadow_md()
            .overflow_hidden()
            .child(fcol)
            .children(fthumb)
    });

    div()
        .id("slash-menu")
        // `relative` so the flyout positions against this box; the main panel
        // is the only in-flow child, so this box's size = the main panel's.
        .relative()
        .child(main_panel)
        .children(fly_panel)
        .into_any_element()
}
