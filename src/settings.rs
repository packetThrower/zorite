//! The Settings window, built on gpui-component's `setting` module: a
//! sidebar of pages with a search box, and cards (`SettingGroup`s) of typed
//! fields — switches and dropdowns that read the live value from `AppView`
//! and write back through its setters — plus custom rows for the parts that
//! aren't a single control (notebooks, the data folder, passwords, updates,
//! the shortcut lists). Typed fields carry their default, so a page shows a
//! reset button whenever something differs from it.

use std::path::PathBuf;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Root, Sizable, TitleBar, WindowExt,
    button::{Button, ButtonVariants as _},
    dialog::{DialogButtonProps, DialogFooter},
    input::{Input, InputState},
    setting::{SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    slider::{Slider, SliderEvent, SliderState},
};

use crate::app::AppView;
use crate::theme::{self, Mode};
use rust_i18n::t;

/// Dropdown choices as gpui-component's setting fields take them: `(stored
/// value, label)`.
type Options = Vec<(SharedString, SharedString)>;

fn opt(id: &str, label: impl Into<SharedString>) -> (SharedString, SharedString) {
    (SharedString::from(id.to_string()), label.into())
}

fn theme_options(app: &WeakEntity<AppView>, cx: &App) -> (Options, SharedString) {
    if let Some(a) = app.upgrade() {
        let a = a.read(cx);
        (
            a.skins()
                .iter()
                .map(|s| opt(&s.id, s.name.clone()))
                .collect(),
            a.active_skin_id().to_string().into(),
        )
    } else {
        (Vec::new(), SharedString::default())
    }
}

/// Installed font families, once: gpui appends its internal fallback aliases
/// (".ZedMono", ".ZedSans", ".SystemUIFont") to the OS list. They only render
/// inside Zed, which bundles the font files those aliases point at — here
/// they'd silently fall back to the default face, so don't offer them
/// ("Default (System)" already covers the system face).
fn installed_font_names(cx: &App) -> Vec<String> {
    let mut names = cx.text_system().all_font_names();
    names.retain(|n| !n.starts_with('.'));
    names.sort();
    names.dedup();
    names
}

/// Font-dropdown choices: Default (named for what it resolves to — the active
/// theme's font, else the system face), then every installed family (including
/// the user-added ones registered at startup / via "Add font file…").
fn font_options(app: &WeakEntity<AppView>, names: &[String], cx: &App) -> (Options, SharedString) {
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
    let mut opts = vec![opt("", default_label)];
    opts.extend(names.iter().map(|n| opt(n, n.clone())));
    let current = app
        .upgrade()
        .map(|a| a.read(cx).ui_font().to_string())
        .unwrap_or_default();
    (opts, current.into())
}

/// Cursor-theme dropdown choices: System, then the bundled pack and every
/// user-added pack on disk (see `cursors::available`). The pack lists are
/// scanned once and after an import, not per frame.
fn cursor_options(packs: &[String], reactive: &[String]) -> (Options, SharedString) {
    let mut opts = vec![
        opt("", t!("settings.opt.cursor_system").to_string()),
        opt(
            crate::cursors::THEME_PACK,
            t!("settings.opt.cursor_bibata").to_string(),
        ),
    ];
    opts.extend(packs.iter().map(|n| opt(n, n.clone())));
    // User packs with SVG sources render theme-reactively as a second entry.
    opts.extend(reactive.iter().map(|n| {
        opt(
            &format!("{}{n}", crate::cursors::THEME_PREFIX),
            format!("{n} (match theme)"),
        )
    }));
    (opts, crate::cursors::selected().unwrap_or_default().into())
}

/// Language-picker choices: Auto (its label is itself localized, so the
/// dropdown reads "自动" / "Auto" with the active locale) plus each offered
/// locale shown in its own script. The persisted ids come from
/// [`crate::i18n::LANGUAGE_OPTS`]. Built per frame, so a live language switch
/// relabels it without reopening Settings.
fn language_options() -> Options {
    crate::i18n::LANGUAGE_OPTS
        .iter()
        .map(|(id, name)| {
            if *id == "auto" {
                opt(id, t!("settings.language.auto").to_string())
            } else {
                opt(id, (*name).to_string())
            }
        })
        .collect()
}

/// Which password dialog is open: first-time set, change, or removal.
#[derive(Clone, Copy, PartialEq)]
enum PwMode {
    Set,
    Change,
    Remove,
}

/// Extra search words per card (keyed by its title's i18n key). The sidebar
/// search matches an item's title, description, and keywords; every item of a
/// card also gets the card's own title + description as keywords, so this is
/// only for synonyms a user might type that aren't already in that text —
/// kept in English so an English-typed synonym finds a card in any locale.
const KEYWORDS: &[(&str, &str)] = &[
    (
        "settings.section.notebooks",
        "vault workspace switch add remove rename folder data set",
    ),
    (
        "settings.section.data_location",
        "folder path database directory move attachments notebook vault",
    ),
    (
        "settings.section.unused_images",
        "cleanup delete orphan gc attachments storage space free",
    ),
    (
        "settings.section.language",
        "language locale i18n chinese english 中文 简体",
    ),
    (
        "settings.section.remember_window",
        "bounds size resize reopen restore placement screen monitor position tabs session",
    ),
    (
        "settings.section.date_format",
        "iso us european calendar day month year /date",
    ),
    (
        "settings.section.time_format",
        "24 hour 12 clock am pm /time",
    ),
    (
        "settings.section.app_theme",
        "skin colors palette built-in custom",
    ),
    (
        "settings.section.appearance",
        "light dark auto system mode variant",
    ),
    (
        "settings.section.font",
        "typeface family typography text ttf otf custom",
    ),
    (
        "settings.section.text_size",
        "font size zoom bigger smaller larger scale px",
    ),
    (
        "settings.section.line_numbers",
        "gutter source rows numbering count editor",
    ),
    (
        "settings.section.mouse_cursor",
        "pointer arrow theme pack xcursor bibata custom",
    ),
    (
        "settings.section.sidebar_position",
        "left right side dock rail move rtl navigation panel",
    ),
    (
        "settings.section.installed_themes",
        "custom user json reload reveal folder",
    ),
    (
        "settings.section.pdf_quality",
        "dpi resolution sharpness speed scale render",
    ),
    (
        "settings.section.wysiwyg",
        "live preview inline formatting bold heading links",
    ),
    (
        "settings.section.list_indent",
        "spaces tab nesting bullets levels",
    ),
    (
        "settings.section.autolink",
        "wiki link auto complete title page",
    ),
    (
        "settings.section.application",
        "keyboard shortcuts hotkeys keys",
    ),
    (
        "settings.section.editing",
        "keyboard shortcuts hotkeys keys editor slash",
    ),
    (
        "settings.section.wb_tools",
        "keyboard shortcuts hotkeys whiteboard tools",
    ),
    (
        "settings.section.wb_editing",
        "keyboard shortcuts hotkeys whiteboard",
    ),
    (
        "settings.section.pdf_viewer",
        "keyboard shortcuts hotkeys pdf zoom",
    ),
    (
        "settings.section.password",
        "encrypt encryption sqlcipher lock security",
    ),
    (
        "settings.section.remember_device",
        "keychain credential store password remember",
    ),
    (
        "settings.section.autolock",
        "idle timeout lock minutes security",
    ),
    (
        "settings.section.updates",
        "version release check download new",
    ),
    (
        "settings.section.auto_check",
        "updates automatic startup check",
    ),
    (
        "settings.section.prereleases",
        "beta alpha rc pre-release updates channel",
    ),
];

fn keywords_for(title_key: &str) -> &'static str {
    KEYWORDS
        .iter()
        .find(|(k, _)| *k == title_key)
        .map_or("", |(_, kw)| kw)
}

pub struct SettingsView {
    app: WeakEntity<AppView>,
    quality_slider: Entity<SliderState>,
    /// Installed font families + cursor packs, scanned once (and again after an
    /// import) rather than on every frame the dropdowns are built.
    font_names: Vec<String>,
    cursor_packs: Vec<String>,
    cursor_reactive: Vec<String>,
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
        let mut subs = Vec::new();

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

        let masked = |ph: &str, window: &mut Window, cx: &mut Context<Self>| {
            let ph = ph.to_string();
            cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(ph))
        };
        let sec_current = masked(&t!("settings.label.current_password"), window, cx);
        let sec_new = masked(&t!("settings.label.new_password"), window, cx);
        let sec_confirm = masked(&t!("settings.label.confirm_password"), window, cx);
        let nb_rename_input = cx.new(|cx| InputState::new(window, cx));

        Self {
            font_names: installed_font_names(cx),
            cursor_packs: crate::cursors::available(),
            cursor_reactive: crate::cursors::reactive_available(),
            app,
            quality_slider,
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

    /// Re-run the update check now (Settings → Updates → "Check now").
    fn check_for_updates(&self, cx: &mut Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |a, cx| a.check_for_updates_now(cx));
        }
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
                this.font_names = installed_font_names(cx);
                cx.notify();
            });
        })
        .detach();
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
            let _ = this.update_in(cx, |this, _window, cx| {
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
                this.cursor_packs = crate::cursors::available();
                this.cursor_reactive = crate::cursors::reactive_available();
                cx.notify();
            });
        })
        .detach();
    }

    /// Re-scan themes on disk; the theme dropdown rebuilds its options on the
    /// next frame.
    fn reload_skins(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.update(cx, |a, cx| a.reload_skins(window, cx));
        cx.notify();
    }

    fn user_theme_names(&self, cx: &App) -> Vec<String> {
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

    // --- Pages -----------------------------------------------------------

    fn general_page(&self, weak: &WeakEntity<Self>) -> SettingPage {
        let app = &self.app;
        let lang_app = app.clone();
        let language = dropdown_item(
            "settings.item.language",
            language_options(),
            {
                let app = app.clone();
                move |cx| {
                    app.upgrade()
                        .map(|a| a.read(cx).language().to_string().into())
                        .unwrap_or_else(|| "auto".into())
                }
            },
            move |v, cx| {
                if let Some(a) = lang_app.upgrade() {
                    a.update(cx, |a, cx| a.set_language(&v, cx));
                }
            },
            "auto",
            weak,
        );

        // Remember-window-position toggle: the sidecar file IS the state (see
        // paths::window_bounds_*), written from the main window's live rect.
        let bounds_app = app.clone();
        let window_bounds = SettingItem::new(
            t!("settings.label.window_size_pos").into_owned(),
            SettingField::switch(
                |_cx| crate::paths::window_bounds_enabled(),
                notify_after(weak, move |on, cx| {
                    if on {
                        with_main_window(&bounds_app, cx, |_, window, _| {
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
                    } else {
                        crate::paths::clear_window_bounds();
                    }
                }),
            )
            .default_value(false),
        );
        // Restore the tab set on relaunch. On enable, arm the sidecar and have
        // the main window write its current tabs (its render persists on
        // change; force skips the change check).
        let tabs_app = app.clone();
        let open_tabs = SettingItem::new(
            t!("settings.label.open_tabs").into_owned(),
            SettingField::switch(
                |_cx| crate::paths::open_tabs_enabled(),
                notify_after(weak, move |on, cx| {
                    if on {
                        crate::paths::save_open_tabs("");
                        if let Some(a) = tabs_app.upgrade() {
                            a.update(cx, |a, cx| {
                                a.force_persist_open_tabs();
                                cx.notify();
                            });
                        }
                    } else {
                        crate::paths::clear_open_tabs();
                    }
                }),
            )
            .default_value(false),
        );

        // Unused-images card body: the cleanup action + its last outcome.
        let gc_weak = weak.clone();
        let image_gc = custom(move |_window, cx| {
            let result = gc_weak
                .upgrade()
                .and_then(|v| v.read(cx).image_gc_result.clone());
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(div().flex().flex_row().child(text_button(
                    "image-gc",
                    &t!("settings.btn.cleanup"),
                    &gc_weak,
                    |this, window, cx| this.confirm_image_gc(window, cx),
                )))
                .children(result.map(status_line))
                .into_any_element()
        });

        let date_app = app.clone();
        let date = dropdown_item(
            "settings.item.format",
            crate::dates::DATE_FORMATS
                .iter()
                .map(|&id| opt(id, crate::dates::date_format_label(id)))
                .collect(),
            |_cx| crate::dates::date_format().into(),
            move |v, cx| {
                if let Some(a) = date_app.upgrade() {
                    a.update(cx, |a, _cx| a.set_date_format(&v));
                }
            },
            "iso",
            weak,
        );
        let time_app = app.clone();
        let time = dropdown_item(
            "settings.item.format",
            crate::dates::TIME_FORMATS
                .iter()
                .map(|&id| opt(id, crate::dates::time_format_label(id)))
                .collect(),
            |_cx| crate::dates::time_format().into(),
            move |v, cx| {
                if let Some(a) = time_app.upgrade() {
                    a.update(cx, |a, _cx| a.set_time_format(&v));
                }
            },
            "24h",
            weak,
        );

        SettingPage::new(t!("settings.nav.general").into_owned()).groups([
            card(
                "settings.section.language",
                "settings.desc.language",
                vec![language],
            ),
            card(
                "settings.section.remember_window",
                "settings.desc.remember_window",
                vec![window_bounds, open_tabs],
            ),
            card(
                "settings.section.unused_images",
                "settings.desc.unused_images",
                vec![image_gc],
            ),
            card(
                "settings.section.date_format",
                "settings.desc.date_format",
                vec![date],
            ),
            card(
                "settings.section.time_format",
                "settings.desc.time_format",
                vec![time],
            ),
        ])
    }

    fn notebooks_page(&self, weak: &WeakEntity<Self>) -> SettingPage {
        // Every registered notebook with per-row actions (switch / rename /
        // reveal / remove), then "Add notebook…".
        let nb_weak = weak.clone();
        let notebooks = custom(move |_window, _cx| {
            let mut col = div().flex().flex_col().gap(px(8.0));
            for nb in crate::paths::notebooks() {
                col = col.child(notebook_row(nb, &nb_weak));
            }
            col.child(div().flex().flex_row().mt(px(2.0)).child(text_button(
                "nb-add",
                &t!("settings.btn.add_notebook"),
                &nb_weak,
                |this, w, cx| this.add_notebook(w, cx),
            )))
            .into_any_element()
        });

        // The current data path, then move / reveal / reset actions.
        let loc_weak = weak.clone();
        let location = custom(move |_window, _cx| {
            let data_path = crate::paths::data_dir().display().to_string();
            let at_default = crate::paths::is_default_location();
            div()
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
                            &loc_weak,
                            |this, w, cx| this.choose_data_location(w, cx),
                        ))
                        .child(text_button(
                            "data-reveal",
                            &t!("settings.btn.reveal"),
                            &loc_weak,
                            |_this, _w, _cx| {
                                crate::app::AppView::reveal_folder(&crate::paths::data_dir());
                            },
                        ))
                        .when(!at_default, |row| {
                            row.child(text_button(
                                "data-reset",
                                &t!("settings.btn.reset_default"),
                                &loc_weak,
                                |this, w, cx| this.confirm_reset_data_location(w, cx),
                            ))
                        }),
                )
                .into_any_element()
        });

        SettingPage::new(t!("settings.nav.notebooks").into_owned()).groups([
            card(
                "settings.section.notebooks",
                "settings.desc.notebooks",
                vec![notebooks],
            ),
            card(
                "settings.section.data_location",
                "settings.desc.data_location",
                vec![location],
            ),
        ])
    }

    fn appearance_page(&self, weak: &WeakEntity<Self>, cx: &App) -> SettingPage {
        let app = &self.app;

        let (theme_opts, _) = theme_options(app, cx);
        let theme_get_app = app.clone();
        let theme_set_app = app.clone();
        let app_theme = dropdown_item(
            "settings.item.theme",
            theme_opts,
            move |cx| theme_options(&theme_get_app, cx).1,
            move |v, cx| {
                with_main_window(&theme_set_app, cx, move |a, window, cx| {
                    a.set_skin(v.to_string(), window, cx)
                })
            },
            "zorite",
            weak,
        );

        let mode_get_app = app.clone();
        let mode_set_app = app.clone();
        let appearance = dropdown_item(
            "settings.item.mode",
            vec![
                opt("light", t!("settings.opt.light").to_string()),
                opt("dark", t!("settings.opt.dark").to_string()),
                opt("auto", t!("settings.opt.auto_appearance").to_string()),
            ],
            move |cx| {
                mode_get_app
                    .upgrade()
                    .map(|a| a.read(cx).theme_mode())
                    .unwrap_or_default()
                    .as_str()
                    .into()
            },
            move |v, cx| {
                with_main_window(&mode_set_app, cx, move |a, window, cx| {
                    a.set_theme_mode(Mode::from_str(&v), window, cx)
                })
            },
            "auto",
            weak,
        );

        // Font: the family dropdown + an import button.
        let (font_opts, _) = font_options(app, &self.font_names, cx);
        let font_get_app = app.clone();
        let font_set_app = app.clone();
        let family = dropdown_item_scrollable(
            "settings.item.family",
            font_opts,
            move |cx| {
                font_get_app
                    .upgrade()
                    .map(|a| a.read(cx).ui_font().to_string().into())
                    .unwrap_or_default()
            },
            move |v, cx| {
                with_main_window(&font_set_app, cx, move |a, window, cx| {
                    a.set_ui_font(v.to_string(), window, cx)
                })
            },
            "",
            weak,
        );
        let font_weak = weak.clone();
        let add_font = custom(move |_window, _cx| {
            div()
                .flex()
                .flex_row()
                .child(text_button(
                    "font-add",
                    &t!("settings.btn.add_font"),
                    &font_weak,
                    |this, w, cx| this.choose_font_file(w, cx),
                ))
                .into_any_element()
        });

        // Note text size — one value for all three views.
        let size_get_app = app.clone();
        let size_set_app = app.clone();
        let text_size = dropdown_item(
            "settings.item.size",
            crate::app::TEXT_SIZES
                .iter()
                .map(|&s| {
                    let label = if s == 16.0 {
                        t!("settings.opt.size_px_default", size = s).to_string()
                    } else {
                        t!("settings.opt.size_px", size = s).to_string()
                    };
                    opt(&format!("{s}"), label)
                })
                .collect(),
            move |cx| {
                size_get_app
                    .upgrade()
                    .map(|a| format!("{}", f32::from(a.read(cx).text_size())).into())
                    .unwrap_or_else(|| "16".into())
            },
            move |v, cx| {
                if let (Ok(size), Some(a)) = (v.parse::<f32>(), size_set_app.upgrade()) {
                    a.update(cx, |a, cx| a.set_text_size(size, cx));
                }
            },
            "16",
            weak,
        );

        let line_numbers = switch_item(
            "settings.item.show_gutter",
            app,
            |a| a.line_numbers(),
            |a, on, cx| a.set_line_numbers(on, cx),
            false,
            weak,
        );

        // Mouse cursor: the pack dropdown + add/reveal actions, the last import
        // error, and (Linux) the relaunch note.
        let (cursor_opts, _) = cursor_options(&self.cursor_packs, &self.cursor_reactive);
        let cursor_weak = weak.clone();
        let cursor = dropdown_item(
            "settings.item.theme",
            cursor_opts,
            |_cx| crate::cursors::selected().unwrap_or_default().into(),
            move |v, cx| {
                crate::cursors::set_selected((!v.is_empty()).then_some(v.as_ref()));
                let _ = cursor_weak.update(cx, |this, _| this.cursor_status = None);
            },
            "",
            weak,
        );
        let cursor_weak = weak.clone();
        let cursor_actions = custom(move |_window, cx| {
            let status = cursor_weak
                .upgrade()
                .and_then(|v| v.read(cx).cursor_status.clone());
            let col = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.0))
                        .child(text_button(
                            "cursor-add",
                            &t!("settings.btn.add_cursor"),
                            &cursor_weak,
                            |this, w, cx| this.choose_cursor_theme(w, cx),
                        ))
                        .child(text_button(
                            "cursor-reveal",
                            &t!("settings.btn.reveal_cursors"),
                            &cursor_weak,
                            |_this, _w, _cx| {
                                let dir = crate::cursors::cursors_dir();
                                let _ = std::fs::create_dir_all(&dir);
                                crate::app::AppView::reveal_folder(&dir);
                            },
                        )),
                )
                .children(status.map(status_line));
            #[cfg(target_os = "linux")]
            let col = col.child(status_line(t!("settings.note.cursor_relaunch").to_string()));
            col.into_any_element()
        });

        let sidebar_right = switch_item(
            "settings.item.dock_right",
            app,
            |a| a.sidebar_right,
            |a, on, cx| a.set_sidebar_right(on, cx),
            false,
            weak,
        );

        // Installed themes: the actions + the list (or empty state).
        let themes_weak = weak.clone();
        let installed = custom(move |_window, cx| {
            let names = themes_weak
                .upgrade()
                .map(|v| v.read(cx).user_theme_names(cx))
                .unwrap_or_default();
            let actions = div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(text_button(
                    "reveal-themes",
                    &t!("settings.btn.reveal_themes"),
                    &themes_weak,
                    |this, _w, cx| {
                        if let Some(app) = this.app.upgrade() {
                            app.read(cx).reveal_themes_folder();
                        }
                    },
                ))
                .child(text_button(
                    "reload-themes",
                    &t!("settings.btn.reload"),
                    &themes_weak,
                    |this, w, cx| this.reload_skins(w, cx),
                ));
            let list = if names.is_empty() {
                div()
                    .text_size(px(13.0))
                    .text_color(theme::text_tertiary())
                    .child(t!("settings.note.no_custom_themes").to_string())
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .children(names.into_iter().map(|name| {
                        div()
                            .px(px(12.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(theme::glass())
                            .text_size(px(13.0))
                            .text_color(theme::text_secondary())
                            .child(name)
                    }))
                    .into_any_element()
            };
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(actions)
                .child(list)
                .into_any_element()
        });

        SettingPage::new(t!("settings.nav.appearance").into_owned()).groups([
            card(
                "settings.section.app_theme",
                "settings.desc.app_theme",
                vec![app_theme],
            ),
            card(
                "settings.section.appearance",
                "settings.desc.appearance",
                vec![appearance],
            ),
            card(
                "settings.section.font",
                "settings.desc.font",
                vec![family, add_font],
            ),
            card(
                "settings.section.text_size",
                "settings.desc.text_size",
                vec![text_size],
            ),
            card(
                "settings.section.line_numbers",
                "settings.desc.line_numbers",
                vec![line_numbers],
            ),
            card(
                "settings.section.mouse_cursor",
                "settings.desc.mouse_cursor",
                vec![cursor, cursor_actions],
            ),
            card(
                "settings.section.sidebar_position",
                "settings.desc.sidebar_position",
                vec![sidebar_right],
            ),
            card(
                "settings.section.installed_themes",
                "settings.desc.installed_themes",
                vec![installed],
            ),
        ])
    }

    fn markdown_page(&self, weak: &WeakEntity<Self>) -> SettingPage {
        let app = &self.app;
        // WYSIWYG live preview: persists + re-applies to open editors via
        // `set_wysiwyg`.
        let wysiwyg = switch_item(
            "settings.item.enabled",
            app,
            |a| a.wysiwyg(),
            |a, on, cx| a.set_wysiwyg(on, cx),
            true,
            weak,
        );
        let indent_get_app = app.clone();
        let indent_set_app = app.clone();
        let indent = dropdown_item(
            "settings.item.spaces_per_level",
            vec![
                opt("2", t!("settings.opt.indent_2").to_string()),
                opt("4", t!("settings.opt.indent_4").to_string()),
                opt("8", t!("settings.opt.indent_8").to_string()),
            ],
            move |cx| {
                indent_get_app
                    .upgrade()
                    .map(|a| a.read(cx).list_indent().to_string().into())
                    .unwrap_or_else(|| "4".into())
            },
            move |v, cx| {
                if let (Ok(spaces), Some(a)) = (v.parse::<usize>(), indent_set_app.upgrade()) {
                    a.update(cx, |a, cx| a.set_list_indent(spaces, cx));
                }
            },
            "4",
            weak,
        );
        let auto_link = switch_item(
            "settings.item.enabled",
            app,
            |a| a.auto_link(),
            |a, on, _cx| a.set_auto_link(on),
            false,
            weak,
        );
        SettingPage::new(t!("settings.nav.markdown").into_owned()).groups([
            card(
                "settings.section.wysiwyg",
                "settings.desc.wysiwyg",
                vec![wysiwyg],
            ),
            card(
                "settings.section.list_indent",
                "settings.desc.list_indent",
                vec![indent],
            ),
            card(
                "settings.section.autolink",
                "settings.desc.autolink",
                vec![auto_link],
            ),
        ])
    }

    fn pdf_page(&self) -> SettingPage {
        let slider = self.quality_slider.clone();
        let app = self.app.clone();
        let quality = SettingItem::new(
            t!("settings.item.quality").into_owned(),
            SettingField::render(move |_opts, _window, cx| {
                let qpct = app
                    .upgrade()
                    .map(|a| (a.read(cx).pdf_quality() * 100.0).round() as i32)
                    .unwrap_or(100);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .min_w(px(220.0))
                    .child(Slider::new(&slider).w_full())
                    .child(status_line(format!("{qpct}%")))
            }),
        );
        SettingPage::new(t!("settings.nav.pdf").into_owned()).groups([card(
            "settings.section.pdf_quality",
            "settings.desc.pdf_quality",
            vec![quality],
        )])
    }

    fn security_page(&self, weak: &WeakEntity<Self>) -> SettingPage {
        let encrypted = crate::db::db_is_encrypted();

        // The password state drives which actions show.
        let pw_weak = weak.clone();
        let password = custom(move |_window, cx| {
            let status = pw_weak
                .upgrade()
                .and_then(|v| v.read(cx).security_status.clone());
            let encrypted = crate::db::db_is_encrypted();
            let mut row = div().flex().flex_row().flex_wrap().gap(px(8.0));
            if encrypted {
                row = row
                    .child(text_button(
                        "sec-change",
                        &t!("settings.btn.change_password"),
                        &pw_weak,
                        |this, w, cx| this.open_password_dialog(PwMode::Change, w, cx),
                    ))
                    .child(text_button(
                        "sec-remove",
                        &t!("settings.btn.remove_password"),
                        &pw_weak,
                        |this, w, cx| this.open_password_dialog(PwMode::Remove, w, cx),
                    ))
                    .child(text_button(
                        "sec-lock",
                        &t!("settings.btn.lock_now"),
                        &pw_weak,
                        |_this, _w, cx| {
                            // Deferred: locking closes this window mid-handler.
                            cx.defer(crate::lock_now);
                        },
                    ));
            } else {
                row = row.child(text_button(
                    "sec-set",
                    &t!("settings.btn.set_password"),
                    &pw_weak,
                    |this, w, cx| this.open_password_dialog(PwMode::Set, w, cx),
                ));
            }
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(row)
                .children(status.map(status_line))
                .into_any_element()
        });

        let remember = SettingItem::new(
            t!("settings.item.enabled").into_owned(),
            SettingField::switch(
                |_cx| crate::db::db_is_encrypted() && crate::security::is_remembered(),
                notify_after(weak, |on, _cx| {
                    if !crate::db::db_is_encrypted() {
                        return;
                    }
                    if on {
                        if let Some(pw) = crate::security::session_key() {
                            crate::security::remember_password(&pw);
                        }
                    } else {
                        crate::security::forget_password();
                    }
                }),
            ),
        )
        .disabled(!encrypted);

        let lock_app = self.app.clone();
        let auto_lock = dropdown_item(
            "settings.item.lock_after",
            vec![
                opt("0", t!("settings.autolock.off").to_string()),
                opt("5", t!("settings.autolock.5min").to_string()),
                opt("15", t!("settings.autolock.15min").to_string()),
                opt("30", t!("settings.autolock.30min").to_string()),
                opt("60", t!("settings.autolock.1hour").to_string()),
            ],
            |_cx| crate::security::auto_lock_minutes().to_string().into(),
            move |v, cx| {
                if !crate::db::db_is_encrypted() {
                    return;
                }
                if let (Ok(mins), Some(a)) = (v.parse::<u64>(), lock_app.upgrade()) {
                    a.update(cx, |a, cx| a.set_auto_lock(mins, cx));
                }
            },
            "0",
            weak,
        )
        .disabled(!encrypted);

        let mut remember_items = vec![remember];
        let mut lock_items = vec![auto_lock];
        if !encrypted {
            let note = || {
                custom(|_w, _cx| {
                    status_line(t!("settings.note.set_password_first").to_string())
                        .into_any_element()
                })
            };
            remember_items.push(note());
            lock_items.push(note());
        }

        SettingPage::new(t!("settings.nav.security").into_owned()).groups([
            card(
                "settings.section.password",
                "settings.desc.password",
                vec![password],
            ),
            card(
                "settings.section.remember_device",
                if cfg!(target_os = "linux") {
                    "settings.desc.remember_device_linux"
                } else {
                    "settings.desc.remember_device"
                },
                remember_items,
            ),
            card(
                "settings.section.autolock",
                "settings.desc.autolock",
                lock_items,
            ),
        ])
    }

    fn updates_page(&self, weak: &WeakEntity<Self>) -> SettingPage {
        let app = &self.app;
        // Current version, the available-update banner (read from the
        // `updater::UpdateState` global), and View-release / Check-now.
        let up_weak = weak.clone();
        let updates = custom(move |_window, cx| {
            let available = cx
                .try_global::<crate::updater::UpdateState>()
                .and_then(|u| u.available.clone());
            let cur_version = env!("CARGO_PKG_VERSION");
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
                    col = col.child(status_line(preview));
                }
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.0))
                        .child(text_button(
                            "updates-view",
                            &t!("settings.btn.view_release"),
                            &up_weak,
                            move |_this, _w, _cx| open_url(&url),
                        ))
                        .child(text_button(
                            "updates-check",
                            &t!("settings.btn.check_now"),
                            &up_weak,
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
                        &up_weak,
                        |this, _w, cx| this.check_for_updates(cx),
                    ));
            }
            col.into_any_element()
        });
        let auto_check = switch_item(
            "settings.item.enabled",
            app,
            |a| a.check_updates(),
            |a, on, _cx| a.set_check_updates(on),
            true,
            weak,
        );
        let prereleases = switch_item(
            "settings.item.enabled",
            app,
            |a| a.include_prerelease(),
            |a, on, cx| a.set_include_prerelease(on, cx),
            false,
            weak,
        );
        SettingPage::new(t!("settings.nav.updates").into_owned()).groups([
            card(
                "settings.section.updates",
                "settings.desc.updates",
                vec![updates],
            ),
            card(
                "settings.section.auto_check",
                "settings.desc.auto_check",
                vec![auto_check],
            ),
            card(
                "settings.section.prereleases",
                "settings.desc.prereleases",
                vec![prereleases],
            ),
        ])
    }
}

/// The read-only shortcut lists (Keyboard page).
fn keyboard_page() -> SettingPage {
    fn list(rows: Vec<(String, Vec<&'static str>)>) -> SettingItem {
        let labels: Vec<SharedString> = rows.iter().map(|(l, _)| l.clone().into()).collect();
        custom(move |_w, _cx| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .children(rows.iter().map(|(label, combo)| shortcut_row(label, combo)))
                .into_any_element()
        })
        .keywords(labels)
    }
    let app_rows: Vec<(String, Vec<&'static str>)> = vec![
        (
            t!("settings.kb.command_palette").to_string(),
            vec![keys::MOD, keys::SHIFT, "P"],
        ),
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
    let edit_rows: Vec<(String, Vec<&'static str>)> = vec![
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
    let wb_tool_rows: Vec<(String, Vec<&'static str>)> = vec![
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
    let wb_edit_rows: Vec<(String, Vec<&'static str>)> = vec![
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
    let pdf_rows: Vec<(String, Vec<&'static str>)> = vec![
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
    SettingPage::new(t!("settings.nav.keyboard").into_owned())
        .resettable(false)
        .groups([
            card(
                "settings.section.application",
                "settings.desc.application",
                vec![list(app_rows)],
            ),
            card(
                "settings.section.editing",
                "settings.desc.editing",
                vec![list(edit_rows)],
            ),
            card(
                "settings.section.wb_tools",
                "settings.desc.wb_tools",
                vec![list(wb_tool_rows)],
            ),
            card(
                "settings.section.wb_editing",
                "settings.desc.wb_editing",
                vec![list(wb_edit_rows)],
            ),
            card(
                "settings.section.pdf_viewer",
                "settings.desc.pdf_viewer",
                vec![list(pdf_rows)],
            ),
        ])
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let pages = vec![
            self.general_page(&weak),
            self.notebooks_page(&weak),
            self.appearance_page(&weak, cx),
            self.markdown_page(&weak),
            self.pdf_page(),
            self.security_page(&weak),
            keyboard_page(),
            self.updates_page(&weak),
        ];
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::bg_window())
            .text_color(theme::text_primary())
            .child(TitleBar::new())
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new("zorite-settings")
                        .small()
                        .sidebar_width(px(190.0))
                        // Appearance is the page people open Settings for most.
                        .default_selected_index(SelectIndex {
                            page_ix: 2,
                            group_ix: None,
                        })
                        .pages(pages),
                ),
            )
            // gpui-component's `Root` stores dialog state but doesn't draw it;
            // the host view must render the dialog layer (as the main window
            // does), or the data-location confirm dialog stays invisible.
            .children(Root::render_dialog_layer(window, cx))
    }
}

// --- Card / field helpers ---------------------------------------------------

/// One settings card: the localized title + description as a group, with
/// every item tagged with the card's words so the sidebar search finds it by
/// them (the component only matches an item's own title/description/keywords).
fn card(title_key: &str, desc_key: &str, items: Vec<SettingItem>) -> SettingGroup {
    let title = t!(title_key).into_owned();
    let desc = t!(desc_key).into_owned();
    let kws: Vec<SharedString> = vec![
        title.clone().into(),
        desc.clone().into(),
        keywords_for(title_key).into(),
    ];
    SettingGroup::new()
        .title(title)
        .description(desc)
        .items(items.into_iter().map(|it| {
            // `keywords` replaces, so carry over what the item brought (a
            // dropdown's option labels, a shortcut list's row labels).
            let own = match &it {
                SettingItem::Item { keywords, .. } | SettingItem::Element { keywords, .. } => {
                    keywords.clone()
                }
            };
            it.keywords(own.into_iter().chain(kws.iter().cloned()))
        }))
}

/// A full-width custom row.
fn custom(render: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) -> SettingItem {
    SettingItem::render(move |_opts, window, cx| render(window, cx))
}

/// A switch bound to an `AppView` getter/setter, with its default so the
/// page's reset button knows when it's changed.
fn switch_item(
    label_key: &str,
    app: &WeakEntity<AppView>,
    get: impl Fn(&AppView) -> bool + 'static,
    set: impl Fn(&mut AppView, bool, &mut Context<AppView>) + 'static,
    default: bool,
    weak: &WeakEntity<SettingsView>,
) -> SettingItem {
    let get_app = app.clone();
    let set_app = app.clone();
    SettingItem::new(
        t!(label_key).into_owned(),
        SettingField::switch(
            move |cx| {
                get_app
                    .upgrade()
                    .map(|a| get(a.read(cx)))
                    .unwrap_or(default)
            },
            notify_after(weak, move |on, cx| {
                if let Some(a) = set_app.upgrade() {
                    a.update(cx, |a, cx| set(a, on, cx));
                }
            }),
        )
        .default_value(default),
    )
}

fn dropdown_item(
    label_key: &str,
    options: Options,
    get: impl Fn(&App) -> SharedString + 'static,
    set: impl Fn(SharedString, &mut App) + 'static,
    default: &str,
    weak: &WeakEntity<SettingsView>,
) -> SettingItem {
    let labels: Vec<SharedString> = options.iter().map(|(_, label)| label.clone()).collect();
    SettingItem::new(
        t!(label_key).into_owned(),
        SettingField::dropdown(options, get, notify_after(weak, set))
            .default_value(SharedString::from(default.to_string())),
    )
    .keywords(labels)
}

/// [`dropdown_item`] whose menu scrolls — for long lists (installed fonts).
fn dropdown_item_scrollable(
    label_key: &str,
    options: Options,
    get: impl Fn(&App) -> SharedString + 'static,
    set: impl Fn(SharedString, &mut App) + 'static,
    default: &str,
    weak: &WeakEntity<SettingsView>,
) -> SettingItem {
    let labels: Vec<SharedString> = options.iter().map(|(_, label)| label.clone()).collect();
    SettingItem::new(
        t!(label_key).into_owned(),
        SettingField::scrollable_dropdown(options, get, notify_after(weak, set))
            .default_value(SharedString::from(default.to_string())),
    )
    .keywords(labels)
}

/// Wrap a field setter so the Settings view re-renders after it runs: the
/// fields read their values at render time, and a setter that only touches
/// `AppView` (or a sidecar file) would otherwise leave the control stale.
fn notify_after<T: 'static>(
    weak: &WeakEntity<SettingsView>,
    set: impl Fn(T, &mut App) + 'static,
) -> impl Fn(T, &mut App) + 'static {
    let weak = weak.clone();
    move |v, cx| {
        set(v, cx);
        let _ = weak.update(cx, |_, cx| cx.notify());
    }
}

/// Run an `AppView` setter that needs a `Window` (theme / font apply) against
/// the main window — a field's setter only gets the `App`.
fn with_main_window(
    app: &WeakEntity<AppView>,
    cx: &mut App,
    f: impl FnOnce(&mut AppView, &mut Window, &mut Context<AppView>),
) {
    let Some(a) = app.upgrade() else {
        return;
    };
    let handle = a.read(cx).window_handle;
    let _ = handle.update(cx, move |_, window, cx| {
        a.update(cx, |a, cx| f(a, window, cx))
    });
}

/// Muted small text under a control: an outcome, a note, a value readout.
fn status_line(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(12.0))
        .text_color(theme::text_tertiary())
        .child(text.into())
}

/// One notebook in the Notebooks card: name + path on the left, its actions
/// on the right (the active row swaps Switch/Remove for a "current" tag).
fn notebook_row(nb: crate::paths::Notebook, weak: &WeakEntity<SettingsView>) -> impl IntoElement {
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
            weak,
            move |this, window, cx| this.switch_notebook(nb_switch.clone(), window, cx),
        ));
    }
    actions = actions
        .child(nb_button(
            SharedString::from(format!("nb-rename:{}", nb.dir)),
            &t!("settings.nb.rename"),
            weak,
            move |this, window, cx| this.rename_notebook(nb_rename.clone(), window, cx),
        ))
        .child(nb_button(
            SharedString::from(format!("nb-reveal:{}", nb.dir)),
            &t!("settings.nb.reveal"),
            weak,
            move |_this, _window, _cx| {
                crate::app::AppView::reveal_folder(&reveal_dir);
            },
        ));
    if !active {
        actions = actions.child(nb_button(
            SharedString::from(format!("nb-forget:{}", nb.dir)),
            &t!("settings.nb.remove"),
            weak,
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

/// [`text_button`]'s small sibling for the notebook rows: a dynamic id (one
/// per notebook) and tighter padding.
fn nb_button(
    id: SharedString,
    label: &str,
    weak: &WeakEntity<SettingsView>,
    on: impl Fn(&mut SettingsView, &mut Window, &mut Context<SettingsView>) + 'static,
) -> impl IntoElement {
    let weak = weak.clone();
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
        .on_click(move |_, window, cx| {
            let _ = weak.update(cx, |this, cx| on(this, window, cx));
        })
}

/// A small action button beside a card's control, sized to sit next to
/// gpui-component's `.small()` fields. The handler runs on the settings view
/// (the fields' closures only get the `App`, so buttons hold a weak handle).
fn text_button(
    id: &'static str,
    label: &str,
    weak: &WeakEntity<SettingsView>,
    on: impl Fn(&mut SettingsView, &mut Window, &mut Context<SettingsView>) + 'static,
) -> impl IntoElement {
    let weak = weak.clone();
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
        .on_click(move |_, window, cx| {
            let _ = weak.update(cx, |this, cx| on(this, window, cx));
        })
}

/// Open a URL in the user's default browser (the "View release" button).
///
/// The URL comes from the GitHub release JSON, so it's only as trustworthy as
/// that response — and it's handed to `open`/`explorer`, which happily take a
/// local path or a leading `-` as a flag. Require https before spawning.
fn open_url(url: &str) {
    if !zorite_markdown::syntax::is_safe_external_url(url) {
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
