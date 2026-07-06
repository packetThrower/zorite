//! The Properties page (All pages → "Properties"): every `key:: value`
//! property in the vault as a managed index — expand a key to see its values
//! and the pages carrying them (click-through), override a key's icon from a
//! curated picker (persisted in the `property_icons` setting), pre-map an icon
//! for a key you haven't used yet, and rename a key across every page.

use gpui::{
    AppContext, ClickEvent, Entity, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px, svg,
};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::app::AppView;
use crate::db::PropKeyInfo;
use crate::theme;

pub struct PropsPageState {
    /// The vault-wide index (key → values → pages), rebuilt on activation.
    pub index: Vec<PropKeyInfo>,
    /// The key expanded to show its values + pages.
    expanded: Option<String>,
    /// The key whose icon picker is open; `Some(String::new())` = the add-row's.
    icon_menu: Option<String>,
    /// The key being renamed (the input holds the new name).
    rename: Option<String>,
    rename_input: Entity<InputState>,
    /// The "add mapping" key field (assign an icon before first use).
    pub new_key_input: Entity<InputState>,
    /// Focused while an icon picker is open, so Esc dismisses it (the lightbox
    /// recipe — no global binding to clash with the editor's Escape).
    picker_focus: gpui::FocusHandle,
    _rename_sub: gpui::Subscription,
}

impl PropsPageState {
    pub fn new(
        index: Vec<PropKeyInfo>,
        window: &mut Window,
        cx: &mut gpui::Context<AppView>,
    ) -> Self {
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("new name"));
        let new_key_input = cx.new(|cx| InputState::new(window, cx).placeholder("key…"));
        let rename_sub = cx.subscribe_in(
            &rename_input,
            window,
            |this: &mut AppView, _st, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.commit_prop_rename(window, cx);
                }
            },
        );
        Self {
            index,
            expanded: None,
            icon_menu: None,
            rename: None,
            rename_input,
            new_key_input,
            picker_focus: cx.focus_handle(),
            _rename_sub: rename_sub,
        }
    }

    /// The key under rename (if any) and its input.
    pub fn rename_state(&self) -> Option<(String, Entity<InputState>)> {
        self.rename.clone().map(|k| (k, self.rename_input.clone()))
    }

    pub fn clear_rename(&mut self) {
        self.rename = None;
    }

    /// Close any open icon picker (after a pick, or a click elsewhere).
    pub fn close_menus(&mut self) {
        self.icon_menu = None;
    }
}

/// Whether `key` is a valid property key under the shared grammar (what a
/// rename / add-mapping accepts).
pub fn valid_key(key: &str) -> bool {
    gpui_markdown::syntax::property(&format!("{key}:: x")).is_some_and(|(k, _)| k == key)
}

pub fn render(app: &AppView, cx: &mut gpui::Context<AppView>) -> impl IntoElement {
    let Some(state) = &app.props_page else {
        return div().id("properties-page");
    };
    let overrides = theme::property_icon_overrides();
    let count = state.index.len();

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .text_size(px(22.0))
                .font_weight(gpui::FontWeight::BOLD)
                .child("Properties"),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme::text_tertiary())
                .child(format!("{count} keys in use")),
        );

    // Column header, matching the All pages table style.
    let columns = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(10.0))
        .text_size(px(11.0))
        .text_color(theme::text_tertiary())
        .child(div().w(px(20.0)).flex_shrink_0())
        .child(div().w(px(160.0)).flex_shrink_0().child("Key"))
        .child(div().w(px(70.0)).flex_shrink_0().child("Pages"))
        .child(div().flex_1().child("Values"))
        .child(div().w(px(120.0)).flex_shrink_0());

    let mut list = div().flex().flex_col();
    for (i, info) in state.index.iter().enumerate() {
        list = list.child(key_row(app, state, &overrides, i, info, cx));
        if state.expanded.as_deref() == Some(info.key.as_str()) {
            list = list.child(value_list(info, cx));
        }
    }

    // Add a mapping for a key that isn't used yet: type the key, pick its icon.
    let add_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(6.0))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme::text_tertiary())
                .child("Map a key before first use:"),
        )
        .child(div().w(px(160.0)).child(Input::new(&state.new_key_input)))
        .child(icon_button(
            "props-add-icon",
            String::new(),
            state.icon_menu.as_deref() == Some(""),
            true,
            state.picker_focus.clone(),
            cx,
        ));

    div()
        .id("properties-page")
        .size_full()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .px(px(28.0))
        .py(px(20.0))
        .child(header)
        .child(columns)
        .child(
            div()
                .id("properties-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .child(list),
        )
        // Pinned below the scroll area so it's discoverable however long the
        // key list grows.
        .child(
            div()
                .flex_shrink_0()
                .border_t_1()
                .border_color(theme::divider())
                .pt(px(8.0))
                .child(add_row),
        )
}

/// One key's row: icon, key (or its rename input), page count, values preview,
/// and the Rename / icon actions. Clicking the row toggles the drill-down.
fn key_row(
    _app: &AppView,
    state: &PropsPageState,
    overrides: &std::collections::HashMap<String, String>,
    i: usize,
    info: &PropKeyInfo,
    cx: &mut gpui::Context<AppView>,
) -> gpui::AnyElement {
    let key = info.key.clone();
    let expanded = state.expanded.as_deref() == Some(info.key.as_str());
    let renaming = state.rename.as_deref() == Some(info.key.as_str());
    let overridden = overrides.contains_key(&info.key.to_ascii_lowercase());
    let preview: String = info
        .values
        .iter()
        .take(4)
        .map(|v| v.value.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let more = info.values.len().saturating_sub(4);
    let preview = if more > 0 {
        format!("{preview}, +{more} more")
    } else {
        preview
    };

    // Key cell: the name (or the rename input + OK/cancel while renaming).
    let key_cell = if renaming {
        div()
            .w(px(220.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(div().flex_1().child(Input::new(&state.rename_input)))
            .child(action_chip(
                ("props-rename-ok", i),
                "OK",
                cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
                    this.commit_prop_rename(window, cx);
                }),
            ))
            .child(action_chip(
                ("props-rename-cancel", i),
                "Cancel",
                cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                    if let Some(s) = &mut this.props_page {
                        s.clear_rename();
                    }
                    cx.notify();
                }),
            ))
            .into_any_element()
    } else {
        div()
            .w(px(160.0))
            .flex_shrink_0()
            .truncate()
            .text_size(px(14.0))
            .text_color(theme::text_primary())
            .child(info.key.clone())
            .into_any_element()
    };

    let toggle_key = key.clone();
    let rename_key = key.clone();
    let mut row = div().id(("props-row", i)).flex();
    if expanded {
        row = row.bg(theme::hover());
    }
    row = row
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover()))
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                if let Some(s) = &mut this.props_page {
                    s.expanded = (s.expanded.as_deref() != Some(toggle_key.as_str()))
                        .then(|| toggle_key.clone());
                }
                cx.notify();
            }),
        )
        .child(
            div()
                .w(px(20.0))
                .flex_shrink_0()
                .children(theme::property_icon(&info.key).map(|p| {
                    svg()
                        .path(p)
                        .w(px(16.0))
                        .h(px(16.0))
                        .text_color(theme::text_secondary())
                })),
        )
        .child(key_cell)
        .child(
            div()
                .w(px(70.0))
                .flex_shrink_0()
                .text_size(px(12.0))
                .text_color(theme::text_tertiary())
                .child(format!("{}", info.page_count)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.0))
                .text_color(theme::text_secondary())
                .child(preview),
        );
    if !renaming {
        row = row.child(
            div()
                .w(px(120.0))
                .flex_shrink_0()
                .flex()
                .flex_row()
                .items_center()
                .justify_end()
                .gap(px(4.0))
                .child(action_chip(
                    ("props-rename", i),
                    "Rename",
                    cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                        if let Some(s) = &mut this.props_page {
                            s.rename = Some(rename_key.clone());
                            s.icon_menu = None;
                            let input = s.rename_input.clone();
                            let key = rename_key.clone();
                            input.update(cx, |st, cx| {
                                st.set_value(key, window, cx);
                                st.focus(window, cx);
                            });
                        }
                        cx.notify();
                    }),
                ))
                .child(icon_button(
                    "props-icon",
                    key,
                    state.icon_menu.as_deref() == Some(info.key.as_str()),
                    false,
                    state.picker_focus.clone(),
                    cx,
                ))
                .children(overridden.then(|| {
                    div()
                        .text_size(px(10.0))
                        .text_color(theme::text_tertiary())
                        .child("•")
                })),
        );
    }
    row.into_any_element()
}

/// The expanded drill-down under a key: each value with its pages, clickable.
fn value_list(info: &PropKeyInfo, cx: &mut gpui::Context<AppView>) -> gpui::AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .pl(px(42.0))
        .pb(px(6.0));
    for (vi, v) in info.values.iter().enumerate() {
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_wrap()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::text_primary())
                    .child(if v.value.is_empty() {
                        "(empty)".to_string()
                    } else {
                        v.value.clone()
                    }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme::text_tertiary())
                    .child("—"),
            );
        for (pi, (id, title)) in v.pages.iter().enumerate() {
            let id = *id;
            row = row.child(
                div()
                    .id(("props-page-link", vi * 1000 + pi))
                    .text_size(px(12.0))
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child(title.clone())
                    .on_click(cx.listener(
                        move |this: &mut AppView, _: &ClickEvent, window, cx| {
                            this.open_page_id(id, window, cx);
                        },
                    )),
            );
        }
        col = col.child(row);
    }
    col.into_any_element()
}

/// The icon-picker button; while open, the curated grid drops below it. `key`
/// is the target property key ("" = the add-mapping row, which reads its key
/// from the input on pick).
fn icon_button(
    id: &'static str,
    key: String,
    open: bool,
    drop_up: bool,
    picker_focus: gpui::FocusHandle,
    cx: &mut gpui::Context<AppView>,
) -> gpui::AnyElement {
    let toggle_key = key.clone();
    let toggle_focus = picker_focus.clone();
    let mut root = div().relative().child(
        div()
            .id(id)
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(6.0))
            .text_size(px(11.0))
            .text_color(theme::text_secondary())
            .hover(|s| s.bg(theme::hover()).text_color(theme::text_primary()))
            .cursor_pointer()
            .child("Icon ▾")
            .on_click(
                cx.listener(move |this: &mut AppView, ev: &ClickEvent, window, cx| {
                    // The row's own click toggles expansion — keep it out.
                    let _ = ev;
                    cx.stop_propagation();
                    if let Some(s) = &mut this.props_page {
                        let opening = s.icon_menu.as_deref() != Some(toggle_key.as_str());
                        s.icon_menu = opening.then(|| toggle_key.clone());
                        // Focus the picker so Esc dismisses it.
                        if opening {
                            window.focus(&toggle_focus, cx);
                        }
                    }
                    cx.notify();
                }),
            ),
    );
    if open {
        // The pinned add-row sits at the window bottom, so its picker opens
        // upward; row pickers drop down and, low in the list, `anchored` snaps
        // the panel back inside the window so it's never clipped.
        let wrapper = if drop_up {
            div().absolute().bottom_full().right_0().mb(px(2.0))
        } else {
            div().absolute().top_full().right_0().mt(px(2.0))
        };
        let panel = div()
            .w(px(232.0))
            .occlude()
            .track_focus(&picker_focus)
            .on_key_down(
                cx.listener(|this: &mut AppView, ev: &gpui::KeyDownEvent, window, cx| {
                    if ev.keystroke.key == "escape" {
                        if let Some(s) = &mut this.props_page {
                            s.icon_menu = None;
                        }
                        window.focus(&this.focus_handle, cx);
                        cx.notify();
                    }
                }),
            )
            .p(px(8.0))
            .bg(theme::elevated())
            .border_1()
            .border_color(theme::divider())
            .rounded(px(6.0))
            .on_mouse_down_out(cx.listener(
                |this: &mut AppView, _: &gpui::MouseDownEvent, _w, cx| {
                    if let Some(s) = &mut this.props_page {
                        s.icon_menu = None;
                    }
                    cx.notify();
                },
            ))
            .child(
                div()
                    .id("props-icon-default")
                    .mb(px(6.0))
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .text_size(px(11.0))
                    .text_color(theme::text_secondary())
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::hover()))
                    .child("Default (built-in)")
                    .on_click({
                        let key = key.clone();
                        cx.listener(move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                            cx.stop_propagation();
                            this.set_property_icon(&key, None, cx);
                        })
                    }),
            )
            .child({
                let mut grid = div().flex().flex_row().flex_wrap().gap(px(4.0));
                for name in theme::PROPERTY_ICON_CHOICES {
                    let key = key.clone();
                    grid = grid.child(
                        div()
                            .id(*name)
                            .p(px(6.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::hover()))
                            .child(
                                svg()
                                    .path(format!("icons/{name}.svg"))
                                    .w(px(16.0))
                                    .h(px(16.0))
                                    .text_color(theme::text_primary()),
                            )
                            .on_click(cx.listener(
                                move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                                    cx.stop_propagation();
                                    this.set_property_icon(&key, Some(name), cx);
                                },
                            )),
                    );
                }
                grid
            });
        root = root.child(
            wrapper.child(gpui::deferred(
                gpui::anchored()
                    .snap_to_window_with_margin(px(8.0))
                    .child(panel),
            )),
        );
    }
    root.into_any_element()
}

/// A small text action button (Rename / OK / Cancel).
fn action_chip(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    div()
        .id(id.into())
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(6.0))
        .text_size(px(11.0))
        .text_color(theme::text_secondary())
        .hover(|s| s.bg(theme::hover()).text_color(theme::text_primary()))
        .cursor_pointer()
        .child(label)
        .on_click(move |ev, window, cx| {
            cx.stop_propagation();
            on_click(ev, window, cx);
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::valid_key;

    #[test]
    fn key_validation_follows_the_shared_grammar() {
        assert!(valid_key("status"));
        assert!(valid_key("due-date"));
        assert!(!valid_key(""));
        assert!(!valid_key("has space"));
        assert!(!valid_key("1st"));
    }
}
