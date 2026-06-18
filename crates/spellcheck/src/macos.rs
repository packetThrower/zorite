//! macOS backend — Apple's system-wide `NSSpellChecker` (AppKit).
//!
//! It uses the user's own languages, learned words, and ignored words, exactly
//! like spell-check in TextEdit/Notes. The methods we call are non-`unsafe` in
//! objc2-app-kit and don't take a `MainThreadMarker`; we still call them on the
//! main thread (see [`crate::SpellChecker`]).
//!
//! `check` (detection) is local and fast. `suggestions` goes through a
//! synchronous XPC call to the system spell service, so it's only invoked lazily
//! for a single word — never in the detection loop (that would both storm the
//! service and deadlock if called before the run loop is pumping).

use objc2::rc::Retained;
use objc2_app_kit::NSSpellChecker;
use objc2_foundation::{NSRange, NSString};

use crate::utf16_to_byte;
use std::ops::Range;

pub(crate) struct Backend {
    checker: Retained<NSSpellChecker>,
    /// The checker's current language, passed to `guesses…` so suggestions match
    /// the language a misspelling was flagged in.
    language: Retained<NSString>,
}

impl Backend {
    pub(crate) fn new() -> Self {
        let checker = NSSpellChecker::sharedSpellChecker();
        let language = checker.language();
        Self { checker, language }
    }

    pub(crate) fn check(&self, text: &str) -> Vec<Range<usize>> {
        if text.is_empty() {
            return Vec::new();
        }
        let map = utf16_to_byte(text);
        let total = map.len() - 1; // total UTF-16 code units
        let ns = NSString::from_str(text);
        let mut out = Vec::new();

        // Walk the string: each call returns the next misspelled word's range
        // (in UTF-16 units) at or after `start`; a zero-length range means none.
        //
        // `wrap: false` is essential — the convenience `checkSpellingOfString:
        // startingAt:` wraps back to the start of the string at the end, which
        // would make this loop re-find the first misspelling forever.
        let mut start: usize = 0;
        while start < total {
            let range = unsafe {
                self.checker
                    .checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(
                        &ns,
                        start as isize,
                        Some(&self.language),
                        false,
                        0,
                        std::ptr::null_mut(),
                    )
            };
            if range.length == 0 {
                break;
            }
            let (u0, u1) = (range.location, range.location + range.length);
            if u1 > total || u0 < start {
                break; // defensive — past the map, or no forward progress
            }
            out.push(map[u0]..map[u1]);
            start = u1;
        }
        out
    }

    pub(crate) fn suggestions(&self, word: &str) -> Vec<String> {
        if word.is_empty() {
            return Vec::new();
        }
        let ns = NSString::from_str(word);
        let range = NSRange {
            location: 0,
            length: word.encode_utf16().count(),
        };
        self.checker
            .guessesForWordRange_inString_language_inSpellDocumentWithTag(
                range,
                &ns,
                Some(&self.language),
                0,
            )
            .map(|arr| arr.iter().map(|guess| guess.to_string()).collect())
            .unwrap_or_default()
    }
}
