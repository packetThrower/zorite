//! Where rumin keeps its data. One SQLite file under the platform's
//! per-user application-data directory, created on first run.

use std::path::PathBuf;

/// The directory holding `rumin.db`. Platform conventions:
/// - macOS:   `~/Library/Application Support/rumin`
/// - Windows: `%APPDATA%\rumin`
/// - Linux:   `$XDG_DATA_HOME/rumin` or `~/.local/share/rumin`
///
/// Falls back to the current directory only if the relevant home /
/// env var is somehow unset — enough to keep a dev run working.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        home.join("Library").join("Application Support").join("rumin")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rumin")
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rumin")
    }
}

/// Absolute path to the SQLite database file.
pub fn db_path() -> PathBuf {
    data_dir().join("rumin.db")
}
