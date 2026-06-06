//! Where zorite keeps its data. One SQLite file under the platform's
//! per-user application-data directory, created on first run.

use std::path::PathBuf;

/// The directory holding `zorite.db`. Platform conventions:
/// - macOS:   `~/Library/Application Support/zorite`
/// - Windows: `%APPDATA%\zorite`
/// - Linux:   `$XDG_DATA_HOME/zorite` or `~/.local/share/zorite`
///
/// Falls back to the current directory only if the relevant home /
/// env var is somehow unset — enough to keep a dev run working.
pub fn data_dir() -> PathBuf {
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
