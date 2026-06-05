//! SQLite storage for rumin: pages, the nested-block outliner, and the
//! `[[wiki-link]]` index that powers backlinks.
//!
//! These are deliberately thin, primitive operations. The tree logic
//! (which block is whose previous sibling, where an indent lands) lives
//! in `app.rs`, which holds the in-memory outline and calls down here to
//! persist each change. Timestamps come from SQLite itself
//! (`datetime('now')` column defaults) so this layer needs no clock.
//!
//! Everything runs synchronously on the UI thread. The working set is
//! one page of blocks, so the queries are tiny; moving writes onto
//! `cx.background_executor()` is a noted follow-up, not a need yet.

use rusqlite::{Connection, OptionalExtension, params};

use crate::models::{Backlink, Block, Page};
use crate::paths;

/// Schema, applied once when `PRAGMA user_version` is 0. Bump the
/// version and add a new block for future migrations.
const SCHEMA_V1: &str = r#"
CREATE TABLE pages (
    id           INTEGER PRIMARY KEY,
    title        TEXT NOT NULL UNIQUE,
    is_journal   INTEGER NOT NULL DEFAULT 0,
    journal_date TEXT UNIQUE,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE blocks (
    id         INTEGER PRIMARY KEY,
    page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    parent_id  INTEGER REFERENCES blocks(id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    content    TEXT NOT NULL DEFAULT '',
    collapsed  INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_blocks_page   ON blocks(page_id);
CREATE INDEX idx_blocks_parent ON blocks(parent_id, position);

CREATE TABLE links (
    source_block_id INTEGER NOT NULL REFERENCES blocks(id) ON DELETE CASCADE,
    target_page_id  INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    PRIMARY KEY (source_block_id, target_page_id)
);
CREATE INDEX idx_links_target ON links(target_page_id);

PRAGMA user_version = 1;
"#;

pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (creating if needed) the database under the platform data
    /// dir, enable foreign keys, and run pending migrations.
    pub fn open() -> rusqlite::Result<Self> {
        let path = paths::db_path();
        if let Some(parent) = path.parent() {
            // Idempotent; if it genuinely fails, `Connection::open`
            // surfaces a clear error next.
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(&path)?;
        // Per-connection — required for the ON DELETE CASCADE rules.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Self::migrate(&conn)?;
        Ok(Db { conn })
    }

    /// In-memory database — a resilient fallback if the on-disk file
    /// can't be opened, so the app still runs (state just won't persist).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Self::migrate(&conn)?;
        Ok(Db { conn })
    }

    fn migrate(conn: &Connection) -> rusqlite::Result<()> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(SCHEMA_V1)?;
        }
        Ok(())
    }

    // --- Pages ---

    /// The journal page for an ISO `YYYY-MM-DD` date, creating it (titled
    /// by the date) on first access.
    pub fn get_or_create_journal(&self, date: &str) -> rusqlite::Result<Page> {
        if let Some(page) = self
            .conn
            .query_row(
                "SELECT id, title, is_journal, journal_date FROM pages WHERE journal_date = ?1",
                params![date],
                row_to_page,
            )
            .optional()?
        {
            return Ok(page);
        }
        self.conn.execute(
            "INSERT INTO pages (title, is_journal, journal_date) VALUES (?1, 1, ?1)",
            params![date],
        )?;
        Ok(Page {
            id: self.conn.last_insert_rowid(),
            title: date.to_string(),
            is_journal: true,
            journal_date: Some(date.to_string()),
        })
    }

    /// A named page by title (case-insensitive), creating it if absent.
    /// This is what a `[[wiki-link]]` resolves to — typing `[[Foo]]`
    /// brings page "Foo" into existence.
    pub fn get_or_create_page(&self, title: &str) -> rusqlite::Result<Page> {
        let title = title.trim();
        if let Some(page) = self
            .conn
            .query_row(
                "SELECT id, title, is_journal, journal_date FROM pages \
                 WHERE title = ?1 COLLATE NOCASE",
                params![title],
                row_to_page,
            )
            .optional()?
        {
            return Ok(page);
        }
        self.conn
            .execute("INSERT INTO pages (title, is_journal) VALUES (?1, 0)", params![title])?;
        Ok(Page {
            id: self.conn.last_insert_rowid(),
            title: title.to_string(),
            is_journal: false,
            journal_date: None,
        })
    }

    pub fn get_page(&self, id: i64) -> rusqlite::Result<Option<Page>> {
        self.conn
            .query_row(
                "SELECT id, title, is_journal, journal_date FROM pages WHERE id = ?1",
                params![id],
                row_to_page,
            )
            .optional()
    }

    /// Journals, most recent first.
    pub fn list_journals(&self, limit: i64) -> rusqlite::Result<Vec<Page>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, is_journal, journal_date FROM pages \
             WHERE is_journal = 1 ORDER BY journal_date DESC LIMIT ?1",
        )?;
        stmt.query_map(params![limit], row_to_page)?.collect()
    }

    /// Named (non-journal) pages, alphabetical.
    pub fn list_pages(&self) -> rusqlite::Result<Vec<Page>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, is_journal, journal_date FROM pages \
             WHERE is_journal = 0 ORDER BY title COLLATE NOCASE",
        )?;
        stmt.query_map([], row_to_page)?.collect()
    }

    // --- Blocks ---

    /// Every block on a page, ordered by position. The caller assembles
    /// these into the parent/child tree.
    pub fn blocks_for_page(&self, page_id: i64) -> rusqlite::Result<Vec<Block>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, page_id, parent_id, position, content, collapsed \
             FROM blocks WHERE page_id = ?1 ORDER BY position ASC",
        )?;
        stmt.query_map(params![page_id], row_to_block)?.collect()
    }

    pub fn create_block(
        &self,
        page_id: i64,
        parent_id: Option<i64>,
        position: i64,
        content: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO blocks (page_id, parent_id, position, content) \
             VALUES (?1, ?2, ?3, ?4)",
            params![page_id, parent_id, position, content],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_block_content(&self, id: i64, content: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE blocks SET content = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![id, content],
        )?;
        Ok(())
    }

    pub fn set_collapsed(&self, id: i64, collapsed: bool) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE blocks SET collapsed = ?2 WHERE id = ?1", params![id, collapsed])?;
        Ok(())
    }

    /// Reparent / reposition a block (the persistence half of an indent,
    /// outdent, or reorder).
    pub fn move_block(
        &self,
        id: i64,
        parent_id: Option<i64>,
        position: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE blocks SET parent_id = ?2, position = ?3, updated_at = datetime('now') \
             WHERE id = ?1",
            params![id, parent_id, position],
        )?;
        Ok(())
    }

    pub fn delete_block(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM blocks WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Make room among a sibling group: bump every sibling positioned
    /// after `after_position` up by one. `parent_id IS ?2` matches NULL
    /// (top-level) and concrete parents alike.
    pub fn shift_siblings_after(
        &self,
        page_id: i64,
        parent_id: Option<i64>,
        after_position: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE blocks SET position = position + 1 \
             WHERE page_id = ?1 AND parent_id IS ?2 AND position > ?3",
            params![page_id, parent_id, after_position],
        )?;
        Ok(())
    }

    /// Highest position among a sibling group, or `None` if the group is
    /// empty (used to append a new last child).
    pub fn max_child_position(
        &self,
        page_id: i64,
        parent_id: Option<i64>,
    ) -> rusqlite::Result<Option<i64>> {
        self.conn.query_row(
            "SELECT MAX(position) FROM blocks WHERE page_id = ?1 AND parent_id IS ?2",
            params![page_id, parent_id],
            |r| r.get::<_, Option<i64>>(0),
        )
    }

    // --- Links / backlinks ---

    /// Replace a block's outgoing links with the given target page
    /// titles, auto-creating any target page that doesn't exist yet.
    pub fn rebuild_links(&self, source_block_id: i64, target_titles: &[String]) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM links WHERE source_block_id = ?1", params![source_block_id])?;
        for title in target_titles {
            let page = self.get_or_create_page(title)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO links (source_block_id, target_page_id) VALUES (?1, ?2)",
                params![source_block_id, page.id],
            )?;
        }
        Ok(())
    }

    /// Blocks on *other* pages that link to `page_id` — the "Linked
    /// References" list.
    pub fn backlinks(&self, page_id: i64) -> rusqlite::Result<Vec<Backlink>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.title, b.content \
             FROM links l \
             JOIN blocks b ON b.id = l.source_block_id \
             JOIN pages  p ON p.id = b.page_id \
             WHERE l.target_page_id = ?1 AND b.page_id != ?1 \
             ORDER BY p.is_journal DESC, p.journal_date DESC, p.title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![page_id], |row| {
            Ok(Backlink {
                source_page_id: row.get(0)?,
                source_page_title: row.get(1)?,
                block_content: row.get(2)?,
            })
        })?;
        rows.collect()
    }
}

fn row_to_page(row: &rusqlite::Row) -> rusqlite::Result<Page> {
    Ok(Page {
        id: row.get(0)?,
        title: row.get(1)?,
        is_journal: row.get::<_, i64>(2)? != 0,
        journal_date: row.get(3)?,
    })
}

fn row_to_block(row: &rusqlite::Row) -> rusqlite::Result<Block> {
    Ok(Block {
        id: row.get(0)?,
        page_id: row.get(1)?,
        parent_id: row.get(2)?,
        position: row.get(3)?,
        content: row.get(4)?,
        collapsed: row.get::<_, i64>(5)? != 0,
    })
}
