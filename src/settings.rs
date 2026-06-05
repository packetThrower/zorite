//! The Settings window — a separate OS window for app preferences. v1
//! exposes the theme mode (Light / Dark / Auto). Changes call back into
//! `AppView` (which owns the DB and applies the theme), so they take
//! effect live in every window via the shared theme global + refresh.

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::TitleBar;

use crate::app::AppView;
use crate::theme::{self, Mode};

pub struct SettingsView {
    app: WeakEntity<AppView>,
}

impl SettingsView {
    pub fn new(app: WeakEntity<AppView>, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { app }
    }

    fn current_mode(&self, cx: &Context<Self>) -> Mode {
        self.app.upgrade().map(|a| a.read(cx).theme_mode()).unwrap_or_default()
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |a, cx| a.set_theme_mode(mode, window, cx));
            cx.notify();
        }
    }

    fn set_skin(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |a, cx| a.set_skin(id, window, cx));
            cx.notify();
        }
    }

    fn reveal_themes_folder(&self, cx: &Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.read(cx).reveal_themes_folder();
        }
    }

    fn reload_skins(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |a, cx| a.reload_skins(window, cx));
            cx.notify();
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current_mode(cx);
        let mut modes_row = div().flex().flex_row().gap(px(8.0));
        for m in [Mode::Light, Mode::Dark, Mode::Auto] {
            modes_row = modes_row.child(mode_button(m, m == current, cx));
        }

        // Theme list + active id, read from AppView (borrow ends with `a`).
        let (active_skin, skin_list): (String, Vec<(String, String)>) =
            if let Some(app) = self.app.upgrade() {
                let a = app.read(cx);
                (
                    a.active_skin_id().to_string(),
                    a.skins().iter().map(|s| (s.id.clone(), s.name.clone())).collect(),
                )
            } else {
                (String::new(), Vec::new())
            };
        let mut skins_col = div().flex().flex_col().gap(px(6.0));
        for (id, name) in skin_list {
            let active = id == active_skin;
            skins_col = skins_col.child(skin_button(id, name, active, cx));
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::bg_window())
            .text_color(theme::text_primary())
            .child(
                TitleBar::new().child(
                    div()
                        .px_2()
                        .text_size(px(13.0))
                        .text_color(theme::text_secondary())
                        .child("Settings"),
                ),
            )
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(24.0))
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .child(section_label("THEME"))
                    .child(skins_col)
                    .child(section_label("APPEARANCE"))
                    .child(modes_row)
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::text_tertiary())
                            .child("Auto follows your system's light/dark setting."),
                    )
                    .child(section_label("USER THEMES"))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.0))
                            .child(text_button(
                                "reveal-themes",
                                "Reveal themes folder",
                                cx,
                                |this, _w, cx| this.reveal_themes_folder(cx),
                            ))
                            .child(text_button("reload-themes", "Reload", cx, |this, w, cx| {
                                this.reload_skins(w, cx)
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::text_tertiary())
                            .child("Drop .json theme files in that folder, then Reload."),
                    ),
            )
    }
}

fn mode_button(mode: Mode, active: bool, cx: &mut Context<SettingsView>) -> impl IntoElement {
    div()
        .id(mode.as_str())
        .px(px(16.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .text_size(px(13.0))
        .cursor_pointer()
        .when(active, |d| {
            d.bg(theme::accent_tint())
                .border_color(theme::accent())
                .text_color(theme::text_primary())
        })
        .when(!active, |d| {
            d.bg(theme::glass())
                .border_color(theme::border_subtle())
                .text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::glass_strong()).text_color(theme::text_primary()))
        })
        .child(mode.label())
        .on_click(cx.listener(move |this: &mut SettingsView, _, window, cx| {
            this.set_mode(mode, window, cx);
        }))
}

fn skin_button(id: String, name: String, active: bool, cx: &mut Context<SettingsView>) -> impl IntoElement {
    let elem_id = SharedString::from(format!("skin-{id}"));
    div()
        .id(elem_id)
        .px(px(12.0))
        .py(px(7.0))
        .rounded(px(8.0))
        .border_1()
        .text_size(px(13.0))
        .cursor_pointer()
        .when(active, |d| {
            d.bg(theme::accent_tint())
                .border_color(theme::accent())
                .text_color(theme::text_primary())
        })
        .when(!active, |d| {
            d.bg(theme::glass())
                .border_color(theme::border_subtle())
                .text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::glass_strong()).text_color(theme::text_primary()))
        })
        .child(name)
        .on_click(cx.listener(move |this: &mut SettingsView, _, window, cx| {
            this.set_skin(id.clone(), window, cx);
        }))
}

fn section_label(text: &str) -> impl IntoElement {
    div().text_size(px(11.0)).text_color(theme::text_tertiary()).child(text.to_string())
}

fn text_button(
    id: &'static str,
    label: &str,
    cx: &mut Context<SettingsView>,
    on: impl Fn(&mut SettingsView, &mut Window, &mut Context<SettingsView>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(12.0))
        .py(px(7.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::glass())
        .text_color(theme::text_secondary())
        .text_size(px(13.0))
        .cursor_pointer()
        .hover(|h| h.bg(theme::glass_strong()).text_color(theme::text_primary()))
        .child(label.to_string())
        .on_click(cx.listener(move |this, _, window, cx| on(this, window, cx)))
}
