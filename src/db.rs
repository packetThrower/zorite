//! SQLite storage for rumin (v2): each page is a single markdown
//! document, plus a `[[wiki-link]]` index over pages for backlinks.
//!
//! Schema v2 replaces the v1 block-outliner model. New databases get v2
//! directly; existing v1 databases are migrated in place — each page's
//! blocks are folded into a markdown bullet list so nothing is lost.
//!
//! Timestamps come from SQLite (`datetime('now')` defaults). Everything
//! runs synchronously on the UI thread (tiny working set).

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::models::{Backlink, Page, SearchHit};
use crate::paths;

/// Fresh-install schema (applied when `user_version` is 0).
const SCHEMA_V2: &str = r#"
CREATE TABLE pages (
    id           INTEGER PRIMARY KEY,
    title        TEXT NOT NULL UNIQUE,
    is_journal   INTEGER NOT NULL DEFAULT 0,
    journal_date TEXT UNIQUE,
    content      TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE page_links (
    source_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    target_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    PRIMARY KEY (source_page_id, target_page_id)
);
CREATE INDEX idx_page_links_target ON page_links(target_page_id);

PRAGMA user_version = 2;
"#;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open() -> rusqlite::Result<Self> {
        let path = paths::db_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Self::migrate(&mut conn)?;
        Ok(Db { conn })
    }

    /// In-memory fallback so the app still runs if the on-disk file can't
    /// be opened (state just won't persist).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Self::migrate(&mut conn)?;
        Ok(Db { conn })
    }

    fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
        let mut version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            conn.execute_batch(SCHEMA_V2)?;
            version = 2;
        } else if version < 2 {
            // Atomic: a half-applied upgrade would otherwise wedge the DB
            // (e.g. a re-added `content` column on the next run).
            let tx = conn.transaction()?;
            migrate_v1_to_v2(&tx)?;
            tx.commit()?;
            version = 2;
        }
        if version < 3 {
            // One-time cleanup: an earlier bug re-indexed links on every
            // keystroke, creating a page for every prefix of a typed
            // `#tag`. Drop empty, unreferenced named pages — journals and
            // any page that's actually linked are kept.
            conn.execute_batch(
                "DELETE FROM pages WHERE is_journal = 0 AND content = '' \
                   AND id NOT IN (SELECT target_page_id FROM page_links);\
                 PRAGMA user_version = 3;",
            )?;
        }
        Ok(())
    }

    // --- Pages ---

    pub fn get_or_create_journal(&self, date: &str) -> rusqlite::Result<Page> {
        if let Some(page) = self.get_journal_by_date(date)? {
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
            content: String::new(),
        })
    }

    /// Look up an existing journal without creating one.
    pub fn get_journal_by_date(&self, date: &str) -> rusqlite::Result<Option<Page>> {
        self.conn
            .query_row(
                "SELECT id, title, is_journal, journal_date, content \
                 FROM pages WHERE journal_date = ?1",
                params![date],
                row_to_page,
            )
            .optional()
    }

    /// A named page by title (case-insensitive), creating it if absent —
    /// what a `[[wiki-link]]` resolves to.
    pub fn get_or_create_page(&self, title: &str) -> rusqlite::Result<Page> {
        let title = title.trim();
        if let Some(page) = self
            .conn
            .query_row(
                "SELECT id, title, is_journal, journal_date, content \
                 FROM pages WHERE title = ?1 COLLATE NOCASE",
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
            content: String::new(),
        })
    }

    pub fn get_page(&self, id: i64) -> rusqlite::Result<Option<Page>> {
        self.conn
            .query_row(
                "SELECT id, title, is_journal, journal_date, content FROM pages WHERE id = ?1",
                params![id],
                row_to_page,
            )
            .optional()
    }

    /// Look up a page by title (case-insensitive) without creating it.
    pub fn get_page_by_title(&self, title: &str) -> rusqlite::Result<Option<Page>> {
        self.conn
            .query_row(
                "SELECT id, title, is_journal, journal_date, content FROM pages \
                 WHERE title = ?1 COLLATE NOCASE",
                params![title.trim()],
                row_to_page,
            )
            .optional()
    }

    pub fn list_journals(&self, limit: i64) -> rusqlite::Result<Vec<Page>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, is_journal, journal_date, content FROM pages \
             WHERE is_journal = 1 ORDER BY journal_date DESC LIMIT ?1",
        )?;
        stmt.query_map(params![limit], row_to_page)?.collect()
    }

    pub fn list_pages(&self) -> rusqlite::Result<Vec<Page>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, is_journal, journal_date, content FROM pages \
             WHERE is_journal = 0 ORDER BY title COLLATE NOCASE",
        )?;
        stmt.query_map([], row_to_page)?.collect()
    }

    pub fn set_page_content(&self, id: i64, content: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE pages SET content = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![id, content],
        )?;
        Ok(())
    }

    // --- Links / backlinks ---

    /// Replace a page's outgoing links with the given target titles,
    /// auto-creating any target page that doesn't exist yet.
    pub fn rebuild_page_links(
        &self,
        source_page_id: i64,
        target_titles: &[String],
    ) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM page_links WHERE source_page_id = ?1", params![source_page_id])?;
        for title in target_titles {
            let target = self.get_or_create_page(title)?;
            if target.id != source_page_id {
                self.conn.execute(
                    "INSERT OR IGNORE INTO page_links (source_page_id, target_page_id) \
                     VALUES (?1, ?2)",
                    params![source_page_id, target.id],
                )?;
            }
        }
        Ok(())
    }

    /// Pages that link to `page_id`, each with the linking line as a
    /// snippet — the "Linked References" list.
    pub fn backlinks(&self, page_id: i64) -> rusqlite::Result<Vec<Backlink>> {
        let Some(target) = self.get_page(page_id)? else { return Ok(Vec::new()) };
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.title, p.content FROM page_links l \
             JOIN pages p ON p.id = l.source_page_id \
             WHERE l.target_page_id = ?1 AND p.id != ?1 \
             ORDER BY p.is_journal DESC, p.journal_date DESC, p.title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![page_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, title, content) = r?;
            out.push(Backlink {
                source_page_id: id,
                source_page_title: title,
                snippet: snippet_for(&content, &target.title),
            });
        }
        Ok(out)
    }
}

impl Db {
    /// Full-text-ish search over page titles and content (substring,
    /// case-insensitive). Title matches sort first, then journals by
    /// date. Returns up to `limit` hits with a snippet around the match.
    pub fn search(&self, query: &str, limit: i64) -> rusqlite::Result<Vec<SearchHit>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let like = format!("%{}%", escape_like(q));
        let mut stmt = self.conn.prepare(
            "SELECT id, title, content FROM pages \
             WHERE title LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\' \
             ORDER BY (CASE WHEN title LIKE ?1 ESCAPE '\\' THEN 0 ELSE 1 END), \
                      is_journal DESC, journal_date DESC, title COLLATE NOCASE \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![like, limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, title, content) = r?;
            out.push(SearchHit { page_id: id, title, snippet: snippet_for_query(&content, q) });
        }
        Ok(out)
    }
}

/// Escape SQL LIKE wildcards so the query is matched literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// The first content line containing `query` (case-insensitive), else
/// the first non-empty line, trimmed and length-capped.
fn snippet_for_query(content: &str, query: &str) -> String {
    let needle = query.to_lowercase();
    let line = content
        .lines()
        .find(|l| l.to_lowercase().contains(&needle))
        .or_else(|| content.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim();
    const MAX: usize = 140;
    if line.chars().count() > MAX {
        line.chars().take(MAX).collect::<String>() + "…"
    } else {
        line.to_string()
    }
}

fn row_to_page(row: &rusqlite::Row) -> rusqlite::Result<Page> {
    Ok(Page {
        id: row.get(0)?,
        title: row.get(1)?,
        is_journal: row.get::<_, i64>(2)? != 0,
        journal_date: row.get(3)?,
        content: row.get(4)?,
    })
}

/// The line of `content` that contains `[[target]]` (else the first
/// non-empty line), trimmed and length-capped for the backlinks panel.
fn snippet_for(content: &str, target_title: &str) -> String {
    let needle = format!("[[{target_title}]]").to_lowercase();
    let line = content
        .lines()
        .find(|l| l.to_lowercase().contains(&needle))
        .or_else(|| content.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim();
    const MAX: usize = 140;
    if line.chars().count() > MAX {
        line.chars().take(MAX).collect::<String>() + "…"
    } else {
        line.to_string()
    }
}

// --- v1 → v2 migration -----------------------------------------------------

fn migrate_v1_to_v2(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("ALTER TABLE pages ADD COLUMN content TEXT NOT NULL DEFAULT '';")?;
    conn.execute_batch(
        "CREATE TABLE page_links (\
            source_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,\
            target_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,\
            PRIMARY KEY (source_page_id, target_page_id));\
         CREATE INDEX idx_page_links_target ON page_links(target_page_id);",
    )?;

    let page_ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM pages")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    // Fold each page's blocks into a markdown bullet list.
    for pid in &page_ids {
        let blocks: Vec<(i64, Option<i64>, i64, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, parent_id, position, content FROM blocks WHERE page_id = ?1")?;
            let rows = stmt.query_map(params![pid], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let md = blocks_to_markdown(&blocks);
        conn.execute("UPDATE pages SET content = ?2 WHERE id = ?1", params![pid, md])?;
    }

    // Re-derive links from the new content.
    for pid in &page_ids {
        let content: String =
            conn.query_row("SELECT content FROM pages WHERE id = ?1", params![pid], |r| r.get(0))?;
        for title in crate::ui::links::parse_links(&content) {
            let tid = migration_target_page_id(conn, &title)?;
            if tid != *pid {
                conn.execute(
                    "INSERT OR IGNORE INTO page_links (source_page_id, target_page_id) \
                     VALUES (?1, ?2)",
                    params![pid, tid],
                )?;
            }
        }
    }

    conn.execute_batch("DROP TABLE IF EXISTS links; DROP TABLE IF EXISTS blocks; PRAGMA user_version = 2;")?;
    Ok(())
}

fn migration_target_page_id(conn: &Connection, title: &str) -> rusqlite::Result<i64> {
    let title = title.trim();
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM pages WHERE title = ?1 COLLATE NOCASE",
            params![title],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute("INSERT INTO pages (title, is_journal, content) VALUES (?1, 0, '')", params![title])?;
    Ok(conn.last_insert_rowid())
}

/// Build a markdown bullet list from v1 blocks, indenting by tree depth.
fn blocks_to_markdown(blocks: &[(i64, Option<i64>, i64, String)]) -> String {
    let mut children: HashMap<Option<i64>, Vec<&(i64, Option<i64>, i64, String)>> = HashMap::new();
    for b in blocks {
        children.entry(b.1).or_default().push(b);
    }
    for kids in children.values_mut() {
        kids.sort_by_key(|b| b.2);
    }

    fn walk<'a>(
        parent: Option<i64>,
        depth: usize,
        children: &HashMap<Option<i64>, Vec<&'a (i64, Option<i64>, i64, String)>>,
        lines: &mut Vec<String>,
    ) {
        let Some(kids) = children.get(&parent) else { return };
        for b in kids {
            lines.push(format!("{}- {}", "  ".repeat(depth), b.3));
            walk(Some(b.0), depth + 1, children, lines);
        }
    }

    let mut lines = Vec::new();
    walk(None, 0, &children, &mut lines);
    lines.join("\n")
}
