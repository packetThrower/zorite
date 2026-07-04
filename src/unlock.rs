//! The unlock screen: a small window shown at boot when the database is
//! encrypted and no valid keychain password exists, and again whenever the
//! idle auto-lock fires. On success it stashes the session key
//! (`security::set_session_key`) and opens the main window.

use gpui::{
    App, AppContext, Bounds, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, TitlebarOptions,
    Window, WindowAppearance, WindowBounds, WindowOptions, div, px, size,
};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Root, TitleBar};

use crate::theme;

pub struct UnlockView {
    input: Entity<InputState>,
    remember: bool,
    error: bool,
    _sub: Subscription,
}

impl UnlockView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });
        let sub = cx.subscribe_in(
            &input,
            window,
            |this: &mut UnlockView, _st, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.try_unlock(window, cx);
                }
            },
        );
        input.update(cx, |s, cx| s.focus(window, cx));
        Self {
            input,
            remember: false,
            error: false,
            _sub: sub,
        }
    }

    fn try_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.input.read(cx).value().to_string();
        if password.is_empty() {
            return;
        }
        if crate::db::Db::verify_key(&password) {
            crate::security::set_session_key(Some(password.clone()));
            crate::security::touch_activity();
            if self.remember {
                crate::security::remember_password(&password);
            }
            // Open the main window BEFORE retiring this one, in one deferred
            // step: on Windows gpui exits when the window count hits zero,
            // and the old defer ran the open AFTER the removal — a correct
            // password closed both windows and the app (macOS survives a
            // windowless beat, which hid the bug).
            let this_window = window.window_handle();
            cx.defer(move |cx| {
                crate::open_main_window(cx);
                let _ = this_window.update(cx, |_, window, _| window.remove_window());
            });
        } else {
            self.error = true;
            self.input.update(cx, |s, cx| {
                s.set_value("", window, cx);
                s.focus(window, cx);
            });
            cx.notify();
        }
    }
}

impl Render for UnlockView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let remember = self.remember;
        div()
            .size_full()
            .bg(theme::bg_window())
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.0))
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::text_primary())
                    .child("Zorite is locked"),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::text_tertiary())
                    .child("Your notes are encrypted. Enter your password to open them."),
            )
            .child(div().w(px(280.0)).child(Input::new(&self.input)))
            .child(
                div().w(px(280.0)).child(
                    Checkbox::new("unlock-remember")
                        .label("Remember on this device")
                        .checked(remember)
                        .on_click(cx.listener(|this: &mut UnlockView, on: &bool, _w, cx| {
                            this.remember = *on;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .id("unlock-go")
                    .w(px(280.0))
                    .py(px(7.0))
                    .rounded(px(8.0))
                    .bg(theme::accent_tint())
                    .text_color(theme::accent())
                    .text_size(px(13.0))
                    .flex()
                    .flex_row()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::accent()).text_color(theme::bg_window()))
                    .on_click(
                        cx.listener(|this: &mut UnlockView, _: &gpui::ClickEvent, w, cx| {
                            this.try_unlock(w, cx);
                        }),
                    )
                    .child("Unlock"),
            )
            .children(self.error.then(|| {
                div()
                    .text_size(px(12.0))
                    .text_color(gpui::hsla(0.0, 0.7, 0.5, 1.0))
                    .child("Wrong password — try again.")
            }))
    }
}

/// Open the unlock window (centered, fixed-size).
pub fn open_unlock_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(420.0), px(300.0)), cx);
    if let Err(e) = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Zorite — locked".into()),
                ..TitleBar::title_bar_options()
            }),
            app_id: Some("zorite".into()),
            ..Default::default()
        },
        |window, cx| {
            // Theme like the app would: the saved skin + mode when the DB is
            // readable; an encrypted DB (theme prefs live inside the lock)
            // falls back to the default skin at the system appearance.
            let (skin_id, mode_str) = crate::db::read_theme(&crate::paths::db_path());
            let mut all = crate::skins::builtin_skins();
            let idx = skin_id
                .as_ref()
                .and_then(|id| all.iter().position(|s| &s.id == id))
                .unwrap_or(0);
            let skin = all.swap_remove(idx);
            let mode = mode_str
                .map(|s| theme::Mode::from_str(&s))
                .unwrap_or_default();
            let is_dark = skin.dark_only
                || match mode {
                    theme::Mode::Light => false,
                    theme::Mode::Dark => true,
                    theme::Mode::Auto => matches!(
                        window.appearance(),
                        WindowAppearance::Dark | WindowAppearance::VibrantDark
                    ),
                };
            theme::apply(
                if is_dark { skin.dark } else { skin.light },
                is_dark,
                window,
                cx,
            );
            let view = cx.new(|cx| UnlockView::new(window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        },
    ) {
        log::error!("open unlock window: {e}");
    }
    cx.activate(true);
}
