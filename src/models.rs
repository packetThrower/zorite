//! Plain data types shared between the SQLite layer (`db`) and the UI.

/// A page. Either a daily journal (titled by its ISO date) or a named
/// page created by the user or auto-created from a `[[wiki-link]]`.
#[derive(Clone, Debug)]
pub struct Page {
    pub id: i64,
    pub title: String,
    pub is_journal: bool,
    /// ISO `YYYY-MM-DD` for journal pages; `None` for named pages.
    pub journal_date: Option<String>,
}

/// One outliner block — a single bullet. `parent_id == None` means a
/// top-level block on its page; `position` orders blocks among their
/// siblings.
#[derive(Clone, Debug)]
pub struct Block {
    pub id: i64,
    /// Owning page. Carried for completeness and future cross-page
    /// features (e.g. search); the current single-page UI scopes work by
    /// `AppView`'s open page rather than reading this back.
    #[allow(dead_code)]
    pub page_id: i64,
    pub parent_id: Option<i64>,
    pub position: i64,
    pub content: String,
    pub collapsed: bool,
}

/// A block flattened into the visible outline for rendering: the block
/// itself plus the depth at which it sits and whether it has children
/// (so a row can draw the right indent and a collapse caret). Produced
/// by walking the parent/position tree, skipping the descendants of
/// collapsed blocks.
#[derive(Clone, Debug)]
pub struct BlockNode {
    pub block: Block,
    pub depth: usize,
    pub has_children: bool,
}

/// One row of a page's "Linked References" panel: a block on some other
/// page whose text links to the page being viewed.
#[derive(Clone, Debug)]
pub struct Backlink {
    pub source_page_id: i64,
    pub source_page_title: String,
    pub block_content: String,
}
