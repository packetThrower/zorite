//! The Settings window — a card-based, two-pane layout styled after
//! Baudrun's Skins screen: a left nav + cards with a title / description /
//! control. The **Appearance** pane has an App Theme dropdown, an
//! Appearance (light/dark/auto) dropdown, and an Installed-themes card
//! (reveal folder + reload + the user themes loaded from disk).
//!
//! The dropdowns are gpui-component `Select`s; selecting one calls back
//! into `AppView` so the change applies live to every window.

use std::path::PathBuf;

use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    IndexPath, Root, TitleBar, WindowExt,
    dialog::DialogButtonProps,
    select::{Select, SelectEvent, SelectItem, SelectState},
    slider::{Slider, SliderEvent, SliderState},
};

use crate::app::AppView;
use crate::theme::{self, Mode};

/// One choice in a `Select`: `id` is the stored value, `title` the label.
#[derive(Clone)]
struct Opt {
    id: String,
    title: SharedString,
}

impl Opt {
    fn new(id: &str, title: &str) -> Self {
        Self {
            id: id.to_string(),
            title: SharedString::from(title.to_string()),
        }
    }
}

impl SelectItem for Opt {
    type Value = String;
    fn title(&self) -> SharedString {
        self.title.clone()
    }
    fn value(&self) -> &Self::Value {
        &self.id
    }
}

fn make_select(
    opts: Vec<Opt>,
    selected: &str,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Entity<SelectState<Vec<Opt>>> {
    let idx = opts
        .iter()
        .position(|o| o.id == selected)
        .map(IndexPath::new);
    cx.new(|cx| SelectState::new(opts, idx, window, cx))
}

fn theme_opts(app: &WeakEntity<AppView>, cx: &Context<SettingsView>) -> (Vec<Opt>, String) {
    if let Some(a) = app.upgrade() {
        let a = a.read(cx);
        (
            a.skins().iter().map(|s| Opt::new(&s.id, &s.name)).collect(),
            a.active_skin_id().to_string(),
        )
    } else {
        (Vec::new(), String::new())
    }
}

/// Which settings category the left nav has selected.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Appearance,
    Pdf,
    Markdown,
    Keyboard,
    Updates,
}

pub struct SettingsView {
    app: WeakEntity<AppView>,
    theme_select: Entity<SelectState<Vec<Opt>>>,
    appearance_select: Entity<SelectState<Vec<Opt>>>,
    quality_slider: Entity<SliderState>,
    indent_select: Entity<SelectState<Vec<Opt>>>,
    date_format_select: Entity<SelectState<Vec<Opt>>>,
    time_format_select: Entity<SelectState<Vec<Opt>>>,
    check_updates_select: Entity<SelectState<Vec<Opt>>>,
    prerelease_select: Entity<SelectState<Vec<Opt>>>,
    /// The selected left-nav category.
    tab: Tab,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(app: WeakEntity<AppView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (t_opts, active_skin) = theme_opts(&app, cx);
        let mode = app
            .upgrade()
            .map(|a| a.read(cx).theme_mode())
            .unwrap_or_default();
        let a_opts = vec![
            Opt::new("light", "Light"),
            Opt::new("dark", "Dark"),
            Opt::new("auto", "Auto (follow system)"),
        ];

        let theme_select = make_select(t_opts, &active_skin, window, cx);
        let appearance_select = make_select(a_opts, mode.as_str(), window, cx);

        let mut subs = Vec::new();
        subs.push(Self::on_theme_select(&theme_select, window, cx));
        subs.push(cx.subscribe_in(
            &appearance_select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev {
                    let mode = Mode::from_str(id);
                    if let Some(app) = this.app.upgrade() {
                        app.update(cx, |a, cx| a.set_theme_mode(mode, window, cx));
                        cx.notify();
                    }
                }
            },
        ));

        // PDF render-quality slider (percentage of native DPI).
        let qpct = app
            .upgrade()
            .map(|a| a.read(cx).pdf_quality() * 100.0)
            .unwrap_or(100.0);
        let quality_slider = cx.new(|_| {
            SliderState::new()
                .min(50.0)
                .max(200.0)
                .step(5.0)
                .default_value(qpct)
        });
        subs.push(cx.subscribe_in(
            &quality_slider,
            window,
            |this: &mut SettingsView, _, ev: &SliderEvent, _window, cx| {
                if let SliderEvent::Change(v) = ev
                    && let Some(app) = this.app.upgrade()
                {
                    app.update(cx, |a, cx| a.set_pdf_quality(v.start() / 100.0, cx));
                    cx.notify();
                }
            },
        ));

        // List-indent select (Markdown pane): 2 / 4 / 8 spaces.
        let cur_indent = app
            .upgrade()
            .map(|a| a.read(cx).list_indent().to_string())
            .unwrap_or_else(|| "4".to_string());
        let indent_select = make_select(
            vec![
                Opt::new("2", "2 spaces"),
                Opt::new("4", "4 spaces"),
                Opt::new("8", "8 spaces"),
            ],
            &cur_indent,
            window,
            cx,
        );
        subs.push(cx.subscribe_in(
            &indent_select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev
                    && let Ok(spaces) = id.parse::<usize>()
                    && let Some(app) = this.app.upgrade()
                {
                    app.update(cx, |a, cx| a.set_list_indent(spaces, cx));
                    cx.notify();
                }
            },
        ));

        // Date / time formats (General pane): the styles used by /date, /time,
        // and the {{date}} / {{time}} template placeholders.
        let date_opts: Vec<Opt> = crate::dates::DATE_FORMATS
            .iter()
            .map(|&id| Opt::new(id, crate::dates::date_format_label(id)))
            .collect();
        let date_format_select = make_select(date_opts, &crate::dates::date_format(), window, cx);
        subs.push(cx.subscribe_in(
            &date_format_select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev
                    && let Some(app) = this.app.upgrade()
                {
                    let id = id.clone();
                    app.update(cx, |a, _cx| a.set_date_format(&id));
                    cx.notify();
                }
            },
        ));

        let time_opts: Vec<Opt> = crate::dates::TIME_FORMATS
            .iter()
            .map(|&id| Opt::new(id, crate::dates::time_format_label(id)))
            .collect();
        let time_format_select = make_select(time_opts, &crate::dates::time_format(), window, cx);
        subs.push(cx.subscribe_in(
            &time_format_select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev
                    && let Some(app) = this.app.upgrade()
                {
                    let id = id.clone();
                    app.update(cx, |a, _cx| a.set_time_format(&id));
                    cx.notify();
                }
            },
        ));

        // Updates pane: On/Off selects for the auto-check + pre-release prefs.
        let on_off = || vec![Opt::new("on", "On"), Opt::new("off", "Off")];
        let cur_check = app
            .upgrade()
            .map(|a| a.read(cx).check_updates())
            .unwrap_or(true);
        let check_updates_select =
            make_select(on_off(), if cur_check { "on" } else { "off" }, window, cx);
        subs.push(cx.subscribe_in(
            &check_updates_select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev
                    && let Some(app) = this.app.upgrade()
                {
                    let on = id == "on";
                    app.update(cx, |a, _cx| a.set_check_updates(on));
                    cx.notify();
                }
            },
        ));

        let cur_pre = app
            .upgrade()
            .map(|a| a.read(cx).include_prerelease())
            .unwrap_or(false);
        let prerelease_select =
            make_select(on_off(), if cur_pre { "on" } else { "off" }, window, cx);
        subs.push(cx.subscribe_in(
            &prerelease_select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev
                    && let Some(app) = this.app.upgrade()
                {
                    let on = id == "on";
                    app.update(cx, |a, cx| a.set_include_prerelease(on, cx));
                    cx.notify();
                }
            },
        ));

        Self {
            app,
            theme_select,
            appearance_select,
            quality_slider,
            indent_select,
            date_format_select,
            time_format_select,
            check_updates_select,
            prerelease_select,
            tab: Tab::Appearance,
            _subs: subs,
        }
    }

    /// Re-run the update check now (Settings → Updates → "Check now").
    fn check_for_updates(&self, cx: &mut Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |a, cx| a.check_for_updates_now(cx));
        }
    }

    /// Subscribe to a theme `Select`'s confirm → apply the picked skin.
    fn on_theme_select(
        select: &Entity<SelectState<Vec<Opt>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev {
                    let id = id.clone();
                    if let Some(app) = this.app.upgrade() {
                        app.update(cx, |a, cx| a.set_skin(id, window, cx));
                        cx.notify();
                    }
                }
            },
        )
    }

    fn reveal_themes_folder(&self, cx: &Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.read(cx).reveal_themes_folder();
        }
    }

    /// Re-scan themes on disk and rebuild the theme dropdown to include them.
    fn reload_skins(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.update(cx, |a, cx| a.reload_skins(window, cx));
        let (opts, active) = theme_opts(&self.app, cx);
        let select = make_select(opts, &active, window, cx);
        let sub = Self::on_theme_select(&select, window, cx);
        self._subs.push(sub);
        self.theme_select = select;
        cx.notify();
    }

    fn user_theme_names(&self, cx: &Context<Self>) -> Vec<String> {
        self.app
            .upgrade()
            .map(|a| {
                a.read(cx)
                    .skins()
                    .iter()
                    .filter(|s| !s.is_builtin)
                    .map(|s| s.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Pick a new data directory, then confirm before recording the change.
    fn choose_data_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(target) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.confirm_relocation(target, window, cx);
            });
        })
        .detach();
    }

    /// Confirm a relocation to `target`, then record it and quit so the change
    /// (and any pending move) applies on the next launch.
    fn confirm_relocation(&mut self, target: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        use crate::paths::Relocation;
        let current = crate::paths::data_dir();
        let (title, body, ok): (&'static str, String, &'static str) =
            match crate::paths::plan_relocation(&target) {
                Relocation::NoOp => return,
                Relocation::Invalid(reason) => {
                    self.alert("Can’t use that folder", reason, window, cx);
                    return;
                }
                Relocation::Switch => (
                    "Switch data location",
                    format!(
                        "“{}” already contains a Zorite database.\n\nZorite will use it the next \
                         time it starts. Your current data stays where it is:\n{}",
                        target.display(),
                        current.display(),
                    ),
                    "Switch & Quit",
                ),
                Relocation::Move => (
                    "Move data location",
                    format!(
                        "Zorite will move your notes, settings, and attachments to:\n{}\n\nThe \
                         change takes effect the next time you open Zorite.",
                        target.display(),
                    ),
                    "Move & Quit",
                ),
            };
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let target = target.clone();
            let body = body.clone();
            dialog
                .title(title)
                .description(SharedString::from(body))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok)
                        .cancel_text("Cancel")
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    match crate::paths::set_location(&target) {
                        Ok(()) => cx.quit(),
                        Err(e) => log::error!("set data location failed: {e}"),
                    }
                    true
                })
        });
    }

    /// Confirm sending the data back to the OS-default location, then quit.
    fn confirm_reset_data_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if crate::paths::is_default_location() {
            return;
        }
        let default = crate::paths::default_location();
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let default = default.clone();
            dialog
                .title("Reset data location")
                .description(SharedString::from(format!(
                    "Zorite will move your data back to the default location:\n{}\n\nThe change \
                     takes effect the next time you open Zorite.",
                    default.display(),
                )))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Reset & Quit")
                        .cancel_text("Cancel")
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    match crate::paths::reset_location() {
                        Ok(()) => cx.quit(),
                        Err(e) => log::error!("reset data location failed: {e}"),
                    }
                    true
                })
        });
    }

    /// A simple message dialog with a single OK button (no action).
    fn alert(
        &self,
        title: &'static str,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let body = body.clone();
            dialog
                .title(title)
                .description(SharedString::from(body))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("OK")
                        .show_cancel(false),
                )
                .on_ok(|_, _window, _cx| true)
        });
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let user_names = self.user_theme_names(cx);

        let qpct = self
            .app
            .upgrade()
            .map(|a| (a.read(cx).pdf_quality() * 100.0).round() as i32)
            .unwrap_or(100);
        let quality_control = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(Slider::new(&self.quality_slider).w_full())
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::text_tertiary())
                    .child(format!("{qpct}%")),
            );

        // Installed-themes card body: the actions + the list (or empty state).
        let actions = div()
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
            }));

        let list = if user_names.is_empty() {
            div()
                .text_size(px(13.0))
                .text_color(theme::text_tertiary())
                .child("No custom themes installed. Drop a .json file in the folder and Reload.")
                .into_any_element()
        } else {
            let mut col = div().flex().flex_col().gap(px(4.0));
            for name in user_names {
                col = col.child(
                    div()
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(theme::glass())
                        .text_size(px(13.0))
                        .text_color(theme::text_secondary())
                        .child(name),
                );
            }
            col.into_any_element()
        };

        let installed = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(actions)
            .child(list);

        // Data-location card body (General): the current path, then change /
        // reveal / reset actions.
        let data_path = crate::paths::data_dir().display().to_string();
        let at_default = crate::paths::is_default_location();
        let location_control = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(theme::glass())
                    .border_1()
                    .border_color(theme::border_subtle())
                    .text_size(px(12.0))
                    .text_color(theme::text_secondary())
                    .child(data_path),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(text_button(
                        "data-change",
                        "Change…",
                        cx,
                        |this, w, cx| this.choose_data_location(w, cx),
                    ))
                    .child(text_button(
                        "data-reveal",
                        "Reveal",
                        cx,
                        |_this, _w, _cx| {
                            crate::app::AppView::reveal_folder(&crate::paths::data_dir());
                        },
                    ))
                    .when(!at_default, |row| {
                        row.child(text_button(
                            "data-reset",
                            "Reset to default",
                            cx,
                            |this, w, cx| this.confirm_reset_data_location(w, cx),
                        ))
                    }),
            );

        // Updates pane: current version, the available-update banner (read from
        // the `updater::UpdateState` global), and View-release / Check-now.
        let available = cx
            .try_global::<crate::updater::UpdateState>()
            .and_then(|u| u.available.clone());
        let cur_version = env!("CARGO_PKG_VERSION");
        let updates_control = {
            let mut col = div().flex().flex_col().gap(px(10.0)).child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme::text_secondary())
                    .child(format!("Current version: v{cur_version}")),
            );
            if let Some(a) = &available {
                let url = a.html_url.clone();
                col = col.child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::accent())
                        .child(format!("Update available: v{}", a.version)),
                );
                // A short preview of the release notes; the full notes are on the
                // release page behind "View release".
                let notes = a.notes.trim();
                if !notes.is_empty() {
                    let mut preview: String = notes.chars().take(280).collect();
                    if notes.chars().count() > 280 {
                        preview.push('…');
                    }
                    col = col.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::text_tertiary())
                            .child(preview),
                    );
                }
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.0))
                        .child(text_button(
                            "updates-view",
                            "View release",
                            cx,
                            move |_this, _w, _cx| open_url(&url),
                        ))
                        .child(text_button(
                            "updates-check",
                            "Check now",
                            cx,
                            |this, _w, cx| this.check_for_updates(cx),
                        )),
                );
            } else {
                col = col
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(theme::text_tertiary())
                            .child("You're on the latest version."),
                    )
                    .child(text_button(
                        "updates-check",
                        "Check now",
                        cx,
                        |this, _w, cx| this.check_for_updates(cx),
                    ));
            }
            col
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::bg_window())
            .text_color(theme::text_primary())
            .child(TitleBar::new())
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(32.0))
                    .pt(px(18.0))
                    .pb(px(14.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(26.0))
                            .font_weight(FontWeight::BOLD)
                            .child("Settings"),
                    )
                    .child(version_chip()),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(nav(self.tab, cx))
                    .child({
                        let content = div()
                            .id("settings-content")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .px(px(24.0))
                            .pb(px(24.0))
                            .flex()
                            .flex_col()
                            .gap(px(16.0));
                        match self.tab {
                            Tab::General => content
                                .child(card(
                                    "Data location",
                                    "Where Zorite keeps your database, settings, and \
                                         attachments. Changing it moves your data to the new \
                                         folder, then reopens Zorite.",
                                    location_control,
                                ))
                                .child(card(
                                    "Date format",
                                    "How /date and the {{date}} template placeholder are \
                                         inserted. Journal day headers are unaffected.",
                                    Select::new(&self.date_format_select).w_full(),
                                ))
                                .child(card(
                                    "Time format",
                                    "How /time and the {{time}} template placeholder are \
                                         inserted.",
                                    Select::new(&self.time_format_select).w_full(),
                                )),
                            Tab::Appearance => content
                                .child(card(
                                    "App Theme",
                                    "Pick a built-in theme or one of your own.",
                                    Select::new(&self.theme_select).w_full(),
                                ))
                                .child(card(
                                    "Appearance",
                                    "Light or dark variant of the active theme. Auto follows \
                                         your system.",
                                    Select::new(&self.appearance_select).w_full(),
                                ))
                                .child(card(
                                    "Installed themes",
                                    "Drop .json theme files in your themes folder, then Reload. \
                                         Any color you omit falls back to the base palette.",
                                    installed,
                                )),
                            Tab::Pdf => content.child(card(
                                "PDF render quality",
                                "Higher is sharper but slower; lower speeds up rendering on \
                                     slower machines. 100% = your display's native resolution.",
                                quality_control,
                            )),
                            Tab::Markdown => content.child(card(
                                "List indentation",
                                "Spaces per nesting level for Tab and bullet nesting. Editing \
                                     and the rendered view use the same width, so they line up.",
                                Select::new(&self.indent_select).w_full(),
                            )),
                            Tab::Keyboard => {
                                let app_rows: Vec<(&str, Vec<&str>)> = vec![
                                    ("New tab (new page)", vec![keys::MOD, "T"]),
                                    ("New window", vec![keys::MOD, "N"]),
                                    ("Close tab", vec![keys::MOD, "W"]),
                                    ("Next tab", vec![keys::CTRL, "Tab"]),
                                    ("Previous tab", vec![keys::CTRL, keys::SHIFT, "Tab"]),
                                    ("Find in page", vec![keys::MOD, "F"]),
                                    ("Search all notes", vec![keys::MOD, keys::SHIFT, "F"]),
                                    (
                                        "Fit oversized images to view",
                                        vec![keys::MOD, keys::SHIFT, "I"],
                                    ),
                                    ("Open settings", vec![keys::MOD, ","]),
                                    ("Quit", vec![keys::MOD, "Q"]),
                                ];
                                let edit_rows: Vec<(&str, Vec<&str>)> = vec![
                                    ("Open the slash command menu", vec!["/"]),
                                    ("Move up / down in the menu", vec!["↑", "↓"]),
                                    ("Insert the selected item", vec!["Enter"]),
                                    ("Close the slash menu", vec!["Esc"]),
                                    ("Indent / nest list item", vec!["Tab"]),
                                    ("Outdent", vec![keys::SHIFT, "Tab"]),
                                    ("Copy", vec![keys::MOD, "C"]),
                                    ("Cut", vec![keys::MOD, "X"]),
                                    ("Paste", vec![keys::MOD, "V"]),
                                    ("Undo", vec![keys::MOD, "Z"]),
                                    ("Redo", keys::redo()),
                                    ("Select all", vec![keys::MOD, "A"]),
                                ];
                                let wb_tool_rows: Vec<(&str, Vec<&str>)> = vec![
                                    ("Select", vec!["V"]),
                                    ("Pan", vec!["H"]),
                                    ("Pen", vec!["P"]),
                                    ("Rectangle", vec!["R"]),
                                    ("Ellipse", vec!["O"]),
                                    ("Diamond", vec!["D"]),
                                    ("Triangle", vec!["G"]),
                                    ("Rounded rectangle", vec!["U"]),
                                    ("Star", vec!["S"]),
                                    ("Hexagon", vec!["X"]),
                                    ("Line", vec!["L"]),
                                    ("Arrow", vec!["A"]),
                                    ("Text", vec!["T"]),
                                    ("Image", vec!["I"]),
                                ];
                                let wb_edit_rows: Vec<(&str, Vec<&str>)> = vec![
                                    ("Undo", vec![keys::MOD, "Z"]),
                                    ("Redo", keys::redo()),
                                    ("Copy", vec![keys::MOD, "C"]),
                                    ("Cut", vec![keys::MOD, "X"]),
                                    ("Paste", vec![keys::MOD, "V"]),
                                    ("Bring forward", vec![keys::MOD, "]"]),
                                    ("Bring to front", vec![keys::MOD, keys::SHIFT, "]"]),
                                    ("Send backward", vec![keys::MOD, "["]),
                                    ("Send to back", vec![keys::MOD, keys::SHIFT, "["]),
                                    ("Delete selection", vec!["Delete"]),
                                    ("Deselect", vec!["Esc"]),
                                ];
                                let pdf_rows: Vec<(&str, Vec<&str>)> = vec![
                                    ("Next page", vec!["PageDown"]),
                                    ("Previous page", vec!["PageUp"]),
                                    ("First page", vec!["Home"]),
                                    ("Last page", vec!["End"]),
                                    ("Zoom in", vec![keys::MOD, "="]),
                                    ("Zoom out", vec![keys::MOD, "−"]),
                                    ("Reset zoom", vec![keys::MOD, "0"]),
                                    ("Find", vec![keys::MOD, "F"]),
                                    ("Next match", vec![keys::MOD, "G"]),
                                    ("Previous match", vec![keys::MOD, keys::SHIFT, "G"]),
                                    ("Toggle highlight mode", vec![keys::MOD, keys::SHIFT, "H"]),
                                    ("Go to page", vec![keys::MOD, keys::ALT, "G"]),
                                ];
                                content
                                    .child(card_list(
                                        "Application",
                                        "App-wide commands — they work anywhere. ⌘ on macOS, \
                                             Ctrl on Windows and Linux.",
                                        app_rows,
                                    ))
                                    .child(card_list(
                                        "Editing",
                                        "The slash menu and text shortcuts, while a note is \
                                             focused.",
                                        edit_rows,
                                    ))
                                    .child(card_list(
                                        "Whiteboard tools",
                                        "Press a letter to pick a tool while a board is focused.",
                                        wb_tool_rows,
                                    ))
                                    .child(card_list(
                                        "Whiteboard editing",
                                        "Acting on the selection while a board is focused.",
                                        wb_edit_rows,
                                    ))
                                    .child(card_list(
                                        "PDF viewer",
                                        "While a PDF tab is focused.",
                                        pdf_rows,
                                    ))
                            }
                            Tab::Updates => content
                                .child(card(
                                    "Software updates",
                                    "Zorite checks GitHub for newer releases at startup. It never \
                                         installs automatically — you review and download from the \
                                         release page.",
                                    updates_control,
                                ))
                                .child(card(
                                    "Automatically check for updates",
                                    "Check for a newer version each time Zorite starts.",
                                    Select::new(&self.check_updates_select).w_full(),
                                ))
                                .child(card(
                                    "Include pre-releases",
                                    "Also offer beta builds (vX.Y.Z-beta.N), not just stable \
                                         releases.",
                                    Select::new(&self.prerelease_select).w_full(),
                                )),
                        }
                    }),
            )
            // gpui-component's `Root` stores dialog state but doesn't draw it;
            // the host view must render the dialog layer (as the main window
            // does), or the data-location confirm dialog stays invisible.
            .children(Root::render_dialog_layer(window, cx))
    }
}

fn nav(active: Tab, cx: &mut Context<SettingsView>) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .w(px(184.0))
        .pl(px(20.0))
        .pr(px(8.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(nav_item("nav-general", "General", Tab::General, active, cx))
        .child(nav_item(
            "nav-appearance",
            "Appearance",
            Tab::Appearance,
            active,
            cx,
        ))
        .child(nav_item("nav-pdf", "PDF", Tab::Pdf, active, cx))
        .child(nav_item(
            "nav-markdown",
            "Markdown",
            Tab::Markdown,
            active,
            cx,
        ))
        .child(nav_item(
            "nav-keyboard",
            "Keyboard",
            Tab::Keyboard,
            active,
            cx,
        ))
        .child(nav_item("nav-updates", "Updates", Tab::Updates, active, cx))
}

/// Open a URL in the user's default browser (the "View release" button).
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd).arg(url).spawn();
}

/// One left-nav category. Highlights when active; clicking switches the pane.
fn nav_item(
    id: &'static str,
    label: &'static str,
    tab: Tab,
    active: Tab,
    cx: &mut Context<SettingsView>,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .text_size(px(14.0))
        .cursor_pointer()
        .when(tab == active, |d| {
            d.bg(theme::accent_tint()).text_color(theme::text_primary())
        })
        .when(tab != active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()))
        })
        .child(label)
        .on_click(cx.listener(move |this, _, _window, cx| {
            this.tab = tab;
            cx.notify();
        }))
}

fn version_chip() -> impl IntoElement {
    div()
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(6.0))
        .bg(theme::glass())
        .border_1()
        .border_color(theme::border_subtle())
        .text_size(px(12.0))
        .text_color(theme::text_secondary())
        .child(concat!("v", env!("CARGO_PKG_VERSION")))
}

/// A settings card: bold title, muted description, then the control.
fn card(title: &str, desc: &str, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .p(px(18.0))
        .rounded(px(12.0))
        .bg(theme::elevated())
        .border_1()
        .border_color(theme::border_subtle())
        .child(
            div()
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(px(13.0))
                .text_color(theme::text_secondary())
                .child(desc.to_string()),
        )
        .child(control)
}

/// Modifier glyphs for the read-only shortcut list. `MOD` is the platform's
/// primary modifier (Cmd on macOS, Ctrl elsewhere) — matching `secondary-` in
/// the keymap; `CTRL` is the literal Control key (for Ctrl+Tab).
#[cfg(target_os = "macos")]
mod keys {
    pub const MOD: &str = "⌘";
    pub const CTRL: &str = "⌃";
    pub const SHIFT: &str = "⇧";
    pub const ALT: &str = "⌥";
    pub fn redo() -> Vec<&'static str> {
        vec![MOD, SHIFT, "Z"]
    }
}
#[cfg(not(target_os = "macos"))]
mod keys {
    pub const MOD: &str = "Ctrl";
    pub const CTRL: &str = "Ctrl";
    pub const SHIFT: &str = "Shift";
    pub const ALT: &str = "Alt";
    pub fn redo() -> Vec<&'static str> {
        vec!["Ctrl", "Y"]
    }
}

/// A settings card whose body is a list of `(label, key combo)` shortcut rows.
fn card_list(title: &str, desc: &str, rows: Vec<(&str, Vec<&str>)>) -> impl IntoElement {
    let mut list = div().flex().flex_col().gap(px(2.0));
    for (label, combo) in rows {
        list = list.child(shortcut_row(label, &combo));
    }
    card(title, desc, list)
}

/// One shortcut row: description on the left, key caps on the right.
fn shortcut_row(label: &str, combo: &[&str]) -> impl IntoElement {
    let mut caps = div().flex().flex_row().gap(px(4.0));
    for key in combo {
        caps = caps.child(kbd(key));
    }
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .py(px(5.0))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(theme::text_secondary())
                .child(label.to_string()),
        )
        .child(caps)
}

/// A single key cap.
fn kbd(key: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(22.0))
        .h(px(20.0))
        .px(px(6.0))
        .rounded(px(6.0))
        .bg(theme::glass())
        .border_1()
        .border_color(theme::border_subtle())
        .text_size(px(12.0))
        .text_color(theme::text_primary())
        .child(key.to_string())
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
        .hover(|h| {
            h.bg(theme::glass_strong())
                .text_color(theme::text_primary())
        })
        .child(label.to_string())
        .on_click(cx.listener(move |this, _, window, cx| on(this, window, cx)))
}
