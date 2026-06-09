//! Where zorite keeps its data. One SQLite file under the platform's
//! per-user application-data directory, created on first run.

use std::path::{Path, PathBuf};

use url::Url;

/// The directory holding `zorite.db`. Platform conventions:
/// - macOS:   `~/Library/Application Support/zorite`
/// - Windows: `%APPDATA%\zorite`
/// - Linux:   `$XDG_DATA_HOME/zorite` or `~/.local/share/zorite`
///
/// `ZORITE_DATA` overrides the whole directory — handy for running against a
/// throwaway data set (the managed `images/`, `pdf/`, and the default DB
/// location all follow it) without touching the real one.
///
/// Falls back to the current directory only if the relevant home /
/// env var is somehow unset — enough to keep a dev run working.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ZORITE_DATA") {
        return PathBuf::from(dir);
    }
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

/// Absolute path to the SQLite database file. `ZORITE_DB` overrides it — handy
/// for running against a throwaway database (tests, benchmarks) without
/// touching the real one.
pub fn db_path() -> PathBuf {
    if let Some(path) = std::env::var_os("ZORITE_DB") {
        return PathBuf::from(path);
    }
    data_dir().join("zorite.db")
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
