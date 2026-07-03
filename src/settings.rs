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
    Disableable, IndexPath, Root, TitleBar, WindowExt,
    button::{Button, ButtonVariants as _},
    dialog::{DialogButtonProps, DialogFooter},
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectItem, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
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

/// Font-dropdown choices: Default, then every installed family (including the
/// user-added ones registered at startup / via "Add font file…").
fn font_opts(app: &WeakEntity<AppView>, cx: &Context<SettingsView>) -> (Vec<Opt>, String) {
    let mut names = cx.text_system().all_font_names();
    names.sort();
    names.dedup();
    let mut opts = vec![Opt::new("", "Default")];
    opts.extend(names.iter().map(|n| Opt::new(n, n)));
    let current = app
        .upgrade()
        .map(|a| a.read(cx).ui_font().to_string())
        .unwrap_or_default();
    (opts, current)
}

/// Which settings category the left nav has selected.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
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
/// `render`. Keywords (lowercase) add synonyms a user might type for a setting
/// that aren't already in its title.
const SECTIONS: &[(Tab, &str, &str)] = &[
    (
        Tab::General,
        "Data location",
        "folder path database directory move attachments",
    ),
    (
        Tab::General,
        "Unused images",
        "cleanup delete orphan gc attachments storage space free",
    ),
    (
        Tab::General,
        "Remember window position",
        "bounds size resize reopen restore placement screen monitor",
    ),
    (
        Tab::General,
        "Date format",
        "iso us european calendar day month year /date",
    ),
    (Tab::General, "Time format", "24 hour 12 clock am pm /time"),
    (
        Tab::Appearance,
        "App Theme",
        "skin colors palette built-in custom",
    ),
    (
        Tab::Appearance,
        "Appearance",
        "light dark auto system mode variant",
    ),
    (
        Tab::Appearance,
        "Font",
        "typeface family typography text ttf otf custom",
    ),
    (
        Tab::Appearance,
        "Text size",
        "font size zoom bigger smaller larger scale px",
    ),
    (
        Tab::Appearance,
        "Installed themes",
        "custom user json reload reveal folder",
    ),
    (
        Tab::Pdf,
        "PDF render quality",
        "dpi resolution sharpness speed scale render",
    ),
    (
        Tab::Markdown,
        "WYSIWYG editing",
        "live preview inline formatting bold heading links",
    ),
    (
        Tab::Markdown,
        "List indentation",
        "spaces tab nesting indent bullet",
    ),
    (
        Tab::Markdown,
        "Auto-link page titles",
        "wiki link automatic typing wrap unlinked references",
    ),
    (
        Tab::Keyboard,
        "Application",
        "shortcuts keys tab window quit settings find search",
    ),
    (
        Tab::Keyboard,
        "Editing",
        "shortcuts keys slash menu copy paste undo redo indent",
    ),
    (
        Tab::Keyboard,
        "Whiteboard tools",
        "shortcuts keys pen shape rectangle ellipse text image",
    ),
    (
        Tab::Keyboard,
        "Whiteboard editing",
        "shortcuts keys z-order delete copy paste",
    ),
    (Tab::Keyboard, "PDF viewer", "shortcuts keys page zoom find"),
    (
        Tab::Security,
        "Password",
        "encrypt encryption lock database sqlcipher passphrase secure",
    ),
    (
        Tab::Security,
        "Remember on this device",
        "keychain credential manager auto unlock remember password",
    ),
    (
        Tab::Security,
        "Auto-lock",
        "idle timeout lock minutes away inactivity",
    ),
    (
        Tab::Updates,
        "Software updates",
        "version release github check download",
    ),
    (
        Tab::Updates,
        "Automatically check for updates",
        "startup auto version",
    ),
    (
        Tab::Updates,
        "Include pre-releases",
        "beta prerelease pre-release unstable",
    ),
];

pub struct SettingsView {
    app: WeakEntity<AppView>,
    theme_select: Entity<SelectState<Vec<Opt>>>,
    appearance_select: Entity<SelectState<Vec<Opt>>>,
    font_select: Entity<SelectState<Vec<Opt>>>,
    text_size_select: Entity<SelectState<Vec<Opt>>>,
    quality_slider: Entity<SliderState>,
    indent_select: Entity<SelectState<Vec<Opt>>>,
    date_format_select: Entity<SelectState<Vec<Opt>>>,
    time_format_select: Entity<SelectState<Vec<Opt>>>,
    /// Header filter box + its current (trimmed, lowercased) text. Empty = no
    /// filter; non-empty dims the cards + nav tabs that don't match.
    filter_input: Entity<InputState>,
    filter: String,
    /// The selected left-nav category.
    tab: Tab,
    /// Last images-GC outcome ("Removed 12 files (3.4 MB)"), shown under the
    /// Unused images button.
    image_gc_result: Option<String>,
    /// Password dialogs' fields (masked) + the last outcome line shown under
    /// the Password card.
    sec_current: Entity<InputState>,
    sec_new: Entity<InputState>,
    sec_confirm: Entity<InputState>,
    security_status: Option<String>,
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
        let (f_opts, current_font) = font_opts(&app, cx);
        let font_select = make_select(f_opts, &current_font, window, cx);

        // Note text size (Appearance pane) — one value for all three views.
        let size_opts: Vec<Opt> = crate::app::TEXT_SIZES
            .iter()
            .map(|&s| {
                let id = format!("{s}");
                let label = if s == 16.0 {
                    format!("{s} px (default)")
                } else {
                    format!("{s} px")
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

        // Header filter box — dims the cards + nav tabs that don't match as the
        // user types (Baudrun's Settings filter). Subscribed on every keystroke
        // (`Change`) so the dim updates live; the value drives `self.filter`.
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter settings…"));
        let masked = |ph: &str, window: &mut Window, cx: &mut Context<Self>| {
            let ph = ph.to_string();
            cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(ph))
        };
        let sec_current = masked("Current password", window, cx);
        let sec_new = masked("New password", window, cx);
        let sec_confirm = masked("Confirm new password", window, cx);
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

        Self {
            app,
            theme_select,
            appearance_select,
            font_select,
            text_size_select,
            quality_slider,
            indent_select,
            date_format_select,
            time_format_select,
            filter_input,
            filter: String::new(),
            tab: Tab::Appearance,
            image_gc_result: None,
            sec_current,
            sec_new,
            sec_confirm,
            security_status: None,
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
            prompt: Some("Use font".into()),
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

    /// Scan for unused images and confirm before deleting — the list of
    /// doomed files is shown, since this is destructive and undo-less.
    fn confirm_image_gc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let orphans = app.read(cx).orphan_images();
        if orphans.is_empty() {
            self.image_gc_result = Some("No unused images found.".to_string());
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
            listing.push(format!("…and {} more", orphans.len() - SHOWN));
        }
        let body = format!(
            "{} file{} ({}) in the images folder {} referenced by any note, \
             whiteboard, or template. They'll be moved to the system trash.\n\n{}",
            orphans.len(),
            if orphans.len() == 1 { "" } else { "s" },
            fmt_size(total),
            if orphans.len() == 1 {
                "isn't"
            } else {
                "aren't"
            },
            listing.join("\n"),
        );
        let weak_app = self.app.clone();
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let orphans = orphans.clone();
            let weak_app = weak_app.clone();
            let this = this.clone();
            dialog
                .title("Move unused images to the trash?")
                .description(SharedString::from(body.clone()))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Move to Trash")
                        .cancel_text("Cancel")
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    let Some(app) = weak_app.upgrade() else {
                        return true;
                    };
                    let (removed, freed) = app.read(cx).remove_orphan_images(&orphans);
                    let _ = this.update(cx, |s, cx| {
                        s.image_gc_result = Some(format!(
                            "Moved {removed} file{} ({}) to the trash.",
                            if removed == 1 { "" } else { "s" },
                            fmt_size(freed),
                        ));
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
        let (title, ok_label) = match mode {
            PwMode::Set => ("Set a password", "Encrypt"),
            PwMode::Change => ("Change password", "Change"),
            PwMode::Remove => ("Remove password", "Decrypt"),
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
                        .child(
                            "Encrypts your entire database. If you forget this \
                             password, your notes are unrecoverable. Earlier plaintext \
                             backups in the data folder stay readable until you delete \
                             them.",
                        ),
                );
            }
            dialog
                .title(title)
                .w(px(440.0))
                .child(body)
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("pw-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("pw-ok").primary().label(ok_label).on_click({
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
                                    this.apply_password_change(mode, cur, new, conf, window, cx);
                                });
                            }
                        })),
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
                "Wrong password",
                "The current password doesn't match.".into(),
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
                        "No password",
                        "The new password is empty.".into(),
                        window,
                        cx,
                    );
                    return;
                }
                if new != confirm {
                    self.alert(
                        "Passwords don't match",
                        "The two entries differ — try again.".into(),
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
            (Ok(()), PwMode::Set) => "Database encrypted.".to_string(),
            (Ok(()), PwMode::Change) => "Password changed.".to_string(),
            (Ok(()), PwMode::Remove) => "Password removed — database decrypted.".to_string(),
            (Err(e), _) => format!("Failed: {e}"),
        });
        cx.notify();
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

    // ---- Header filter (Baudrun-style): dim cards + tabs that don't match ----

    /// A settings card that fades when it doesn't match the current filter. It
    /// stays interactive — the user can change a dimmed setting without first
    /// clearing the filter.
    fn section_card(
        &self,
        title: &'static str,
        desc: &str,
        control: impl IntoElement,
    ) -> gpui::Div {
        card(title, desc, control).opacity(self.filter_opacity(title))
    }

    /// Filter-aware wrapper for the shortcut-list cards on the Keyboard pane.
    fn section_list(
        &self,
        title: &'static str,
        desc: &str,
        rows: Vec<(&str, Vec<&str>)>,
    ) -> gpui::Div {
        card_list(title, desc, rows).opacity(self.filter_opacity(title))
    }

    fn filter_opacity(&self, title: &str) -> f32 {
        if self.section_matches(title) {
            1.0
        } else {
            0.3
        }
    }

    /// Whether `title`'s card matches the filter: an empty filter matches all;
    /// otherwise the title or its `SECTIONS` keywords must contain the text.
    fn section_matches(&self, title: &str) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        if title.to_lowercase().contains(self.filter.as_str()) {
            return true;
        }
        SECTIONS
            .iter()
            .find(|(_, t, _)| *t == title)
            .is_some_and(|(_, _, kw)| kw.contains(self.filter.as_str()))
    }

    /// Whether `tab` has at least one matching card — drives the rail dim.
    fn tab_has_matches(&self, tab: Tab) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        SECTIONS.iter().any(|(t, title, kw)| {
            *t == tab
                && (title.to_lowercase().contains(self.filter.as_str())
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
                "General",
                Tab::General,
                active,
                !self.tab_has_matches(Tab::General),
                cx,
            ))
            .child(nav_item(
                "nav-appearance",
                "Appearance",
                Tab::Appearance,
                active,
                !self.tab_has_matches(Tab::Appearance),
                cx,
            ))
            .child(nav_item(
                "nav-pdf",
                "PDF",
                Tab::Pdf,
                active,
                !self.tab_has_matches(Tab::Pdf),
                cx,
            ))
            .child(nav_item(
                "nav-markdown",
                "Markdown",
                Tab::Markdown,
                active,
                !self.tab_has_matches(Tab::Markdown),
                cx,
            ))
            .child(nav_item(
                "nav-keyboard",
                "Keyboard",
                Tab::Keyboard,
                active,
                !self.tab_has_matches(Tab::Keyboard),
                cx,
            ))
            .child(nav_item(
                "nav-security",
                "Security",
                Tab::Security,
                active,
                !self.tab_has_matches(Tab::Security),
                cx,
            ))
            .child(nav_item(
                "nav-updates",
                "Updates",
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
        let wysiwyg_switch =
            Switch::new("wysiwyg-toggle")
                .checked(wys_on)
                .on_click(move |checked, _window, cx| {
                    if let Some(app) = wys_app.upgrade() {
                        app.update(cx, |a, cx| a.set_wysiwyg(*checked, cx));
                    }
                });

        // Auto-link-as-you-type toggle (Markdown pane).
        let al_on = self
            .app
            .upgrade()
            .map(|a| a.read(cx).auto_link())
            .unwrap_or(false);
        let al_app = self.app.clone();
        let auto_link_switch =
            Switch::new("auto-link-toggle")
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
        let prerelease_switch = Switch::new("prerelease-toggle").checked(pre_on).on_click(
            move |checked, _window, cx| {
                if let Some(app) = pre_app.upgrade() {
                    app.update(cx, |a, cx| a.set_include_prerelease(*checked, cx));
                }
            },
        );

        // Font card body: the family dropdown + an import button.
        let font_control = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(Select::new(&self.font_select).w_full())
            .child(div().flex().flex_row().child(text_button(
                "font-add",
                "Add font file…",
                cx,
                |this, w, cx| this.choose_font_file(w, cx),
            )));

        // Installed-themes card body: the actions + the list (or empty state).
        let actions = div()
            .flex()
            .flex_row()
            .gap(px(8.0))
            .child(text_button(
                "reveal-themes",
                "Reveal themes folder",
                cx,
                |this, _w, cx| {
                    if let Some(app) = this.app.upgrade() {
                        app.read(cx).reveal_themes_folder();
                    }
                },
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

        // Unused-images card body (General): the cleanup action + its last
        // outcome.
        let image_gc_control = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(div().flex().flex_row().child(text_button(
                "image-gc",
                "Clean up now",
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

        // Security cards: the password state drives which actions show.
        let encrypted = crate::db::db_is_encrypted();
        let password_control = {
            let mut row = div().flex().flex_row().flex_wrap().gap(px(8.0));
            if encrypted {
                row = row
                    .child(text_button(
                        "sec-change",
                        "Change password…",
                        cx,
                        |this, w, cx| {
                            this.open_password_dialog(PwMode::Change, w, cx);
                        },
                    ))
                    .child(text_button(
                        "sec-remove",
                        "Remove password…",
                        cx,
                        |this, w, cx| {
                            this.open_password_dialog(PwMode::Remove, w, cx);
                        },
                    ))
                    .child(text_button("sec-lock", "Lock now", cx, |_this, _w, cx| {
                        // Deferred: locking closes this window mid-handler.
                        cx.defer(crate::lock_now);
                    }));
            } else {
                row = row.child(text_button(
                    "sec-set",
                    "Set password…",
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
                        .child("Set a password first.")
                }))
        };
        let auto_lock_control = {
            let current = crate::security::auto_lock_minutes();
            let mut row = div().flex().flex_row().flex_wrap().gap(px(6.0));
            for (label, mins) in [
                ("Off", 0u64),
                ("5 min", 5),
                ("15 min", 15),
                ("30 min", 30),
                ("1 hour", 60),
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
                        .child("Set a password first.")
                }))
        };

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
                            Tab::General => content
                                .child(self.section_card(
                                    "Data location",
                                    "Where Zorite keeps your database, settings, and \
                                         attachments. Changing it moves your data to the new \
                                         folder, then reopens Zorite.",
                                    location_control,
                                ))
                                .child(self.section_card(
                                    "Remember window position",
                                    "Reopen Zorite with the size and position it had when \
                                         you left. Falls back to centered if the saved spot's \
                                         display is gone.",
                                    window_bounds_switch,
                                ))
                                .child(self.section_card(
                                    "Unused images",
                                    "Delete images in the managed store that no note, \
                                         whiteboard, or template references. Files added in \
                                         the last hour are kept.",
                                    image_gc_control,
                                ))
                                .child(self.section_card(
                                    "Date format",
                                    "How /date and the {{date}} template placeholder are \
                                         inserted. Journal day headers are unaffected.",
                                    Select::new(&self.date_format_select).w_full(),
                                ))
                                .child(self.section_card(
                                    "Time format",
                                    "How /time and the {{time}} template placeholder are \
                                         inserted.",
                                    Select::new(&self.time_format_select).w_full(),
                                )),
                            Tab::Appearance => content
                                .child(self.section_card(
                                    "App Theme",
                                    "Pick a built-in theme or one of your own.",
                                    Select::new(&self.theme_select).w_full(),
                                ))
                                .child(self.section_card(
                                    "Appearance",
                                    "Light or dark variant of the active theme. Auto follows \
                                         your system.",
                                    Select::new(&self.appearance_select).w_full(),
                                ))
                                .child(self.section_card(
                                    "Font",
                                    "The typeface for the app and your notes. Default follows \
                                         the active theme's font, if it names one. Add a .ttf or \
                                         .otf file to use a font that isn't installed on your \
                                         system.",
                                    font_control,
                                ))
                                .child(self.section_card(
                                    "Text size",
                                    "Size of note text when editing and reading. Headings and \
                                         inline math scale with it.",
                                    Select::new(&self.text_size_select).w_full(),
                                ))
                                .child(self.section_card(
                                    "Installed themes",
                                    "Drop .json theme files in your themes folder, then Reload. \
                                         Any color you omit falls back to the base palette.",
                                    installed,
                                )),
                            Tab::Pdf => content.child(self.section_card(
                                "PDF render quality",
                                "Higher is sharper but slower; lower speeds up rendering on \
                                     slower machines. 100% = your display's native resolution.",
                                quality_control,
                            )),
                            Tab::Markdown => content
                                .child(self.section_card(
                                    "WYSIWYG editing",
                                    "On shows formatting (bold, headings, links) inline as you \
                                     type. Off edits plain Markdown and shows the rendered page \
                                     on Esc.",
                                    wysiwyg_switch,
                                ))
                                .child(self.section_card(
                                    "List indentation",
                                    "Spaces per nesting level for Tab and bullet nesting. Editing \
                                     and the rendered view use the same width, so they line up.",
                                    Select::new(&self.indent_select).w_full(),
                                ))
                                .child(self.section_card(
                                    "Auto-link page titles",
                                    "Typing a word or phrase that matches an existing page's \
                                     title wraps it as a [[wiki-link]] when you finish the word. \
                                     Undo reverts a wrap.",
                                    auto_link_switch,
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
                                    ("Export active tab as PDF", vec![keys::MOD, "P"]),
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
                                    .child(self.section_list(
                                        "Application",
                                        "App-wide commands — they work anywhere. ⌘ on macOS, \
                                             Ctrl on Windows and Linux.",
                                        app_rows,
                                    ))
                                    .child(self.section_list(
                                        "Editing",
                                        "The slash menu and text shortcuts, while a note is \
                                             focused.",
                                        edit_rows,
                                    ))
                                    .child(self.section_list(
                                        "Whiteboard tools",
                                        "Press a letter to pick a tool while a board is focused.",
                                        wb_tool_rows,
                                    ))
                                    .child(self.section_list(
                                        "Whiteboard editing",
                                        "Acting on the selection while a board is focused.",
                                        wb_edit_rows,
                                    ))
                                    .child(self.section_list(
                                        "PDF viewer",
                                        "While a PDF tab is focused.",
                                        pdf_rows,
                                    ))
                            }
                            Tab::Security => content
                                .child(self.section_card(
                                    "Password",
                                    "Encrypt the database with a password (SQLCipher). Zorite \
                                         asks for it at launch; without it the file on disk is \
                                         unreadable. A forgotten password is unrecoverable.",
                                    password_control,
                                ))
                                .child(self.section_card(
                                    "Remember on this device",
                                    if cfg!(target_os = "linux") {
                                        "Keep the password in the kernel keyring and unlock \
                                         automatically at launch. On Linux this lasts until \
                                         reboot; the idle auto-lock always requires typing it \
                                         again."
                                    } else {
                                        "Keep the password in the system keychain and unlock \
                                         automatically at launch. The idle auto-lock always \
                                         requires typing it again."
                                    },
                                    remember_control,
                                ))
                                .child(self.section_card(
                                    "Auto-lock",
                                    "Lock Zorite after this much inactivity, requiring the \
                                         password to continue.",
                                    auto_lock_control,
                                )),
                            Tab::Updates => content
                                .child(self.section_card(
                                    "Software updates",
                                    "Zorite checks GitHub for newer releases at startup. It never \
                                         installs automatically — you review and download from the \
                                         release page.",
                                    updates_control,
                                ))
                                .child(self.section_card(
                                    "Automatically check for updates",
                                    "Check for a newer version each time Zorite starts.",
                                    check_updates_switch,
                                ))
                                .child(self.section_card(
                                    "Include pre-releases",
                                    "Also offer beta builds (vX.Y.Z-beta.N), not just stable \
                                         releases.",
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
fn card_list(title: &str, desc: &str, rows: Vec<(&str, Vec<&str>)>) -> gpui::Div {
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

/// Human size for the images-GC listing: KB under a megabyte, else MB.
fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{:.0} KB", (bytes as f64 / 1024.0).max(1.0))
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
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
