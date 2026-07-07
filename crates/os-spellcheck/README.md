# os-spellcheck

Native **OS spell-checking** with a tiny, host-agnostic API:

- **macOS** — Apple's `NSSpellChecker` (AppKit).
- **Windows** — the Win32 Spell Checking API (`ISpellChecker`, Windows 8+).
- **Everywhere else** (currently Linux) — a no-op that returns empty results.

Plain `&str` and UTF-8 byte ranges in and out — **no `gpui` dependency** — so any
app can use it. Because the unsupported platform is a no-op, callers never need
their own `#[cfg]`s.

**📖 Full reference:** every public item, with signatures, parameter tables,
return contracts, edge cases, and cost notes, lives in [API.md](API.md).

## Overview

Two operations, deliberately split by cost:

| Method | Cost | When to call |
| --- | --- | --- |
| `check` | cheap, local | on every edit |
| `suggestions` | a system spell-service round-trip (macOS) | lazily — e.g. only on right-click of a flagged word |

Keeping them separate means a host can re-detect misspellings per keystroke
without ever paying for suggestions until the user actually asks for them.

## Adding the dependency

```toml
[dependencies]
os-spellcheck = "0.1"
```

No features to configure — the platform backend is selected by `cfg`. On macOS it
pulls in `objc2` / `objc2-app-kit` (`NSSpellChecker`); on Windows the `windows`
crate's spell-checking + COM features. On other platforms there are no extra
dependencies.

## Quick start

```rust
use os_spellcheck::SpellChecker;

let checker = SpellChecker::new();
let text = "Some mispelled wrds.";

// Per-edit: misspelled spans as UTF-8 byte ranges into `text`.
for range in checker.check(text) {
    println!("misspelled: {:?}", &text[range]);   // "mispelled", "wrds"
}

// Lazy: replacements for one word, best first.
let fixes = checker.suggestions("mispelled");      // e.g. ["misspelled", …]
```

That's the entire surface — one struct, two methods. The exact contracts
(range guarantees, failure behavior, threading) are in [API.md](API.md).

## Platform notes

- **Call on the main thread.** The macOS backend talks to AppKit; the Windows
  backend uses COM, which must be initialized on the calling thread (a GPUI host
  already does this for the UI thread).
- **macOS** uses the user's own spell-check languages, learned words, and
  ignored words — behavior matches TextEdit/Notes exactly.
- **Windows** currently creates its checker for `en-US`; following the system
  UI language is a known follow-up.
- **Linux / other:** both methods return empty vectors — text is never flagged
  and no suggestions are offered. No system speller is integrated yet.
- **UTF-16 → byte ranges:** the OS APIs report ranges in UTF-16 code units; the
  backends convert them to the UTF-8 byte ranges this crate returns, so callers
  work in plain byte offsets throughout.

## Using it with [`gpui-editor`](../gpui-editor)

`gpui-editor` consumes exactly this shape — byte-range diagnostics plus a lazy
suggestion provider:

```rust
use gpui_editor::Diagnostic;
use os_spellcheck::SpellChecker;

// On each edit: feed the misspelled ranges in as diagnostics (red squiggles).
let diagnostics = SpellChecker::new()
    .check(text)
    .into_iter()
    .map(|range| Diagnostic { range })
    .collect();
editor.update(cx, |ed, cx| ed.set_diagnostics(diagnostics, cx));

// Once at setup: the lazy provider, consulted only on right-click.
editor.update(cx, |ed, _| {
    ed.on_suggest(|word| SpellChecker::new().suggestions(word));
});
```

## License

GPL-3.0-or-later.
