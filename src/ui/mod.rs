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
    AnyElement, Context, Div, InteractiveElement, IntoElement, MouseButton, ParentElement as _,
    SharedString, Stateful, Styled as _, div,
};
use gpui_component::menu::ContextMenuExt;
use gpui_component::{ActiveTheme as _, Icon, IconName};

use crate::actions::{
    CopyPageContents, CopyPageContentsMarkdown, CopyPageLink, DeletePage, ExportPdf, NewSubPage,
    OpenInNewTab, OpenInNewWindow, RenamePage, ToggleFavorite,
};
use crate::app::AppView;
use rust_i18n::t;

/// A bundled Lucide face (served by the app's `Assets`) as a menu-row icon.
pub(crate) fn menu_icon(name: &str) -> Icon {
    Icon::empty().path(SharedString::from(format!("icons/{name}.svg")))
}

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
        SharedString::from(t!("page_ctx.remove_favorite"))
    } else {
        SharedString::from(t!("page_ctx.add_favorite"))
    };
    row.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this: &mut AppView, _, _window, _cx| {
            this.set_context_page(id, title.clone());
        }),
    )
    .context_menu(move |menu, _window, cx| {
        let danger = cx.theme().danger;
        let fav_icon = if is_fav {
            Icon::from(IconName::StarOff)
        } else {
            Icon::from(IconName::Star)
        };
        menu.menu_with_icon(fav_label.clone(), fav_icon, Box::new(ToggleFavorite))
            .separator()
            .menu_with_icon(
                t!("page_ctx.open_new_tab"),
                menu_icon("arrow-up-right"),
                Box::new(OpenInNewTab),
            )
            .menu_with_icon(
                t!("page_ctx.open_new_window"),
                menu_icon("app-window"),
                Box::new(OpenInNewWindow),
            )
            .separator()
            .menu_with_icon(
                t!("page_ctx.copy_link"),
                menu_icon("link"),
                Box::new(CopyPageLink),
            )
            .menu_with_icon(
                t!("page_ctx.copy_contents"),
                IconName::Copy,
                Box::new(CopyPageContents),
            )
            .menu_with_icon(
                t!("page_ctx.copy_contents_md"),
                menu_icon("file-text"),
                Box::new(CopyPageContentsMarkdown),
            )
            .separator()
            .menu_with_icon(
                t!("page_ctx.export_pdf"),
                menu_icon("file-down"),
                Box::new(ExportPdf),
            )
            .separator()
            .menu_with_icon(
                t!("page_ctx.new_sub_page"),
                menu_icon("sticky-note-plus"),
                Box::new(NewSubPage),
            )
            .menu_with_icon(
                t!("page_ctx.rename_page"),
                menu_icon("pencil"),
                Box::new(RenamePage),
            )
            // Destructive: red label + red icon (Cditor-style).
            .menu_element_with_icon(
                menu_icon("trash-2").text_color(danger),
                Box::new(DeletePage),
                move |_, _| div().text_color(danger).child(t!("page_ctx.delete_page")),
            )
    })
    .into_any_element()
}
