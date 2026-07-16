//! Render helpers for the workspace chrome and the two surfaces (the
//! journal feed and a single page). Free functions taking the `AppView`
//! to read and a `Context<AppView>` for event listeners.

pub mod all_pages;
pub mod embed;
pub mod game;
pub mod graph;
pub mod image;
pub mod journal;
pub mod links;
pub mod math;
pub mod mermaid;
pub mod month_cal;
pub mod page_view;
pub mod properties_page;
pub mod property_editor;
pub mod search;
pub mod sidebar;
pub mod slash_menu;
pub mod tab_bar;
pub mod table_picker;

use gpui::{
    AnyElement, Context, Div, InteractiveElement, IntoElement, MouseButton, SharedString, Stateful,
};
use gpui_component::menu::ContextMenuExt;

use crate::actions::{
    CopyPageContents, CopyPageContentsMarkdown, CopyPageLink, DeletePage, ExportPdf, NewSubPage,
    OpenInNewTab, OpenInNewWindow, RenamePage, ToggleFavorite,
};
use crate::app::AppView;

/// Attach THE page context menu to a row — one menu for every page-like
/// surface (sidebar rows, All Pages, search results, backlinks), so the same
/// actions are reachable wherever a page shows up. Right-click stores the
/// target (`set_context_page`); the items dispatch the page actions.
pub(crate) fn with_page_menu(
    row: Stateful<Div>,
    id: i64,
    title: SharedString,
    is_fav: bool,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let fav_label = if is_fav {
        "Remove from favorites"
    } else {
        "Add to favorites"
    };
    row.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this: &mut AppView, _, _window, _cx| {
            this.set_context_page(id, title.clone());
        }),
    )
    .context_menu(move |menu, _window, _cx| {
        menu.menu(fav_label, Box::new(ToggleFavorite))
            .separator()
            .menu("Open in new tab", Box::new(OpenInNewTab))
            .menu("Open in new window", Box::new(OpenInNewWindow))
            .separator()
            .menu("Copy link", Box::new(CopyPageLink))
            .menu("Copy contents", Box::new(CopyPageContents))
            .menu(
                "Copy contents as Markdown",
                Box::new(CopyPageContentsMarkdown),
            )
            .separator()
            .menu("Export as PDF…", Box::new(ExportPdf))
            .separator()
            .menu("New sub-page", Box::new(NewSubPage))
            .menu("Rename page", Box::new(RenamePage))
            .menu("Delete page", Box::new(DeletePage))
    })
    .into_any_element()
}
