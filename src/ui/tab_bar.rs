//! The tab strip above the content area. Tab 0 is the pinned Journal;
//! the rest are opened pages. Scrollable with an overflow menu when the
//! tabs don't fit (gpui-component `TabBar`).

use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, div, px,
};
use gpui_component::tab::{Tab, TabBar};

use crate::app::AppView;
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let mut bar = TabBar::new("tabs")
        .menu(true)
        .track_scroll(&app.tab_scroll)
        .selected_index(app.active);

    for (i, tab) in app.tabs.iter().enumerate() {
        let mut t = Tab::new().label(tab.title.clone()).on_click(cx.listener(
            move |this: &mut AppView, _ev, window, cx| {
                this.activate_tab(i, window, cx);
            },
        ));
        // Every tab except the pinned journal (index 0) gets a close ×.
        if i != 0 {
            t = t.suffix(close_button(i, cx));
        }
        bar = bar.child(t);
    }

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
