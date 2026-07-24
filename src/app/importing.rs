//! Import/export glue: the Logseq/Obsidian import flows (pickers, option
//! dialogs, background runs, summaries) and the notebook markdown / PDF
//! export flows — split from `app.rs`. The engines live in `crate::import`,
//! `crate::export_md`, and `crate::export`.

use super::*;

impl AppView {
    /// `ExportPdf` handler (tab right-click): render the tab's markdown to a
    /// print-styled HTML file and open it in the browser — its print dialog's
    /// "Save as PDF" does the actual PDF (see `export.rs`). The Journal tab
    /// exports its loaded feed days under date headings; PDF / whiteboard
    /// tabs have nothing to print.
    pub(super) fn on_export_pdf(
        &mut self,
        _: &ExportPdf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Tab right-click sets context_target; the sidebar menu sets
        // context_page. (The active-tab path is its own action so a dismissed
        // menu's leftover context can't hijack a later secondary-p.)
        let target = self
            .context_target
            .take()
            .or_else(|| self.context_page.take().map(|(id, _)| TabKind::Page(id)));
        let Some(target) = target else {
            return;
        };
        self.export_tab(target, window, cx);
    }

    /// `ExportActivePdf` handler (File menu / secondary-p): export the active tab.
    pub(super) fn on_export_active_pdf(
        &mut self,
        _: &ExportActivePdf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.tabs[self.active].kind.clone();
        self.export_tab(target, window, cx);
    }

    /// Render `target`'s markdown to a PDF behind a native save dialog (see
    /// `export.rs`). The Journal exports its loaded feed days under date
    /// headings; PDF / whiteboard tabs have nothing to print.
    fn export_tab(&mut self, target: TabKind, window: &mut Window, cx: &mut Context<Self>) {
        let (title, source) = match &target {
            TabKind::Page(id) => match self.db.get_page(*id) {
                Ok(Some(page)) => (page.title, page.content),
                _ => return,
            },
            TabKind::Journal => {
                let mut out = String::new();
                for i in 0..self.loaded_days.max(1) {
                    let date = date_for_offset(i);
                    let content = match self.day_editors.get(&date) {
                        Some(de) => de.state.read(cx).value().to_string(),
                        None => self
                            .db
                            .get_journal_by_date(&date)
                            .ok()
                            .flatten()
                            .map(|p| p.content)
                            .unwrap_or_default(),
                    };
                    if content.trim().is_empty() {
                        continue;
                    }
                    out.push_str(&format!(
                        "# {}\n\n{}\n\n",
                        date_label(i),
                        content.trim_end()
                    ));
                }
                ("Journal".to_string(), out)
            }
            TabKind::Pdf(_)
            | TabKind::Whiteboard(_)
            | TabKind::AllPages
            | TabKind::Graph
            | TabKind::Properties
            | TabKind::Game => {
                return;
            }
        };
        // Native save dialog, then write the PDF (fast enough to run inline).
        let name: String = title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == ' ' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let rx = cx.prompt_for_new_path(
            &crate::paths::desktop_dir(),
            Some(&format!("{}.pdf", name.trim())),
        );
        cx.spawn_in(window, async move |_this, _cx| {
            let Ok(Ok(Some(path))) = rx.await else {
                return;
            };
            if let Err(e) =
                crate::export::export_pdf(&title, &source, &crate::paths::data_dir(), &path)
            {
                log::error!("export {title}: {e}");
            }
        })
        .detach();
    }

    /// `ImportLogseq` handler: pick a Logseq graph folder, then choose how
    /// the outline converts before importing.
    pub(super) fn on_import_logseq(
        &mut self,
        _: &ImportLogseq,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(root) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.show_logseq_options(root, window, cx);
            });
        })
        .detach();
    }

    /// `ImportObsidian` handler: pick a vault folder, then confirm options.
    pub(super) fn on_import_obsidian(
        &mut self,
        _: &ImportObsidian,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(root) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.show_obsidian_options(root, window, cx);
            });
        })
        .detach();
    }

    /// Confirm how a vault's folders map, then run the import.
    fn show_obsidian_options(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let (root_ns, root_flat, root_ok) = (root.clone(), root.clone(), root.clone());
            let (weak_ns, weak_flat, weak_ok) = (weak.clone(), weak.clone(), weak.clone());
            dialog
                .title("Import from Obsidian")
                .w(px(500.0))
                // Enter runs the primary action (namespaces), like the button.
                .on_ok(move |_, window, cx| {
                    window.close_dialog(cx);
                    let root = root_ok.clone();
                    let _ = weak_ok.update(cx, |this, cx| {
                        this.run_obsidian_import(root, true, window, cx)
                    });
                    false
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.0))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(format!("Importing \u{201c}{}\u{201d}.", root.display())),
                        )
                        .child(div().text_color(theme::text_secondary()).child(
                            "Obsidian folders can become Zorite namespaces. \u{201c}Preserve \
                             folders\u{201d} turns Projects/Tasks.md into the page \
                             Projects::Tasks (links resolve to it); \u{201c}Flatten\u{201d} \
                             uses just the note name.",
                        ))
                        .child(
                            DialogFooter::new()
                                .child(
                                    Button::new("ob-import-cancel")
                                        .label("Cancel")
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(Button::new("ob-import-flat").label("Flatten").on_click(
                                    move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let root = root_flat.clone();
                                        let _ = weak_flat.update(cx, |this, cx| {
                                            this.run_obsidian_import(root, false, window, cx)
                                        });
                                    },
                                ))
                                .child(
                                    Button::new("ob-import-ns")
                                        .primary()
                                        .label("Preserve folders")
                                        .on_click(move |_, window, cx| {
                                            window.close_dialog(cx);
                                            let root = root_ns.clone();
                                            let _ = weak_ns.update(cx, |this, cx| {
                                                this.run_obsidian_import(root, true, window, cx)
                                            });
                                        }),
                                ),
                        ),
                )
        });
    }

    /// File → Export → Notebook as Markdown…: pick an empty folder, then lay
    /// the whole notebook out as portable markdown + assets (see `export_md`).
    pub(super) fn on_export_notebook(
        &mut self,
        _: &ExportNotebook,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Export here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(dest) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.run_notebook_export(dest, window, cx);
            });
        })
        .detach();
    }

    fn run_notebook_export(&mut self, dest: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let data_dir = crate::paths::data_dir();
        let task = cx.background_executor().spawn(async move {
            let key = crate::security::session_key();
            let db =
                Db::open(key.as_deref()).map_err(|e| format!("open database: {}", e.source))?;
            let pages = db.export_pages().map_err(|e| format!("read pages: {e}"))?;
            let plan = crate::export_md::plan_export(&pages);
            crate::export_md::write_export(&data_dir, &dest, plan)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.show_export_summary(result, window, cx);
            });
        })
        .detach();
    }

    /// The completion dialog for a notebook export: counts, or the error.
    fn show_export_summary(
        &mut self,
        result: Result<crate::export_md::ExportSummary, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let dialog = dialog.w(px(460.0)).on_ok(|_, _, _| true);
            match &result {
                Ok(s) => {
                    let mut lines = vec![format!(
                        "{} pages, {} journal days, {} whiteboard{}, and {} asset file{} written.",
                        s.pages,
                        s.days,
                        s.boards,
                        if s.boards == 1 { "" } else { "s" },
                        s.assets,
                        if s.assets == 1 { "" } else { "s" }
                    )];
                    lines.extend(s.warnings.iter().cloned());
                    dialog.title("Export complete").child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .text_color(theme::text_secondary())
                            .children(lines),
                    )
                }
                Err(e) => dialog
                    .title("Export failed")
                    .child(div().text_color(theme::text_secondary()).child(e.clone())),
            }
        });
    }

    fn run_obsidian_import(
        &mut self,
        root: PathBuf,
        namespaces: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.open_dialog(cx, |dialog, _window, _cx| {
            dialog
                .title("Importing from Obsidian\u{2026}")
                .w(px(400.0))
                .child(
                    div()
                        .text_color(theme::text_secondary())
                        .child("Copying notes and assets \u{2014} this may take a minute."),
                )
                .on_ok(|_, _window, _cx| false)
                .on_cancel(|_, _window, _cx| true)
        });
        let data_dir = crate::paths::data_dir();
        let task = cx.background_executor().spawn(async move {
            let key = crate::security::session_key();
            let db =
                Db::open(key.as_deref()).map_err(|e| format!("open database: {}", e.source))?;
            let opts = crate::import::obsidian::Options { namespaces };
            let bundle = crate::import::obsidian::read_vault(&root, &opts)?;
            crate::import::write_bundle(&db, &data_dir, bundle, |_, _| {})
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                window.close_dialog(cx);
                this.refresh_sidebar();
                this.signal_doc_changed(cx);
                this.show_import_summary("Obsidian", result, window, cx);
            });
        })
        .detach();
    }

    /// Ask how Logseq's all-bullets outline should convert, then run the import.
    fn show_logseq_options(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let (root_flat, root_list, root_ok) = (root.clone(), root.clone(), root.clone());
            let (weak_flat, weak_list, weak_ok) = (weak.clone(), weak.clone(), weak.clone());
            dialog
                .title("Import from Logseq")
                .w(px(500.0))
                // Enter runs the primary action (Flatten outline), like the button.
                .on_ok(move |_, window, cx| {
                    window.close_dialog(cx);
                    let root = root_ok.clone();
                    let _ = weak_ok.update(cx, |this, cx| {
                        this.run_logseq_import(root, true, window, cx)
                    });
                    false
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_color(theme::text_secondary())
                                .child(format!("Importing “{}”.", root.display())),
                        )
                        .child(div().text_color(theme::text_secondary()).child(
                            "Logseq makes every line a bullet. “Flatten outline” turns each \
                             top-level bullet into a paragraph or heading (nested bullets stay \
                             lists) so pages read like Zorite pages; “Keep bullets” preserves \
                             the outline exactly. Existing pages keep their content — imported \
                             text is appended below it.",
                        )),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("ls-import-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("ls-import-bullets")
                                .label("Keep bullets")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let root = root_list.clone();
                                    let _ = weak_list.update(cx, |this, cx| {
                                        this.run_logseq_import(root, false, window, cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new("ls-import-flatten")
                                .primary()
                                .label("Flatten outline")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let root = root_flat.clone();
                                    let _ = weak_flat.update(cx, |this, cx| {
                                        this.run_logseq_import(root, true, window, cx)
                                    });
                                }),
                        ),
                )
                .on_cancel(|_, _window, _cx| true)
        });
    }

    /// Import `root` on a background thread (its own DB connection — WAL keeps
    /// it concurrent with this one), then show the summary and refresh.
    fn run_logseq_import(
        &mut self,
        root: PathBuf,
        flatten: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.open_dialog(cx, |dialog, _window, _cx| {
            dialog
                .title("Importing from Logseq…")
                .w(px(400.0))
                .child(
                    div()
                        .text_color(theme::text_secondary())
                        .child("Copying notes and assets — this may take a minute."),
                )
                // A progress indicator has no confirm action — Enter shouldn't
                // dismiss it (Escape still cancels).
                .on_ok(|_, _window, _cx| false)
                .on_cancel(|_, _window, _cx| true)
        });
        let data_dir = crate::paths::data_dir();
        let task = cx.background_executor().spawn(async move {
            let key = crate::security::session_key();
            let db =
                Db::open(key.as_deref()).map_err(|e| format!("open database: {}", e.source))?;
            let opts = crate::import::logseq::Options { flatten };
            let bundle = crate::import::logseq::read_graph(&root, &opts)?;
            crate::import::write_bundle(&db, &data_dir, bundle, |_, _| {})
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                window.close_dialog(cx);
                this.refresh_sidebar();
                // Reload journal days / the open page from the DB everywhere.
                this.signal_doc_changed(cx);
                this.show_import_summary("Logseq", result, window, cx);
            });
        })
        .detach();
    }

    /// Post-import summary (or failure) dialog.
    fn show_import_summary(
        &mut self,
        source: &'static str,
        result: Result<crate::import::Summary, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        /// At most `n` names, with a `+ N more` tail when the list is long.
        fn sample(list: &[String], n: usize) -> String {
            let mut s = list.iter().take(n).cloned().collect::<Vec<_>>().join(", ");
            if list.len() > n {
                s.push_str(&format!(" — and {} more", list.len() - n));
            }
            s
        }
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let (title, lines) = match &result {
                Ok(s) => {
                    let mut lines = vec![format!(
                        "{} pages, {} journal days, {} PDF-highlight pages, \
                         {} whiteboards; {} assets copied; {} favorites.",
                        s.pages,
                        s.journals,
                        s.highlight_pages,
                        s.whiteboards,
                        s.assets_copied,
                        s.favorites
                    )];
                    if !s.appended.is_empty() {
                        lines.push(format!(
                            "Appended below existing content: {}.",
                            sample(&s.appended, 6)
                        ));
                    }
                    if !s.warnings.is_empty() {
                        lines.push(format!("Warnings: {}", sample(&s.warnings, 6)));
                    }
                    (format!("{source} import complete"), lines)
                }
                Err(e) => (format!("{source} import failed"), vec![e.clone()]),
            };
            dialog
                .title(title)
                .w(px(520.0))
                .child(
                    div().flex().flex_col().gap(px(8.0)).children(
                        lines
                            .into_iter()
                            .map(|l| div().text_color(theme::text_secondary()).child(l)),
                    ),
                )
                .footer(
                    DialogFooter::new().child(
                        Button::new("ls-import-done")
                            .primary()
                            .label("Done")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
                )
                .on_cancel(|_, _window, _cx| true)
        });
    }
}
