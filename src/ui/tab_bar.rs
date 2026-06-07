//! The tab strip above the content area. Tab 0 is the pinned Journal;
//! the rest are opened pages. Scrollable with an overflow menu when the
//! tabs don't fit (gpui-component `TabBar`).

use gpui::{
    AppContext, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::menu::ContextMenuExt;
use gpui_component::tab::{Tab, TabBar};

use crate::actions::OpenInNewWindow;
use crate::app::{AppView, TabDrag};
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let mut bar = TabBar::new("tabs")
        .menu(true)
        .track_scroll(&app.tab_scroll)
        .selected_index(app.active);

    for (i, tab) in app.tabs.iter().enumerate() {
        // The tab's label hosts a right-click "Open in new window" menu. The
        // gpui-component `ContextMenu` can't be attached to a `Tab` directly
        // (`TabBar::child` wants `Into<Tab>`, but `context_menu` returns a
        // wrapper), so it lives on the tab's child element. Right-click also
        // records which tab is the target.
        let kind = tab.kind.clone();
        let title = tab.title.clone();
        let label = div()
            .id(("tab-label", i))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this: &mut AppView, _ev, _window, _cx| {
                    this.set_context_target(kind.clone());
                }),
            )
            .child(title.clone())
            .context_menu(|menu, _window, _cx| {
                menu.menu("Open in new window", Box::new(OpenInNewWindow))
            });
        let mut t = Tab::new().child(label).on_click(cx.listener(
            move |this: &mut AppView, _ev, window, cx| {
                this.activate_tab(i, window, cx);
            },
        ));
        // The pinned Journal (index 0) has no close × and isn't draggable. Every
        // other tab can be dragged to reorder (drop on another tab) or torn off
        // into a new window (drop in the content area).
        if i != 0 {
            t = t
                .suffix(close_button(i, cx))
                .on_drag(TabDrag { ix: i, title }, |drag, _offset, _window, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                })
                .drag_over::<TabDrag>(|style, _drag, _window, _cx| {
                    style.border_l_2().border_color(theme::accent())
                })
                .on_drop(
                    cx.listener(move |this: &mut AppView, drag: &TabDrag, window, cx| {
                        this.reorder_tab(drag.ix, i, window, cx);
                    }),
                );
        }
        bar = bar.child(t);
    }

    // A flex-grow drop zone filling the strip past the last tab, so a tab can be
    // dropped at the very end (the tabs themselves only cover up-to-their-slot).
    bar = bar.last_empty_space(
        div()
            .id("tab-bar-end")
            .h_full()
            .flex_grow()
            .min_w(px(40.0))
            .drag_over::<TabDrag>(|style, _drag, _window, _cx| {
                style.border_l_2().border_color(theme::accent())
            })
            .on_drop(
                cx.listener(|this: &mut AppView, drag: &TabDrag, window, cx| {
                    let end = this.tabs.len();
                    this.reorder_tab(drag.ix, end, window, cx);
                }),
            ),
    );

    div().flex_shrink_0().w_full().child(bar)
}

fn close_button(ix: usize, cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id(("tab-close", ix))
        .px(px(4.0))
        .rounded(px(4.0))
        .text_color(theme::text_tertiary())
        .cursor_pointer()
        .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        .child("×")
        // Close on press and stop the event so the tab doesn't also switch.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this: &mut AppView, _ev, window, cx| {
                cx.stop_propagation();
                this.close_tab(ix, window, cx);
            }),
        )
}
