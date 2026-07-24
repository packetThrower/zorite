//! Dialog builders — the `open_dialog` / `open_alert_dialog` prompts (new
//! page, rename, delete confirmations, whiteboard embeds and templates, the
//! DB-error modal) and their submit helpers — split from `app.rs`.

use super::*;

impl AppView {
    /// Open the "insert page card" dialog, then place the chosen page as a card
    /// at world `(x, y)` on board `board_id`.
    pub(super) fn place_embed_dialog(
        &mut self,
        board_id: i64,
        x: f32,
        y: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_page_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let input = self.new_page_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let weak_ok = weak.clone();
            let weak_btn = weak.clone();
            dialog
                .title("Insert page card")
                .w(px(420.0))
                // Enter inserts (the dialog binds enter → ConfirmDialog → on_ok).
                .on_ok(move |_, _window, cx| {
                    let _ = weak_ok.update(cx, |this, cx| {
                        this.insert_embed_from_input(board_id, x, y, cx)
                    });
                    true
                })
                .child(Input::new(&input))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("embed-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("embed-insert")
                                .primary()
                                .label("Insert")
                                .on_click(move |_, window, cx| {
                                    let _ = weak_btn.update(cx, |this, cx| {
                                        this.insert_embed_from_input(board_id, x, y, cx)
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
        });
    }

    /// Resolve `title` to a page (creating it) and add it as a card on the board.
    fn insert_embed(&mut self, board_id: i64, title: &str, x: f32, y: f32, cx: &mut Context<Self>) {
        let page = match self.db.get_or_create_page(title) {
            Ok(p) => p,
            Err(e) => {
                log::error!("embed page {title:?}: {e}");
                return;
            }
        };
        if let Some(view) = self.whiteboard_views.get(&board_id).cloned() {
            // Persist here (not via the view's on_change) — we're already inside
            // an AppView update, so a re-entrant save would panic.
            let json = view.update(cx, |v, cx| {
                v.add_embed(page.id, page.title.clone(), x, y, cx);
                v.scene().to_json()
            });
            self.save_board(board_id, &json);
        }
        self.record_recent(page.id);
        self.refresh_sidebar();
        cx.notify();
    }

    /// Insert the page named in the shared input as a card (no-op if blank).
    /// Shared by the Insert button and Enter (`on_ok`).
    fn insert_embed_from_input(&mut self, board_id: i64, x: f32, y: f32, cx: &mut Context<Self>) {
        let title = self.new_page_input.read(cx).value().trim().to_string();
        if !title.is_empty() {
            self.insert_embed(board_id, &title, x, y, cx);
        }
    }

    /// Prompt for a name, then persist the selection JSON as a whiteboard
    /// template (invoked from a board's right-click "Save as template").
    pub(super) fn save_template_dialog(
        &mut self,
        json: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_page_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let input = self.new_page_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let weak_ok = weak.clone();
            let json_ok = json.clone();
            let weak_btn = weak.clone();
            let json_btn = json.clone();
            dialog
                .title("Save as template")
                .w(px(420.0))
                // The dialog binds `enter` → ConfirmDialog → `on_ok`; without
                // this, Enter closes the dialog without saving (looks like
                // Cancel). Save here, same as the button.
                .on_ok(move |_, _window, cx| {
                    let _ = weak_ok.update(cx, |this, cx| this.save_template_named(&json_ok, cx));
                    true
                })
                .child(Input::new(&input))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("tmpl-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("tmpl-save").primary().label("Save").on_click(
                            move |_, window, cx| {
                                let _ = weak_btn
                                    .update(cx, |this, cx| this.save_template_named(&json_btn, cx));
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    /// Read the template name from the shared page-name input (blank → a default
    /// title) and store it. Shared by the Save button and Enter (`on_ok`).
    fn save_template_named(&mut self, json: &str, cx: &mut Context<Self>) {
        let name = self.new_page_input.read(cx).value().trim().to_string();
        let name = if name.is_empty() {
            "Untitled template".to_string()
        } else {
            name
        };
        self.save_template(&name, json, cx);
    }

    /// Store a template and push the refreshed list to every open board.
    fn save_template(&mut self, name: &str, json: &str, cx: &mut Context<Self>) {
        match self.db.create_template(name, json) {
            Ok(_) => self.refresh_templates(cx),
            Err(e) => log::error!("save template {name:?}: {e}"),
        }
    }

    /// Confirm, then delete a template (invoked from a right-click on its card).
    pub(super) fn confirm_delete_template(
        &mut self,
        tid: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self
            .db
            .list_templates()
            .unwrap_or_default()
            .into_iter()
            .find(|(id, ..)| *id == tid)
            .map(|(_, name, _)| name)
            .unwrap_or_else(|| "this template".to_string());
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let weak = weak.clone();
            dialog
                .title("Delete template?")
                .description(SharedString::from(format!(
                    "“{name}” will be permanently deleted. This can't be undone."
                )))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("Cancel")
                        .show_cancel(true),
                )
                .on_ok(move |_, _window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if let Err(e) = this.db.delete_template(tid) {
                            log::error!("delete template {tid}: {e}");
                        } else {
                            this.refresh_templates(cx);
                        }
                    });
                    true
                })
        });
    }

    /// Surface a failed database open as a one-time modal, so the user learns why
    /// their notes look empty and where the pre-migration backup is — rather than
    /// silently landing in a blank workspace. Changes made here aren't persisted.
    pub(super) fn show_db_error_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(err) = self.db_error.as_ref() else {
            return;
        };
        let folder = err.folder.clone();
        // SQLite reports batch failures as "<reason> in <whole SQL> at offset N";
        // keep just the reason (the full text is in the log) so the dialog stays
        // readable, and cap length defensively.
        let detail: String = err
            .message
            .split(" in ")
            .next()
            .unwrap_or(&err.message)
            .chars()
            .take(200)
            .collect();
        let recovery = match &err.backup {
            Some(b) => format!(
                "Your notes were backed up before the update and are safe — restore them from {}",
                b.display()
            ),
            None => format!("Your notes on disk are unchanged, in {}", folder.display()),
        };
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let folder = folder.clone();
            dialog
                .title("Couldn't open your notes database")
                .w(px(480.0))
                // Enter triggers the primary action (Quit); the temporary
                // workspace isn't saved, so there's nothing to lose.
                .on_ok(|_, _window, cx| {
                    cx.quit();
                    true
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(div().text_color(theme::text_secondary()).child(
                            "Zorite opened a temporary, empty workspace because the database \
                                 couldn't be opened or upgraded. Changes here won't be saved.",
                        ))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(theme::text_secondary())
                                .child(detail.clone()),
                        )
                        .child(div().child(recovery.clone())),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("db-error-reveal")
                                .label("Reveal Backup")
                                .on_click(move |_, _window, _cx| AppView::reveal_folder(&folder)),
                        )
                        .child(
                            Button::new("db-error-quit")
                                .primary()
                                .label("Quit")
                                .on_click(|_, _window, cx| cx.quit()),
                        ),
                )
        });
    }

    /// `DeletePage` handler: confirm, then delete the remembered page.
    pub(super) fn on_delete_page(
        &mut self,
        _: &DeletePage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((id, title)) = self.context_page.take() else {
            return;
        };
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _window, _cx| {
            let weak = weak.clone();
            dialog
                .title("Delete page?")
                .description(SharedString::from(format!(
                    "“{title}” will be permanently deleted. This can't be undone."
                )))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("Cancel")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| this.delete_page(id, window, cx));
                    true
                })
        });
    }

    /// A one-button error dialog — the voice for user-initiated operations
    /// (rename, delete, export, form writes) whose failures used to be
    /// log-only. NOTE: `feat/notebooks` adds an identical helper; drop one
    /// copy when the branches merge.
    pub(super) fn show_error_dialog(
        &mut self,
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

    fn delete_page(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.delete_page(id) {
            Ok(true) => {
                // Drop a deleted page from favorites so the dead id doesn't linger.
                if let Some(pos) = self.favorites.iter().position(|&x| x == id) {
                    self.favorites.remove(pos);
                    self.persist_favorites();
                }
                // Close any tabs showing the deleted page (journal at 0 is safe).
                let mut i = self.tabs.len();
                while i > 1 {
                    i -= 1;
                    if matches!(self.tabs[i].kind, TabKind::Page(pid) if pid == id) {
                        self.tabs.remove(i);
                        if self.active > i {
                            self.active -= 1;
                        } else if self.active == i {
                            self.active = self.active.min(self.tabs.len() - 1);
                        }
                    }
                }
                self.refresh_sidebar();
                self.signal_doc_changed(cx);
                self.activate_tab(self.active, window, cx);
            }
            Ok(false) => {}
            Err(e) => {
                log::error!("delete page {id}: {e}");
                self.show_error_dialog("Couldn’t delete the page", e.to_string(), window, cx);
            }
        }
    }

    pub(super) fn open_new_page_dialog(
        &mut self,
        dialog_title: &'static str,
        prefill: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_page_input
            .update(cx, |s, cx| s.set_value(prefill, window, cx));
        let input = self.new_page_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_body = input.clone();
            let input_btn = input.clone();
            let input_key = input.clone();
            let weak_btn = weak.clone();
            let weak_key = weak.clone();
            dialog
                .title(dialog_title)
                .w(px(420.0))
                .child(Input::new(&input_body))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("new-page-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("new-page-create")
                                .primary()
                                .label("Create")
                                .on_click(move |_, window, cx| {
                                    let title = input_btn.read(cx).value().trim().to_string();
                                    if !title.is_empty() {
                                        let _ = weak_btn.update(cx, |this, cx| {
                                            this.open_page_title(&title, window, cx)
                                        });
                                    }
                                    window.close_dialog(cx);
                                }),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let title = input_key.read(cx).value().trim().to_string();
                    if !title.is_empty() {
                        let _ = weak_key
                            .update(cx, |this, cx| this.open_page_title(&title, window, cx));
                    }
                    true
                })
                .on_cancel(|_, _window, _cx| true)
        });
        self.new_page_input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// `RenamePage` handler: open a dialog with a text field, pre-filled
    /// with the current title, to rename the right-clicked page.
    pub(super) fn on_rename_page(
        &mut self,
        _: &RenamePage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((id, title)) = self.context_page.take() else {
            return;
        };
        self.rename_target = Some(id);
        *self.rename_error.borrow_mut() = None;
        self.rename_input
            .update(cx, |s, cx| s.set_value(title.to_string(), window, cx));

        // `AlertDialog` is title/description-only; a text field needs the
        // generic `Dialog` (it impls `ParentElement`, so the Input goes in as
        // a child) with a footer we build ourselves. Enter/Escape are wired
        // via on_ok/on_cancel.
        let input = self.rename_input.clone();
        let err = self.rename_error.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_body = input.clone();
            let input_btn = input.clone();
            let input_key = input.clone();
            let weak_btn = weak.clone();
            let weak_key = weak.clone();
            // A failed rename (collision, DB error) reports INSIDE this dialog
            // and keeps it open — a second dialog on top would pop this one off
            // the dialog stack and read as "nothing happened".
            let error = err.borrow().clone();
            dialog
                .title("Rename page")
                .w(px(420.0))
                .child(Input::new(&input_body))
                .children(error.map(|e| {
                    div()
                        .mt(px(6.0))
                        .text_size(px(12.0))
                        .text_color(gpui::rgb(0xE5484D))
                        .child(e)
                }))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("rename-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(Button::new("rename-ok").primary().label("Rename").on_click(
                            move |_, window, cx| {
                                let title = input_btn.read(cx).value().to_string();
                                let done = weak_btn
                                    .update(cx, |this, cx| this.commit_rename(title, window, cx))
                                    .unwrap_or(true);
                                if done {
                                    window.close_dialog(cx);
                                }
                            },
                        )),
                )
                .on_ok(move |_, window, cx| {
                    let title = input_key.read(cx).value().to_string();
                    weak_key
                        .update(cx, |this, cx| this.commit_rename(title, window, cx))
                        .unwrap_or(true)
                })
                .on_cancel(|_, _window, _cx| true)
        });
        self.rename_input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Apply a confirmed rename: rewrite `[[links]]`, refresh the sidebar,
    /// and update any open tab titles for the page.
    /// Returns whether the rename dialog should close: a success or a
    /// cancel-like no-op (empty/unchanged name) closes; a collision or DB
    /// error keeps it open showing `rename_error` inline.
    fn commit_rename(
        &mut self,
        new_title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.rename_target else {
            return true;
        };
        match self.db.rename_page(id, &new_title) {
            Ok(true) => {
                self.rename_target = None;
                *self.rename_error.borrow_mut() = None;
                let title: SharedString = new_title.trim().to_string().into();
                for tab in &mut self.tabs {
                    if matches!(tab.kind, TabKind::Page(pid) if pid == id) {
                        tab.title = title.clone();
                    }
                }
                self.refresh_sidebar();
                self.reload_day_editors(window, cx);
                self.signal_doc_changed(cx);
                self.activate_tab(self.active, window, cx);
                true
            }
            Ok(false) => {
                // Only a collision needs a voice — an empty or unchanged name
                // reads as cancelling the dialog.
                let t = new_title.trim().to_string();
                if !t.is_empty() && self.title_collides(id, &t) {
                    *self.rename_error.borrow_mut() =
                        Some(format!("A page named “{t}” already exists.").into());
                    cx.notify();
                    false
                } else {
                    self.rename_target = None;
                    true
                }
            }
            Err(e) => {
                log::error!("rename page {id}: {e}");
                *self.rename_error.borrow_mut() = Some(format!("Rename failed: {e}").into());
                cx.notify();
                false
            }
        }
    }

    /// Whether another page already owns `title` (the silent-no-op reason a
    /// rename most often fails).
    fn title_collides(&self, id: i64, title: &str) -> bool {
        self.db
            .get_page_by_title(title)
            .ok()
            .flatten()
            .is_some_and(|p| p.id != id)
    }

    /// The row's ✎ button: the shared rename dialog, targeted at a notebook.
    pub fn rename_notebook_dialog(
        &mut self,
        nb: crate::paths::Notebook,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notebook_popover = false;
        self.notebook_rename_target = Some(nb.dir.clone());
        self.rename_input
            .update(cx, |s, cx| s.set_value(nb.name, window, cx));
        let input = self.rename_input.clone();
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let input_body = input.clone();
            let input_btn = input.clone();
            let input_key = input.clone();
            let weak_btn = weak.clone();
            let weak_key = weak.clone();
            dialog
                .title("Rename notebook")
                .w(px(420.0))
                .child(Input::new(&input_body))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("nb-rename-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("nb-rename-ok")
                                .primary()
                                .label("Rename")
                                .on_click(move |_, window, cx| {
                                    let name = input_btn.read(cx).value().to_string();
                                    let _ = weak_btn.update(cx, |this, cx| {
                                        this.commit_notebook_rename(name, cx)
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                )
                .on_ok(move |_, _window, cx| {
                    let name = input_key.read(cx).value().to_string();
                    let _ = weak_key.update(cx, |this, cx| this.commit_notebook_rename(name, cx));
                    true
                })
                .on_cancel(|_, _window, _cx| true)
        });
        self.rename_input.update(cx, |s, cx| s.focus(window, cx));
    }

    fn commit_notebook_rename(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(dir) = self.notebook_rename_target.take() else {
            return;
        };
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Err(e) = crate::paths::rename_notebook(&dir, name) {
            log::error!("rename notebook: {e}");
        }
        self.refresh_notebooks();
        cx.notify();
    }

    /// Rename the open page from its inline title field. Updates state in
    /// place (no tab reload) so the title field keeps focus; reverts the
    /// field if the new name is empty, a duplicate, or a journal.
    pub(super) fn commit_title_rename(
        &mut self,
        id: i64,
        new_title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((current, title_state)) = self
            .page_editor
            .as_ref()
            .map(|pe| (pe.title.clone(), pe.title_state.clone()))
        else {
            return;
        };
        if new_title == current {
            return;
        }
        match self.db.rename_page(id, &new_title) {
            Ok(true) => {
                // Backlink snippets now show the rewritten `[[new]]` text.
                let backlinks = self.db.backlinks(id).unwrap_or_default();
                let unlinked = self.db.unlinked_mentions(id).unwrap_or_default();
                self.warm_backlink_labels(&backlinks, cx);
                if let Some(pe) = self.page_editor.as_mut() {
                    pe.title = new_title.clone();
                    pe.backlinks = backlinks;
                    pe.unlinked = unlinked;
                }
                let title: SharedString = new_title.into();
                for tab in &mut self.tabs {
                    if matches!(tab.kind, TabKind::Page(pid) if pid == id) {
                        tab.title = title.clone();
                    }
                }
                self.refresh_sidebar();
                self.reload_day_editors(window, cx);
                self.signal_doc_changed(cx);
                cx.notify();
            }
            Ok(false) => {
                // Empty, duplicate, or journal — revert the field, and say
                // why when it was a collision (the only surprising case).
                let t = new_title.trim().to_string();
                title_state.update(cx, |s, cx| s.set_value(current, window, cx));
                if !t.is_empty() && self.title_collides(id, &t) {
                    self.show_error_dialog(
                        "Can’t rename",
                        format!("A page named “{t}” already exists."),
                        window,
                        cx,
                    );
                }
                cx.notify();
            }
            Err(e) => {
                log::error!("rename page {id} (inline): {e}");
                title_state.update(cx, |s, cx| s.set_value(current, window, cx));
                self.show_error_dialog("Rename failed", e.to_string(), window, cx);
            }
        }
    }
}
