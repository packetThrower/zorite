//! `AppView` — the root view. Owns the database handle, the current
//! page, the in-memory outline (all blocks + the flattened visible
//! tree), one text-input entity per visible block, and the sidebar /
//! backlink panels. All outliner structural edits (new / indent /
//! outdent / delete / collapse) run through here: each mutates SQLite,
//! then rebuilds the in-memory tree and re-syncs the editors.

use std::collections::{HashMap, HashSet};

use gpui::{
    AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div, px,
};
use gpui_component::{
    TitleBar,
    input::{InputEvent, InputState},
};

use crate::actions::{FocusDown, FocusUp, Indent, Outdent};
use crate::db::Db;
use crate::models::{Backlink, Block, BlockNode, Page};
use crate::theme;
use crate::ui;

/// A block's text input plus the subscription wiring its events back to
/// `AppView`. Dropping it (when a block leaves the visible set) tears
/// down the subscription.
pub struct BlockEditor {
    pub state: Entity<InputState>,
    _sub: Subscription,
}

pub struct AppView {
    db: Db,
    /// The page currently open in the main pane.
    page: Page,
    /// Every block on the current page (source of truth for tree math).
    blocks: Vec<Block>,
    /// The flattened, currently-visible outline (collapsed subtrees
    /// omitted) — what the page pane renders, in order.
    pub nodes: Vec<BlockNode>,
    /// One editor per visible block, keyed by block id.
    pub editors: HashMap<i64, BlockEditor>,
    /// The block whose editor holds focus (drives edit-vs-rendered).
    pub focused_block: Option<i64>,

    /// Sidebar: recent journals and all named pages.
    pub journals: Vec<Page>,
    pub pages: Vec<Page>,
    /// "Linked References" for the current page.
    pub backlinks: Vec<Backlink>,
    /// Sidebar "find or create page" box.
    pub new_page_input: Entity<InputState>,

    /// Non-block subscriptions (the new-page box) kept alive here.
    _subs: Vec<Subscription>,
    /// Root focus handle so window-level action dispatch has a path.
    pub focus_handle: FocusHandle,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let db = Db::open()
            .or_else(|e| {
                log::error!("open database on disk failed ({e}); using in-memory store");
                Db::open_in_memory()
            })
            .expect("initialize database");

        let new_page_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find or create page…"));
        let np_sub = cx.subscribe_in(
            &new_page_input,
            window,
            |this: &mut AppView, state, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    let title = state.read(cx).value().trim().to_string();
                    if !title.is_empty() {
                        this.open_page_title(&title, window, cx);
                        state.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                }
            },
        );

        let mut this = Self {
            db,
            page: Page {
                id: 0,
                title: String::new(),
                is_journal: false,
                journal_date: None,
            },
            blocks: Vec::new(),
            nodes: Vec::new(),
            editors: HashMap::new(),
            focused_block: None,
            journals: Vec::new(),
            pages: Vec::new(),
            backlinks: Vec::new(),
            new_page_input,
            _subs: vec![np_sub],
            focus_handle: cx.focus_handle(),
        };
        this.open_today(window, cx);
        this
    }

    // --- Navigation ---

    pub fn open_today(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.get_or_create_journal(&today_iso()) {
            Ok(page) => self.load_page(page, window, cx),
            Err(e) => log::error!("open today's journal: {e}"),
        }
    }

    pub fn open_page_id(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.get_page(id) {
            Ok(Some(page)) => self.load_page(page, window, cx),
            Ok(None) => log::warn!("page {id} not found"),
            Err(e) => log::error!("open page {id}: {e}"),
        }
    }

    pub fn open_page_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.db.get_or_create_page(title) {
            Ok(page) => self.load_page(page, window, cx),
            Err(e) => log::error!("open page '{title}': {e}"),
        }
    }

    fn load_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.focused_block = None;
        // Every page shows at least one bullet to type into.
        if self.db.blocks_for_page(self.page.id).map(|b| b.is_empty()).unwrap_or(true) {
            let _ = self.db.create_block(self.page.id, None, 0, "");
        }
        // Fresh page → fresh editors (drops old subscriptions).
        self.editors.clear();
        self.rebuild(window, cx);
        if let Some(first) = self.nodes.first().map(|n| n.block.id) {
            self.focus_block(first, window, cx);
        }
    }

    // --- Tree (re)build ---

    /// Re-read the current page's blocks, recompute the visible tree,
    /// reconcile the editor map, refresh the side panels, and repaint.
    fn rebuild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.blocks = self.db.blocks_for_page(self.page.id).unwrap_or_default();
        self.nodes = flatten(&self.blocks);
        self.sync_editors(window, cx);
        self.refresh_side_panels();
        cx.notify();
    }

    /// Create editors for newly-visible blocks; drop editors for blocks
    /// that left the visible set (deleted or collapsed away).
    fn sync_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let visible: HashSet<i64> = self.nodes.iter().map(|n| n.block.id).collect();
        self.editors.retain(|id, _| visible.contains(id));

        let to_create: Vec<(i64, String)> = self
            .nodes
            .iter()
            .filter(|n| !self.editors.contains_key(&n.block.id))
            .map(|n| (n.block.id, n.block.content.clone()))
            .collect();
        for (id, content) in to_create {
            let editor = Self::make_editor(id, &content, window, cx);
            self.editors.insert(id, editor);
        }
    }

    fn make_editor(
        id: i64,
        content: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> BlockEditor {
        let state = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(content, window, cx);
            s
        });
        let sub = cx.subscribe_in(
            &state,
            window,
            move |this: &mut AppView, state, ev: &InputEvent, window, cx| match ev {
                InputEvent::Focus => {
                    this.focused_block = Some(id);
                    cx.notify();
                }
                InputEvent::Blur => {
                    if this.focused_block == Some(id) {
                        this.focused_block = None;
                    }
                    // A link may have changed; keep panels current.
                    this.refresh_side_panels();
                    cx.notify();
                }
                InputEvent::Change => {
                    let value = state.read(cx).value().to_string();
                    this.on_block_edited(id, &value);
                }
                InputEvent::PressEnter { .. } => {
                    this.new_block_after(id, window, cx);
                }
            },
        );
        BlockEditor { state, _sub: sub }
    }

    fn refresh_side_panels(&mut self) {
        self.journals = self.db.list_journals(30).unwrap_or_default();
        self.pages = self.db.list_pages().unwrap_or_default();
        self.backlinks = self.db.backlinks(self.page.id).unwrap_or_default();
    }

    // --- Editing ---

    /// Persist a block's text and re-index its `[[links]]`. Does not
    /// rebuild the tree — that would steal focus mid-keystroke.
    fn on_block_edited(&mut self, id: i64, value: &str) {
        if let Err(e) = self.db.update_block_content(id, value) {
            log::error!("save block {id}: {e}");
        }
        let titles = ui::links::parse_links(value);
        if let Err(e) = self.db.rebuild_links(id, &titles) {
            log::error!("rebuild links for block {id}: {e}");
        }
        if let Some(b) = self.blocks.iter_mut().find(|b| b.id == id) {
            b.content = value.to_string();
        }
    }

    // --- Structural ops ---

    fn new_block_after(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(block) = self.find_block(id) else { return };
        let (parent, pos) = (block.parent_id, block.position);
        if let Err(e) = self.db.shift_siblings_after(self.page.id, parent, pos) {
            log::error!("shift siblings: {e}");
            return;
        }
        let new_id = match self.db.create_block(self.page.id, parent, pos + 1, "") {
            Ok(id) => id,
            Err(e) => {
                log::error!("create block: {e}");
                return;
            }
        };
        self.rebuild(window, cx);
        self.focus_block(new_id, window, cx);
    }

    fn indent(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(block) = self.find_block(id) else { return };
        // Indent = become the last child of the previous sibling. The
        // first child of a group has no previous sibling, so it can't
        // indent.
        let Some(prev) = self.prev_sibling(&block) else { return };
        let new_pos = self
            .db
            .max_child_position(self.page.id, Some(prev.id))
            .ok()
            .flatten()
            .map_or(0, |m| m + 1);
        if prev.collapsed {
            let _ = self.db.set_collapsed(prev.id, false);
        }
        if let Err(e) = self.db.move_block(id, Some(prev.id), new_pos) {
            log::error!("indent block {id}: {e}");
            return;
        }
        self.rebuild(window, cx);
        self.focus_block(id, window, cx);
    }

    fn outdent(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(block) = self.find_block(id) else { return };
        // Outdent = become the next sibling of the parent. A top-level
        // block has no parent, so it can't outdent.
        let Some(parent_id) = block.parent_id else { return };
        let Some(parent) = self.find_block(parent_id) else { return };
        let grandparent = parent.parent_id;
        if let Err(e) = self.db.shift_siblings_after(self.page.id, grandparent, parent.position) {
            log::error!("shift siblings: {e}");
            return;
        }
        if let Err(e) = self.db.move_block(id, grandparent, parent.position + 1) {
            log::error!("outdent block {id}: {e}");
            return;
        }
        self.rebuild(window, cx);
        self.focus_block(id, window, cx);
    }

    pub fn delete_block(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        // Remember the row above, to land focus there afterward.
        let prev_id = self
            .nodes
            .iter()
            .position(|n| n.block.id == id)
            .filter(|&i| i > 0)
            .map(|i| self.nodes[i - 1].block.id);

        if let Err(e) = self.db.delete_block(id) {
            log::error!("delete block {id}: {e}");
            return;
        }
        self.rebuild(window, cx);

        if self.nodes.is_empty() {
            // A page is never left empty.
            if let Ok(new_id) = self.db.create_block(self.page.id, None, 0, "") {
                self.rebuild(window, cx);
                self.focus_block(new_id, window, cx);
            }
        } else if let Some(pid) = prev_id.filter(|pid| self.editors.contains_key(pid)) {
            self.focus_block(pid, window, cx);
        } else if let Some(first) = self.nodes.first().map(|n| n.block.id) {
            self.focus_block(first, window, cx);
        }
    }

    pub fn toggle_collapsed(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(block) = self.find_block(id) else { return };
        if let Err(e) = self.db.set_collapsed(id, !block.collapsed) {
            log::error!("toggle collapse {id}: {e}");
            return;
        }
        self.rebuild(window, cx);
    }

    // --- Focus ---

    pub fn focus_block(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.editors.get(&id) {
            editor.state.update(cx, |s, cx| s.focus(window, cx));
            self.focused_block = Some(id);
        }
    }

    fn focus_relative(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cur) = self.focused_block else { return };
        let Some(i) = self.nodes.iter().position(|n| n.block.id == cur) else { return };
        let j = i as isize + delta;
        if j < 0 || j as usize >= self.nodes.len() {
            return;
        }
        let target = self.nodes[j as usize].block.id;
        self.focus_block(target, window, cx);
    }

    // --- Tree helpers (operate on `self.blocks`) ---

    fn find_block(&self, id: i64) -> Option<Block> {
        self.blocks.iter().find(|b| b.id == id).cloned()
    }

    fn prev_sibling(&self, block: &Block) -> Option<Block> {
        self.blocks
            .iter()
            .filter(|b| b.parent_id == block.parent_id && b.position < block.position)
            .max_by_key(|b| b.position)
            .cloned()
    }

    pub fn page_title(&self) -> &str {
        &self.page.title
    }

    pub fn current_page_id(&self) -> i64 {
        self.page.id
    }

    /// Whether the page on screen is today's journal — drives the
    /// sidebar "Today" highlight.
    pub fn is_viewing_today(&self) -> bool {
        self.page.is_journal && self.page.journal_date.as_deref() == Some(today_iso().as_str())
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::bg_window())
            .text_color(theme::text_primary())
            // Outliner actions. Bound (in actions.rs) to keys in the
            // "Input" context, so they dispatch up from the focused
            // block editor to these handlers on the root.
            .on_action(cx.listener(|this: &mut AppView, _: &Indent, window, cx| {
                if let Some(id) = this.focused_block {
                    this.indent(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &Outdent, window, cx| {
                if let Some(id) = this.focused_block {
                    this.outdent(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &FocusUp, window, cx| {
                this.focus_relative(-1, window, cx);
            }))
            .on_action(cx.listener(|this: &mut AppView, _: &FocusDown, window, cx| {
                this.focus_relative(1, window, cx);
            }))
            .child(
                TitleBar::new().child(
                    div()
                        .px_2()
                        .text_size(px(13.0))
                        .text_color(theme::text_secondary())
                        .child("rumin"),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(ui::sidebar::render(self, cx))
                    .child(ui::page_view::render(self, window, cx)),
            )
    }
}

/// Today's local date as `YYYY-MM-DD` (the journal title). Falls back to
/// UTC if the local offset can't be determined.
fn today_iso() -> String {
    let now = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!("{:04}-{:02}-{:02}", now.year(), u8::from(now.month()), now.day())
}

/// Walk the parent/position tree into a flat, render-ordered list,
/// skipping the descendants of collapsed blocks.
fn flatten(blocks: &[Block]) -> Vec<BlockNode> {
    let mut children: HashMap<Option<i64>, Vec<&Block>> = HashMap::new();
    for b in blocks {
        children.entry(b.parent_id).or_default().push(b);
    }
    for kids in children.values_mut() {
        kids.sort_by_key(|b| b.position);
    }

    fn walk(
        parent: Option<i64>,
        depth: usize,
        children: &HashMap<Option<i64>, Vec<&Block>>,
        out: &mut Vec<BlockNode>,
    ) {
        let Some(kids) = children.get(&parent) else { return };
        for b in kids {
            let has_children = children.get(&Some(b.id)).is_some_and(|c| !c.is_empty());
            out.push(BlockNode {
                block: (*b).clone(),
                depth,
                has_children,
            });
            if !b.collapsed {
                walk(Some(b.id), depth + 1, children, out);
            }
        }
    }

    let mut out = Vec::new();
    walk(None, 0, &children, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::flatten;
    use crate::models::Block;

    fn blk(id: i64, parent: Option<i64>, pos: i64, collapsed: bool) -> Block {
        Block {
            id,
            page_id: 1,
            parent_id: parent,
            position: pos,
            content: String::new(),
            collapsed,
        }
    }

    #[test]
    fn orders_by_position_and_assigns_depth() {
        // 1            depth 0
        //   2          depth 1
        //   3          depth 1
        // 4            depth 0
        let blocks = vec![
            blk(4, None, 1, false),
            blk(1, None, 0, false),
            blk(3, Some(1), 1, false),
            blk(2, Some(1), 0, false),
        ];
        let nodes = flatten(&blocks);
        assert_eq!(nodes.iter().map(|n| n.block.id).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert_eq!(nodes.iter().map(|n| n.depth).collect::<Vec<_>>(), vec![0, 1, 1, 0]);
        assert!(nodes[0].has_children);
        assert!(!nodes[3].has_children);
    }

    #[test]
    fn collapsed_block_hides_descendants_but_keeps_caret() {
        let blocks = vec![blk(1, None, 0, true), blk(2, Some(1), 0, false)];
        let nodes = flatten(&blocks);
        assert_eq!(nodes.iter().map(|n| n.block.id).collect::<Vec<_>>(), vec![1]);
        assert!(nodes[0].has_children);
    }
}
