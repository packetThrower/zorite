//! Block references: the `((id))` frontend ↔ `[[Page#^id]]` DB translation,
//! labels, counts, and backlink navigation — split from `app.rs`.

use super::*;

/// `(page, ^id)` → the target block's text — the shape both engines'
/// styles carry for block-link labels.
type BlockLabelFn = std::rc::Rc<dyn Fn(&str, &str) -> Option<String>>;

impl AppView {
    /// The anchor id of `line` in page `page_id`, creating one when the line
    /// has none: a ` ^id` (8 hex chars, hashed from the page + line) appends
    /// to the line — through the OPEN editor when there is one (undo +
    /// autosave), else straight to the DB. `None` when the line can't carry
    /// an anchor (shifted content, fences, table rows).
    pub(super) fn ensure_block_anchor(
        &mut self,
        page_id: i64,
        line_idx: usize,
        cx: &mut Context<Self>,
    ) -> Option<(String, usize)> {
        let p = self.db.get_page(page_id).ok()??;
        let lines: Vec<&str> = p.content.split('\n').collect();
        let line = *lines.get(line_idx)?;
        if let Some((_, id)) = gpui_markdown::syntax::block_id(line) {
            return Some((id.to_string(), usize::MAX));
        }
        let t = line.trim_start();
        if t.is_empty() || t.starts_with("```") || t.starts_with('|') {
            return None;
        }
        let id = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            page_id.hash(&mut h);
            line_idx.hash(&mut h);
            line.hash(&mut h);
            format!("{:08x}", h.finish() as u32)
        };
        // Byte range of the line's trimmed end, so the anchor lands before
        // any trailing whitespace is dropped.
        let start: usize = lines[..line_idx].iter().map(|l| l.len() + 1).sum();
        let end = start + line.trim_end().len();
        let anchor = format!(" ^{id}");
        // Route through the open editor (keeps its undo history; its Changed
        // event autosaves) — the day's editor for journals, the open page's
        // otherwise. Fall back to a direct DB save.
        let open_editor = if p.is_journal {
            p.journal_date
                .as_deref()
                .and_then(|d| self.day_editors.get(d))
                .map(|de| de.state.clone())
        } else {
            self.page_editor
                .as_ref()
                .filter(|pe| pe.id == page_id)
                .map(|pe| pe.state.clone())
        };
        match open_editor {
            Some(state) => {
                state.update(cx, |e, cx| {
                    // The anchor edit must not move the USER'S caret — this
                    // may be the very document the palette is open in.
                    let old = e.cursor();
                    e.replace_range(end..end, &anchor, cx);
                    let restored = if old >= end { old + anchor.len() } else { old };
                    e.set_cursor(restored.min(e.value().len()), cx);
                    cx.emit(gpui_editor::EditorEvent::Changed);
                });
            }
            None => {
                let mut new = p.content.clone();
                new.replace_range(end..end, &anchor);
                match (p.is_journal, p.journal_date.as_deref()) {
                    (true, Some(d)) => {
                        let d = d.to_string();
                        self.save_journal(&d, &new, cx);
                    }
                    _ => self.save_page_content(page_id, &new, cx),
                }
            }
        }
        Some((id, end))
    }

    /// DB → frontend: unaliased `[[Page#^id]]` block links become `((id))`
    /// (recording id → page in the index for the reverse trip). Idempotent.
    pub(super) fn to_frontend_refs(&self, content: &str) -> String {
        if !content.contains("#^") {
            return content.to_string();
        }
        let mut out = String::with_capacity(content.len());
        let mut rest = content;
        while let Some(open) = rest.find("[[") {
            let Some(close) = rest[open..].find("]]") else {
                break;
            };
            let inner = &rest[open + 2..open + close];
            out.push_str(&rest[..open]);
            match inner.split_once("#^") {
                Some((page, id))
                    if !page.is_empty()
                        && !id.is_empty()
                        && !id.contains('|')
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
                {
                    self.block_ref_index
                        .borrow_mut()
                        .insert(id.to_string(), page.to_string());
                    out.push_str(&format!("(({id}))"));
                }
                _ => out.push_str(&rest[open..open + close + 2]),
            }
            rest = &rest[open + close + 2..];
        }
        out.push_str(rest);
        out
    }

    /// Frontend → DB: `((id))` expands to `[[Page#^id]]` via the index; ids
    /// the index doesn't know stay literal. Idempotent.
    pub(super) fn to_db_refs(&self, content: &str) -> String {
        if !content.contains("((") {
            return content.to_string();
        }
        let mut out = String::with_capacity(content.len());
        let mut rest = content;
        while let Some(open) = rest.find("((") {
            let Some(close) = rest[open..].find("))") else {
                break;
            };
            let id = &rest[open + 2..open + close];
            out.push_str(&rest[..open]);
            match self.block_ref_index.borrow().get(id) {
                Some(page)
                    if !id.is_empty()
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
                {
                    out.push_str(&format!("[[{page}#^{id}]]"));
                }
                _ => out.push_str(&rest[open..open + close + 2]),
            }
            rest = &rest[open + close + 2..];
        }
        out.push_str(rest);
        out
    }

    /// The block-label resolver both engines' styles carry: a pure store
    /// lookup (never the DB — this runs during render).
    pub(crate) fn block_label_resolver(&self) -> BlockLabelFn {
        let store = self.block_label_store.clone();
        std::rc::Rc::new(move |page, id| {
            store.borrow().get(id).and_then(|(p, label)| {
                (page.is_empty() || *p == page.to_lowercase()).then(|| label.clone())
            })
        })
    }

    /// The block reference-count resolver both engines' styles carry: a pure
    /// store lookup (never the DB — this runs during render); 0 = no badge.
    pub(crate) fn block_ref_count_resolver(&self) -> Rc<dyn Fn(&str) -> usize> {
        let counts = self.block_ref_counts.clone();
        Rc::new(move |id| counts.borrow().get(id).copied().unwrap_or(0))
    }

    /// The editor syntax style with the block-label resolver + generation
    /// installed — every `set_markdown_style` call routes through here.
    pub(super) fn editor_style(&self) -> gpui_editor::SyntaxStyle {
        let mut st = theme::editor_syntax_style();
        st.block_label = Some(self.block_label_resolver());
        st.block_label_gen = self.block_label_gen;
        st.block_ref_count = Some(self.block_ref_count_resolver());
        st
    }

    /// Resolve every `[[Page#^id]]` in `content` into the label store (the
    /// target block's text, prefix-stripped + truncated). A change bumps the
    /// generation and re-pushes editor styles so cached lines re-key.
    pub(super) fn ensure_content_block_labels(&mut self, content: &str, cx: &mut Context<Self>) {
        let mut changed = false;
        let rest = content;
        // Both ref forms: `[[Page#^id]]` (DB/reader form — also warms the
        // id→page index) and `((id))` (frontend form — page via the index).
        let mut refs: Vec<(String, String)> = Vec::new();
        {
            let mut r = rest;
            while let Some(open) = r.find("[[") {
                r = &r[open + 2..];
                let Some(close) = r.find("]]") else { break };
                let inner = &r[..close];
                r = &r[close + 2..];
                if let Some((page, id)) = inner.split_once("#^")
                    && !page.is_empty()
                    && !id.is_empty()
                    && !id.contains('|')
                {
                    self.block_ref_index
                        .borrow_mut()
                        .insert(id.to_string(), page.to_string());
                    refs.push((page.to_string(), id.to_string()));
                }
            }
            let mut r = rest;
            while let Some(open) = r.find("((") {
                r = &r[open + 2..];
                let Some(close) = r.find("))") else { break };
                let id = &r[..close];
                r = &r[close + 2..];
                if id.is_empty()
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    continue;
                }
                // Index first; else find the page CARRYING the anchor (ids
                // saved as literal `((id))` before the index warmed) and
                // record it — self-healing for pre-index content. Two
                // statements: the read borrow must END before the fallback
                // takes the write borrow.
                let hit = self.block_ref_index.borrow().get(id).cloned();
                let page = hit.or_else(|| {
                    let hit = self
                        .db
                        .page_referencing(&format!(" ^{id}"))
                        .ok()
                        .flatten()?;
                    let p = self.db.get_page(hit).ok().flatten()?;
                    let title = p.journal_date.clone().unwrap_or(p.title);
                    self.block_ref_index
                        .borrow_mut()
                        .insert(id.to_string(), title.clone());
                    Some(title)
                });
                if let Some(page) = page {
                    refs.push((page, id.to_string()));
                }
            }
        }
        for (page, id) in refs {
            let Ok(Some(p)) = self.db.get_page_by_title(&page) else {
                continue;
            };
            let lines: Vec<&str> = p.content.split('\n').collect();
            let label = lines.iter().enumerate().find_map(|(li, line)| {
                let (cut, lid) = gpui_markdown::syntax::block_id(line)?;
                (lid == id).then(|| {
                    let mut t = line[..cut].trim();
                    // Strip list/task/heading dressing for a clean label.
                    for p in ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ "] {
                        if let Some(r) = t.strip_prefix(p) {
                            t = r.trim_start();
                            break;
                        }
                    }
                    t = t.trim_start_matches('#').trim_start();
                    let indent = |l: &str| l.len() - l.trim_start().len();
                    let base = indent(line);
                    let has_children = lines
                        .get(li + 1)
                        .is_some_and(|n| !n.trim().is_empty() && indent(n) > base);
                    let mut label: String = t.chars().take(60).collect();
                    if t.chars().count() > 60 || has_children {
                        label.push('…');
                    }
                    label
                })
            });
            if let Some(label) = label {
                let entry = (page.to_lowercase(), label);
                if self.block_label_store.borrow().get(&id) != Some(&entry) {
                    self.block_label_store.borrow_mut().insert(id, entry);
                    changed = true;
                }
            }
        }
        // The other side: `^id` anchors this content CARRIES — referenced
        // ones get a count badge. Only cold ids query the DB; the debounced
        // doc-changed refresh keeps the known set current.
        for line in content.split('\n') {
            if let Some((_, id)) = gpui_markdown::syntax::block_id(line)
                && !self.block_ref_counts.borrow().contains_key(id)
            {
                let n = self.db.count_block_refs(id).unwrap_or(0);
                self.block_ref_counts.borrow_mut().insert(id.to_string(), n);
                changed |= n > 0;
            }
        }
        if changed {
            self.bump_block_meta(cx);
        }
    }

    /// Bump the block-metadata generation (labels / ref counts changed) and
    /// re-push editor styles so cached lines re-key.
    fn bump_block_meta(&mut self, cx: &mut Context<Self>) {
        self.block_label_gen += 1;
        let states: Vec<Entity<EditorState>> = self
            .day_editors
            .values()
            .map(|de| de.state.clone())
            .chain(self.page_editor.as_ref().map(|pe| pe.state.clone()))
            .collect();
        let style = self.editor_style();
        for state in states {
            state.update(cx, |editor, cx| {
                editor.set_markdown_style(style.clone(), cx)
            });
        }
    }

    /// Re-query every known block-ref count (a save may have added or removed
    /// references anywhere); runs on the debounced doc-changed refresh.
    pub(super) fn refresh_block_ref_counts(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.block_ref_counts.borrow().keys().cloned().collect();
        let mut changed = false;
        for id in ids {
            let n = self.db.count_block_refs(&id).unwrap_or(0);
            changed |= self.block_ref_counts.borrow_mut().insert(id, n) != Some(n);
        }
        if changed {
            self.bump_block_meta(cx);
        }
    }

    /// Resolve the block labels referenced inside backlink snippets, so the
    /// linked-reference cards render `[[Page#^id]]` refs as the block's text.
    pub(super) fn warm_backlink_labels(&mut self, links: &[Backlink], cx: &mut Context<Self>) {
        let snippets: Vec<String> = links
            .iter()
            .filter(|b| b.snippet.contains("#^") || b.snippet.contains("(("))
            .map(|b| b.snippet.clone())
            .collect();
        for s in snippets {
            self.ensure_content_block_labels(&s, cx);
        }
    }

    /// Open a linked-reference card's source page and seat the caret at the
    /// referencing line (Logseq-style jump-to-block). `line` is the 0-based
    /// line index from [`Backlink`]; the byte offset is recomputed against the
    /// editor's (frontend-form) content once the page is up.
    pub fn open_backlink(
        &mut self,
        page_id: i64,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.db.is_whiteboard(page_id) {
            self.open_whiteboard(page_id, window, cx);
            return;
        }
        match self.db.get_page(page_id) {
            Ok(Some(page)) => {
                self.open_page_foreground(page, window, cx);
                let weak = cx.entity().downgrade();
                window.defer(cx, move |window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let Some(pe) = this.page_editor.as_ref().filter(|pe| pe.id == page_id)
                        else {
                            return;
                        };
                        let source = pe.state.read(cx).value().to_string();
                        let offset = source
                            .split_inclusive('\n')
                            .take(line)
                            .map(str::len)
                            .sum::<usize>();
                        this.edit_page_at_offset(offset, px(160.0), window, cx);
                    });
                });
            }
            Ok(None) => log::warn!("page {page_id} not found"),
            Err(e) => log::error!("open page {page_id}: {e}"),
        }
    }

    /// The count badge on a referenced block was clicked: a dialog listing
    /// every page referencing the block, each row jumping to its reference.
    pub(super) fn show_block_refs_dialog(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let refs = self.db.block_referencers(&id).unwrap_or_default();
        if refs.is_empty() {
            return;
        }
        self.warm_backlink_labels(&refs, cx);
        let mut style = theme::markdown_style(self.list_indent(), px(13.0));
        style.block_label = Some(self.block_label_resolver());
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, _cx| {
            let mut list = div().flex().flex_col().gap_2();
            for (i, bl) in refs.iter().enumerate() {
                let weak_row = weak.clone();
                let page_id = bl.source_page_id;
                let line = bl.line;
                list = list.child(
                    div()
                        .id(("block-ref-row", i))
                        .px_3()
                        .py_2()
                        .rounded(px(6.0))
                        .bg(theme::glass())
                        .cursor_pointer()
                        .hover(|h| h.bg(theme::glass_strong()))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme::accent())
                                .child(bl.source_page_title.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(theme::text_secondary())
                                .child(
                                    gpui_markdown::MarkdownView::new(
                                        format!("block-ref-md-{i}"),
                                        bl.snippet.clone(),
                                    )
                                    .style(style.clone()),
                                ),
                        )
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let _ = weak_row.update(cx, |this, cx| {
                                this.open_backlink(page_id, line, window, cx)
                            });
                        }),
                );
            }
            dialog
                .title(format!("Linked references ({})", refs.len()))
                .w(px(480.0))
                // The default dialog seat (10% down) reads awkwardly for a
                // click-through list — drop it toward the window's middle.
                .margin_top(window.viewport_size().height * 0.3)
                .child(list)
        });
    }
}
