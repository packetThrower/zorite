//! SQLite storage for Zorite (v2): each page is a single markdown
//! document, plus a `[[wiki-link]]` index over pages for backlinks.
//!
//! Schema v2 replaces the v1 block-outliner model. New databases get v2
//! directly; existing v1 databases are migrated in place — each page's
//! blocks are folded into a markdown bullet list so nothing is lost.
//!
//! Timestamps come from SQLite (`datetime('now')` defaults). Everything
//! runs synchronously on the UI thread (tiny working set).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::models::{Backlink, Page};
use crate::paths;

/// Read a single `settings` value by key, best-effort (`None` if absent or the
/// query fails). Used by the read-only boot probes below.
fn read_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// Read the saved `theme_skin` and `theme_mode` from a database file, read-only
/// and without migrating or write-locking it — used to theme the data-move
/// progress window before the database is opened normally. Best-effort:
/// `(None, None)` if the file can't be read. The connection is dropped before
/// returning, so it never holds the file against the impending move.
pub fn read_theme(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(conn) = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return (None, None);
    };
    (
        read_setting(&conn, "theme_skin"),
        read_setting(&conn, "theme_mode"),
    )
}

/// Read the update-check preferences from a database file, read-only. Used by
/// the boot check before the app's main DB handle is wired. Defaults:
/// auto-check on, pre-releases off. Best-effort — defaults if unreadable.
pub fn read_update_prefs(path: &Path) -> (bool, bool) {
    let Ok(conn) = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return (true, false);
    };
    let check = read_setting(&conn, "check_updates")
        .map(|v| v != "0")
        .unwrap_or(true);
    let prerelease = read_setting(&conn, "include_prerelease")
        .map(|v| v == "1")
        .unwrap_or(false);
    (check, prerelease)
}

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

/// The newest schema version [`Db::migrate`] upgrades to. Bump this with each new
/// migration step so [`Db::open`] knows when a pre-migration backup is warranted.
const SCHEMA_VERSION: i64 = 7;

/// A failed on-disk database open. Carries the pre-migration backup path (when one
/// was taken) so the caller can point the user at their recoverable data instead
/// of silently dropping them into an empty workspace.
#[derive(Debug)]
pub struct OpenError {
    pub source: rusqlite::Error,
    /// The `<db>.bak-v<N>` snapshot taken before the migration that failed, if any.
    pub backup: Option<PathBuf>,
}

impl OpenError {
    /// A failure with no associated backup (couldn't even open / configure the file).
    fn bare(source: rusqlite::Error) -> Self {
        Self {
            source,
            backup: None,
        }
    }
}

/// Whether the on-disk database is SQLCipher-encrypted: an encrypted file has
/// no plaintext `SQLite format 3` magic. `false` for missing or empty files
/// (fresh installs). The boot flow uses this to route to the unlock screen —
/// and to keep an encrypted file out of the corruption-recovery path.
pub fn db_is_encrypted() -> bool {
    file_is_encrypted(&paths::db_path())
}

fn file_is_encrypted(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 16];
    match f.read_exact(&mut magic) {
        Ok(()) => &magic != b"SQLite format 3\0",
        Err(_) => false, // shorter than a header: empty / fresh
    }
}

impl Db {
    /// Open the app database, decrypting with `key` when one is given. A
    /// wrong (or missing) key on an encrypted file surfaces as an error from
    /// the first real statement — callers distinguish that from corruption
    /// via [`db_is_encrypted`].
    pub fn open(key: Option<&str>) -> Result<Self, OpenError> {
        let path = paths::db_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Self::open_at(&path, key)
    }

    fn open_at(path: &Path, key: Option<&str>) -> Result<Self, OpenError> {
        let mut conn = Connection::open(path).map_err(OpenError::bare)?;
        // The key must be the FIRST statement on the connection — anything
        // else touches the (unreadable) pages first and fails.
        if let Some(key) = key {
            conn.pragma_update(None, "key", key)
                .map_err(OpenError::bare)?;
        }
        // WAL keeps the per-keystroke autosave write off the fsync-per-commit path
        // (rollback journal + synchronous=FULL fsyncs twice per write); in WAL,
        // synchronous=NORMAL fsyncs only at checkpoint and is still durable across an
        // app crash (only a power loss can drop the last txn — fine for autosave). It
        // also lets a second window read while this one writes (no reader/writer block),
        // and busy_timeout makes a rare concurrent-write retry instead of erroring.
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;\
             PRAGMA journal_mode = WAL;\
             PRAGMA synchronous = NORMAL;\
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(OpenError::bare)?;
        // Snapshot the DB before any schema upgrade so a buggy migration is
        // recoverable; carry the snapshot path in the error if migration fails.
        let backup = Self::backup_before_migration(&conn, path);
        Self::migrate(&mut conn).map_err(|source| OpenError { source, backup })?;
        Ok(Db { conn })
    }

    /// Re-encrypt the database in place: SQLCipher-export a sibling copy under
    /// `new_key` (`None` = plaintext), swap it in atomically, and reopen the
    /// connection. The caller owns UX (confirmations, keychain updates). The
    /// path comes from the connection itself, so tests run on temp files.
    pub fn set_encryption(&mut self, new_key: Option<&str>) -> rusqlite::Result<()> {
        let path: PathBuf = self
            .conn
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name = 'main'",
                [],
                |r| r.get::<_, String>(0),
            )?
            .into();
        let tmp = path.with_extension("db.reenc");
        let _ = std::fs::remove_file(&tmp);
        // Fold the WAL in so the export sees every committed write.
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        self.conn.execute(
            "ATTACH DATABASE ?1 AS reenc KEY ?2",
            params![tmp.to_string_lossy(), new_key.unwrap_or("")],
        )?;
        let export = self
            .conn
            .query_row("SELECT sqlcipher_export('reenc')", [], |_| Ok(()));
        // sqlcipher_export copies schema + data but NOT user_version; without
        // it the reopen below would re-run every migration on a full DB.
        let version: i64 = self
            .conn
            .query_row("PRAGMA main.user_version", [], |r| r.get(0))?;
        let set_version = self
            .conn
            .execute_batch(&format!("PRAGMA reenc.user_version = {version};"));
        let detach = self.conn.execute("DETACH DATABASE reenc", []);
        export?;
        set_version?;
        detach?;
        // Swap: release the file (SQLite holds it open), move the new one in,
        // and clear stale WAL/SHM siblings from the old incarnation.
        let placeholder = Connection::open_in_memory()?;
        drop(std::mem::replace(&mut self.conn, placeholder));
        std::fs::rename(&tmp, &path).map_err(|e| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
                Some(format!("swap re-encrypted database: {e}")),
            )
        })?;
        for ext in ["db-wal", "db-shm"] {
            let _ = std::fs::remove_file(path.with_extension(ext));
        }
        *self = Self::open_at(&path, new_key).map_err(|e| e.source)?;
        Ok(())
    }

    /// Whether `key` opens the current on-disk database — used by the unlock
    /// screen and to verify the current password before a change/removal, on
    /// a throwaway connection.
    pub fn verify_key(key: &str) -> bool {
        Self::verify_key_at(&paths::db_path(), key)
    }

    fn verify_key_at(path: &Path, key: &str) -> bool {
        let Ok(conn) = Connection::open(path) else {
            return false;
        };
        conn.pragma_update(None, "key", key).is_ok()
            && conn
                .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                    r.get::<_, i64>(0)
                })
                .is_ok()
    }

    /// If an on-disk schema upgrade is pending, copy the database to
    /// `<db>.bak-v<from>` first so a bad migration is recoverable. Best-effort:
    /// returns the backup path, or `None` for a fresh / already-current database
    /// or if the copy failed (logged — the transactional migrations are the
    /// primary safety; this guards against a migration that *succeeds* but mangles
    /// data). One snapshot per source version; re-running an upgrade just rewrites it.
    fn backup_before_migration(conn: &Connection, path: &Path) -> Option<PathBuf> {
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .ok()?;
        if version == 0 || version >= SCHEMA_VERSION {
            return None; // fresh install or already current — nothing to lose
        }
        // Fold the WAL into the main file so the plain-file copy is complete.
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        let name = path.file_name()?.to_string_lossy();
        let bak = path.with_file_name(format!("{name}.bak-v{version}"));
        match std::fs::copy(path, &bak) {
            Ok(_) => {
                log::info!(
                    "backed up database to {} before migrating from v{version}",
                    bak.display()
                );
                Some(bak)
            }
            Err(e) => {
                log::warn!("pre-migration backup to {} failed: {e}", bak.display());
                None
            }
        }
    }

    /// Open a database at an arbitrary path — encryption round-trip tests
    /// run on temp files instead of the real data dir.
    #[cfg(test)]
    fn open_file(path: &Path, key: Option<&str>) -> Result<Self, OpenError> {
        Self::open_at(path, key)
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
            let tx = conn.transaction()?;
            tx.execute_batch(SCHEMA_V2)?;
            tx.commit()?;
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
            let tx = conn.transaction()?;
            tx.execute_batch(
                "DELETE FROM pages WHERE is_journal = 0 AND content = '' \
                   AND id NOT IN (SELECT target_page_id FROM page_links);\
                 PRAGMA user_version = 3;",
            )?;
            tx.commit()?;
        }
        if version < 4 {
            // Page aliases (Logseq-style `alias::` property): alternate names
            // that resolve to a page.
            let tx = conn.transaction()?;
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS page_aliases (\
                    page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,\
                    alias   TEXT NOT NULL,\
                    PRIMARY KEY (page_id, alias)\
                 );\
                 CREATE INDEX IF NOT EXISTS idx_page_aliases_alias \
                    ON page_aliases(alias COLLATE NOCASE);\
                 PRAGMA user_version = 4;",
            )?;
            tx.commit()?;
        }
        if version < 5 {
            // Full-text search: a **trigram** FTS5 index over page title + content keeps
            // the same case-insensitive *substring* matching the old `LIKE` scan did, but
            // indexed so it scales to many pages. External-content (`content='pages'`) — no
            // duplicate storage — kept in sync by triggers. Wrapped in a transaction so a
            // partial build can't wedge the DB.
            let tx = conn.transaction()?;
            tx.execute_batch(
                "CREATE VIRTUAL TABLE pages_fts USING fts5(\
                    title, content, content='pages', content_rowid='id', tokenize='trigram'\
                 );\
                 INSERT INTO pages_fts(rowid, title, content) \
                    SELECT id, title, content FROM pages;\
                 CREATE TRIGGER pages_fts_ai AFTER INSERT ON pages BEGIN \
                    INSERT INTO pages_fts(rowid, title, content) \
                       VALUES (new.id, new.title, new.content); \
                 END;\
                 CREATE TRIGGER pages_fts_ad AFTER DELETE ON pages BEGIN \
                    INSERT INTO pages_fts(pages_fts, rowid, title, content) \
                       VALUES ('delete', old.id, old.title, old.content); \
                 END;\
                 CREATE TRIGGER pages_fts_au AFTER UPDATE ON pages BEGIN \
                    INSERT INTO pages_fts(pages_fts, rowid, title, content) \
                       VALUES ('delete', old.id, old.title, old.content); \
                    INSERT INTO pages_fts(rowid, title, content) \
                       VALUES (new.id, new.title, new.content); \
                 END;\
                 PRAGMA user_version = 5;",
            )?;
            tx.commit()?;
        }
        if version < 6 {
            // Whiteboards are pages with `kind = 'whiteboard'` (their canvas
            // stored as JSON in `content`). Add the discriminator, defaulting
            // every existing row to 'page'. The FTS index is for text notes, so
            // re-create its triggers to skip non-'page' rows — a board's JSON
            // body would otherwise pollute search. `kind` is immutable per row
            // in practice (a page never becomes a board), so guarding INSERT /
            // UPDATE on `new.kind` and DELETE on `old.kind` is sufficient.
            let tx = conn.transaction()?;
            tx.execute_batch(
                "ALTER TABLE pages ADD COLUMN kind TEXT NOT NULL DEFAULT 'page';\
                 DROP TRIGGER IF EXISTS pages_fts_ai;\
                 DROP TRIGGER IF EXISTS pages_fts_ad;\
                 DROP TRIGGER IF EXISTS pages_fts_au;\
                 CREATE TRIGGER pages_fts_ai AFTER INSERT ON pages WHEN new.kind = 'page' BEGIN \
                    INSERT INTO pages_fts(rowid, title, content) \
                       VALUES (new.id, new.title, new.content); \
                 END;\
                 CREATE TRIGGER pages_fts_ad AFTER DELETE ON pages WHEN old.kind = 'page' BEGIN \
                    INSERT INTO pages_fts(pages_fts, rowid, title, content) \
                       VALUES ('delete', old.id, old.title, old.content); \
                 END;\
                 CREATE TRIGGER pages_fts_au AFTER UPDATE ON pages WHEN new.kind = 'page' BEGIN \
                    INSERT INTO pages_fts(pages_fts, rowid, title, content) \
                       VALUES ('delete', old.id, old.title, old.content); \
                    INSERT INTO pages_fts(rowid, title, content) \
                       VALUES (new.id, new.title, new.content); \
                 END;\
                 PRAGMA user_version = 6;",
            )?;
            tx.commit()?;
        }
        if version < 7 {
            // Whiteboard templates: a reusable group of canvas elements, stored
            // as JSON (a normalized `Vec<Element>` from the gpui-whiteboard
            // crate). Global — not tied to any one board — and shown as cards in
            // the board toolbar's Pages & Images flyout.
            let tx = conn.transaction()?;
            tx.execute_batch(
                "CREATE TABLE whiteboard_templates (\
                    id         INTEGER PRIMARY KEY,\
                    name       TEXT NOT NULL,\
                    content    TEXT NOT NULL,\
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))\
                 );\
                 PRAGMA user_version = 7;",
            )?;
            tx.commit()?;
        }
        // Key/value app settings (theme mode, etc.). Idempotent, so no
        // `user_version` bump is needed.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
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
            created_at: None,
            updated_at: None,
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

    /// A named page by title (case-insensitive), creating it if absent — what a
    /// `[[wiki-link]]` resolves to. An exact title wins; failing that, a page
    /// whose `alias::` list contains the name resolves to that page; otherwise a
    /// new page is created.
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
        if let Some(page) = self.get_page_by_alias(title)? {
            return Ok(page);
        }
        self.conn.execute(
            "INSERT INTO pages (title, is_journal) VALUES (?1, 0)",
            params![title],
        )?;
        Ok(Page {
            id: self.conn.last_insert_rowid(),
            title: title.to_string(),
            is_journal: false,
            journal_date: None,
            content: String::new(),
            created_at: None,
            updated_at: None,
        })
    }

    /// Create a brand-new, empty whiteboard with a unique "Untitled Whiteboard"
    /// title (suffixed `2`, `3`, … if taken) — so the user can keep many
    /// distinct boards. A whiteboard is a `kind = 'whiteboard'` page storing its
    /// canvas JSON in `content` (empty `{}` for a new board); it stays out of the
    /// page tree ([`list_pages`](Self::list_pages)) and full-text search but is
    /// listed by [`list_whiteboards`](Self::list_whiteboards) for the sidebar.
    /// The whiteboard with `title` (case-insensitive), if any — wiki-links
    /// check this before the page path so `[[Board]]` opens the canvas
    /// instead of its scene JSON as a text page.
    pub fn get_whiteboard_by_title(&self, title: &str) -> rusqlite::Result<Option<Page>> {
        self.conn
            .query_row(
                "SELECT id, title, is_journal, journal_date, content \
                 FROM pages WHERE kind = 'whiteboard' AND title = ?1 COLLATE NOCASE",
                params![title.trim()],
                row_to_page,
            )
            .optional()
    }

    pub fn create_whiteboard(&self) -> rusqlite::Result<Page> {
        let base = "Untitled Whiteboard";
        let mut title = base.to_string();
        let mut n = 2;
        while self.get_page_by_title(&title)?.is_some() {
            title = format!("{base} {n}");
            n += 1;
        }
        self.conn.execute(
            "INSERT INTO pages (title, is_journal, kind, content) VALUES (?1, 0, 'whiteboard', '{}')",
            params![title],
        )?;
        Ok(Page {
            id: self.conn.last_insert_rowid(),
            title,
            is_journal: false,
            journal_date: None,
            content: "{}".to_string(),
            created_at: None,
            updated_at: None,
        })
    }

    /// Create a whiteboard with a given `title` (deduped if taken) and scene
    /// `content` — used by the importer. Like [`create_whiteboard`](Self::create_whiteboard)
    /// but named and pre-filled.
    pub fn create_whiteboard_with(&self, title: &str, content: &str) -> rusqlite::Result<Page> {
        let mut t = title.to_string();
        let mut n = 2;
        while self.get_page_by_title(&t)?.is_some() {
            t = format!("{title} {n}");
            n += 1;
        }
        self.conn.execute(
            "INSERT INTO pages (title, is_journal, kind, content) VALUES (?1, 0, 'whiteboard', ?2)",
            params![t, content],
        )?;
        Ok(Page {
            id: self.conn.last_insert_rowid(),
            title: t,
            is_journal: false,
            journal_date: None,
            content: content.to_string(),
            created_at: None,
            updated_at: None,
        })
    }

    /// Every whiteboard, most-recently-updated first, for the sidebar's
    /// "Whiteboards" section. Content is not loaded (like [`list_pages`]).
    ///
    /// [`list_pages`]: Self::list_pages
    pub fn list_whiteboards(&self) -> rusqlite::Result<Vec<Page>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, is_journal, journal_date, created_at, updated_at FROM pages \
             WHERE kind = 'whiteboard' ORDER BY updated_at DESC, id DESC",
        )?;
        stmt.query_map([], |row| {
            Ok(Page {
                id: row.get(0)?,
                title: row.get(1)?,
                is_journal: row.get::<_, i64>(2)? != 0,
                journal_date: row.get(3)?,
                content: String::new(),
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?
        .collect()
    }

    // --- Whiteboard templates ---

    /// Every saved whiteboard template as `(id, name, content_json)`, newest
    /// first. `content` is a serialized `Vec<Element>` (origin-normalized).
    pub fn list_templates(&self) -> rusqlite::Result<Vec<(i64, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, content FROM whiteboard_templates \
             ORDER BY created_at DESC, id DESC",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
    }

    /// Store a new template, returning its id.
    pub fn create_template(&self, name: &str, content: &str) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO whiteboard_templates (name, content) VALUES (?1, ?2)",
            params![name, content],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Delete a template by id.
    pub fn delete_template(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM whiteboard_templates WHERE id = ?1",
            params![id],
        )?;
        Ok(())
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

    /// Look up a page by one of its `alias::` names (case-insensitive). If two
    /// pages claim the same alias, the lowest page id wins (arbitrary but stable).
    pub fn get_page_by_alias(&self, alias: &str) -> rusqlite::Result<Option<Page>> {
        self.conn
            .query_row(
                "SELECT p.id, p.title, p.is_journal, p.journal_date, p.content \
                 FROM page_aliases a JOIN pages p ON p.id = a.page_id \
                 WHERE a.alias = ?1 COLLATE NOCASE ORDER BY p.id LIMIT 1",
                params![alias.trim()],
                row_to_page,
            )
            .optional()
    }

    /// The named pages for the sidebar tree and autocomplete. Content is
    /// intentionally **not** loaded — this runs on every sidebar refresh and
    /// content dominates the cost (≈4× slower at 10k pages). The returned
    /// `Page.content` is therefore empty; use [`get_page`](Self::get_page) when
    /// you need a page's body.
    pub fn list_pages(&self) -> rusqlite::Result<Vec<Page>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, is_journal, journal_date, created_at, updated_at FROM pages \
             WHERE is_journal = 0 AND kind = 'page' ORDER BY title COLLATE NOCASE",
        )?;
        stmt.query_map([], |row| {
            Ok(Page {
                id: row.get(0)?,
                title: row.get(1)?,
                is_journal: row.get::<_, i64>(2)? != 0,
                journal_date: row.get(3)?,
                content: String::new(),
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?
        .collect()
    }

    /// The ids of the most-recently-updated named pages, newest first. Used to
    /// seed the sidebar's "recent" list before the user has viewed anything.
    pub fn recent_page_ids(&self, limit: usize) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM pages WHERE is_journal = 0 \
             ORDER BY updated_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, i64>(0))?;
        rows.collect()
    }

    pub fn set_page_content(&self, id: i64, content: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE pages SET content = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![id, content],
        )?;
        Ok(())
    }

    /// Delete a named page. The `is_journal = 0` guard makes deleting a
    /// journal day impossible even if this is called with a journal id.
    /// `page_links` rows that reference the page (in either direction) are
    /// removed by the `ON DELETE CASCADE` foreign keys. Returns whether a
    /// row was actually deleted.
    pub fn delete_page(&self, id: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM pages WHERE id = ?1 AND is_journal = 0",
            params![id],
        )?;
        Ok(n > 0)
    }

    /// Rename a named page and rewrite `[[old]]` → `[[new]]` everywhere it's
    /// referenced (so backlinks stay connected). Journals can't be renamed.
    /// Returns `false` (no change) for a journal, an empty/unchanged title,
    /// or a title already taken by another page. The page id is unchanged,
    /// so `page_links` (keyed by id) stay valid.
    pub fn rename_page(&self, id: i64, new_title: &str) -> rusqlite::Result<bool> {
        let new_title = new_title.trim();
        let Some(page) = self.get_page(id)? else {
            return Ok(false);
        };
        if page.is_journal
            || new_title.is_empty()
            || new_title.eq_ignore_ascii_case(page.title.trim())
        {
            return Ok(false);
        }
        // Reject a collision with a different existing page.
        if let Some(existing) = self.get_page_by_title(new_title)?
            && existing.id != id
        {
            return Ok(false);
        }

        // Cascade: renaming a namespace takes its `Foo::*` children along
        // (`Foo::Task` follows `Foo` → `Bar` as `Bar::Task`). Every rename —
        // the page itself plus each child — rewrites its exact `[[links]]`.
        let prefix = format!("{}{}", page.title, crate::hierarchy::SEP);
        let mut renames = vec![(id, page.title.clone(), new_title.to_string())];
        {
            let like = format!("{}%", escape_like(&prefix));
            let mut stmt = self.conn.prepare(
                "SELECT id, title FROM pages WHERE is_journal = 0 AND title LIKE ?1 ESCAPE '\\'",
            )?;
            let rows = stmt.query_map(params![like], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (cid, old) = row?;
                let new = format!("{new_title}{}", &old[page.title.len()..]);
                renames.push((cid, old, new));
            }
        }
        // Any child landing on an existing title aborts the whole rename, so
        // there's never a half-moved namespace.
        for (cid, _, new) in &renames[1..] {
            if let Some(existing) = self.get_page_by_title(new)?
                && existing.id != *cid
            {
                return Ok(false);
            }
        }

        let tx = self.conn.unchecked_transaction()?;
        for (pid, old, new) in &renames {
            tx.execute(
                "UPDATE pages SET title = ?2, updated_at = datetime('now') WHERE id = ?1 AND is_journal = 0",
                params![pid, new],
            )?;
            let old_link = format!("[[{old}]]");
            let new_link = format!("[[{new}]]");
            let like = format!("%{}%", escape_like(&old_link));
            let affected: Vec<(i64, String)> = {
                let mut stmt =
                    tx.prepare("SELECT id, content FROM pages WHERE content LIKE ?1 ESCAPE '\\'")?;
                let rows = stmt.query_map(params![like], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (pid, content) in &affected {
                let updated = content.replace(&old_link, &new_link);
                tx.execute(
                    "UPDATE pages SET content = ?2 WHERE id = ?1",
                    params![pid, updated],
                )?;
            }
        }
        tx.commit()?;
        Ok(true)
    }

    // --- App settings (key/value) ---

    /// Read a setting, or `None` if absent (errors are swallowed to a None).
    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Upsert a setting.
    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    // --- Aliases ---

    /// A page's alias names, sorted — used to populate the alias field.
    pub fn get_page_aliases(&self, page_id: i64) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT alias FROM page_aliases WHERE page_id = ?1 ORDER BY alias COLLATE NOCASE",
        )?;
        stmt.query_map(params![page_id], |r| r.get::<_, String>(0))?
            .collect()
    }

    /// Replace a page's aliases with the given list (empty clears them).
    pub fn rebuild_page_aliases(&self, page_id: i64, aliases: &[String]) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM page_aliases WHERE page_id = ?1",
            params![page_id],
        )?;
        for alias in aliases {
            let alias = alias.trim();
            if !alias.is_empty() {
                self.conn.execute(
                    "INSERT OR IGNORE INTO page_aliases (page_id, alias) VALUES (?1, ?2)",
                    params![page_id, alias],
                )?;
            }
        }
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
        self.conn.execute(
            "DELETE FROM page_links WHERE source_page_id = ?1",
            params![source_page_id],
        )?;
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

    /// Replace `source_page_id`'s outgoing links with edges to `target_ids`.
    /// Used for whiteboard page-cards, which carry page ids directly (unlike
    /// markdown `[[links]]`, which resolve by title via [`rebuild_page_links`]).
    pub fn set_page_links(&self, source_page_id: i64, target_ids: &[i64]) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM page_links WHERE source_page_id = ?1",
            params![source_page_id],
        )?;
        for &target in target_ids {
            if target != source_page_id {
                self.conn.execute(
                    "INSERT OR IGNORE INTO page_links (source_page_id, target_page_id) \
                     VALUES (?1, ?2)",
                    params![source_page_id, target],
                )?;
            }
        }
        Ok(())
    }

    /// Whether the page with `id` is a whiteboard (so callers can route it to the
    /// canvas viewer rather than the markdown editor).
    pub fn is_whiteboard(&self, id: i64) -> bool {
        self.conn
            .query_row("SELECT kind FROM pages WHERE id = ?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .map(|k| k == "whiteboard")
            .unwrap_or(false)
    }

    /// Whether `needle` appears in any page's content (all kinds — markdown
    /// pages and whiteboard scenes) or any whiteboard template — the
    /// reference check for the images GC. A plain substring match:
    /// conservative, so it may keep garbage but never drops a referenced
    /// file.
    pub fn content_references(&self, needle: &str) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pages WHERE instr(content, ?1) > 0) \
             OR EXISTS(SELECT 1 FROM whiteboard_templates WHERE instr(content, ?1) > 0)",
            params![needle],
            |r| r.get(0),
        )
    }

    /// Every journal-day page (id + title only), for the graph view's
    /// Journals toggle.
    pub fn list_journal_pages(&self) -> rusqlite::Result<Vec<Page>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, is_journal, journal_date FROM pages \
             WHERE is_journal = 1 ORDER BY journal_date DESC",
        )?;
        stmt.query_map([], |row| {
            Ok(Page {
                id: row.get(0)?,
                title: row.get(1)?,
                is_journal: row.get::<_, i64>(2)? != 0,
                journal_date: row.get(3)?,
                content: String::new(),
                created_at: None,
                updated_at: None,
            })
        })?
        .collect()
    }

    /// Every `page_links` edge, for the graph view.
    pub fn all_page_links(&self) -> rusqlite::Result<Vec<(i64, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source_page_id, target_page_id FROM page_links")?;
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect()
    }

    /// Pages that link to `page_id`, each with the linking line as a
    /// snippet — the "Linked References" list.
    pub fn backlinks(&self, page_id: i64) -> rusqlite::Result<Vec<Backlink>> {
        let Some(target) = self.get_page(page_id)? else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.title, p.content FROM page_links l \
             JOIN pages p ON p.id = l.source_page_id \
             WHERE l.target_page_id = ?1 AND p.id != ?1 \
             ORDER BY p.is_journal DESC, p.journal_date DESC, p.title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![page_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, title, content) = r?;
            out.push(Backlink {
                source_page_id: id,
                source_page_title: title,
                snippet: snippet(&content, &format!("[[{}]]", target.title)),
            });
        }
        Ok(out)
    }
}

impl Db {
    /// Full-text-ish search over page titles and content (substring,
    /// case-insensitive). Title matches sort first, then journals by
    /// date. Returns up to `limit` hits with a snippet around the match.
    /// The raw `(id, title, content)` of pages matching `query`, ordered title
    /// matches first, then journals newest-first. Exposes the content so the
    /// type-aware [`search`](crate::search) layer can extract the PDF / image
    /// files referenced on each matched page (and build page snippets).
    pub fn search_rows(
        &self,
        query: &str,
        limit: i64,
    ) -> rusqlite::Result<Vec<(i64, String, String)>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let like = format!("%{}%", escape_like(q));
        // The trigram index needs ≥3 chars; 1–2 char queries fall back to the (rare,
        // small) LIKE scan. Either way: title matches first, then journals by date,
        // capped to `limit`.
        let rows: Vec<(i64, String, String)> = if q.chars().count() >= 3 {
            // Case-insensitive substring via the trigram FTS index. The query is a
            // quoted phrase (inner quotes doubled) so punctuation can't break MATCH.
            let fts = format!("\"{}\"", q.replace('"', "\"\""));
            let mut stmt = self.conn.prepare(
                "SELECT p.id, p.title, p.content \
                 FROM pages_fts f JOIN pages p ON p.id = f.rowid \
                 WHERE pages_fts MATCH ?1 \
                 ORDER BY (CASE WHEN p.title LIKE ?2 ESCAPE '\\' THEN 0 ELSE 1 END), \
                          p.is_journal DESC, p.journal_date DESC, p.title COLLATE NOCASE \
                 LIMIT ?3",
            )?;
            stmt.query_map(params![fts, like, limit], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, title, content FROM pages \
                 WHERE title LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\' \
                 ORDER BY (CASE WHEN title LIKE ?1 ESCAPE '\\' THEN 0 ELSE 1 END), \
                          is_journal DESC, journal_date DESC, title COLLATE NOCASE \
                 LIMIT ?2",
            )?;
            stmt.query_map(params![like, limit], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        Ok(rows)
    }

    /// The id of a page whose content references `needle` (e.g. an image path) —
    /// used to jump to the page that shows a file found in search. First match by
    /// the usual ordering (journals newest-first, then title).
    pub fn page_referencing(&self, needle: &str) -> rusqlite::Result<Option<i64>> {
        let like = format!("%{}%", escape_like(needle));
        self.conn
            .query_row(
                "SELECT id FROM pages WHERE content LIKE ?1 ESCAPE '\\' \
                 ORDER BY is_journal DESC, journal_date DESC, title COLLATE NOCASE LIMIT 1",
                params![like],
                |r| r.get::<_, i64>(0),
            )
            .optional()
    }
}

/// Escape SQL LIKE wildcards so the query is matched literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// The first content line containing `needle` (case-insensitive), else
/// the first non-empty line, trimmed and length-capped.
pub(crate) fn snippet(content: &str, needle: &str) -> String {
    let needle = needle.to_lowercase();
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
        created_at: None,
        updated_at: None,
    })
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
        let blocks: Vec<V1Block> = {
            let mut stmt = conn.prepare(
                "SELECT id, parent_id, position, content FROM blocks WHERE page_id = ?1",
            )?;
            let rows = stmt.query_map(params![pid], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let md = blocks_to_markdown(&blocks);
        conn.execute(
            "UPDATE pages SET content = ?2 WHERE id = ?1",
            params![pid, md],
        )?;
    }

    // Re-derive links from the new content.
    for pid in &page_ids {
        let content: String = conn.query_row(
            "SELECT content FROM pages WHERE id = ?1",
            params![pid],
            |r| r.get(0),
        )?;
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

    conn.execute_batch(
        "DROP TABLE IF EXISTS links; DROP TABLE IF EXISTS blocks; PRAGMA user_version = 2;",
    )?;
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
    conn.execute(
        "INSERT INTO pages (title, is_journal, content) VALUES (?1, 0, '')",
        params![title],
    )?;
    Ok(conn.last_insert_rowid())
}

/// A v1 outliner block row: `(id, parent_id, sort_order, text)`.
type V1Block = (i64, Option<i64>, i64, String);

/// Build a markdown bullet list from v1 blocks, indenting by tree depth.
fn blocks_to_markdown(blocks: &[V1Block]) -> String {
    let mut children: HashMap<Option<i64>, Vec<&V1Block>> = HashMap::new();
    for b in blocks {
        children.entry(b.1).or_default().push(b);
    }
    for kids in children.values_mut() {
        kids.sort_by_key(|b| b.2);
    }

    fn walk(
        parent: Option<i64>,
        depth: usize,
        children: &HashMap<Option<i64>, Vec<&V1Block>>,
        lines: &mut Vec<String>,
    ) {
        let Some(kids) = children.get(&parent) else {
            return;
        };
        for b in kids {
            lines.push(format!("{}- {}", "  ".repeat(depth), b.3));
            walk(Some(b.0), depth + 1, children, lines);
        }
    }

    let mut lines = Vec::new();
    walk(None, 0, &children, &mut lines);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts5_trigram_available_and_substring_case_insensitive() {
        // Gate: the bundled SQLite must ship FTS5 + the trigram tokenizer, which is
        // what the search index relies on for case-insensitive *substring* matching.
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE VIRTUAL TABLE t USING fts5(body, tokenize='trigram');\
             INSERT INTO t(rowid, body) VALUES (1, 'The oscillation circuit');",
        )
        .expect("bundled SQLite should have FTS5 + trigram");
        // Mid-word substring (LIKE-equivalent) matches.
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM t WHERE t MATCH '\"scill\"'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "trigram matches a mid-word substring");
        // ...and case-insensitively.
        let n2: i64 = c
            .query_row(
                "SELECT count(*) FROM t WHERE t MATCH '\"OSCILL\"'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n2, 1, "trigram is case-insensitive");
    }

    #[test]
    fn encryption_round_trip() {
        let dir = std::env::temp_dir().join(format!("zorite-enc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.db");
        let _ = std::fs::remove_file(&path);

        // Plaintext: create, write, and confirm the header magic is visible.
        let mut db = Db::open_file(&path, None).unwrap();
        let page = db.get_or_create_page("Secret").unwrap();
        db.set_page_content(page.id, "classified").unwrap();
        assert!(!file_is_encrypted(&path));

        // Encrypt in place: header becomes opaque, data survives, and the
        // connection keeps working.
        db.set_encryption(Some("hunter2")).unwrap();
        assert!(file_is_encrypted(&path));
        assert_eq!(db.get_page(page.id).unwrap().unwrap().content, "classified");
        drop(db);

        // No key / wrong key: unreadable. Right key: readable.
        assert!(Db::open_file(&path, None).is_err());
        assert!(Db::open_file(&path, Some("wrong")).is_err());
        assert!(Db::verify_key_at(&path, "hunter2"));
        assert!(!Db::verify_key_at(&path, "wrong"));
        let mut db = Db::open_file(&path, Some("hunter2")).unwrap();
        assert_eq!(db.get_page(page.id).unwrap().unwrap().content, "classified");

        // Change the password, then remove it: plaintext again.
        db.set_encryption(Some("correct horse")).unwrap();
        drop(db);
        assert!(Db::open_file(&path, Some("hunter2")).is_err());
        let mut db = Db::open_file(&path, Some("correct horse")).unwrap();
        db.set_encryption(None).unwrap();
        assert!(!file_is_encrypted(&path));
        assert_eq!(db.get_page(page.id).unwrap().unwrap().content, "classified");
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_cascades_to_namespace_children() {
        let db = Db::open_in_memory().unwrap();
        let parent = db.get_or_create_page("Projects").unwrap();
        let child = db.get_or_create_page("Projects::Tasks").unwrap();
        let deep = db.get_or_create_page("Projects::Tasks::Old").unwrap();
        let other = db.get_or_create_page("Notes").unwrap();
        db.set_page_content(other.id, "see [[Projects]] and [[Projects::Tasks]]")
            .unwrap();
        assert!(db.rename_page(parent.id, "Work").unwrap());
        assert_eq!(db.get_page(child.id).unwrap().unwrap().title, "Work::Tasks");
        assert_eq!(
            db.get_page(deep.id).unwrap().unwrap().title,
            "Work::Tasks::Old"
        );
        assert_eq!(
            db.get_page(other.id).unwrap().unwrap().content,
            "see [[Work]] and [[Work::Tasks]]"
        );
    }

    #[test]
    fn rename_aborts_when_a_child_would_collide() {
        let db = Db::open_in_memory().unwrap();
        let parent = db.get_or_create_page("Foo").unwrap();
        let child = db.get_or_create_page("Foo::A").unwrap();
        db.get_or_create_page("Bar::A").unwrap();
        // "Bar" itself is free, but Foo::A -> Bar::A collides: nothing moves.
        assert!(!db.rename_page(parent.id, "Bar").unwrap());
        assert_eq!(db.get_page(parent.id).unwrap().unwrap().title, "Foo");
        assert_eq!(db.get_page(child.id).unwrap().unwrap().title, "Foo::A");
    }

    #[test]
    fn content_references_scans_pages_and_templates() {
        let db = Db::open_in_memory().unwrap();
        let page = db.get_or_create_page("Notes").unwrap();
        db.set_page_content(page.id, "text ![](images/photo-1.png) more")
            .unwrap();
        db.create_template("Grid", r#"[{"src":"images/board-bg.webp"}]"#)
            .unwrap();
        // Markdown reference, whiteboard-template reference, and a miss.
        assert!(db.content_references("photo-1.png").unwrap());
        assert!(db.content_references("board-bg.webp").unwrap());
        assert!(!db.content_references("unused.png").unwrap());
    }

    #[test]
    fn whiteboard_templates_crud() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.list_templates().unwrap().is_empty());
        let a = db.create_template("Grid", "[]").unwrap();
        let _b = db.create_template("Flow", "[{\"id\":1}]").unwrap();
        let all = db.list_templates().unwrap();
        assert_eq!(all.len(), 2);
        // (id, name, content) round-trips.
        assert!(
            all.iter()
                .any(|(_, n, c)| n == "Flow" && c == "[{\"id\":1}]")
        );
        db.delete_template(a).unwrap();
        let after = db.list_templates().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].1, "Flow");
    }

    #[test]
    fn whiteboard_cards_link_and_show_as_backlinks() {
        let db = Db::open_in_memory().unwrap();
        let board = db.create_whiteboard().unwrap();
        let page = db.get_or_create_page("Target").unwrap();
        assert!(db.is_whiteboard(board.id));
        assert!(!db.is_whiteboard(page.id));
        // A page-card on the board links board → page; the page's backlinks show it.
        db.set_page_links(board.id, &[page.id]).unwrap();
        let bl = db.backlinks(page.id).unwrap();
        assert!(
            bl.iter().any(|b| b.source_page_id == board.id),
            "the board should appear in the page's backlinks"
        );
        // Removing the card (re-save with no targets) clears the link.
        db.set_page_links(board.id, &[]).unwrap();
        assert!(db.backlinks(page.id).unwrap().is_empty());
    }

    #[test]
    fn whiteboards_are_distinct_listed_and_kept_out_of_the_page_tree_and_search() {
        let db = Db::open_in_memory().unwrap();
        // Each create makes a brand-new board with an auto, unique title and an
        // empty-scene placeholder in its content column.
        let a = db.create_whiteboard().unwrap();
        assert_eq!(a.content, "{}");
        let b = db.create_whiteboard().unwrap();
        assert_ne!(a.id, b.id, "each board is distinct");
        assert_ne!(a.title, b.title, "auto-titles are deduped");
        // Boards show in the Whiteboards section, not the page tree/sidebar tree.
        let wb = db.list_whiteboards().unwrap();
        assert!(
            wb.iter().any(|p| p.id == a.id) && wb.iter().any(|p| p.id == b.id),
            "both boards should be listed"
        );
        assert!(
            db.list_pages().unwrap().iter().all(|p| p.id != a.id),
            "whiteboard leaked into the page list"
        );
        // Its canvas JSON stays out of FTS (the kind-guarded triggers), while a
        // normal page's content is still indexed — the v6 control. (`pages_fts`
        // is external-content, so probe via MATCH, not a rowid lookup.)
        let hits = |term: &str| -> i64 {
            db.conn
                .query_row(
                    "SELECT count(*) FROM pages_fts WHERE pages_fts MATCH ?1",
                    params![format!("\"{term}\"")],
                    |r| r.get(0),
                )
                .unwrap()
        };
        db.set_page_content(a.id, r#"{"marker":"zphirium","camera":{}}"#)
            .unwrap();
        assert_eq!(
            hits("zphirium"),
            0,
            "whiteboard content must not be searchable"
        );

        let p = db.get_or_create_page("Notes").unwrap();
        db.set_page_content(p.id, "hello zphirium world").unwrap();
        assert_eq!(hits("zphirium"), 1, "a normal page should be searchable");
    }

    #[test]
    fn search_substring_case_insensitive_and_stays_synced() {
        let db = Db::open_in_memory().unwrap();
        let p = db.get_or_create_page("Datasheet").unwrap();
        db.set_page_content(p.id, "The voltage on VCC is insufficient for oscillation")
            .unwrap();

        // Trigram FTS (≥3 chars): mid-word + case-insensitive substring, and title.
        let has = |q: &str| {
            db.search_rows(q, 10)
                .unwrap()
                .iter()
                .any(|(id, _, _)| *id == p.id)
        };
        assert!(has("scill"), "mid-word substring");
        assert!(has("vcc"), "case-insensitive");
        assert!(has("datash"), "title match");

        // Editing the page re-syncs the index (UPDATE trigger).
        db.set_page_content(p.id, "now about resistors").unwrap();
        assert!(db.search_rows("oscillation", 10).unwrap().is_empty());
        assert!(has("resistor"));
        // 1–2 char queries fall back to LIKE (below the trigram minimum).
        assert!(has("re"));

        // Deleting the page drops it from the index (DELETE trigger).
        assert!(db.delete_page(p.id).unwrap());
        assert!(db.search_rows("resistor", 10).unwrap().is_empty());
    }

    #[test]
    fn v5_migration_indexes_preexisting_pages() {
        // The upgrade path: a pre-FTS (v4) DB with existing pages should get them
        // indexed when the v5 migration runs (not just fresh installs).
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(SCHEMA_V2).unwrap();
        conn.execute_batch("PRAGMA user_version = 4;").unwrap();
        conn.execute(
            "INSERT INTO pages (title, content) VALUES ('Old Page', 'pre-existing oscillation text')",
            [],
        )
        .unwrap();
        Db::migrate(&mut conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let db = Db { conn };
        assert!(
            db.search_rows("oscillation", 10)
                .unwrap()
                .iter()
                .any(|(_, title, _)| title == "Old Page"),
            "existing pages should be searchable after the FTS migration"
        );
    }

    #[test]
    fn backup_before_migration_snapshots_pending_upgrade() {
        // A pre-v5 on-disk DB gets a `.bak-v<N>` snapshot before it's migrated.
        let dir = std::env::temp_dir().join("zorite-test-bak-pending");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zorite.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V2).unwrap();
            conn.execute(
                "INSERT INTO pages (title, content) VALUES ('Keep', 'precious')",
                [],
            )
            .unwrap();
            conn.execute_batch("PRAGMA user_version = 4;").unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        let bak =
            Db::backup_before_migration(&conn, &path).expect("a backup for a pending upgrade");
        assert!(bak.exists());
        assert!(bak.to_string_lossy().ends_with(".bak-v4"));
        // The snapshot is a usable copy with the page intact.
        let bconn = Connection::open(&bak).unwrap();
        let n: i64 = bconn
            .query_row("SELECT COUNT(*) FROM pages WHERE title = 'Keep'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_skipped_when_fresh_or_current() {
        let dir = std::env::temp_dir().join("zorite-test-bak-skip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zorite.db");
        // Fresh file (user_version 0) — nothing to lose yet.
        {
            let conn = Connection::open(&path).unwrap();
            assert!(Db::backup_before_migration(&conn, &path).is_none());
        }
        // Already at the current version — no upgrade pending.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V2).unwrap();
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))
                .unwrap();
            assert!(Db::backup_before_migration(&conn, &path).is_none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_sequence_backs_up_then_migrates_wal_db() {
        // Mirror Db::open's real sequence on a WAL database: a v4 file with a page
        // gets snapshotted and then migrated to the current version, data intact.
        let dir = std::env::temp_dir().join("zorite-test-open-seq");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zorite.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
                .unwrap();
            conn.execute_batch(SCHEMA_V2).unwrap();
            conn.execute(
                "INSERT INTO pages (title, content) VALUES ('Note', 'resonant cavity')",
                [],
            )
            .unwrap();
            conn.execute_batch("PRAGMA user_version = 4;").unwrap();
        }
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
        let bak = Db::backup_before_migration(&conn, &path).expect("snapshot before upgrade");
        assert!(bak.exists() && bak.to_string_lossy().ends_with(".bak-v4"));
        Db::migrate(&mut conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        // Migrated DB is usable; the backup retains the pre-upgrade copy.
        let db = Db { conn };
        assert!(
            db.search_rows("resonant", 10)
                .unwrap()
                .iter()
                .any(|(_, title, _)| title == "Note")
        );
        let bconn = Connection::open(&bak).unwrap();
        let bver: i64 = bconn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(bver, 4, "backup retains the pre-migration schema version");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_rewrites_links_and_guards() {
        let db = Db::open_in_memory().unwrap();
        let foo = db.get_or_create_page("Foo").unwrap();
        let other = db.get_or_create_page("Other").unwrap();
        db.set_page_content(other.id, "see [[Foo]] and more")
            .unwrap();

        // Rename Foo -> Bar: title changes and the link is rewritten.
        assert!(db.rename_page(foo.id, "Bar").unwrap());
        assert_eq!(db.get_page(foo.id).unwrap().unwrap().title, "Bar");
        assert_eq!(
            db.get_page(other.id).unwrap().unwrap().content,
            "see [[Bar]] and more"
        );

        // Guards: a name taken by another page, an empty name, and journals.
        assert!(!db.rename_page(foo.id, "Other").unwrap());
        assert!(!db.rename_page(foo.id, "   ").unwrap());
        let journal = db.get_or_create_journal("2099-01-01").unwrap();
        assert!(!db.rename_page(journal.id, "Nope").unwrap());
    }
}
