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
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton,
    MouseUpEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Disableable, IndexPath, Root, Sizable, TitleBar, WindowExt,
    button::{Button, ButtonVariants as _},
    dialog::{DialogButtonProps, DialogFooter},
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectItem, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
};

use crate::app::AppView;
use crate::theme::{self, Mode};
use rust_i18n::t;

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

/// Font-dropdown choices: Default (named for what it resolves to — the active
/// theme's font, else the system face), then every installed family (including
/// the user-added ones registered at startup / via "Add font file…").
fn font_opts(app: &WeakEntity<AppView>, cx: &Context<SettingsView>) -> (Vec<Opt>, String) {
    let mut names = cx.text_system().all_font_names();
    // gpui appends its internal fallback aliases (".ZedMono", ".ZedSans",
    // ".SystemUIFont") to the OS list. They only render inside Zed, which
    // bundles the font files those aliases point at — here they'd silently
    // fall back to the default face, so don't offer them ("Default (System)"
    // already covers the system face).
    names.retain(|n| !n.starts_with('.'));
    names.sort();
    names.dedup();
    let default_font = app.upgrade().and_then(|a| {
        let a = a.read(cx);
        a.skins()
            .iter()
            .find(|s| s.id == a.active_skin_id())
            .and_then(|s| s.font.clone())
    });
    let default_name = default_font
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| t!("settings.opt.system").to_string());
    let default_label = t!("settings.opt.font_default", name = default_name).to_string();
    let mut opts = vec![Opt::new("", &default_label)];
    opts.extend(names.iter().map(|n| Opt::new(n, n)));
    let current = app
        .upgrade()
        .map(|a| a.read(cx).ui_font().to_string())
        .unwrap_or_default();
    (opts, current)
}

/// Cursor-theme dropdown choices: System, then the bundled pack and every
/// user-added pack on disk (see `cursors::available`).
fn cursor_opts() -> (Vec<Opt>, String) {
    let mut opts = vec![
        Opt::new("", &t!("settings.opt.cursor_system")),
        Opt::new(
            crate::cursors::THEME_PACK,
            &t!("settings.opt.cursor_bibata"),
        ),
    ];
    opts.extend(crate::cursors::available().iter().map(|n| Opt::new(n, n)));
    // User packs with SVG sources render theme-reactively as a second entry.
    opts.extend(crate::cursors::reactive_available().iter().map(|n| {
        Opt::new(
            &format!("{}{n}", crate::cursors::THEME_PREFIX),
            &format!("{n} (match theme)"),
        )
    }));
    (opts, crate::cursors::selected().unwrap_or_default())
}

/// Language-picker choices: Auto (its label is itself localized, so the
/// dropdown reads "自动" / "Auto" with the active locale) plus each offered
/// locale shown in its own script. The persisted ids come from
/// [`crate::i18n::LANGUAGE_OPTS`].
fn language_opts() -> Vec<Opt> {
    crate::i18n::LANGUAGE_OPTS
        .iter()
        .map(|(id, name)| {
            let title = if *id == "auto" {
                rust_i18n::t!("settings.language.auto").to_string()
            } else {
                (*name).to_string()
            };
            Opt::new(id, &title)
        })
        .collect()
}

/// Which settings category the left nav has selected.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Notebooks,
    Appearance,
    Pdf,
    Markdown,
    Security,
    Keyboard,
    Updates,
}

/// Which password dialog is open: first-time set, change, or removal.
#[derive(Clone, Copy, PartialEq)]
enum PwMode {
    Set,
    Change,
    Remove,
}

/// Every settings card: `(tab, card title, extra search keywords)`. Drives the
/// header filter — `section_matches` / `tab_has_matches` look cards up here, so
/// the titles MUST stay in sync with the `card(…)` / `card_list(…)` calls in
/// `render`. The title is an i18n key (`settings.section.*`); the rendered
/// nav/card titles call `t!(key)`. Keywords (lowercase) add synonyms a user
/// might type for a setting that aren't already in its title - kept in
/// English so an English-typed synonym still finds a card in any locale.
const SECTIONS: &[(Tab, &str, &str)] = &[
    (
        Tab::Notebooks,
        "settings.section.notebooks",
        "vault workspace switch add remove rename folder data set",
    ),
    (
        Tab::Notebooks,
        "settings.section.data_location",
        "folder path database directory move attachments notebook vault",
    ),
    (
        Tab::General,
        "settings.section.unused_images",
        "cleanup delete orphan gc attachments storage space free",
    ),
    (
        Tab::General,
        "settings.section.language",
        "language locale i18n chinese english 中文 简体",
    ),
    (
        Tab::General,
        "settings.section.remember_window",
        "bounds size resize reopen restore placement screen monitor position tabs session",
    ),
    (
        Tab::General,
        "settings.section.date_format",
        "iso us european calendar day month year /date",
    ),
    (
        Tab::General,
        "settings.section.time_format",
        "24 hour 12 clock am pm /time",
    ),
    (
        Tab::Appearance,
        "settings.section.app_theme",
        "skin colors palette built-in custom",
    ),
    (
        Tab::Appearance,
        "settings.section.appearance",
        "light dark auto system mode variant",
    ),
    (
        Tab::Appearance,
        "settings.section.font",
        "typeface family typography text ttf otf custom",
    ),
    (
        Tab::Appearance,
        "settings.section.text_size",
        "font size zoom bigger smaller larger scale px",
    ),
    (
        Tab::Appearance,
        "settings.section.line_numbers",
        "gutter source rows numbering count editor",
    ),
    (
        Tab::Appearance,
        "settings.section.mouse_cursor",
        "pointer arrow theme pack xcursor bibata custom",
    ),
    (
        Tab::Appearance,
        "settings.section.installed_themes",
        "custom user json reload reveal folder",
    ),
    (
        Tab::Pdf,
        "settings.section.pdf_quality",
        "dpi resolution sharpness speed scale render",
    ),
    (
        Tab::Markdown,
        "settings.section.wysiwyg",
        "live preview inline formatting bold heading links",
    ),
    (
        Tab::Markdown,
        "settings.section.list_indent",
        "spaces tab nesting indent bullet",
    ),
    (
        Tab::Markdown,
        "settings.section.autolink",
        "wiki link automatic typing wrap unlinked references",
    ),
    (
        Tab::Keyboard,
        "settings.section.application",
        "shortcuts keys tab window quit settings find search",
    ),
    (
        Tab::Keyboard,
        "settings.section.editing",
        "shortcuts keys slash menu copy paste undo redo indent",
    ),
    (
        Tab::Keyboard,
        "settings.section.wb_tools",
        "shortcuts keys pen shape rectangle ellipse text image",
    ),
    (
        Tab::Keyboard,
        "settings.section.wb_editing",
        "shortcuts keys z-order delete copy paste",
    ),
    (
        Tab::Keyboard,
        "settings.section.pdf_viewer",
        "shortcuts keys page zoom find",
    ),
    (
        Tab::Security,
        "settings.section.password",
        "encrypt encryption lock database sqlcipher passphrase secure",
    ),
    (
        Tab::Security,
        "settings.section.remember_device",
        "keychain credential manager auto unlock remember password",
    ),
    (
        Tab::Security,
        "settings.section.autolock",
        "idle timeout lock minutes away inactivity",
    ),
    (
        Tab::Updates,
        "settings.section.updates",
        "version release github check download",
    ),
    (
        Tab::Updates,
        "settings.section.auto_check",
        "startup auto version",
    ),
    (
        Tab::Updates,
        "settings.section.prereleases",
        "beta prerelease pre-release unstable",
    ),
];

pub struct SettingsView {
    app: WeakEntity<AppView>,
    theme_select: Entity<SelectState<Vec<Opt>>>,
    appearance_select: Entity<SelectState<Vec<Opt>>>,
    font_select: Entity<SelectState<Vec<Opt>>>,
    cursor_select: Entity<SelectState<Vec<Opt>>>,
    text_size_select: Entity<SelectState<Vec<Opt>>>,
    quality_slider: Entity<SliderState>,
    indent_select: Entity<SelectState<Vec<Opt>>>,
    date_format_select: Entity<SelectState<Vec<Opt>>>,
    time_format_select: Entity<SelectState<Vec<Opt>>>,
    /// Settings -> General -> Language picker. Its options are rebuilt when the
    /// active locale changes (the "Auto" entry is itself localized), so a live
    /// language switch updates the dropdown without reopening Settings.
    language_select: Entity<SelectState<Vec<Opt>>>,
    lang_select_locale: String,
    /// Header filter box + its current (trimmed, lowercased) text. Empty = no
    /// filter; non-empty dims the cards + nav tabs that don't match.
    filter_input: Entity<InputState>,
    filter: String,
    /// The selected left-nav category.
    tab: Tab,
    /// Last images-GC outcome ("Removed 12 files (3.4 MB)"), shown under the
    /// Unused images button.
    image_gc_result: Option<String>,
    /// The last cursor-theme import error, shown under the Mouse cursor card.
    cursor_status: Option<String>,
    /// Password dialogs' fields (masked) + the last outcome line shown under
    /// the Password card.
    sec_current: Entity<InputState>,
    sec_new: Entity<InputState>,
    sec_confirm: Entity<InputState>,
    security_status: Option<String>,
    /// The Notebooks tab's rename dialog: its field, and the target's dir.
    nb_rename_input: Entity<InputState>,
    nb_rename_target: Option<String>,
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
            Opt::new("light", &t!("settings.opt.light")),
            Opt::new("dark", &t!("settings.opt.dark")),
            Opt::new("auto", &t!("settings.opt.auto_appearance")),
        ];

        let theme_select = make_select(t_opts, &active_skin, window, cx);
        let appearance_select = make_select(a_opts, mode.as_str(), window, cx);
        let (f_opts, current_font) = font_opts(&app, cx);
        let font_select = make_select(f_opts, &current_font, window, cx);
        let (c_opts, current_cursor) = cursor_opts();
        let cursor_select = make_select(c_opts, &current_cursor, window, cx);

        // Note text size (Appearance pane) — one value for all three views.
        let size_opts: Vec<Opt> = crate::app::TEXT_SIZES
            .iter()
            .map(|&s| {
                let id = format!("{s}");
                let label = if s == 16.0 {
                    t!("settings.opt.size_px_default", size = s).to_string()
                } else {
                    t!("settings.opt.size_px", size = s).to_string()
                };
                Opt::new(&id, &label)
            })
            .collect();
        let cur_size = app
            .upgrade()
            .map(|a| format!("{}", f32::from(a.read(cx).text_size())))
            .unwrap_or_else(|| "16".to_string());
        let text_size_select = make_select(size_opts, &cur_size, window, cx);

        let mut subs = Vec::new();
        subs.push(Self::on_theme_select(&theme_select, window, cx));
        subs.push(Self::on_font_select(&font_select, window, cx));
        subs.push(Self::on_cursor_select(&cursor_select, window, cx));
        subs.push(cx.subscribe_in(
            &text_size_select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev
                    && let Ok(size) = id.parse::<f32>()
                    && let Some(app) = this.app.upgrade()
                {
                    app.update(cx, |a, cx| a.set_text_size(size, cx));
                    cx.notify();
                }
            },
        ));
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
                Opt::new("2", &t!("settings.opt.indent_2")),
                Opt::new("4", &t!("settings.opt.indent_4")),
                Opt::new("8", &t!("settings.opt.indent_8")),
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
            .map(|&id| Opt::new(id, &crate::dates::date_format_label(id)))
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
            .map(|&id| Opt::new(id, &crate::dates::time_format_label(id)))
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

        // Settings -> General -> Language. Options: Auto (localized) + each
        // offered locale shown in its own script. Confirming pushes the choice
        // into `AppView::set_language`, which switches the locale live.
        let lang_opts = language_opts();
        let cur_language = app
            .upgrade()
            .map(|a| a.read(cx).language().to_string())
            .unwrap_or_else(|| "auto".to_string());
        let language_select = make_select(lang_opts, &cur_language, window, cx);
        subs.push(cx.subscribe_in(
            &language_select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev
                    && let Some(app) = this.app.upgrade()
                {
                    let id = id.clone();
                    app.update(cx, |a, cx| a.set_language(&id, cx));
                    cx.notify();
                }
            },
        ));

        // Header filter box — dims the cards + nav tabs that don't match as the
        // user types (Baudrun's Settings filter). Subscribed on every keystroke
        // (`Change`) so the dim updates live; the value drives `self.filter`.
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("settings.filter_placeholder")));
        let masked = |ph: &str, window: &mut Window, cx: &mut Context<Self>| {
            let ph = ph.to_string();
            cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(ph))
        };
        let sec_current = masked(&t!("settings.label.current_password"), window, cx);
        let sec_new = masked(&t!("settings.label.new_password"), window, cx);
        let sec_confirm = masked(&t!("settings.label.confirm_password"), window, cx);
        subs.push(cx.subscribe(
            &filter_input,
            |this: &mut SettingsView, input, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let next = input.read(cx).value().trim().to_lowercase();
                    if next != this.filter {
                        this.filter = next;
                        cx.notify();
                    }
                }
            },
        ));

        let nb_rename_input = cx.new(|cx| InputState::new(window, cx));

        Self {
            app,
            theme_select,
            appearance_select,
            font_select,
            cursor_select,
            text_size_select,
            quality_slider,
            indent_select,
            date_format_select,
            time_format_select,
            language_select,
            lang_select_locale: rust_i18n::locale().to_string(),
            filter_input,
            filter: String::new(),
            tab: Tab::Appearance,
            image_gc_result: None,
            cursor_status: None,
            sec_current,
            sec_new,
            sec_confirm,
            security_status: None,
            nb_rename_input,
            nb_rename_target: None,
            _subs: subs,
        }
    }

    /// Tell every note window the registry changed (chip + title refresh).
    fn notify_app_notebooks(&self, cx: &mut Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |a, cx| a.notebooks_changed(cx));
        }
    }

    /// Confirm and relaunch into `nb` (Settings → Notebooks "Switch").
    fn switch_notebook(
        &mut self,
        nb: crate::paths::Notebook,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if nb.is_active() {
            return;
        }
        let fresh = !std::path::Path::new(&nb.dir).join("zorite.db").exists();
        let (title, body): (SharedString, String) = if fresh {
            (
                t!("settings.dlg.create_notebook_title").into(),
                t!(
                    "settings.dlg.create_notebook_body",
                    name = nb.name.as_str(),
                    dir = nb.dir.as_str()
                )
                .into_owned(),
            )
        } else {
            (
                t!("settings.dlg.switch_notebook_title").into(),
                t!(
                    "settings.dlg.switch_notebook_body",
                    name = nb.name.as_str(),
                    dir = nb.dir.as_str()
                )
                .into_owned(),
            )
        };
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let nb = nb.clone();
            let body = body.clone();
            dialog
                .title(title.clone())
                .description(SharedString::from(body))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("settings.btn.relaunch"))
                        .cancel_text(t!("common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    match crate::paths::switch_notebook(&nb.dir) {
                        Ok(()) => crate::app::relaunch(cx),
                        Err(e) => log::error!("switch notebook failed: {e}"),
                    }
                    true
                })
        });
    }

    /// "Add notebook…": pick a folder — empty starts fresh, one holding a
    /// `zorite.db` opens as-is — register it, and offer the relaunch.
    fn add_notebook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("settings.dlg.use_folder").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                match crate::paths::register_dir(&dir) {
                    Ok(nb) => {
                        this.notify_app_notebooks(cx);
                        cx.notify();
                        this.switch_notebook(nb, window, cx);
                    }
                    Err(e) => this.alert(t!("settings.alert.cant_use_folder"), e, window, cx),
                }
            });
        })
        .detach();
    }

    /// The row's Rename button: a small input dialog, committed to the
    /// registry (and the notebook's own name sidecar).
    fn rename_notebook(
        &mut self,
        nb: crate::paths::Notebook,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.nb_rename_target = Some(nb.dir.clone());
        self.nb_rename_input
            .update(cx, |s, cx| s.set_value(nb.name, window, cx));
        let input = self.nb_rename_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_body = input.clone();
            let input_btn = input.clone();
            let input_key = input.clone();
            let weak_btn = weak.clone();
            let weak_key = weak.clone();
            dialog
                .title(t!("settings.dlg.rename_notebook_title"))
                .w(px(420.0))
                .child(Input::new(&input_body))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("nb-rn-cancel")
                                .label(t!("common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("nb-rn-ok")
                                .primary()
                                .label(t!("common.rename"))
                                .on_click(move |_, window, cx| {
                                    let name = input_btn.read(cx).value().to_string();
                                    let _ = weak_btn
                                        .update(cx, |this, cx| this.commit_nb_rename(name, cx));
                                    window.close_dialog(cx);
                                }),
                        ),
                )
                .on_ok(move |_, _window, cx| {
                    let name = input_key.read(cx).value().to_string();
                    let _ = weak_key.update(cx, |this, cx| this.commit_nb_rename(name, cx));
                    true
                })
                .on_cancel(|_, _window, _cx| true)
        });
        self.nb_rename_input.update(cx, |s, cx| s.focus(window, cx));
    }

    fn commit_nb_rename(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(dir) = self.nb_rename_target.take() else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Err(e) = crate::paths::rename_notebook(&dir, name) {
            log::error!("rename notebook: {e}");
        }
        self.notify_app_notebooks(cx);
        cx.notify();
    }

    /// Remove from the list — never touches the notebook's files.
    fn forget_notebook(&mut self, nb: crate::paths::Notebook, cx: &mut Context<Self>) {
        if nb.is_active() {
            return;
        }
        if let Err(e) = crate::paths::forget_notebook(&nb.dir) {
            log::error!("forget notebook: {e}");
        }
        self.notify_app_notebooks(cx);
        cx.notify();
    }

    /// One notebook in the Notebooks card: name + path on the left, its
    /// actions on the right (the active row swaps Switch/Remove for a
    /// "current" tag).
    fn notebook_settings_row(
        &self,
        nb: crate::paths::Notebook,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = nb.is_active();
        let name = nb.name.clone();
        let dir = nb.dir.clone();
        let nb_switch = nb.clone();
        let nb_rename = nb.clone();
        let nb_forget = nb.clone();
        let reveal_dir = PathBuf::from(&nb.dir);
        let mut actions = div().flex().flex_row().items_center().gap(px(6.0));
        if active {
            actions = actions.child(
                div()
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .bg(theme::accent_tint())
                    .text_size(px(11.0))
                    .text_color(theme::accent())
                    .child(t!("settings.label.current").to_string()),
            );
        } else {
            actions = actions.child(nb_button(
                SharedString::from(format!("nb-switch:{}", nb.dir)),
                &t!("settings.nb.switch"),
                cx,
                move |this, window, cx| this.switch_notebook(nb_switch.clone(), window, cx),
            ));
        }
        actions = actions
            .child(nb_button(
                SharedString::from(format!("nb-rename:{}", nb.dir)),
                &t!("settings.nb.rename"),
                cx,
                move |this, window, cx| this.rename_notebook(nb_rename.clone(), window, cx),
            ))
            .child(nb_button(
                SharedString::from(format!("nb-reveal:{}", nb.dir)),
                &t!("settings.nb.reveal"),
                cx,
                move |_this, _window, _cx| {
                    crate::app::AppView::reveal_folder(&reveal_dir);
                },
            ));
        if !active {
            actions = actions.child(nb_button(
                SharedString::from(format!("nb-forget:{}", nb.dir)),
                &t!("settings.nb.remove"),
                cx,
                move |this, _window, cx| this.forget_notebook(nb_forget.clone(), cx),
            ));
        }
        div()
            .px(px(10.0))
            .py(px(8.0))
            .rounded(px(8.0))
            .bg(theme::glass())
            .border_1()
            .border_color(theme::border_subtle())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(theme::text_primary())
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme::text_tertiary())
                            .truncate()
                            .child(dir),
                    ),
            )
            .child(actions)
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

    /// Subscribe to the font `Select`'s confirm → apply the picked family.
    fn on_font_select(
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
                        app.update(cx, |a, cx| a.set_ui_font(id, window, cx));
                        cx.notify();
                    }
                }
            },
        )
    }

    /// Pick a font file, import it via the app (validate / copy / apply), and
    /// rebuild the font dropdown so the new family shows up selected.
    fn choose_font_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("settings.dlg.use_font").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(app) = this.app.upgrade() {
                    app.update(cx, |a, cx| {
                        a.add_ui_font_file(path, window, cx);
                    });
                }
                this.rebuild_font_select(window, cx);
            });
        })
        .detach();
    }

    fn rebuild_font_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (opts, current) = font_opts(&self.app, cx);
        let select = make_select(opts, &current, window, cx);
        self._subs.push(Self::on_font_select(&select, window, cx));
        self.font_select = select;
        cx.notify();
    }

    /// Rebuild the Language picker options after a live locale switch (the
    /// "Auto" entry's label is itself localized). The selected id is unchanged.
    fn rebuild_language_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .app
            .upgrade()
            .map(|a| a.read(cx).language().to_string())
            .unwrap_or_else(|| "auto".to_string());
        let select = make_select(language_opts(), &current, window, cx);
        self._subs.push(cx.subscribe_in(
            &select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev
                    && let Some(app) = this.app.upgrade()
                {
                    let id = id.clone();
                    app.update(cx, |a, cx| a.set_language(&id, cx));
                    cx.notify();
                }
            },
        ));
        self.language_select = select;
        self.lang_select_locale = rust_i18n::locale().to_string();
        cx.notify();
    }

    /// Subscribe to the cursor `Select`'s confirm → persist + apply the pack
    /// (live on macOS/Windows; next launch on Linux).
    fn on_cursor_select(
        select: &Entity<SelectState<Vec<Opt>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            select,
            window,
            |this: &mut SettingsView, _, ev: &SelectEvent<Vec<Opt>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = ev {
                    crate::cursors::set_selected((!id.is_empty()).then_some(id.as_str()));
                    this.cursor_status = None;
                    cx.notify();
                }
            },
        )
    }

    /// Pick an XCursor theme folder, import it into the managed `cursors/`
    /// dir, and select it (or surface the import error under the card).
    fn choose_cursor_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("settings.dlg.use_theme").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                match crate::cursors::import(&path) {
                    Ok(name) => {
                        // An SVG-only pack has no fixed-color variant — select
                        // its theme-reactive entry instead.
                        if crate::cursors::available().contains(&name) {
                            crate::cursors::set_selected(Some(&name));
                        } else {
                            crate::cursors::set_selected(Some(&format!(
                                "{}{name}",
                                crate::cursors::THEME_PREFIX
                            )));
                        }
                        this.cursor_status = None;
                    }
                    Err(e) => this.cursor_status = Some(e),
                }
                this.rebuild_cursor_select(window, cx);
            });
        })
        .detach();
    }

    fn rebuild_cursor_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (opts, current) = cursor_opts();
        let select = make_select(opts, &current, window, cx);
        self._subs.push(Self::on_cursor_select(&select, window, cx));
        self.cursor_select = select;
        cx.notify();
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
            prompt: Some(t!("settings.dlg.choose").into()),
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

    /// Scan for unused images and confirm before deleting — the list of
    /// doomed files is shown, since this is destructive and undo-less.
    fn confirm_image_gc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let orphans = app.read(cx).orphan_images();
        if orphans.is_empty() {
            self.image_gc_result = Some(t!("settings.status.no_unused").into_owned());
            cx.notify();
            return;
        }
        let total: u64 = orphans.iter().map(|(_, s)| s).sum();
        const SHOWN: usize = 15;
        let mut listing: Vec<String> = orphans
            .iter()
            .take(SHOWN)
            .map(|(n, s)| format!("•  {n}  ({})", fmt_size(*s)))
            .collect();
        if orphans.len() > SHOWN {
            listing.push(format!(
                "{}",
                t!(
                    "settings.status.and_more",
                    n = (orphans.len() - SHOWN) as i64
                )
            ));
        }
        let body = if orphans.len() == 1 {
            t!(
                "settings.dlg.image_gc_body_one",
                count = orphans.len() as i64,
                size = fmt_size(total),
                listing = listing.join("\n")
            )
            .into_owned()
        } else {
            t!(
                "settings.dlg.image_gc_body_many",
                count = orphans.len() as i64,
                size = fmt_size(total),
                listing = listing.join("\n")
            )
            .into_owned()
        };
        let weak_app = self.app.clone();
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let orphans = orphans.clone();
            let weak_app = weak_app.clone();
            let this = this.clone();
            dialog
                .title(t!("settings.dlg.image_gc_title"))
                .description(SharedString::from(body.clone()))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("settings.btn.move_to_trash"))
                        .cancel_text(t!("common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    let Some(app) = weak_app.upgrade() else {
                        return true;
                    };
                    let (removed, freed) = app.read(cx).remove_orphan_images(&orphans);
                    let _ = this.update(cx, |s, cx| {
                        s.image_gc_result = Some(if removed == 1 {
                            t!(
                                "settings.status.moved_one",
                                n = removed as i64,
                                size = fmt_size(freed)
                            )
                            .into_owned()
                        } else {
                            t!(
                                "settings.status.moved_many",
                                n = removed as i64,
                                size = fmt_size(freed)
                            )
                            .into_owned()
                        });
                        cx.notify();
                    });
                    true
                })
        });
    }

    /// Open the set/change/remove password dialog. Validation runs on OK;
    /// failures surface as an alert and nothing changes.
    fn open_password_dialog(&mut self, mode: PwMode, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.sec_current, &self.sec_new, &self.sec_confirm] {
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        let (title, ok_label): (SharedString, SharedString) = match mode {
            PwMode::Set => (
                t!("settings.dlg.set_pw_title").into(),
                t!("settings.btn.encrypt").into(),
            ),
            PwMode::Change => (
                t!("settings.dlg.change_pw_title").into(),
                t!("settings.btn.change").into(),
            ),
            PwMode::Remove => (
                t!("settings.dlg.remove_pw_title").into(),
                t!("settings.btn.decrypt").into(),
            ),
        };
        let current = self.sec_current.clone();
        let newpw = self.sec_new.clone();
        let confirm = self.sec_confirm.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let current_i = current.clone();
            let new_i = newpw.clone();
            let confirm_i = confirm.clone();
            let weak = weak.clone();
            let mut body = div().flex().flex_col().gap(px(10.0));
            if mode != PwMode::Set {
                body = body.child(Input::new(&current_i));
            }
            if mode != PwMode::Remove {
                body = body.child(Input::new(&new_i)).child(Input::new(&confirm_i));
            }
            if mode == PwMode::Set {
                body = body.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme::text_tertiary())
                        .child(t!("settings.dlg.set_pw_note").to_string()),
                );
            }
            dialog
                .title(title.clone())
                .w(px(440.0))
                .child(body)
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("pw-cancel")
                                .label(t!("common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("pw-ok")
                                .primary()
                                .label(ok_label.clone())
                                .on_click({
                                    let current_i = current_i.clone();
                                    let new_i = new_i.clone();
                                    let confirm_i = confirm_i.clone();
                                    let weak = weak.clone();
                                    move |_, window, cx| {
                                        let cur = current_i.read(cx).value().to_string();
                                        let new = new_i.read(cx).value().to_string();
                                        let conf = confirm_i.read(cx).value().to_string();
                                        window.close_dialog(cx);
                                        let _ = weak.update(cx, |this, cx| {
                                            this.apply_password_change(
                                                mode, cur, new, conf, window, cx,
                                            );
                                        });
                                    }
                                }),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let cur = current_i.read(cx).value().to_string();
                    let new = new_i.read(cx).value().to_string();
                    let conf = confirm_i.read(cx).value().to_string();
                    let _ = weak.update(cx, |this, cx| {
                        this.apply_password_change(mode, cur, new, conf, window, cx);
                    });
                    true
                })
                .on_cancel(|_, _window, _cx| true)
        });
        let first = if mode == PwMode::Set {
            self.sec_new.clone()
        } else {
            self.sec_current.clone()
        };
        first.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Validate and apply a password set/change/removal, updating the status
    /// line under the Password card.
    fn apply_password_change(
        &mut self,
        mode: PwMode,
        current: String,
        new: String,
        confirm: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode != PwMode::Set && !crate::db::Db::verify_key(&current) {
            self.alert(
                t!("settings.alert.wrong_password"),
                t!("settings.alert.wrong_password_body").into_owned(),
                window,
                cx,
            );
            return;
        }
        let new_key = match mode {
            PwMode::Remove => None,
            _ => {
                if new.is_empty() {
                    self.alert(
                        t!("settings.alert.no_password"),
                        t!("settings.alert.no_password_body").into_owned(),
                        window,
                        cx,
                    );
                    return;
                }
                if new != confirm {
                    self.alert(
                        t!("settings.alert.pw_mismatch"),
                        t!("settings.alert.pw_mismatch_body").into_owned(),
                        window,
                        cx,
                    );
                    return;
                }
                Some(new)
            }
        };
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let result = app.update(cx, |a, _| a.set_db_password(new_key.as_deref()));
        self.security_status = Some(match (&result, mode) {
            (Ok(()), PwMode::Set) => t!("settings.status.db_encrypted").into_owned(),
            (Ok(()), PwMode::Change) => t!("settings.status.pw_changed").into_owned(),
            (Ok(()), PwMode::Remove) => t!("settings.status.pw_removed").into_owned(),
            (Err(e), _) => t!("settings.status.failed", err = e.to_string()).into_owned(),
        });
        cx.notify();
    }

    /// Confirm a relocation to `target`, then record it and quit so the change
    /// (and any pending move) applies on the next launch. Move-only: a folder
    /// that already holds a database is a notebook — opening it belongs to the
    /// sidebar switcher, not a silent repoint from here.
    fn confirm_relocation(&mut self, target: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        use crate::paths::Relocation;
        let (title, body, ok): (SharedString, String, SharedString) =
            match crate::paths::plan_relocation(&target) {
                Relocation::NoOp => return,
                Relocation::Invalid(reason) => {
                    self.alert(t!("settings.alert.cant_use_folder"), reason, window, cx);
                    return;
                }
                Relocation::Switch => {
                    self.alert(
                        t!("settings.dlg.that_folder_notebook_title"),
                        t!(
                            "settings.dlg.that_folder_notebook_body",
                            name = target.display().to_string()
                        )
                        .into_owned(),
                        window,
                        cx,
                    );
                    return;
                }
                Relocation::Move => (
                    t!("settings.dlg.move_data_title").into(),
                    t!(
                        "settings.dlg.move_data_body",
                        path = target.display().to_string()
                    )
                    .into_owned(),
                    t!("settings.btn.move_quit").into(),
                ),
            };
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let target = target.clone();
            let body = body.clone();
            dialog
                .title(title.clone())
                .description(SharedString::from(body))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok.clone())
                        .cancel_text(t!("common.cancel"))
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
                .title(t!("settings.dlg.reset_data_title"))
                .description(SharedString::from(
                    t!(
                        "settings.dlg.reset_data_body",
                        path = default.display().to_string()
                    )
                    .into_owned(),
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("settings.btn.reset_quit"))
                        .cancel_text(t!("common.cancel"))
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
        title: impl Into<SharedString>,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title: SharedString = title.into();
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let body = body.clone();
            dialog
                .title(title.clone())
                .description(SharedString::from(body))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("common.ok"))
                        .show_cancel(false),
                )
                .on_ok(|_, _window, _cx| true)
        });
    }

    // ---- Header filter (Baudrun-style): dim cards + tabs that don't match ----

    /// A settings card that fades when it doesn't match the current filter. It
    /// stays interactive - the user can change a dimmed setting without first
    /// clearing the filter.
    fn section_card(
        &self,
        title_key: &str,
        desc_key: &str,
        control: impl IntoElement,
    ) -> gpui::Div {
        card(&t!(title_key), &t!(desc_key), control).opacity(self.filter_opacity(title_key))
    }

    /// Filter-aware wrapper for the shortcut-list cards on the Keyboard pane.
    fn section_list(
        &self,
        title_key: &str,
        desc_key: &str,
        rows: Vec<(String, Vec<&str>)>,
    ) -> gpui::Div {
        card_list(&t!(title_key), &t!(desc_key), rows).opacity(self.filter_opacity(title_key))
    }

    fn filter_opacity(&self, title_key: &str) -> f32 {
        if self.section_matches(title_key) {
            1.0
        } else {
            0.3
        }
    }

    /// Whether `title_key`'s card matches the filter: an empty filter matches
    /// all; otherwise the localized title or its `SECTIONS` keywords must
    /// contain the text.
    fn section_matches(&self, title_key: &str) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        if t!(title_key).to_lowercase().contains(self.filter.as_str()) {
            return true;
        }
        SECTIONS
            .iter()
            .find(|(_, t, _)| *t == title_key)
            .is_some_and(|(_, _, kw)| kw.contains(self.filter.as_str()))
    }

    /// Whether `tab` has at least one matching card - drives the rail dim.
    fn tab_has_matches(&self, tab: Tab) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        SECTIONS.iter().copied().any(|(t, title_key, kw)| {
            t == tab
                && (t!(title_key).to_lowercase().contains(self.filter.as_str())
                    || kw.contains(self.filter.as_str()))
        })
    }

    /// Left-nav rail. Tabs whose cards all miss the filter render dimmed but
    /// stay clickable, so a typo doesn't lock you out of the other panes.
    fn nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.tab;
        div()
            .flex_shrink_0()
            .w(px(184.0))
            .pl(px(20.0))
            .pr(px(8.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(nav_item(
                "nav-general",
                &t!("settings.nav.general"),
                Tab::General,
                active,
                !self.tab_has_matches(Tab::General),
                cx,
            ))
            .child(nav_item(
                "nav-notebooks",
                &t!("settings.nav.notebooks"),
                Tab::Notebooks,
                active,
                !self.tab_has_matches(Tab::Notebooks),
                cx,
            ))
            .child(nav_item(
                "nav-appearance",
                &t!("settings.nav.appearance"),
                Tab::Appearance,
                active,
                !self.tab_has_matches(Tab::Appearance),
                cx,
            ))
            .child(nav_item(
                "nav-pdf",
                &t!("settings.nav.pdf"),
                Tab::Pdf,
                active,
                !self.tab_has_matches(Tab::Pdf),
                cx,
            ))
            .child(nav_item(
                "nav-markdown",
                &t!("settings.nav.markdown"),
                Tab::Markdown,
                active,
                !self.tab_has_matches(Tab::Markdown),
                cx,
            ))
            .child(nav_item(
                "nav-keyboard",
                &t!("settings.nav.keyboard"),
                Tab::Keyboard,
                active,
                !self.tab_has_matches(Tab::Keyboard),
                cx,
            ))
            .child(nav_item(
                "nav-security",
                &t!("settings.nav.security"),
                Tab::Security,
                active,
                !self.tab_has_matches(Tab::Security),
                cx,
            ))
            .child(nav_item(
                "nav-updates",
                &t!("settings.nav.updates"),
                Tab::Updates,
                active,
                !self.tab_has_matches(Tab::Updates),
                cx,
            ))
    }

    /// The header filter box: a 220px input + a hand-rolled × clear that shows
    /// once there's text (gpui-component's built-in clear icon needs an SVG we
    /// don't bundle — Baudrun's approach).
    fn filter_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .w(px(220.0))
            .child(
                Input::new(&self.filter_input)
                    .small()
                    .appearance(true)
                    .text_size(px(13.0)),
            )
            .when(!self.filter.is_empty(), |row| {
                row.child(
                    div()
                        .id("settings-filter-clear")
                        .absolute()
                        .top(px(0.0))
                        .right(px(8.0))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(15.0))
                        .text_color(theme::text_tertiary())
                        .cursor_pointer()
                        .hover(|h| h.text_color(theme::text_primary()))
                        .child("\u{00D7}")
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseUpEvent, window, cx| {
                                this.filter_input
                                    .update(cx, |state, cx| state.set_value("", window, cx));
                                if !this.filter.is_empty() {
                                    this.filter.clear();
                                    cx.notify();
                                }
                            }),
                        ),
                )
            })
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A live language switch (this window or another) changes rust-i18n's
        // global locale; rebuild the Language picker so its "Auto" label and
        // the resolved selection track the new locale.
        let cur_locale = rust_i18n::locale();
        if self.lang_select_locale != *cur_locale {
            self.rebuild_language_select(window, cx);
        }

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

        // WYSIWYG live-preview toggle (Markdown pane), as a switch. Controlled:
        // `.checked` reflects the persisted setting each render; the click
        // persists + re-applies to open editors via `set_wysiwyg`.
        let wys_on = self
            .app
            .upgrade()
            .map(|a| a.read(cx).wysiwyg())
            .unwrap_or(true);
        let wys_app = self.app.clone();
        let wysiwyg_switch = Switch::new("wysiwyg-toggle")
            .small()
            .checked(wys_on)
            .on_click(move |checked, _window, cx| {
                if let Some(app) = wys_app.upgrade() {
                    app.update(cx, |a, cx| a.set_wysiwyg(*checked, cx));
                }
            });

        // Line-number gutter toggle (Markdown pane).
        let ln_on = self
            .app
            .upgrade()
            .map(|a| a.read(cx).line_numbers())
            .unwrap_or(false);
        let ln_app = self.app.clone();
        let line_numbers_switch = Switch::new("line-numbers-toggle")
            .small()
            .checked(ln_on)
            .on_click(move |checked, _window, cx| {
                if let Some(app) = ln_app.upgrade() {
                    app.update(cx, |a, cx| a.set_line_numbers(*checked, cx));
                }
            });

        // Auto-link-as-you-type toggle (Markdown pane).
        let al_on = self
            .app
            .upgrade()
            .map(|a| a.read(cx).auto_link())
            .unwrap_or(false);
        let al_app = self.app.clone();
        let auto_link_switch = Switch::new("auto-link-toggle")
            .small()
            .checked(al_on)
            .on_click(move |checked, _window, cx| {
                if let Some(app) = al_app.upgrade() {
                    app.update(cx, |a, _cx| a.set_auto_link(*checked));
                }
            });

        // Updates pane toggles — switches, like the WYSIWYG one. Controlled by
        // the persisted prefs; the click persists + (for pre-releases) re-checks.
        let check_on = self
            .app
            .upgrade()
            .map(|a| a.read(cx).check_updates())
            .unwrap_or(true);
        let check_app = self.app.clone();
        let check_updates_switch = Switch::new("check-updates-toggle")
            .small()
            .checked(check_on)
            .on_click(move |checked, _window, cx| {
                if let Some(app) = check_app.upgrade() {
                    app.update(cx, |a, _cx| a.set_check_updates(*checked));
                }
            });
        let pre_on = self
            .app
            .upgrade()
            .map(|a| a.read(cx).include_prerelease())
            .unwrap_or(false);
        let pre_app = self.app.clone();
        let prerelease_switch = Switch::new("prerelease-toggle")
            .small()
            .checked(pre_on)
            .on_click(move |checked, _window, cx| {
                if let Some(app) = pre_app.upgrade() {
                    app.update(cx, |a, cx| a.set_include_prerelease(*checked, cx));
                }
            });

        // Font card body: the family dropdown + an import button.
        let font_control = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(Select::new(&self.font_select).small().w_full())
            .child(div().flex().flex_row().child(text_button(
                "font-add",
                &t!("settings.btn.add_font"),
                cx,
                |this, w, cx| this.choose_font_file(w, cx),
            )));

        // Mouse-cursor card body: the pack dropdown + add/reveal actions, the
        // last import error, and (Linux) the relaunch note.
        #[allow(unused_mut)]
        let mut cursor_control = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(Select::new(&self.cursor_select).small().w_full())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(text_button(
                        "cursor-add",
                        &t!("settings.btn.add_cursor"),
                        cx,
                        |this, w, cx| this.choose_cursor_theme(w, cx),
                    ))
                    .child(text_button(
                        "cursor-reveal",
                        &t!("settings.btn.reveal_cursors"),
                        cx,
                        |_this, _w, _cx| {
                            let dir = crate::cursors::cursors_dir();
                            let _ = std::fs::create_dir_all(&dir);
                            crate::app::AppView::reveal_folder(&dir);
                        },
                    )),
            )
            .children(self.cursor_status.clone().map(|msg| {
                div()
                    .text_size(px(12.0))
                    .text_color(theme::text_tertiary())
                    .child(msg)
            }));
        #[cfg(target_os = "linux")]
        {
            cursor_control = cursor_control.child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::text_tertiary())
                    .child(t!("settings.note.cursor_relaunch").to_string()),
            );
        }

        // Installed-themes card body: the actions + the list (or empty state).
        let actions = div()
            .flex()
            .flex_row()
            .gap(px(8.0))
            .child(text_button(
                "reveal-themes",
                &t!("settings.btn.reveal_themes"),
                cx,
                |this, _w, cx| {
                    if let Some(app) = this.app.upgrade() {
                        app.read(cx).reveal_themes_folder();
                    }
                },
            ))
            .child(text_button(
                "reload-themes",
                &t!("settings.btn.reload"),
                cx,
                |this, w, cx| this.reload_skins(w, cx),
            ));

        let list = if user_names.is_empty() {
            div()
                .text_size(px(13.0))
                .text_color(theme::text_tertiary())
                .child(t!("settings.note.no_custom_themes").to_string())
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

        // Unused-images card body (General): the cleanup action + its last
        // outcome.
        let image_gc_control = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(div().flex().flex_row().child(text_button(
                "image-gc",
                &t!("settings.btn.cleanup"),
                cx,
                |this, window, cx| this.confirm_image_gc(window, cx),
            )))
            .children(self.image_gc_result.clone().map(|msg| {
                div()
                    .text_size(px(12.0))
                    .text_color(theme::text_tertiary())
                    .child(msg)
            }));

        // Remember-window-position toggle: the sidecar file IS the state (see
        // paths::window_bounds_*), written from the main window's live rect.
        let window_bounds_switch = {
            let weak = cx.entity().downgrade();
            Switch::new("window-bounds")
                .small()
                .checked(crate::paths::window_bounds_enabled())
                .on_click(move |on: &bool, _w, cx| {
                    if *on {
                        let _ = weak.update(cx, |this, cx| {
                            if let Some(app) = this.app.upgrade() {
                                let handle = app.read(cx).window_handle;
                                let _ = handle.update(cx, |_, window, _| {
                                    if let gpui::WindowBounds::Windowed(b)
                                    | gpui::WindowBounds::Maximized(b) = window.window_bounds()
                                    {
                                        crate::paths::save_window_bounds(
                                            f32::from(b.origin.x),
                                            f32::from(b.origin.y),
                                            f32::from(b.size.width),
                                            f32::from(b.size.height),
                                            matches!(
                                                window.window_bounds(),
                                                gpui::WindowBounds::Maximized(_)
                                            ),
                                        );
                                    }
                                });
                            }
                            cx.notify();
                        });
                    } else {
                        crate::paths::clear_window_bounds();
                        let _ = weak.update(cx, |_, cx| cx.notify());
                    }
                })
        };
        // Second row of the same card: restore the tab set on relaunch. On
        // enable, arm the sidecar and have the main window write its current
        // tabs (its render persists on change; force skips the change check).
        let open_tabs_switch = {
            let weak = cx.entity().downgrade();
            Switch::new("open-tabs")
                .small()
                .checked(crate::paths::open_tabs_enabled())
                .on_click(move |on: &bool, _w, cx| {
                    if *on {
                        crate::paths::save_open_tabs("");
                        let _ = weak.update(cx, |this, cx| {
                            if let Some(app) = this.app.upgrade() {
                                app.update(cx, |a, cx| {
                                    a.force_persist_open_tabs();
                                    cx.notify();
                                });
                            }
                            cx.notify();
                        });
                    } else {
                        crate::paths::clear_open_tabs();
                        let _ = weak.update(cx, |_, cx| cx.notify());
                    }
                })
        };
        let labeled_row = |label: &str, control: Switch| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(theme::text_secondary())
                        .child(label.to_string()),
                )
                .child(control)
        };
        let remember_window_control = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(labeled_row(
                &t!("settings.label.window_size_pos"),
                window_bounds_switch,
            ))
            .child(labeled_row(
                &t!("settings.label.open_tabs"),
                open_tabs_switch,
            ));

        // Security cards: the password state drives which actions show.
        let encrypted = crate::db::db_is_encrypted();
        let password_control = {
            let mut row = div().flex().flex_row().flex_wrap().gap(px(8.0));
            if encrypted {
                row = row
                    .child(text_button(
                        "sec-change",
                        &t!("settings.btn.change_password"),
                        cx,
                        |this, w, cx| {
                            this.open_password_dialog(PwMode::Change, w, cx);
                        },
                    ))
                    .child(text_button(
                        "sec-remove",
                        &t!("settings.btn.remove_password"),
                        cx,
                        |this, w, cx| {
                            this.open_password_dialog(PwMode::Remove, w, cx);
                        },
                    ))
                    .child(text_button(
                        "sec-lock",
                        &t!("settings.btn.lock_now"),
                        cx,
                        |_this, _w, cx| {
                            // Deferred: locking closes this window mid-handler.
                            cx.defer(crate::lock_now);
                        },
                    ));
            } else {
                row = row.child(text_button(
                    "sec-set",
                    &t!("settings.btn.set_password"),
                    cx,
                    |this, w, cx| {
                        this.open_password_dialog(PwMode::Set, w, cx);
                    },
                ));
            }
            div().flex().flex_col().gap(px(10.0)).child(row).children(
                self.security_status.clone().map(|msg| {
                    div()
                        .text_size(px(12.0))
                        .text_color(theme::text_tertiary())
                        .child(msg)
                }),
            )
        };
        let remember_control = {
            let weak = cx.entity().downgrade();
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    Switch::new("sec-remember")
                        .small()
                        .checked(encrypted && crate::security::is_remembered())
                        .disabled(!encrypted)
                        .on_click(move |on: &bool, _w, cx| {
                            if !crate::db::db_is_encrypted() {
                                return;
                            }
                            if *on {
                                if let Some(pw) = crate::security::session_key() {
                                    crate::security::remember_password(&pw);
                                }
                            } else {
                                crate::security::forget_password();
                            }
                            let _ = weak.update(cx, |_, cx| cx.notify());
                        }),
                )
                .children((!encrypted).then(|| {
                    div()
                        .text_size(px(12.0))
                        .text_color(theme::text_tertiary())
                        .child(t!("settings.note.set_password_first").to_string())
                }))
        };
        let auto_lock_control = {
            let current = crate::security::auto_lock_minutes();
            let mut row = div().flex().flex_row().flex_wrap().gap(px(6.0));
            for (label, mins) in [
                (t!("settings.autolock.off").to_string(), 0u64),
                (t!("settings.autolock.5min").to_string(), 5),
                (t!("settings.autolock.15min").to_string(), 15),
                (t!("settings.autolock.30min").to_string(), 30),
                (t!("settings.autolock.1hour").to_string(), 60),
            ] {
                let app = self.app.clone();
                let mut chip = div()
                    .id(SharedString::from(format!("autolock-{mins}")))
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .cursor_pointer();
                chip = if encrypted && current == mins {
                    chip.bg(theme::accent_tint()).text_color(theme::accent())
                } else {
                    chip.bg(theme::glass()).text_color(theme::text_secondary())
                };
                row = row.child(
                    chip.on_click(cx.listener(move |_this, _: &gpui::ClickEvent, _w, cx| {
                        if !crate::db::db_is_encrypted() {
                            return;
                        }
                        if let Some(app) = app.upgrade() {
                            app.update(cx, |a, cx| a.set_auto_lock(mins, cx));
                        }
                        cx.notify();
                    }))
                    .child(label),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(row)
                .children((!encrypted).then(|| {
                    div()
                        .text_size(px(12.0))
                        .text_color(theme::text_tertiary())
                        .child(t!("settings.note.set_password_first").to_string())
                }))
        };

        // Notebooks card body: every registered notebook with per-row actions
        // (switch / rename / reveal / remove), then "Add notebook…".
        let notebooks_control = {
            let mut col = div().flex().flex_col().gap(px(8.0));
            for nb in crate::paths::notebooks() {
                col = col.child(self.notebook_settings_row(nb, cx));
            }
            col.child(div().flex().flex_row().mt(px(2.0)).child(text_button(
                "nb-add",
                &t!("settings.btn.add_notebook"),
                cx,
                |this, w, cx| this.add_notebook(w, cx),
            )))
        };

        // Data-location card body (Notebooks): the current path, then move /
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
                        "data-move",
                        &t!("settings.btn.move"),
                        cx,
                        |this, w, cx| this.choose_data_location(w, cx),
                    ))
                    .child(text_button(
                        "data-reveal",
                        &t!("settings.btn.reveal"),
                        cx,
                        |_this, _w, _cx| {
                            crate::app::AppView::reveal_folder(&crate::paths::data_dir());
                        },
                    ))
                    .when(!at_default, |row| {
                        row.child(text_button(
                            "data-reset",
                            &t!("settings.btn.reset_default"),
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
                    .child(t!("settings.status.current_version", ver = cur_version).to_string()),
            );
            if let Some(a) = &available {
                let url = a.html_url.clone();
                col = col.child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::accent())
                        .child(t!("settings.status.update_available", ver = a.version).to_string()),
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
                            &t!("settings.btn.view_release"),
                            cx,
                            move |_this, _w, _cx| open_url(&url),
                        ))
                        .child(text_button(
                            "updates-check",
                            &t!("settings.btn.check_now"),
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
                            .child(t!("settings.note.latest_version").to_string()),
                    )
                    .child(text_button(
                        "updates-check",
                        &t!("settings.btn.check_now"),
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
                            .child(t!("settings.title").to_string()),
                    )
                    .child(version_chip())
                    .child(div().flex_1())
                    .child(self.filter_bar(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(self.nav(cx))
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
                            Tab::Notebooks => content
                                .child(self.section_card(
                                    "settings.section.notebooks",
                                    "settings.desc.notebooks",
                                    notebooks_control,
                                ))
                                .child(self.section_card(
                                    "settings.section.data_location",
                                    "settings.desc.data_location",
                                    location_control,
                                )),
                            Tab::General => content
                                .child(self.section_card(
                                    "settings.section.language",
                                    "settings.desc.language",
                                    Select::new(&self.language_select).small().w_full(),
                                ))
                                .child(self.section_card(
                                    "settings.section.remember_window",
                                    "settings.desc.remember_window",
                                    remember_window_control,
                                ))
                                .child(self.section_card(
                                    "settings.section.unused_images",
                                    "settings.desc.unused_images",
                                    image_gc_control,
                                ))
                                .child(self.section_card(
                                    "settings.section.date_format",
                                    "settings.desc.date_format",
                                    Select::new(&self.date_format_select).small().w_full(),
                                ))
                                .child(self.section_card(
                                    "settings.section.time_format",
                                    "settings.desc.time_format",
                                    Select::new(&self.time_format_select).small().w_full(),
                                )),
                            Tab::Appearance => content
                                .child(self.section_card(
                                    "settings.section.app_theme",
                                    "settings.desc.app_theme",
                                    Select::new(&self.theme_select).small().w_full(),
                                ))
                                .child(self.section_card(
                                    "settings.section.appearance",
                                    "settings.desc.appearance",
                                    Select::new(&self.appearance_select).small().w_full(),
                                ))
                                .child(self.section_card(
                                    "settings.section.font",
                                    "settings.desc.font",
                                    font_control,
                                ))
                                .child(self.section_card(
                                    "settings.section.text_size",
                                    "settings.desc.text_size",
                                    Select::new(&self.text_size_select).small().w_full(),
                                ))
                                .child(self.section_card(
                                    "settings.section.line_numbers",
                                    "settings.desc.line_numbers",
                                    line_numbers_switch,
                                ))
                                .child(self.section_card(
                                    "settings.section.mouse_cursor",
                                    "settings.desc.mouse_cursor",
                                    cursor_control,
                                ))
                                .child(self.section_card(
                                    "settings.section.installed_themes",
                                    "settings.desc.installed_themes",
                                    installed,
                                )),
                            Tab::Pdf => content.child(self.section_card(
                                "settings.section.pdf_quality",
                                "settings.desc.pdf_quality",
                                quality_control,
                            )),
                            Tab::Markdown => content
                                .child(self.section_card(
                                    "settings.section.wysiwyg",
                                    "settings.desc.wysiwyg",
                                    wysiwyg_switch,
                                ))
                                .child(self.section_card(
                                    "settings.section.list_indent",
                                    "settings.desc.list_indent",
                                    Select::new(&self.indent_select).small().w_full(),
                                ))
                                .child(self.section_card(
                                    "settings.section.autolink",
                                    "settings.desc.autolink",
                                    auto_link_switch,
                                )),
                            Tab::Keyboard => {
                                let app_rows: Vec<(String, Vec<&str>)> = vec![
                                    (t!("settings.kb.new_tab").to_string(), vec![keys::MOD, "T"]),
                                    (
                                        t!("settings.kb.new_window").to_string(),
                                        vec![keys::MOD, "N"],
                                    ),
                                    (
                                        t!("settings.kb.close_tab").to_string(),
                                        vec![keys::MOD, "W"],
                                    ),
                                    (
                                        t!("settings.kb.next_tab").to_string(),
                                        vec![keys::CTRL, "Tab"],
                                    ),
                                    (
                                        t!("settings.kb.prev_tab").to_string(),
                                        vec![keys::CTRL, keys::SHIFT, "Tab"],
                                    ),
                                    (
                                        t!("settings.kb.find_in_page").to_string(),
                                        vec![keys::MOD, "F"],
                                    ),
                                    (
                                        t!("settings.kb.search_all").to_string(),
                                        vec![keys::MOD, keys::SHIFT, "F"],
                                    ),
                                    (
                                        t!("settings.kb.fit_images").to_string(),
                                        vec![keys::MOD, keys::SHIFT, "I"],
                                    ),
                                    (
                                        t!("settings.kb.export_pdf").to_string(),
                                        vec![keys::MOD, "P"],
                                    ),
                                    (
                                        t!("settings.kb.open_settings").to_string(),
                                        vec![keys::MOD, ","],
                                    ),
                                    // Windows quits with the OS convention.
                                    #[cfg(target_os = "windows")]
                                    (t!("settings.kb.quit").to_string(), vec!["Alt", "F4"]),
                                    #[cfg(not(target_os = "windows"))]
                                    (t!("settings.kb.quit").to_string(), vec![keys::MOD, "Q"]),
                                ];
                                let edit_rows: Vec<(String, Vec<&str>)> = vec![
                                    (t!("settings.kb.slash_menu").to_string(), vec!["/"]),
                                    (t!("settings.kb.menu_nav").to_string(), vec!["↑", "↓"]),
                                    (t!("settings.kb.menu_insert").to_string(), vec!["Enter"]),
                                    (t!("settings.kb.menu_close").to_string(), vec!["Esc"]),
                                    (t!("settings.kb.indent").to_string(), vec!["Tab"]),
                                    (
                                        t!("settings.kb.outdent").to_string(),
                                        vec![keys::SHIFT, "Tab"],
                                    ),
                                    (t!("settings.kb.copy").to_string(), vec![keys::MOD, "C"]),
                                    (t!("settings.kb.cut").to_string(), vec![keys::MOD, "X"]),
                                    (t!("settings.kb.paste").to_string(), vec![keys::MOD, "V"]),
                                    (t!("settings.kb.undo").to_string(), vec![keys::MOD, "Z"]),
                                    (t!("settings.kb.redo").to_string(), keys::redo()),
                                    (
                                        t!("settings.kb.select_all").to_string(),
                                        vec![keys::MOD, "A"],
                                    ),
                                ];
                                let wb_tool_rows: Vec<(String, Vec<&str>)> = vec![
                                    (t!("settings.kb.select").to_string(), vec!["V"]),
                                    (t!("settings.kb.pan").to_string(), vec!["H"]),
                                    (t!("settings.kb.pen").to_string(), vec!["P"]),
                                    (t!("settings.kb.rectangle").to_string(), vec!["R"]),
                                    (t!("settings.kb.ellipse").to_string(), vec!["O"]),
                                    (t!("settings.kb.diamond").to_string(), vec!["D"]),
                                    (t!("settings.kb.triangle").to_string(), vec!["G"]),
                                    (t!("settings.kb.rounded_rect").to_string(), vec!["U"]),
                                    (t!("settings.kb.star").to_string(), vec!["S"]),
                                    (t!("settings.kb.hexagon").to_string(), vec!["X"]),
                                    (t!("settings.kb.line").to_string(), vec!["L"]),
                                    (t!("settings.kb.arrow").to_string(), vec!["A"]),
                                    (t!("settings.kb.text").to_string(), vec!["T"]),
                                    (t!("settings.kb.image").to_string(), vec!["I"]),
                                ];
                                let wb_edit_rows: Vec<(String, Vec<&str>)> = vec![
                                    (t!("settings.kb.undo").to_string(), vec![keys::MOD, "Z"]),
                                    (t!("settings.kb.redo").to_string(), keys::redo()),
                                    (t!("settings.kb.copy").to_string(), vec![keys::MOD, "C"]),
                                    (t!("settings.kb.cut").to_string(), vec![keys::MOD, "X"]),
                                    (t!("settings.kb.paste").to_string(), vec![keys::MOD, "V"]),
                                    (
                                        t!("settings.kb.bring_forward").to_string(),
                                        vec![keys::MOD, "]"],
                                    ),
                                    (
                                        t!("settings.kb.bring_front").to_string(),
                                        vec![keys::MOD, keys::SHIFT, "]"],
                                    ),
                                    (
                                        t!("settings.kb.send_backward").to_string(),
                                        vec![keys::MOD, "["],
                                    ),
                                    (
                                        t!("settings.kb.send_back").to_string(),
                                        vec![keys::MOD, keys::SHIFT, "["],
                                    ),
                                    (t!("settings.kb.delete_sel").to_string(), vec!["Delete"]),
                                    (t!("settings.kb.deselect").to_string(), vec!["Esc"]),
                                ];
                                let pdf_rows: Vec<(String, Vec<&str>)> = vec![
                                    (t!("settings.kb.next_page").to_string(), vec!["PageDown"]),
                                    (t!("settings.kb.prev_page").to_string(), vec!["PageUp"]),
                                    (t!("settings.kb.first_page").to_string(), vec!["Home"]),
                                    (t!("settings.kb.last_page").to_string(), vec!["End"]),
                                    (t!("settings.kb.zoom_in").to_string(), vec![keys::MOD, "="]),
                                    (t!("settings.kb.zoom_out").to_string(), vec![keys::MOD, "−"]),
                                    (
                                        t!("settings.kb.reset_zoom").to_string(),
                                        vec![keys::MOD, "0"],
                                    ),
                                    (t!("settings.kb.find").to_string(), vec![keys::MOD, "F"]),
                                    (
                                        t!("settings.kb.next_match").to_string(),
                                        vec![keys::MOD, "G"],
                                    ),
                                    (
                                        t!("settings.kb.prev_match").to_string(),
                                        vec![keys::MOD, keys::SHIFT, "G"],
                                    ),
                                    (
                                        t!("settings.kb.toggle_highlight").to_string(),
                                        vec![keys::MOD, keys::SHIFT, "H"],
                                    ),
                                    (
                                        t!("settings.kb.go_to_page").to_string(),
                                        vec![keys::MOD, keys::ALT, "G"],
                                    ),
                                ];
                                content
                                    .child(self.section_list(
                                        "settings.section.application",
                                        "settings.desc.application",
                                        app_rows,
                                    ))
                                    .child(self.section_list(
                                        "settings.section.editing",
                                        "settings.desc.editing",
                                        edit_rows,
                                    ))
                                    .child(self.section_list(
                                        "settings.section.wb_tools",
                                        "settings.desc.wb_tools",
                                        wb_tool_rows,
                                    ))
                                    .child(self.section_list(
                                        "settings.section.wb_editing",
                                        "settings.desc.wb_editing",
                                        wb_edit_rows,
                                    ))
                                    .child(self.section_list(
                                        "settings.section.pdf_viewer",
                                        "settings.desc.pdf_viewer",
                                        pdf_rows,
                                    ))
                            }
                            Tab::Security => content
                                .child(self.section_card(
                                    "settings.section.password",
                                    "settings.desc.password",
                                    password_control,
                                ))
                                .child(self.section_card(
                                    "settings.section.remember_device",
                                    if cfg!(target_os = "linux") {
                                        "settings.desc.remember_device_linux"
                                    } else {
                                        "settings.desc.remember_device"
                                    },
                                    remember_control,
                                ))
                                .child(self.section_card(
                                    "settings.section.autolock",
                                    "settings.desc.autolock",
                                    auto_lock_control,
                                )),
                            Tab::Updates => content
                                .child(self.section_card(
                                    "settings.section.updates",
                                    "settings.desc.updates",
                                    updates_control,
                                ))
                                .child(self.section_card(
                                    "settings.section.auto_check",
                                    "settings.desc.auto_check",
                                    check_updates_switch,
                                ))
                                .child(self.section_card(
                                    "settings.section.prereleases",
                                    "settings.desc.prereleases",
                                    prerelease_switch,
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

/// Open a URL in the user's default browser (the "View release" button).
///
/// The URL comes from the GitHub release JSON, so it's only as trustworthy as
/// that response — and it's handed to `open`/`explorer`, which happily take a
/// local path or a leading `-` as a flag. Require https before spawning.
fn open_url(url: &str) {
    if !gpui_markdown::syntax::is_safe_external_url(url) {
        log::warn!("refusing to open non-https url from release metadata");
        return;
    }
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
    label: &str,
    tab: Tab,
    active: Tab,
    dimmed: bool,
    cx: &mut Context<SettingsView>,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .text_size(px(14.0))
        .cursor_pointer()
        .when(dimmed, |d| d.opacity(0.35))
        .when(tab == active, |d| {
            d.bg(theme::accent_tint()).text_color(theme::text_primary())
        })
        .when(tab != active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()))
        })
        .child(label.to_string())
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
fn card(title: &str, desc: &str, control: impl IntoElement) -> gpui::Div {
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
fn card_list(title: &str, desc: &str, rows: Vec<(String, Vec<&str>)>) -> gpui::Div {
    let mut list = div().flex().flex_col().gap(px(2.0));
    for (label, combo) in rows {
        list = list.child(shortcut_row(&label, &combo));
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

/// Human size for the images-GC listing: KB under a megabyte, else MB.
fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{:.0} KB", (bytes as f64 / 1024.0).max(1.0))
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// [`text_button`]'s small sibling for the notebook rows: a dynamic id (one
/// per notebook) and tighter padding.
fn nb_button(
    id: SharedString,
    label: &str,
    cx: &mut Context<SettingsView>,
    on: impl Fn(&mut SettingsView, &mut Window, &mut Context<SettingsView>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::glass())
        .text_color(theme::text_secondary())
        .text_size(px(12.0))
        .cursor_pointer()
        .hover(|h| {
            h.bg(theme::glass_strong())
                .text_color(theme::text_primary())
        })
        .child(label.to_string())
        .on_click(cx.listener(move |this, _, window, cx| on(this, window, cx)))
}

fn text_button(
    id: &'static str,
    label: &str,
    cx: &mut Context<SettingsView>,
    on: impl Fn(&mut SettingsView, &mut Window, &mut Context<SettingsView>) + 'static,
) -> impl IntoElement {
    // Sized to sit beside gpui-component's `.small()` controls (the cards'
    // dropdowns/switches) — a notch above the per-notebook row buttons.
    div()
        .id(id)
        .px(px(10.0))
        .py(px(5.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::glass())
        .text_color(theme::text_secondary())
        .text_size(px(12.0))
        .cursor_pointer()
        .hover(|h| {
            h.bg(theme::glass_strong())
                .text_color(theme::text_primary())
        })
        .child(label.to_string())
        .on_click(cx.listener(move |this, _, window, cx| on(this, window, cx)))
}
