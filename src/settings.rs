//! The Settings window — a separate OS window for app preferences. v1
//! exposes the theme mode (Light / Dark / Auto). Changes call back into
//! `AppView` (which owns the DB and applies the theme), so they take
//! effect live in every window via the shared theme global + refresh.

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
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
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current_mode(cx);
        let mut modes_row = div().flex().flex_row().gap(px(8.0));
        for m in [Mode::Light, Mode::Dark, Mode::Auto] {
            modes_row = modes_row.child(mode_button(m, m == current, cx));
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
                    .flex_1()
                    .min_h_0()
                    .p(px(24.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme::text_tertiary())
                            .child("APPEARANCE"),
                    )
                    .child(modes_row)
                    .child(
                        div()
                            .pt(px(2.0))
                            .text_size(px(12.0))
                            .text_color(theme::text_tertiary())
                            .child("Auto follows your system's light/dark setting."),
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
