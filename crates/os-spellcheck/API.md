# os-spellcheck API

The complete public API of [`os-spellcheck`](README.md) — every exported item,
with its signature, parameters, return contract, edge cases, and cost. For the
what-and-why (platform backends, quick start, wiring it into an editor), see
the [README](README.md).

## Public API at a glance

The crate exports exactly one type. Everything below is the complete public
surface — if it isn't listed here, it isn't public.

| Item | Kind | Signature | Purpose |
| --- | --- | --- | --- |
| [`SpellChecker`](#struct-spellchecker) | struct | — | Handle to the host OS spell-checking service |
| [`SpellChecker::new`](#spellcheckernew) | constructor | `fn new() -> Self` | Connect to the system spell checker (infallible) |
| [`SpellChecker::check`](#spellcheckercheck) | method | `fn check(&self, text: &str) -> Vec<Range<usize>>` | Misspelled spans in a string, as UTF-8 byte ranges |
| [`SpellChecker::suggestions`](#spellcheckersuggestions) | method | `fn suggestions(&self, word: &str) -> Vec<String>` | Replacement candidates for one word, best first |
| `impl Default for SpellChecker` | trait impl | `fn default() -> Self` | Identical to [`new`](#spellcheckernew) |

---

## `struct SpellChecker`

```rust
pub struct SpellChecker { /* private */ }
```

A handle to the host OS spell-checking service. Construct one with
[`new`](#spellcheckernew) (or `Default`) and keep it around — it holds the
backend connection (the shared `NSSpellChecker` on macOS, an `ISpellChecker`
COM instance on Windows), so reusing one instance avoids reconnecting per
call.

**Thread affinity** — create it and call its methods **on the main thread**.
The macOS backend talks to AppKit; the Windows backend uses COM, which must be
initialized on the calling thread (a GPUI host already does both for the UI
thread).

---

## `SpellChecker::new`

```rust
pub fn new() -> Self
```

Connect to the system spell checker.

**Parameters** — none.

**Returns** — a ready `SpellChecker`. **Always succeeds** — there is no
`Result`. Failure to reach the OS service is absorbed: the handle is still
returned, and [`check`](#spellcheckercheck) /
[`suggestions`](#spellcheckersuggestions) simply return empty results.

**Guarantees & edge cases**

- Infallible by design: an unsupported platform (Linux), a COM initialization
  failure, or an unsupported system language all degrade to "spell-check
  unavailable" (empty results) rather than an error the caller must route.
- `SpellChecker::default()` is identical.

**Cost & threading** — cheap on macOS (grabs the process-wide shared checker
and reads its current language). On Windows it creates the spell-checker COM
instance (`CoCreateInstance` + `CreateSpellChecker`) — still light, but
another reason to construct once and reuse. Main thread (see
[`struct SpellChecker`](#struct-spellchecker)).

**Per platform**

| Platform | Behavior |
| --- | --- |
| macOS | `NSSpellChecker::sharedSpellChecker()`; captures the checker's current language so suggestions match detection |
| Windows | Creates an `ISpellChecker`; requires COM initialized on this thread, else the handle is silently inert |
| other | Nothing to connect to; the handle is inert |

**Example**

```rust
let checker = SpellChecker::new();   // keep this around; don't rebuild per keystroke
```

---

## `SpellChecker::check`

```rust
pub fn check(&self, text: &str) -> Vec<Range<usize>>
```

Find the misspelled words in `text`.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `text` | `&str` | The document (or any snippet) to scan. Any length; any UTF-8. |

**Returns** — `Vec<Range<usize>>`: one entry per misspelled word, each a
**UTF-8 byte range into `text`**, so `&text[range]` is exactly the offending
word. Ranges are in document order, non-overlapping, and always on `char`
boundaries (the OS reports UTF-16 unit ranges; the crate converts them, so
multi-byte and multi-unit characters — accents, CJK, emoji — index safely).

**Guarantees & edge cases**

- Empty `text` → empty vec (no OS call).
- Unsupported platform, unreachable service, or an OS-level error mid-scan →
  the spans found so far (possibly none) — never a panic, never an error.
- Words the user has learned or ignored in the OS settings are **not**
  flagged.
- Detection language: on macOS, the user's own spell-check language(s) — the
  same behavior as TextEdit/Notes; on Windows, the checker's language (see
  [Platform notes](#platform-notes)).

**Cost & threading** — cheap and local; intended to run **on every edit**.
Main thread.

**Example**

```rust
let text = "teh quick brown fox";
let ranges = checker.check(text);
assert_eq!(&text[ranges[0].clone()], "teh");
```

---

## `SpellChecker::suggestions`

```rust
pub fn suggestions(&self, word: &str) -> Vec<String>
```

Suggested replacements for a single (presumably misspelled) `word`.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `word` | `&str` | One word — typically the `&text[range]` slice from a [`check`](#spellcheckercheck) result. Not a sentence or document. |

**Returns** — `Vec<String>` of replacement candidates, **best first** (the OS
service's own ranking). Possibly empty: no suggestions, an empty `word`, or an
unavailable service all return an empty vec.

**Guarantees & edge cases**

- Empty `word` → empty vec (no OS call).
- Never panics, never errors — service failures return empty.
- The suggestion language matches the detection language, so a word flagged by
  [`check`](#spellcheckercheck) gets suggestions from the same dictionary.

**Cost & threading** — **potentially slow.** On macOS this is a synchronous
XPC round-trip to the system spell service — calling it per keystroke or
across a whole document would storm the service (and can deadlock if invoked
before the run loop is pumping). Call it **lazily, for one word at a time** —
the intended trigger is the user right-clicking a flagged word. Main thread.

**Example**

```rust
let fixes = checker.suggestions("mispelled");
// e.g. ["misspelled", "dispelled", …] — best first; take the head few for a menu
```

---

## Platform notes

- **Call on the main thread.** The macOS backend talks to AppKit; the Windows
  backend uses COM, which must be initialized on the calling thread (a GPUI
  host already does this for the UI thread).
- **macOS** uses the user's own spell-check languages, learned words, and
  ignored words — behavior matches TextEdit/Notes exactly.
- **Windows** currently creates its checker for `en-US`; if that language (or
  COM) is unavailable, the handle is inert and both methods return empty.
  Following the system UI language is a known follow-up.
- **Linux / other:** both methods return empty vectors — text is never flagged
  and no suggestions are offered. No system speller is integrated yet.
- **UTF-16 → byte ranges:** the OS APIs report ranges in UTF-16 code units; the
  backends convert them to the UTF-8 byte ranges this crate returns, so callers
  work in plain byte offsets throughout.

