//! Where Zorite keeps its data. One SQLite file under the platform's
//! per-user application-data directory, created on first run — or a
//! user-chosen directory recorded in a small pointer file (see
//! [`set_location`]) that stays put even after the data moves elsewhere.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use url::Url;

/// The OS-default data directory, ignoring any user override. Platform
/// conventions:
/// - macOS:   `~/Library/Application Support/zorite`
/// - Windows: `%APPDATA%\zorite`
/// - Linux:   `$XDG_DATA_HOME/zorite` or `~/.local/share/zorite`
///
/// Also the fixed home of the location-pointer file, so the pointer stays put
/// even after the data it points to moves. Falls back to the current directory
/// only if the relevant home / env var is somehow unset.
fn default_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join("Library")
            .join("Application Support")
            .join("zorite")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("zorite")
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
            .join("zorite")
    }
}

/// Resolve the active data directory (no side effects). Precedence:
/// 1. `ZORITE_DATA` — a full override for throwaway/dev data sets.
/// 2. The user-chosen directory from the location-pointer file.
/// 3. The OS default ([`default_data_dir`]).
fn resolve_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ZORITE_DATA") {
        return PathBuf::from(dir);
    }
    if let Some(p) = read_pointer() {
        let dir = p.dir.trim();
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    default_data_dir()
}

/// The directory holding `zorite.db` and the managed `images/`, `pdf/`,
/// `themes/`, and `fonts/` folders. Resolved once per process — a change made
/// via [`set_location`] takes effect on the next launch, which is also when any
/// pending move runs (before the database is opened).
pub fn data_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(resolve_data_dir).clone()
}

// --- Window-bounds persistence (Settings → General toggle) ---
//
// A tiny sidecar file, NOT the settings table: the bounds are needed when
// the main window opens, which is before an encrypted database unlocks.
// File presence doubles as the on/off state.

fn window_bounds_file() -> PathBuf {
    data_dir().join("window-bounds")
}

pub fn window_bounds_enabled() -> bool {
    window_bounds_file().exists()
}

/// The saved `(x, y, w, h, maximized)`, if persistence is on and the file
/// parses.
pub fn saved_window_bounds() -> Option<(f32, f32, f32, f32, bool)> {
    let text = std::fs::read_to_string(window_bounds_file()).ok()?;
    let mut it = text.split_whitespace();
    let mut next = || it.next()?.parse::<f32>().ok();
    let (x, y, w, h) = (next()?, next()?, next()?, next()?);
    let maximized = it.next() == Some("m");
    (w > 50.0 && h > 50.0).then_some((x, y, w, h, maximized))
}

pub fn save_window_bounds(x: f32, y: f32, w: f32, h: f32, maximized: bool) {
    let state = if maximized { "m" } else { "w" };
    let _ = std::fs::write(window_bounds_file(), format!("{x} {y} {w} {h} {state}"));
}

pub fn clear_window_bounds() {
    let _ = std::fs::remove_file(window_bounds_file());
}

// --- Open-tabs persistence (the second switch on the same Settings card) ---
//
// Same sidecar pattern as window-bounds: file presence is the on/off state.
// Holds the main window's tab list (`active N` + one `page/pdf/whiteboard/
// allpages/graph/properties` line per tab, journal excluded) — written by
// `AppView::persist_open_tabs` whenever the set changes, read once at startup.

fn open_tabs_file() -> PathBuf {
    data_dir().join("open-tabs")
}

pub fn open_tabs_enabled() -> bool {
    open_tabs_file().exists()
}

pub fn save_open_tabs(serialized: &str) {
    let _ = std::fs::write(open_tabs_file(), serialized);
}

pub fn load_open_tabs() -> Option<String> {
    std::fs::read_to_string(open_tabs_file()).ok()
}

pub fn clear_open_tabs() {
    let _ = std::fs::remove_file(open_tabs_file());
}

/// The user's Desktop directory — the default location a "save as" dialog opens at (e.g. for
/// exporting a formula). Platform conventions:
/// - macOS:   `~/Desktop`
/// - Windows: `%USERPROFILE%\Desktop`
/// - Linux:   `$XDG_DESKTOP_DIR`, else `~/Desktop`
///
/// Falls back to the current directory if the home / env var is unset (the dialog then opens at
/// its own default rather than failing).
pub fn desktop_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Desktop"))
            .unwrap_or_else(|| PathBuf::from("."))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .map(|h| PathBuf::from(h).join("Desktop"))
            .unwrap_or_else(|| PathBuf::from("."))
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_DESKTOP_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Desktop")))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Absolute path to the SQLite database file. `ZORITE_DB` overrides it — handy
/// for running against a throwaway database (tests, benchmarks) without
/// touching the real one.
pub fn db_path() -> PathBuf {
    if let Some(path) = std::env::var_os("ZORITE_DB") {
        return PathBuf::from(path);
    }
    data_dir().join("zorite.db")
}

// --- User-configurable data location -----------------------------------------
// The chosen directory is recorded in a small JSON pointer file kept in the OS
// default dir (so it never moves with the data). Because the data dir resolves
// once at startup, a change takes effect on the next launch — which also lets a
// pending move run before the database is opened, sidestepping the open-file
// locks that make moving the live database unsafe (notably on Windows).

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct LocationPointer {
    /// Absolute path of the chosen data directory.
    dir: String,
    /// When non-empty, a directory whose data the next startup moves into `dir`
    /// before opening the database; cleared once the move completes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    migrate_from: String,
    /// Registered notebooks — every data directory the user has created or
    /// added, including the active one. Empty until a second notebook exists
    /// (or one is renamed), so a plain single-data-set install keeps the old
    /// pointer shape; old builds ignore the field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notebooks: Vec<Notebook>,
}

/// One registered notebook: a self-contained data directory (database +
/// assets) the user can switch to. See [`notebooks`].
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Debug)]
pub struct Notebook {
    pub name: String,
    /// Absolute path of the notebook's data directory.
    pub dir: String,
}

impl Notebook {
    pub fn is_active(&self) -> bool {
        Path::new(&self.dir) == data_dir()
    }
}

/// The pointer file's fixed home — the OS default dir, so it survives the data
/// being relocated elsewhere.
fn pointer_path() -> PathBuf {
    default_data_dir().join("data_location.json")
}

fn read_pointer() -> Option<LocationPointer> {
    let s = std::fs::read_to_string(pointer_path()).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_pointer(p: &LocationPointer) -> std::io::Result<()> {
    let path = pointer_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(p).unwrap_or_default())
}

/// The OS-default data directory (where "reset to default" sends the data).
pub fn default_location() -> PathBuf {
    default_data_dir()
}

/// Whether the active data dir is the OS default (no user relocation in effect).
pub fn is_default_location() -> bool {
    data_dir() == default_data_dir()
}

/// What pointing the data location at `target` would do — drives the confirm
/// step before anything is written.
pub enum Relocation {
    /// `target` is already the active directory.
    NoOp,
    /// `target` can't be used; the string explains why.
    Invalid(String),
    /// `target` already holds a database; Zorite would switch to it in place.
    Switch,
    /// `target` has no database; Zorite would move the current data into it.
    Move,
}

/// Decide (without changing anything) what pointing at `target` would do.
pub fn plan_relocation(target: &Path) -> Relocation {
    let current = data_dir();
    if target == current {
        return Relocation::NoOp;
    }
    if !target.is_dir() {
        return Relocation::Invalid("That isn't a folder.".to_string());
    }
    if target.starts_with(&current) || current.starts_with(target) {
        return Relocation::Invalid(
            "Pick a folder that's neither inside nor the parent of the current data folder."
                .to_string(),
        );
    }
    if !is_writable(target) {
        return Relocation::Invalid("That folder isn't writable.".to_string());
    }
    if target.join("zorite.db").exists() {
        Relocation::Switch
    } else {
        Relocation::Move
    }
}

/// Record `target` as the data directory. If it has no database yet, also
/// schedules a move of the current data into it on the next startup. Takes
/// effect after the app restarts.
pub fn set_location(target: &Path) -> std::io::Result<()> {
    let current = data_dir();
    update_pointer(|p| {
        p.dir = target.to_string_lossy().into_owned();
        p.migrate_from.clear();
        if !target.join("zorite.db").exists() {
            p.migrate_from = current.to_string_lossy().into_owned();
            // A move relocates the active notebook — its entry follows the data.
            for n in &mut p.notebooks {
                if Path::new(&n.dir) == current {
                    n.dir = p.dir.clone();
                }
            }
        }
    })
}

/// Send the data back to the OS-default location (moving it there on restart).
pub fn reset_location() -> std::io::Result<()> {
    set_location(&default_data_dir())
}

// --- Notebooks: registered data directories the user switches between --------
// The registry lives in the location-pointer file; `dir` stays the single
// source of truth for what's active, so switching notebooks is rewriting
// `dir` and relaunching. `ZORITE_DATA` outranks all of it (resolve_data_dir).

/// Read-modify-write the pointer file, preserving whatever fields the closure
/// doesn't touch. An absent/blank `dir` is seeded with the OS default so a
/// registry write never accidentally repoints the data.
fn update_pointer(f: impl FnOnce(&mut LocationPointer)) -> std::io::Result<()> {
    let mut p = read_pointer().unwrap_or_default();
    if p.dir.trim().is_empty() {
        p.dir = default_data_dir().to_string_lossy().into_owned();
    }
    f(&mut p);
    write_pointer(&p)
}

/// Registered notebooks, with the active data directory always present. When
/// the registry doesn't know it yet (first launch after the update, or a
/// pointer written by an older build) the active entry is synthesized — named
/// "Main", or by its folder when other notebooks exist — and persisted on the
/// first registry mutation.
pub fn notebooks() -> Vec<Notebook> {
    let mut list = read_pointer().map(|p| p.notebooks).unwrap_or_default();
    if !list.iter().any(Notebook::is_active) {
        let active = data_dir();
        let name = saved_notebook_name(&active).unwrap_or_else(|| {
            if list.is_empty() {
                "Main".to_string()
            } else {
                active
                    .file_name()
                    .map_or_else(|| "Main".to_string(), |n| n.to_string_lossy().into_owned())
            }
        });
        list.insert(
            0,
            Notebook {
                name,
                dir: active.to_string_lossy().into_owned(),
            },
        );
    }
    list
}

/// The active notebook's name when more than one is registered — the switcher
/// chip label and the window-title suffix stay quiet for a single data set.
pub fn active_notebook_name() -> Option<String> {
    let list = notebooks();
    (list.len() > 1)
        .then(|| list.into_iter().find(Notebook::is_active))
        .flatten()
        .map(|n| n.name)
}

/// The note-window title: "Zorite", gaining the notebook's name when more
/// than one is registered.
pub fn window_title() -> String {
    match active_notebook_name() {
        Some(name) => format!("Zorite — {name}"),
        None => "Zorite".to_string(),
    }
}

/// Validate and register a picked folder as a notebook: rejects nesting with
/// the current data dir (one notebook's move/sweep could eat another), names
/// it from its `notebook-name` sidecar (a previously renamed notebook being
/// re-added) or its folder name. Shared by the sidebar switcher and Settings.
pub fn register_dir(dir: &Path) -> Result<Notebook, String> {
    let current = data_dir();
    if *dir != current && (dir.starts_with(&current) || current.starts_with(dir)) {
        return Err(
            "Pick a folder that's neither inside nor the parent of the current data folder."
                .to_string(),
        );
    }
    let name = saved_notebook_name(dir).unwrap_or_else(|| {
        dir.file_name().map_or_else(
            || "Notebook".to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    });
    add_notebook(&name, dir).map_err(|e| e.to_string())?;
    Ok(Notebook {
        name,
        dir: dir.to_string_lossy().into_owned(),
    })
}

/// Register `dir` as a notebook named `name` (no-op when already registered).
/// The first mutation also persists the synthesized active entry, so the
/// registry is complete from then on.
pub fn add_notebook(name: &str, dir: &Path) -> std::io::Result<()> {
    let all = notebooks();
    update_pointer(move |p| {
        p.notebooks = all;
        if !p.notebooks.iter().any(|n| Path::new(&n.dir) == dir) {
            p.notebooks.push(Notebook {
                name: name.to_string(),
                dir: dir.to_string_lossy().into_owned(),
            });
        }
    })
}

/// A notebook's display name, persisted as a tiny sidecar *inside* its data
/// dir — so a custom name survives remove/re-add and travels when the folder
/// is shared or moved. The registry stays the display source of truth; this
/// file only seeds it. (Renaming never touches the folder itself: the active
/// notebook's directory is a live, open database.)
fn notebook_name_file(dir: &Path) -> PathBuf {
    dir.join("notebook-name")
}

/// The name saved inside a notebook dir, if any.
pub fn saved_notebook_name(dir: &Path) -> Option<String> {
    let s = std::fs::read_to_string(notebook_name_file(dir)).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

pub fn rename_notebook(dir: &str, new_name: &str) -> std::io::Result<()> {
    // Best-effort: the name rides inside the notebook so re-adding finds it.
    let _ = std::fs::write(notebook_name_file(Path::new(dir)), new_name);
    let all = notebooks();
    update_pointer(move |p| {
        p.notebooks = all;
        for n in &mut p.notebooks {
            if n.dir == dir {
                n.name = new_name.to_string();
            }
        }
    })
}

/// Drop a notebook from the registry. Its files are never touched.
pub fn forget_notebook(dir: &str) -> std::io::Result<()> {
    update_pointer(|p| p.notebooks.retain(|n| n.dir != dir))
}

/// Point the next launch at `dir`. Unlike [`set_location`] this never
/// schedules a move: an empty notebook directory boots a fresh database, a
/// populated one opens in place. The caller relaunches the app.
pub fn switch_notebook(dir: &str) -> std::io::Result<()> {
    update_pointer(|p| {
        p.dir = dir.to_string();
        p.migrate_from.clear();
    })
}

/// Shared progress for a startup data move: total bytes to move, bytes done so
/// far, and whether it has finished. Written by the move thread, read by the
/// progress window.
pub struct MigrationProgress {
    total: u64,
    done: AtomicU64,
    finished: AtomicBool,
}

impl MigrationProgress {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            done: AtomicU64::new(0),
            finished: AtomicBool::new(false),
        }
    }

    /// Completion fraction in `0.0..=1.0` (1.0 when there's nothing to copy —
    /// e.g. an instant same-volume rename).
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            (self.done.load(Ordering::Relaxed) as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

/// If a startup data move is scheduled (see [`set_location`]), return its
/// `(source, target, total_bytes)`. Trivial cases — nothing to move, or the
/// target already populated — are resolved here by clearing the flag and
/// returning `None`. Skipped under `ZORITE_DATA`. Must run before the database
/// is opened (the move can't safely touch an open database, notably on Windows).
pub fn pending_migration() -> Option<(PathBuf, PathBuf, u64)> {
    if std::env::var_os("ZORITE_DATA").is_some() {
        return None;
    }
    let mut p = read_pointer()?;
    if p.migrate_from.trim().is_empty() {
        return None;
    }
    let target = PathBuf::from(p.dir.trim());
    let source = PathBuf::from(p.migrate_from.trim());
    // Already populated, or nothing at the source → clear the flag, no move.
    if target.join("zorite.db").exists() || !source.join("zorite.db").exists() {
        p.migrate_from.clear();
        let _ = write_pointer(&p);
        return None;
    }
    let total = move_total_bytes(&source);
    Some((source, target, total))
}

/// Perform a scheduled move (from [`pending_migration`]), reporting byte
/// progress through `progress`, then settle the pointer: cleared on success,
/// reverted to the source on failure — so a launch never opens an empty target
/// while the data still sits at the source. Marks `progress` finished when done.
pub fn run_migration(source: &Path, target: &Path, progress: &MigrationProgress) {
    match relocate(source, target, &progress.done) {
        Ok(()) => settle_pointer(target),
        Err(e) => {
            log::error!("data move {source:?} -> {target:?} failed: {e}; keeping data in place");
            settle_pointer(source);
        }
    }
    progress.finished.store(true, Ordering::Release);
}

/// After a move settles, record `dir` as the location with the move flag
/// cleared. A pointer at the OS default with no notebook registry is
/// redundant and is removed instead — but a registry always keeps the file.
fn settle_pointer(dir: &Path) {
    let Some(mut p) = read_pointer() else { return };
    if dir == default_data_dir().as_path() && p.notebooks.is_empty() {
        let _ = std::fs::remove_file(pointer_path());
        return;
    }
    p.dir = dir.to_string_lossy().into_owned();
    p.migrate_from.clear();
    let _ = write_pointer(&p);
}

/// Total size of the entries a move would carry — the `zorite.db*` files plus
/// the asset folders — used to scale the progress bar.
fn move_total_bytes(source: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(source) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("zorite.db") {
                total += entry_size(&e.path());
            }
        }
    }
    for dir in ["images", "pdf", "themes", "fonts"] {
        let p = source.join(dir);
        if p.exists() {
            total += dir_size(&p);
        }
    }
    total
}

/// Move Zorite's managed data from `source` into `target`, rename-first with a
/// cross-filesystem copy+remove fallback, adding copied bytes to `done`. Moves
/// every `zorite.db*` file (the database, its `-wal`/`-shm` sidecars, the
/// rollback journal, and any migration `.bak-*` backups), the `notebook-name`
/// sidecar, plus the `images/`, `pdf/`, `themes/`, `fonts/` asset folders.
/// Only those entries move, so unrelated files (and the pointer in the
/// default dir) stay put.
fn relocate(source: &Path, target: &Path, done: &AtomicU64) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        // The db + sidecars, and the notebook's display-name sidecar.
        if name.to_string_lossy().starts_with("zorite.db")
            || name == "notebook-name"
            || name == "open-tabs"
        {
            move_path(&entry.path(), &target.join(&name), done)?;
        }
    }
    for dir in ["images", "pdf", "themes", "fonts"] {
        let from = source.join(dir);
        if from.exists() {
            move_path(&from, &target.join(dir), done)?;
        }
    }
    Ok(())
}

/// Move one file or directory: try a rename (instant, credited whole), falling
/// back to a byte-counted copy-then-remove when the rename crosses filesystems
/// (`EXDEV`) or otherwise fails.
fn move_path(from: &Path, to: &Path, done: &AtomicU64) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        done.fetch_add(entry_size(to), Ordering::Relaxed);
        return Ok(());
    }
    if from.is_dir() {
        copy_dir_all(from, to, done)?;
        std::fs::remove_dir_all(from)
    } else {
        copy_file(from, to, done)?;
        std::fs::remove_file(from)
    }
}

/// Recursively copy a directory tree, adding copied bytes to `done`.
fn copy_dir_all(from: &Path, to: &Path, done: &AtomicU64) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dst, done)?;
        } else {
            copy_file(&entry.path(), &dst, done)?;
        }
    }
    Ok(())
}

/// Copy a single file in chunks, adding each chunk's byte count to `done` so the
/// bar advances within large files (the database in particular).
fn copy_file(from: &Path, to: &Path, done: &AtomicU64) -> std::io::Result<()> {
    use std::io::{Read as _, Write as _};
    let mut reader = std::fs::File::open(from)?;
    let mut writer = std::fs::File::create(to)?;
    let mut buf = vec![0u8; 1 << 20]; // 1 MiB
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    writer.flush()
}

/// Total bytes under a path (recursive); 0 for a missing path.
fn dir_size(p: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            total += if path.is_dir() {
                dir_size(&path)
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            };
        }
    }
    total
}

/// Size of one entry — a file's length, or a directory's recursive size.
fn entry_size(p: &Path) -> u64 {
    if p.is_dir() {
        dir_size(p)
    } else {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    }
}

/// Can we create (and remove) a file in `dir`? Rejects read-only targets.
fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".zorite-write-test");
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        true
    } else {
        false
    }
}

/// Directory for user-supplied JSON theme files.
pub fn themes_dir() -> PathBuf {
    data_dir().join("themes")
}

/// Directory for images pasted or dropped into notes. Markdown references them
/// relatively (`images/<name>`), resolved against [`data_dir`].
pub fn images_dir() -> PathBuf {
    data_dir().join("images")
}

/// Directory for PDFs dropped into notes. Markdown references them relatively
/// (`pdf/<name>`), resolved against [`data_dir`] by the PDF viewer.
pub fn pdf_dir() -> PathBuf {
    data_dir().join("pdf")
}

/// Directory for user-uploaded whiteboard fonts. A board's chosen face is stored
/// relatively (`fonts/<name>`), resolved against [`data_dir`].
pub fn fonts_dir() -> PathBuf {
    data_dir().join("fonts")
}

/// Resolve a markdown image/file `src` to a local filesystem path, cross-platform.
///
/// - `http(s)://` → `None` (remote, not a local file).
/// - `file://…` → the referenced path, via [`Url`] so Windows `file:///C:/…` and
///   percent-encoded names (`%20`) decode correctly.
/// - an absolute path (`/x`, `C:\x`, `\\unc\…`) → used as-is. Absoluteness is
///   decided by [`Path::is_absolute`], which is platform-correct (so a Windows
///   drive path isn't mistaken for a relative one, as `starts_with('/')` would).
/// - anything else → treated as relative to the [`data_dir`] (where the managed
///   `images/` and `pdf/` folders live); the stored refs use `/` separators,
///   which Windows accepts.
///
/// Existence is *not* checked — callers decide what to do with a missing file.
pub fn resolve_local(src: &str) -> Option<PathBuf> {
    let src = src.trim();
    if src.starts_with("http://") || src.starts_with("https://") {
        return None;
    }
    if src.starts_with("file://") {
        return Url::parse(src).ok().and_then(|u| u.to_file_path().ok());
    }
    let path = Path::new(src);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        data_dir().join(src)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_file_is_compatible_both_ways() {
        // A pointer written by a pre-notebooks build reads cleanly (empty
        // registry)…
        let old: LocationPointer = serde_json::from_str(r#"{"dir": "/x/y"}"#).unwrap();
        assert_eq!(old.dir, "/x/y");
        assert!(old.notebooks.is_empty());
        // …and one we write without a registry keeps the old shape, so a
        // downgrade reads it too.
        assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"dir":"/x/y"}"#);
        // The registry round-trips.
        let new: LocationPointer = serde_json::from_str(
            r#"{"dir": "/a", "notebooks": [{"name": "Main", "dir": "/a"}, {"name": "Work", "dir": "/b"}]}"#,
        )
        .unwrap();
        assert_eq!(new.notebooks.len(), 2);
        assert_eq!(new.notebooks[1].name, "Work");
    }

    #[test]
    fn relative_resolves_under_data_dir() {
        let p = resolve_local("images/a.png").unwrap();
        assert!(p.starts_with(data_dir()));
        assert!(p.ends_with("images/a.png"));
    }

    #[test]
    fn remote_urls_are_not_local() {
        assert_eq!(resolve_local("https://example.com/a.png"), None);
        assert_eq!(resolve_local("http://example.com/a.png"), None);
    }

    #[test]
    fn relocate_moves_managed_entries_and_leaves_the_rest() {
        let tmp = std::env::temp_dir().join("zorite-test-relocate");
        let _ = std::fs::remove_dir_all(&tmp);
        let (src, dst) = (tmp.join("src"), tmp.join("dst"));
        std::fs::create_dir_all(src.join("images")).unwrap();
        std::fs::create_dir_all(src.join("pdf")).unwrap();
        std::fs::write(src.join("zorite.db"), b"db").unwrap();
        std::fs::write(src.join("zorite.db-wal"), b"wal").unwrap();
        std::fs::write(src.join("zorite.db.bak-v5"), b"bak").unwrap();
        std::fs::write(src.join("images/a.png"), b"img").unwrap();
        std::fs::write(src.join("pdf/b.pdf"), b"pdf").unwrap();
        std::fs::write(src.join("unrelated.txt"), b"keep").unwrap();

        let done = AtomicU64::new(0);
        relocate(&src, &dst, &done).unwrap();

        // Managed entries moved across — db, its sidecars, and its backup.
        assert_eq!(std::fs::read(dst.join("zorite.db")).unwrap(), b"db");
        assert_eq!(std::fs::read(dst.join("zorite.db-wal")).unwrap(), b"wal");
        assert_eq!(std::fs::read(dst.join("zorite.db.bak-v5")).unwrap(), b"bak");
        assert_eq!(std::fs::read(dst.join("images/a.png")).unwrap(), b"img");
        assert_eq!(std::fs::read(dst.join("pdf/b.pdf")).unwrap(), b"pdf");
        assert!(!src.join("zorite.db").exists());
        assert!(!src.join("zorite.db.bak-v5").exists());
        assert!(!src.join("images").exists());
        // Anything we don't manage stays put.
        assert!(src.join("unrelated.txt").exists());
        // Progress accounted for every managed byte (2+3+3 db files + 3 + 3 assets).
        assert_eq!(done.load(Ordering::Relaxed), 14);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn copy_dir_all_recurses_and_keeps_source() {
        let tmp = std::env::temp_dir().join("zorite-test-copydir");
        let _ = std::fs::remove_dir_all(&tmp);
        let (src, dst) = (tmp.join("s"), tmp.join("d"));
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("top.txt"), b"top").unwrap();
        std::fs::write(src.join("sub/deep.txt"), b"deep").unwrap();

        let done = AtomicU64::new(0);
        copy_dir_all(&src, &dst, &done).unwrap();

        assert_eq!(std::fs::read(dst.join("top.txt")).unwrap(), b"top");
        assert_eq!(std::fs::read(dst.join("sub/deep.txt")).unwrap(), b"deep");
        // A copy leaves the source intact.
        assert!(src.join("top.txt").exists());
        // Every copied byte was counted (3 + 4).
        assert_eq!(done.load(Ordering::Relaxed), 7);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn unix_absolute_and_file_url() {
        assert_eq!(
            resolve_local("/tmp/a.png"),
            Some(PathBuf::from("/tmp/a.png"))
        );
        // `file://` with a percent-encoded space decodes to a real path.
        assert_eq!(
            resolve_local("file:///tmp/my%20file.png"),
            Some(PathBuf::from("/tmp/my file.png"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_absolute_and_file_url() {
        assert_eq!(
            resolve_local(r"C:\docs\a.png"),
            Some(PathBuf::from(r"C:\docs\a.png"))
        );
        assert_eq!(
            resolve_local("file:///C:/docs/a.png"),
            Some(PathBuf::from(r"C:\docs\a.png"))
        );
    }
}
