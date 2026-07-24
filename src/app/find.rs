//! Find in page and find in the journal feed: the ⌘F bars, match
//! stepping, highlights, and scroll-to-match — split from `app.rs`.

use super::*;

/// Find-in-the-journal-feed state (⌘F on the Journal tab): a floating bar
/// like the PDF viewer's, matching across every loaded day.
pub struct FeedFind {
    pub input: Entity<InputState>,
    pub query: String,
    /// Flat matches in feed order: `(day date, source byte range)`.
    matches: Vec<(String, std::ops::Range<usize>)>,
    pub current: usize,
    /// Recomputes on typing; steps on Enter / ⇧Enter. Kept alive here.
    _sub: gpui::Subscription,
}

impl FeedFind {
    pub fn count(&self) -> usize {
        self.matches.len()
    }

    /// The active match's index *within* `date`'s matches (what
    /// `MarkdownView::search` wants), or a past-the-end index when the
    /// current match lives in another day (soft highlights only).
    pub fn current_in_day(&self, date: &str) -> usize {
        let mut in_day = 0;
        for (i, (d, _)) in self.matches.iter().enumerate() {
            if d == date {
                if i == self.current {
                    return in_day;
                }
                in_day += 1;
            }
        }
        usize::MAX
    }
}

/// In-page find state. The query field's Change events recompute `count` against
/// the active page; `current` + `count` size the bar's "n of m" and pick which
/// match [`gpui_markdown::MarkdownView::search`] emphasizes.
pub struct PageFind {
    pub input: Entity<InputState>,
    pub query: String,
    pub current: usize,
    pub count: usize,
    /// Block index (per `gpui_markdown::find_matches`) of each match, used to scroll
    /// the active match's block into view (reader mode).
    match_blocks: Vec<usize>,
    /// Source byte range of each match (per `gpui_editor::find_in_source`),
    /// driving the editor's highlights + scroll in WYSIWYG/editing mode.
    ranges: Vec<std::ops::Range<usize>>,
    _sub: Subscription,
}

impl AppView {
    // --- In-page find (⌘F) ---

    /// Open (or refocus) the in-page find bar for the active named page. No-op
    /// unless a Page tab is showing — PDFs have their own find, and the journal
    /// feed uses the global search (⌘⇧F).
    pub fn open_page_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(
            self.tabs.get(self.active).map(|t| &t.kind),
            Some(TabKind::Page(_))
        ) {
            return;
        }
        if let Some(pf) = self.page_find.as_ref() {
            pf.input.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in page…"));
        let sub = cx.subscribe_in(
            &input,
            window,
            |this: &mut AppView, _st, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => this.recompute_page_find(cx),
                // Enter steps to the next match, Shift+Enter to the previous.
                InputEvent::PressEnter { shift, .. } => {
                    this.page_find_step(if *shift { -1 } else { 1 }, cx)
                }
                _ => {}
            },
        );
        input.update(cx, |s, cx| s.focus(window, cx));
        self.page_find = Some(PageFind {
            input,
            query: String::new(),
            current: 0,
            count: 0,
            match_blocks: Vec::new(),
            ranges: Vec::new(),
            _sub: sub,
        });
        cx.notify();
    }

    /// Recompute the match count against the active page after the query changed,
    /// resetting to the first match.
    fn recompute_page_find(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.page_find.as_ref().map(|pf| pf.input.clone()) else {
            return;
        };
        let query = input.read(cx).value().to_string();
        let content = self
            .page_editor
            .as_ref()
            .map(|pe| pe.state.read(cx).value().to_string())
            .unwrap_or_default();
        let blocks = gpui_markdown::find_matches(&content, &query);
        let ranges = if query.trim().is_empty() {
            Vec::new()
        } else {
            gpui_editor::find_in_source(&content, &query)
        };
        // The count follows the surface doing the finding: source matches in
        // the editor (WYSIWYG/editing), rendered blocks in the reader.
        let editing = self.wysiwyg || self.is_page_editing();
        if let Some(pf) = self.page_find.as_mut() {
            pf.query = query;
            pf.count = if editing { ranges.len() } else { blocks.len() };
            pf.current = 0;
            pf.match_blocks = blocks;
            pf.ranges = ranges;
        }
        self.apply_page_find_highlights(cx);
        self.scroll_to_current_match(cx);
        cx.notify();
    }

    /// Push the find matches into the page editor's highlights (WYSIWYG /
    /// editing mode) — or clear them (reader mode / bar closed).
    fn apply_page_find_highlights(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.page_editor.as_ref().map(|pe| pe.state.clone()) else {
            return;
        };
        let (ranges, active) = match self.page_find.as_ref() {
            Some(pf) if self.wysiwyg || self.is_page_editing() => {
                (pf.ranges.clone(), Some(pf.current))
            }
            _ => (Vec::new(), None),
        };
        state.update(cx, |editor, cx| editor.set_search(ranges, active, cx));
    }

    /// Step the active find match (`delta`: +1 next, -1 prev), wrapping.
    pub fn page_find_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(pf) = self.page_find.as_mut()
            && pf.count > 0
        {
            let n = pf.count as isize;
            pf.current = (pf.current as isize + delta).rem_euclid(n) as usize;
        }
        self.apply_page_find_highlights(cx);
        self.scroll_to_current_match(cx);
        cx.notify();
    }

    /// Scroll the page so the active find match's block is comfortably visible (a
    /// little below the viewport top). No-op if the block isn't laid out yet or is
    /// already in view, so starting a find on text you're reading doesn't yank it.
    fn scroll_to_current_match(&self, cx: &Context<Self>) {
        let Some(pf) = self.page_find.as_ref() else {
            return;
        };
        // WYSIWYG/editing: the reader's blocks aren't painted — scroll to the
        // match's row via the editor's own geometry. Reader mode: the block.
        let (block_top, block_bottom) = if self.wysiwyg || self.is_page_editing() {
            let Some(top) = pf.ranges.get(pf.current).and_then(|r| {
                self.page_editor
                    .as_ref()?
                    .state
                    .read(cx)
                    .offset_screen_top(r.start)
            }) else {
                return;
            };
            (top, top + px(24.0))
        } else {
            let Some(&block) = pf.match_blocks.get(pf.current) else {
                return;
            };
            let Some(b) = self.md_block_scroll.bounds_for_item(block) else {
                return;
            };
            (b.origin.y, b.origin.y + b.size.height)
        };
        let viewport = self.page_scroll.bounds();
        if viewport.size.height <= px(0.0) {
            return;
        }
        let margin = px(48.0);
        let (v_top, v_bottom) = (viewport.origin.y, viewport.origin.y + viewport.size.height);
        // Already comfortably visible — leave the view put.
        if block_top >= v_top + margin && block_bottom <= v_bottom - margin {
            return;
        }
        // Bring the block to `margin` below the viewport top. Clamp only at the top
        // (offset 0); the target is always a real, laid-out block, so it can't
        // over-scroll past the content. (Not clamping at `max_offset`, which this
        // plain scroll element doesn't populate — clamping there pinned the offset
        // to 0 and blocked all downward scrolling.)
        let new_y = (self.page_scroll.offset().y - (block_top - (v_top + margin))).min(px(0.0));
        self.page_scroll.set_offset(gpui::point(px(0.0), new_y));
    }

    /// Close the in-page find bar.
    pub fn close_page_find(&mut self, cx: &mut Context<Self>) {
        if self.page_find.take().is_some() {
            self.apply_page_find_highlights(cx);
            cx.notify();
        }
    }

    /// Open (or refocus) the journal feed's find bar (⌘F on the Journal tab).
    pub fn open_feed_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ff) = self.feed_find.as_ref() {
            ff.input.update(cx, |s, cx| s.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in journal…"));
        let sub = cx.subscribe_in(
            &input,
            window,
            |this: &mut AppView, _st, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => this.recompute_feed_find(cx),
                InputEvent::PressEnter { shift, .. } => {
                    this.feed_find_step(if *shift { -1 } else { 1 }, cx)
                }
                _ => {}
            },
        );
        input.update(cx, |s, cx| s.focus(window, cx));
        self.feed_find = Some(FeedFind {
            input,
            query: String::new(),
            matches: Vec::new(),
            current: 0,
            _sub: sub,
        });
        cx.notify();
    }

    /// Re-scan every loaded day for the query (feed order), reset to the
    /// first match, refresh the editors' highlights, and scroll to it.
    fn recompute_feed_find(&mut self, cx: &mut Context<Self>) {
        let Some(input) = self.feed_find.as_ref().map(|ff| ff.input.clone()) else {
            return;
        };
        let query = input.read(cx).value().to_string();
        let mut matches = Vec::new();
        if !query.trim().is_empty() {
            for i in 0..self.loaded_days {
                let date = date_for_offset(i);
                if let Some(day) = self.day_editors.get(&date) {
                    let content = day.state.read(cx).value().to_string();
                    for r in gpui_editor::find_in_source(&content, &query) {
                        matches.push((date.clone(), r));
                    }
                }
            }
        }
        if let Some(ff) = self.feed_find.as_mut() {
            ff.query = query;
            ff.matches = matches;
            ff.current = 0;
        }
        self.apply_feed_find_highlights(cx);
        self.scroll_to_current_feed_match(cx);
        cx.notify();
    }

    /// Step the active feed match (`delta`: +1 next, -1 prev), wrapping.
    pub fn feed_find_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(ff) = self.feed_find.as_mut()
            && !ff.matches.is_empty()
        {
            let n = ff.matches.len() as isize;
            ff.current = (ff.current as isize + delta).rem_euclid(n) as usize;
        }
        self.apply_feed_find_highlights(cx);
        self.scroll_to_current_feed_match(cx);
        cx.notify();
    }

    /// Push each loaded day's match ranges (and the active index, for the day
    /// holding the current match) into its editor's search highlights.
    fn apply_feed_find_highlights(&mut self, cx: &mut Context<Self>) {
        let (matches, current) = match self.feed_find.as_ref() {
            Some(ff) => (ff.matches.clone(), ff.current),
            None => (Vec::new(), 0),
        };
        let states: Vec<(String, Entity<EditorState>)> = self
            .day_editors
            .iter()
            .map(|(d, de)| (d.clone(), de.state.clone()))
            .collect();
        for (date, state) in states {
            let mut ranges = Vec::new();
            let mut active = None;
            for (i, (d, r)) in matches.iter().enumerate() {
                if *d == date {
                    if i == current {
                        active = Some(ranges.len());
                    }
                    ranges.push(r.clone());
                }
            }
            state.update(cx, |editor, cx| editor.set_search(ranges, active, cx));
        }
    }

    /// Scroll the feed so the current match's row sits comfortably below the
    /// viewport top (mirrors `scroll_to_current_match`'s clamp-at-top).
    fn scroll_to_current_feed_match(&self, cx: &mut Context<Self>) {
        let Some(ff) = self.feed_find.as_ref() else {
            return;
        };
        let Some((date, range)) = ff.matches.get(ff.current) else {
            return;
        };
        let Some(day) = self.day_editors.get(date) else {
            return;
        };
        // WYSIWYG (or this day being edited): the editor's row geometry.
        // Reader mode: the editor isn't painted — locate the match's rendered
        // block via the day view's tracked blocks (best-effort index; block
        // counting can differ slightly from the source scan).
        let row_top = if self.wysiwyg || self.is_editing_day(date) {
            day.state.read(cx).offset_screen_top(range.start)
        } else {
            let content = day.state.read(cx).value().to_string();
            let blocks = gpui_markdown::find_matches(&content, &ff.query);
            let in_day = ff.current_in_day(date).min(blocks.len().saturating_sub(1));
            blocks
                .get(in_day)
                .and_then(|b| day.md_scroll.bounds_for_item(*b))
                .map(|b| b.origin.y)
        };
        let Some(row_top) = row_top else {
            return;
        };
        let viewport = self.feed_scroll.bounds();
        if viewport.size.height <= px(0.0) {
            return;
        }
        let margin = px(64.0);
        let (v_top, v_bottom) = (viewport.origin.y, viewport.origin.y + viewport.size.height);
        if row_top >= v_top + margin && row_top <= v_bottom - margin {
            return;
        }
        let new_y = (self.feed_scroll.offset().y - (row_top - (v_top + margin))).min(px(0.0));
        self.feed_scroll.set_offset(gpui::point(px(0.0), new_y));
    }

    /// Close the feed find bar and clear every day's highlights.
    pub fn close_feed_find(&mut self, cx: &mut Context<Self>) {
        if self.feed_find.take().is_some() {
            self.apply_feed_find_highlights(cx);
            cx.notify();
        }
    }

    /// Focus the sidebar's global search field (expanding the rail if collapsed).
    /// Drives ⌘⇧F — the journal feed's "find", and a quick jump from anywhere.
    pub fn focus_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = false;
        self.search_input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }
}
