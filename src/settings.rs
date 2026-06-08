//! The Settings window — a card-based, two-pane layout styled after
//! Baudrun's Skins screen: a left nav + cards with a title / description /
//! control. The **Appearance** pane has an App Theme dropdown, an
//! Appearance (light/dark/auto) dropdown, and an Installed-themes card
//! (reveal folder + reload + the user themes loaded from disk).
//!
//! The dropdowns are gpui-component `Select`s; selecting one calls back
//! into `AppView` so the change applies live to every window.

use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    IndexPath, TitleBar,
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
    Appearance,
    Pdf,
    Markdown,
}

pub struct SettingsView {
    app: WeakEntity<AppView>,
    theme_select: Entity<SelectState<Vec<Opt>>>,
    appearance_select: Entity<SelectState<Vec<Opt>>>,
    quality_slider: Entity<SliderState>,
    indent_select: Entity<SelectState<Vec<Opt>>>,
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

        Self {
            app,
            theme_select,
            appearance_select,
            quality_slider,
            indent_select,
            tab: Tab::Appearance,
            _subs: subs,
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
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                        }
                    }),
            )
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
