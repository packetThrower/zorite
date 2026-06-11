//! The tab strip above the content area. Tab 0 is the pinned Journal;
//! the rest are opened pages. Scrollable with an overflow menu when the
//! tabs don't fit (gpui-component `TabBar`).

use gpui::{
    AppContext, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, canvas, div, px,
};
use gpui_component::menu::ContextMenuExt;
use gpui_component::tab::{Tab, TabBar};

use crate::actions::OpenInNewWindow;
use crate::app::{AppView, DraggingTab, GlobalDraggingTab, TabDrag};
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
                .on_drag(
                    TabDrag {
                        ix: i,
                        kind: tab.kind.clone(),
                        title,
                    },
                    |drag, _offset, window, cx| {
                        // Record the drag app-wide so the source window can route the
                        // release (to another window, or a new one) when it lands off
                        // the strip.
                        cx.global_mut::<GlobalDraggingTab>().0 = Some(DraggingTab {
                            source: window.window_handle(),
                            kind: drag.kind.clone(),
                            title: drag.title.clone(),
                        });
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    },
                )
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

    // While a tab from another window is dragged over this one, show a translucent
    // "ghost tab" at the end of the strip — where the dropped tab will land.
    if let Some(title) = app.drop_ghost_title(cx) {
        bar = bar.child(ghost_tab(title));
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

    // Record the strip's window-relative rect (behind the bar, so it never
    // intercepts clicks) so another window's drag can tell when the cursor is
    // over this tab bar — the cross-window "move here" target.
    let strip_bounds = app.tab_strip_bounds.clone();
    div()
        .flex_shrink_0()
        .w_full()
        .relative()
        .child(
            canvas(
                move |bounds, _window, _cx| strip_bounds.set(bounds),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .child(bar)
}

/// A translucent, dashed placeholder tab showing where a tab dragged in from
/// another window would land. Non-interactive — purely an indicator.
fn ghost_tab(title: SharedString) -> Tab {
    Tab::new()
        .disabled(true)
        .child(
            // `h_full` makes the dashed outline fill the tab's content height so
            // the ghost reads as a full-size tab, not a small inner chip. A small
            // negative left margin cancels the dashed box's own border+padding so
            // the label lines up with the other tabs' text instead of sitting inset.
            div()
                .h_full()
                .flex()
                .items_center()
                .gap_1()
                .ml(px(-8.0))
                .border_1()
                .border_dashed()
                .border_color(theme::accent())
                .rounded(px(6.0))
                .px(px(8.0))
                .text_color(theme::accent())
                .child("＋")
                .child(title),
        )
        .opacity(0.6)
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
