//! Windows backend — the Win32 Spell Checking API (`ISpellChecker`, Windows 8+).
//!
//! Requires COM to be initialized on the calling thread; the gpui host does that
//! for its UI thread. If COM isn't up, or the language is unsupported, we get no
//! checker and report no misspellings (so the app degrades gracefully).
//!
//! `check` (detection) collects the error spans; `suggestions` asks the checker
//! for replacements for one word, lazily — mirroring the macOS split.

use windows::Win32::Globalization::{
    GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::core::{HSTRING, PWSTR};

use crate::utf16_to_byte;
use std::ops::Range;

pub(crate) struct Backend {
    checker: Option<ISpellChecker>,
}

impl Backend {
    pub(crate) fn new() -> Self {
        Backend { checker: connect() }
    }

    pub(crate) fn check(&self, text: &str) -> Vec<Range<usize>> {
        let Some(checker) = self.checker.as_ref() else {
            return Vec::new();
        };
        if text.is_empty() {
            return Vec::new();
        }
        let map = utf16_to_byte(text);
        let total = map.len() - 1;
        let wide = HSTRING::from(text);
        let mut out = Vec::new();

        unsafe {
            let Ok(errors) = checker.Check(&wide) else {
                return out;
            };
            loop {
                // The enumerator writes `Some` and returns S_OK for each error,
                // or writes `None` (S_FALSE) when finished.
                let mut item = None;
                if errors.Next(&mut item).is_err() {
                    break;
                }
                let Some(err) = item else {
                    break;
                };
                let (Ok(start), Ok(len)) = (err.StartIndex(), err.Length()) else {
                    continue;
                };
                let (u0, u1) = (start as usize, start as usize + len as usize);
                if u1 > total {
                    continue;
                }
                out.push(map[u0]..map[u1]);
            }
        }
        out
    }

    pub(crate) fn suggestions(&self, word: &str) -> Vec<String> {
        let Some(checker) = self.checker.as_ref() else {
            return Vec::new();
        };
        if word.is_empty() {
            return Vec::new();
        }
        let wide = HSTRING::from(word);
        let mut out = Vec::new();
        unsafe {
            let Ok(enumerator) = checker.Suggest(&wide) else {
                return out;
            };
            loop {
                let mut buf = [PWSTR::null()];
                let mut fetched = 0u32;
                if enumerator.Next(&mut buf, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                if !buf[0].is_null() {
                    out.push(read_and_free(buf[0]));
                }
            }
        }
        out
    }
}

/// Create the system spell checker for the user's UI language
/// (`GetUserDefaultLocaleName`, e.g. "de-DE"), falling back to `en-US` when
/// the lookup fails or that language has no checker installed. `None` on any
/// failure (COM not initialized, no language supported) — the caller treats
/// that as "spell-check unavailable".
fn connect() -> Option<ISpellChecker> {
    unsafe {
        let factory: ISpellCheckerFactory =
            CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        // 85 = LOCALE_NAME_MAX_LENGTH; the returned count includes the NUL.
        let mut buf = [0u16; 85];
        let n = GetUserDefaultLocaleName(&mut buf);
        if n > 1 {
            let lang = HSTRING::from(String::from_utf16_lossy(&buf[..(n - 1) as usize]));
            if factory
                .IsSupported(&lang)
                .map(|b| b.as_bool())
                .unwrap_or(false)
                && let Ok(sc) = factory.CreateSpellChecker(&lang)
            {
                return Some(sc);
            }
        }
        let lang = HSTRING::from("en-US");
        factory.CreateSpellChecker(&lang).ok()
    }
}

/// Read a COM-allocated wide string into an owned `String`, then free it (the
/// Spell Checking API allocates these with `CoTaskMemAlloc`; the caller frees).
fn read_and_free(pw: PWSTR) -> String {
    unsafe {
        let s = pw.to_string().unwrap_or_default();
        CoTaskMemFree(Some(pw.0 as *const _));
        s
    }
}
