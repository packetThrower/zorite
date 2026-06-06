//! Plain data types shared between the SQLite layer (`db`) and the UI.

/// A page. Either a daily journal (titled by its ISO date) or a named
/// page created by the user or auto-created from a `[[wiki-link]]`. Its
/// body is a single markdown document.
#[derive(Clone, Debug)]
pub struct Page {
    pub id: i64,
    pub title: String,
    /// Whether this is a daily journal page (vs. a user-named page).
    pub is_journal: bool,
    /// ISO `YYYY-MM-DD` for journal pages; `None` for named pages. Currently
    /// only written — the DB column drives query ordering, and the date is
    /// otherwise read from the page title. Kept (hence `allow(dead_code)`) for
    /// the planned jump-to-date / calendar feature (see TODO.md).
    #[allow(dead_code)]
    pub journal_date: Option<String>,
    /// The page's markdown text.
    pub content: String,
}

/// One row of a page's "Linked References" panel: another page whose
/// text links to the page being viewed, with the linking line as a
/// snippet.
#[derive(Clone, Debug)]
pub struct Backlink {
    pub source_page_id: i64,
    pub source_page_title: String,
    pub snippet: String,
}

/// One full-text search result.
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub page_id: i64,
    pub title: String,
    pub snippet: String,
}
