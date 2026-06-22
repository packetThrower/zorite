//! Native OS spell-check: `NSSpellChecker` on macOS, `ISpellChecker` on Windows.
//!
//! Two operations, deliberately split:
//! - [`SpellChecker::check`] finds the misspelled spans in a string (each a
//!   **UTF-8 byte range** into it). This is cheap and local, so a host can run
//!   it on every edit.
//! - [`SpellChecker::suggestions`] returns replacement suggestions for a single
//!   word. On macOS this is a synchronous XPC call to the system spell service,
//!   so it's kept separate and meant to run lazily — e.g. only when the user
//!   right-clicks a flagged word — never in a per-keystroke loop.
//!
//! On a platform with no system speller (currently Linux) both are no-ops, so
//! callers don't need their own `cfg`s. Host-agnostic: plain `&str` and byte
//! ranges in, no gpui dependency.

use std::ops::Range;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod backend;

#[cfg(windows)]
#[path = "windows.rs"]
mod backend;

/// A handle to the host OS spell-checking service.
pub struct SpellChecker {
    #[cfg(any(target_os = "macos", windows))]
    backend: backend::Backend,
}

impl SpellChecker {
    /// Connect to the system spell checker. Always succeeds; on an unsupported
    /// platform — or if the OS service can't be reached — the methods below just
    /// return empty results.
    pub fn new() -> Self {
        Self {
            #[cfg(any(target_os = "macos", windows))]
            backend: backend::Backend::new(),
        }
    }

    /// Find the misspelled words in `text`, returning their UTF-8 byte ranges
    /// (so `&text[range]` is the offending word). Cheap enough to run on edit.
    ///
    /// Call on the main thread: the macOS backend talks to AppKit.
    pub fn check(&self, text: &str) -> Vec<Range<usize>> {
        #[cfg(any(target_os = "macos", windows))]
        {
            self.backend.check(text)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = text;
            Vec::new()
        }
    }

    /// Suggested replacements for a single (presumably misspelled) `word`, best
    /// first. Potentially slow (a system spell-service round-trip on macOS) —
    /// call it lazily, for one word at a time, not across a whole document.
    ///
    /// Call on the main thread.
    pub fn suggestions(&self, word: &str) -> Vec<String> {
        #[cfg(any(target_os = "macos", windows))]
        {
            self.backend.suggestions(word)
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = word;
            Vec::new()
        }
    }
}

impl Default for SpellChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a map from UTF-16 code-unit offset to UTF-8 byte offset in `text`.
///
/// `map[u]` is the byte offset of the char that UTF-16 unit `u` belongs to, and
/// `map[n]` (with `n` = the total number of UTF-16 units) is `text.len()`. The
/// OS APIs report ranges in UTF-16 units; this converts those to the byte ranges
/// the rest of the app uses. A misspelled span `[u0, u1)` becomes
/// `map[u0]..map[u1]`.
#[cfg(any(target_os = "macos", windows))]
pub(crate) fn utf16_to_byte(text: &str) -> Vec<usize> {
    let mut map = Vec::with_capacity(text.len() + 1);
    for (byte, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            map.push(byte);
        }
    }
    map.push(text.len());
    map
}
