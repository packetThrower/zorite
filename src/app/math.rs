//! Math / LaTeX: the `$$…$$` raster pipeline, the structural (RaTeX)
//! in-line editor session, and the formula menu handlers — split from `app.rs`.

use super::*;

/// Map the editor's `<!-- math:ALIGN -->` marker alignment to the in-line editor's, and back.
fn to_ratex_align(a: gpui_editor::MathAlign) -> ratex_gpui::MathAlign {
    match a {
        gpui_editor::MathAlign::Left => ratex_gpui::MathAlign::Left,
        gpui_editor::MathAlign::Center => ratex_gpui::MathAlign::Center,
        gpui_editor::MathAlign::Right => ratex_gpui::MathAlign::Right,
    }
}
fn to_editor_align(a: ratex_gpui::MathAlign) -> gpui_editor::MathAlign {
    match a {
        ratex_gpui::MathAlign::Left => gpui_editor::MathAlign::Left,
        ratex_gpui::MathAlign::Center => gpui_editor::MathAlign::Center,
        ratex_gpui::MathAlign::Right => gpui_editor::MathAlign::Right,
    }
}

pub(super) struct MathEdit {
    editor: Entity<ratex_gpui::MathEditor>,
    source: Entity<EditorState>,
    target: SlashTarget,
    /// `true` when editing an inline `$…$` span (commit splices `$…$`), `false` for a `$$`
    /// block (splices `$$\n…\n$$` + alignment marker).
    inline: bool,
    /// Commits the edit when the math editor loses focus (click-away). Kept alive here.
    _blur_sub: gpui::Subscription,
    /// Flows the caret back to the text when an arrow hits a formula boundary. Kept alive.
    _nav_sub: gpui::Subscription,
}

impl AppView {
    /// Ensure the `$$…$$` block `source` is typesetting/typeset (idempotent). Called from
    /// a not-yet-rendered formula's placeholder the first time it paints: claims the slot,
    /// then typesets the LaTeX via RaTeX off-thread and repaints when it lands.
    pub fn ensure_math_loaded(&mut self, source: SharedString, cx: &mut Context<Self>) {
        // Tint formulas in the current theme's text color; set_color drops the cached rasters
        // if the theme changed, so a light/dark switch re-renders them.
        let color = theme::text_primary();
        {
            let mut store = self.math_store.borrow_mut();
            store.set_color(color);
            if !store.begin(source.clone()) {
                return; // already rendering / ready / failed
            }
        }
        let store = self.math_store.clone();
        cx.spawn(async move |this, cx| {
            let src = source.to_string();
            let result = cx
                .background_executor()
                .spawn(async move {
                    ratex_gpui::render::render_latex(
                        &src,
                        crate::math::FONT_SIZE,
                        crate::math::DPR,
                        color,
                    )
                    .map(|r| (r.image, r.width, r.height))
                })
                .await;
            store.borrow_mut().finish(source, result);
            // See the analogous comment in `ensure_mermaid_loaded`: `cx.notify()`
            // alone can leave a stale cached row layout, painted before the
            // formula existed.
            let _ = this.update(cx, |_, cx| {
                cx.notify();
                cx.refresh_windows();
            });
        })
        .detach();
    }

    pub fn open_math_menu(
        &mut self,
        source: SharedString,
        anchor: Point<Pixels>,
        alignable: bool,
        cx: &mut Context<Self>,
    ) {
        self.ctx_menu = Some(CtxMenu {
            anchor,
            kind: CtxKind::Formula { source, alignable },
        });
        cx.notify();
    }

    /// Re-justify the formula being edited (the right-click "Align" items). Live feedback;
    /// the marker persists on commit.
    pub(super) fn ctx_menu_align(&mut self, align: ratex_gpui::MathAlign, cx: &mut Context<Self>) {
        self.ctx_menu = None;
        if let Some(me) = &self.math_edit {
            me.editor.update(cx, |ed, cx| ed.set_align(align, cx));
        }
        cx.notify();
    }

    /// The LaTeX source of the open formula menu, taken (closing the menu), or `None` if the
    /// open menu isn't a formula one.
    fn take_ctx_formula(&mut self) -> Option<String> {
        match self.ctx_menu.take()? {
            CtxMenu {
                kind: CtxKind::Formula { source, .. },
                ..
            } => Some(source.to_string()),
            _ => None,
        }
    }

    pub(super) fn math_menu_copy_latex(&mut self, cx: &mut Context<Self>) {
        if let Some(source) = self.take_ctx_formula() {
            cx.write_to_clipboard(ClipboardItem::new_string(source));
        }
        cx.notify();
    }

    pub(super) fn math_menu_export_png(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.take_ctx_formula() else {
            return;
        };
        cx.notify();
        let rx = cx.prompt_for_new_path(crate::paths::desktop_dir().as_path(), Some("formula.png"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let result = ratex_gpui::render::render_latex_to_png(&source, 48.0, 4.0)
                .ok_or_else(|| "the formula didn’t render".to_string())
                .and_then(|png| std::fs::write(&path, png).map_err(|e| e.to_string()));
            if let Err(e) = result {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.show_error_dialog("Export failed", e, window, cx);
                });
            }
        })
        .detach();
    }

    pub(super) fn math_menu_export_svg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.take_ctx_formula() else {
            return;
        };
        cx.notify();
        let rx = cx.prompt_for_new_path(crate::paths::desktop_dir().as_path(), Some("formula.svg"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let result = ratex_gpui::render::render_latex_to_svg(&source, 48.0)
                .ok_or_else(|| "the formula didn’t render".to_string())
                .and_then(|svg| std::fs::write(&path, svg).map_err(|e| e.to_string()));
            if let Err(e) = result {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.show_error_dialog("Export failed", e, window, cx);
                });
            }
        })
        .detach();
    }

    /// Open the structural editor for a `$$` block (clicked or arrowed into): seed it from
    /// `latex`, remember the note editor + document + byte range to write back to, and focus
    /// it with the caret at the formula's end (`at_end`) or its start.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn open_math_edit(
        &mut self,
        source: Entity<EditorState>,
        target: SlashTarget,
        range: std::ops::Range<usize>,
        latex: SharedString,
        at_end: bool,
        inline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Clicking another formula opens its editor; commit the one we were editing first, so
        // its edits (incl. justification) persist instead of being dropped when math_edit is
        // replaced. Always re-find this block's range against the current content afterward: a
        // just-committed formula (here, or via its deferred blur) may have shifted byte offsets
        // by adding/removing an alignment marker.
        let mut range = range;
        if self.math_edit.is_some() {
            self.commit_math_edit(cx);
        }
        // Re-find THIS block by its exact LaTeX against the current content (the commit above,
        // or this block's own deferred blur, may have shifted offsets). Matching the source —
        // not a guessed-nearest range — keeps us on the right block; bail if it's gone.
        let found = if inline {
            source.read(cx).find_inline_math(&latex, range.start)
        } else {
            source.read(cx).find_math_block(&latex, range.start)
        };
        match found {
            Some(r) => range = r,
            None => return,
        }
        // Inline `$…$` renders at text size with no alignment; a `$$` block at its larger
        // display font and its saved justification.
        let (font_size, align) = if inline {
            (self.text_size, ratex_gpui::MathAlign::default())
        } else {
            (
                crate::math::FONT_SIZE,
                to_ratex_align(source.read(cx).math_align(range.start)),
            )
        };
        let editor = cx.new(|cx| {
            ratex_gpui::MathEditor::from_latex(
                &latex,
                font_size,
                at_end,
                align,
                ratex_gpui::MathTheme {
                    fg: theme::text_primary(),
                    muted: theme::text_secondary(),
                    panel: theme::elevated(),
                    border: theme::divider(),
                    accent: theme::accent(),
                    accent_bg: theme::accent_tint(),
                },
                cx,
            )
        });
        let focus = editor.read(cx).focus_handle();
        // Seat the editor: an inline `$…$` overlays the formula's spot (surrounding text stays
        // put); a `$$` block reserves a full-width gap at its row, sized to the cached render.
        if inline {
            source.update(cx, |e, cx| {
                e.set_editing_inline(range, editor.clone().into(), cx)
            });
        } else {
            let height = self
                .math_store
                .borrow()
                .get(&latex)
                .map_or(px(56.0), |(_, _, h)| px(h + 16.0));
            source.update(cx, |e, cx| {
                e.set_editing_block(range, editor.clone().into(), height, cx)
            });
        }
        // Commit when the math editor loses focus (the user clicks away). Guard on identity:
        // if a click on another formula already committed + replaced us, this stale blur must
        // not commit the NEW edit. (Compare the active edit's editor to ours.)
        let weak = cx.entity().downgrade();
        let editor_id = editor.entity_id();
        let blur_sub = window.on_focus_out(&focus, cx, move |_ev, _window, cx| {
            weak.update(cx, |this: &mut AppView, cx| {
                if this
                    .math_edit
                    .as_ref()
                    .is_some_and(|m| m.editor.entity_id() == editor_id)
                {
                    this.commit_math_edit(cx);
                }
            })
            .ok();
        });
        // Arrowing past a formula boundary flows the caret back into the surrounding text;
        // a right-click while editing opens the formula menu (copy LaTeX / export).
        let nav_sub = cx.subscribe_in(
            &editor,
            window,
            |this, editor, ev: &ratex_gpui::MathNav, window, cx| match ev {
                ratex_gpui::MathNav::Exit { after } => this.exit_math_edit(*after, window, cx),
                ratex_gpui::MathNav::ContextMenu { position } => {
                    let latex = editor.read(cx).to_latex();
                    // Editing → offer Align (the in-line editor can re-justify live).
                    this.open_math_menu(latex.into(), *position, true, cx);
                }
            },
        );
        self.math_edit = Some(MathEdit {
            editor,
            source,
            target,
            inline,
            _blur_sub: blur_sub,
            _nav_sub: nav_sub,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Commit the structural edit: serialize the formula to LaTeX, splice it back into the
    /// `$$…$$` block, persist, and return the note editor + the block's new byte range (so the
    /// caret can flow out to it). No-op (→ `None`) if the source range shifted out of bounds.
    fn commit_math_edit(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(Entity<EditorState>, std::ops::Range<usize>)> {
        let edit = self.math_edit.take()?;
        let latex = edit.editor.read(cx).to_latex();
        // Inline `$…$`: end the overlay, splice `$latex$` back at the span (guarded against a
        // stale range), persist. No `$$` fences, no alignment marker.
        if edit.inline {
            let range = edit.source.update(cx, |e, cx| e.end_editing_inline(cx))?;
            if !edit.source.read(cx).is_inline_math_range(&range) {
                cx.notify();
                return None;
            }
            // A same-line `$$…$$` display pair keeps its double delimiters.
            let content = edit.source.read(cx).text().to_string();
            let dollars = if content[range.clone()].starts_with("$$") {
                "$$"
            } else {
                "$"
            };
            let pair = format!("{dollars}{latex}{dollars}");
            // `$$…$$` is DISPLAY math — it never stays words-mixed. When the
            // span shares its line with other text, the commit splits the
            // line (words / formula / words) so the formula renders as a
            // block with the words visible (issue #54). Single-`$` spans
            // stay inline in place.
            let line_start = content[..range.start].rfind('\n').map_or(0, |p| p + 1);
            let line_end = content[range.end..]
                .find('\n')
                .map_or(content.len(), |p| range.end + p);
            let before = content[line_start..range.start].trim_end();
            let after = content[range.end..line_end].trim_start();
            let (edit_range, replacement, pair_start) =
                if dollars == "$$" && (!before.is_empty() || !after.is_empty()) {
                    let mut repl = String::new();
                    if !before.is_empty() {
                        repl.push_str(before);
                        repl.push('\n');
                    }
                    let ps = repl.len();
                    repl.push_str(&pair);
                    if !after.is_empty() {
                        repl.push('\n');
                        repl.push_str(after);
                    }
                    (line_start..line_end, repl, ps)
                } else {
                    (range.clone(), pair.clone(), 0)
                };
            let new_range =
                edit_range.start + pair_start..edit_range.start + pair_start + pair.len();
            edit.source
                .update(cx, |e, cx| e.replace_range(edit_range, &replacement, cx));
            let new = edit.source.read(cx).text().to_string();
            self.ensure_content_math(&new, cx);
            self.ensure_content_embeds(&new, cx);
            match &edit.target {
                SlashTarget::Day(key) => self.save_journal(key, &new, cx),
                SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
            }
            cx.notify();
            return Some((edit.source, new_range));
        }
        let align = to_editor_align(edit.editor.read(cx).align());
        // End the in-line edit (closes the gap, re-renders the formula) + get the range.
        let range = edit.source.update(cx, |e, cx| e.end_editing_block(cx))?;
        // Safety: only splice if the range still starts a `$$` block. A stale/shifted range
        // would otherwise insert the block at the wrong offset and corrupt the document — drop
        // the (rare) edit instead.
        if !edit.source.read(cx).is_math_block_range(&range) {
            cx.notify();
            return None;
        }
        // A one-line `$$…$$` source keeps its one-line form (issue #54);
        // fenced blocks keep fences. Multi-line LaTeX always needs fences.
        let was_one_line = !edit.source.read(cx).text()[range.clone()].contains('\n');
        let block = if was_one_line && !latex.contains('\n') {
            format!("$${latex}$$")
        } else {
            format!("$$\n{latex}\n$$")
        };
        // Fold the alignment marker into the same recorded edit: replace the block (and any
        // existing `<!-- math:ALIGN -->` line above it) with `<marker?>` + the new block.
        let (full_range, prefix) = edit.source.read(cx).math_marker_edit(range, align);
        let replacement = format!("{prefix}{block}");
        // The new block sits after the (possibly empty) marker prefix.
        let block_start = full_range.start + prefix.len();
        let new_range = block_start..block_start + block.len();
        // Recorded (undoable) splice — NOT `set_text`, which would wipe the document's undo
        // history. `replace_range` snaps to char boundaries, so a shifted/stale range can't
        // panic; read the result back rather than splicing the string ourselves.
        // The caret: replace_range parks it at the splice END — the closing `$$`
        // — which reveal-on-caret would show raw. On a blur-commit (clicking
        // elsewhere) the click already seated the caret where the user wants
        // it: preserve that, shifted by the splice delta, and if it still
        // lands inside the block, step to the line after it.
        let old_caret = edit.source.read(cx).cursor();
        edit.source.update(cx, |e, cx| {
            e.replace_range(full_range.clone(), &replacement, cx)
        });
        let new = edit.source.read(cx).text().to_string();
        let delta = replacement.len() as isize - (full_range.end - full_range.start) as isize;
        let caret = if old_caret >= full_range.end {
            (old_caret as isize + delta).max(0) as usize
        } else {
            old_caret
        };
        let caret = if caret >= new_range.start && caret <= new_range.end {
            new_range.end + 1
        } else {
            caret
        };
        edit.source
            .update(cx, |e, cx| e.set_cursor(caret.min(new.len()), cx));
        // Rasterize the edited formula into the shared store, or the block-math provider
        // can't find the (now-changed) LaTeX and the block shows raw `$$…$$`.
        self.ensure_content_math(&new, cx);
        self.ensure_content_embeds(&new, cx);
        match &edit.target {
            SlashTarget::Day(key) => self.save_journal(key, &new, cx),
            SlashTarget::Page(pid) => self.save_page_content(*pid, &new, cx),
        }
        cx.notify();
        Some((edit.source, new_range))
    }

    /// The caret arrowed past a formula's edge: commit the edit, then seat the text caret beside
    /// it (and re-focus the note editor) — the keyboard path out of the structural editor, as
    /// opposed to clicking away. An inline `$…$` seats the caret right beside the span on the
    /// same line; a `$$` block seats it on the adjacent line.
    fn exit_math_edit(&mut self, after: bool, window: &mut Window, cx: &mut Context<Self>) {
        let inline = self.math_edit.as_ref().is_some_and(|m| m.inline);
        if let Some((source, block)) = self.commit_math_edit(cx) {
            source.update(cx, |e, cx| {
                if inline {
                    e.focus(window, cx);
                    e.set_cursor(if after { block.end } else { block.start }, cx);
                } else {
                    e.exit_math(block, after, window, cx);
                }
            });
        }
    }

    /// Kick off the off-thread typeset of every `$$…$$` block in `content`, so an editor
    /// in WYSIWYG mode can render them as equations. Idempotent; a finished render
    /// notifies → repaint → the editor's math provider finds the bitmap.
    pub(super) fn ensure_content_math(&mut self, content: &str, cx: &mut Context<Self>) {
        self.ensure_content_block_labels(content, cx);
        for source in gpui_editor::math_sources(content) {
            self.ensure_math_loaded(source, cx);
        }
        // Inline `$…$` formulas typeset into the same store (keyed by LaTeX); the editor reuses
        // the raster scaled to text size.
        for source in gpui_editor::inline_math_sources(content) {
            self.ensure_math_loaded(source, cx);
        }
    }
}
