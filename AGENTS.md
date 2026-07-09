# AGENTS.md

Guidance for AI agents and human contributors. Keep changes consistent with what's
already here — match the surrounding code, and prefer deleting over adding.

## What this is

Zorite — a cross-platform (macOS / Windows / Linux) Markdown daily-journal desktop app.
Rust + [GPUI](https://www.gpui.rs) + gpui-component + SQLite. The repo is a Cargo
workspace (edition 2024): the app at the root, plus six reusable crates under `crates/`.

## Layout

- `src/` — the app: window, journal feed, editor wiring, SQLite (`db.rs`), import,
  settings, and the host-side renderers (`ui/`).
- `crates/gpui-editor` — from-scratch text editor for GPUI (the WYSIWYG markdown surface).
- `crates/gpui-markdown` — Markdown reading-view renderer.
- `crates/gpui-pdf` — page-virtualized PDF viewer (pure-Rust `hayro`, no native libs).
- `crates/gpui-whiteboard` — infinite pan/zoom whiteboard canvas.
- `crates/ratex-gpui` — LaTeX math renderer + structural editor (RaTeX engine).
- `crates/os-spellcheck` — native OS spell-check (no deps; macOS/Windows, Linux no-op).
- `docs/` — Astro Starlight docs site (auto-deploys on push to `main`).

## The three views

A note renders in exactly one of three views. Use these names — in code,
comments, and commits — so a search for the view name finds its implementation:

| View | What it is | Owner | Switched by |
|---|---|---|---|
| **WYSIWYG** | Live-preview *editing*: markers dimmed/hidden, headings sized, images/tables/math render inline, reveal-on-caret | `crates/gpui-editor` with a `markdown_style` installed | `AppView.wysiwyg` on (the default; Settings → Markdown) |
| **raw** | Plain-text *editing*: the bare markdown source, no styling | `crates/gpui-editor` with no `markdown_style` | `AppView.wysiwyg` off, while editing |
| **reader** | *Read-only* rendered markdown (clickable links, checkboxes) | `crates/gpui-markdown` (`MarkdownView`) | `AppView.wysiwyg` off, when not editing |

The app picks the view in `day_section` (`src/ui/journal.rs`) and its
`src/ui/page_view.rs` twin; hosts wire renderers/handlers to both crates from
`src/ui/` and the `EditorEvent` subscriptions in `src/app.rs`.

**The cross-view rule:** any user-facing markdown behavior — rendering a
construct, clicking it, hover cursors — must be implemented (or knowingly
skipped) in **both** the reader (`gpui-markdown`) and WYSIWYG
(`gpui-editor`); raw is plain text by design. They are separate engines and
share nothing: a feature added to one does NOT appear in the other. This has
bitten before — links (`[[wiki]]`, `#tag`, `[text](url)`) navigated in the
reader for months while WYSIWYG silently ignored clicks (fixed in 0.4.1).
When you touch one side, grep the other for the same construct.

**Parity direction:** when the two views disagree visually, WYSIWYG's look is
the baseline — object widths and spacing (tables, code cards, body line
height 1.45) converge toward it. The exceptions: bullet-list item spacing
AND list indentation (bullet→text gap) follow the READER's roomier look.
(User-set rules, 2026-07-02.)

`gpui-editor` comments use `W1`/`W2`/`W4a–c`/`W6` milestone codes for WYSIWYG
features — the legend is in that crate's `lib.rs` top doc.

## Build, run, and the gate

```
cargo run                                            # launch the app (root bin: zorite)
```

Before every commit, run what CI runs (`.github/workflows/ci.yml`) — these must pass:

```
cargo fmt --check                                    # or `cargo fmt` to fix
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo check` alone is **not** the gate — clippy is, and any warning fails the build.
CI also compiles on five targets (macOS, Linux x64/arm, Windows x64/arm), so every
change must stay cross-platform.

## Code conventions

- **No speculative abstraction.** This codebase has zero one-impl traits and no
  factories or config nobody sets. Climb the ladder: does it need to exist? → reuse an
  existing helper → stdlib → one line → only then new code. Don't add a dependency for
  what a few lines do.
- **Surgical diffs.** Every changed line should trace to the change you're making. Don't
  reformat or "improve" adjacent code. It's a `-D warnings` repo — leave no orphaned
  `dead_code`/`unused` behind your edit.
- **Crates stay host-agnostic.** `crates/*` depend on `gpui` only — not `gpui-component`,
  not the app — and run on all three platforms with no native libraries. One
  sanctioned sibling dependency: `gpui-editor` → `gpui-markdown`, for the shared
  construct **recognition** in `gpui_markdown::syntax` (alert kinds, table
  styles, heading scales) — never for rendering. Keep the
  editor/rendering cores GUI-free where a crate already splits them (e.g. ratex-gpui's
  `editor::{model,cursor,geometry,input,latex}` are GUI-free; only `view` is gpui glue).
- **The app owns rendering.** Renderers in the crates are host-agnostic; the app supplies
  the concrete one (see the `MarkdownView::on_math` / `on_inline_math` wiring in `ui/`).
- **Cross-platform IO.** No `$HOME` or Unix-only assumptions — use `paths::*`
  (e.g. `paths::desktop_dir()`) and gate platform code with `#[cfg(...)]`.
- **Optional functionality sits behind a Cargo feature** so consumers can drop it
  (ratex-gpui's default-on `editor`; gpui-pdf's opt-in `markup`/`search`).
- **Chrome controls are `.small()`.** gpui-component controls in app chrome —
  settings cards, side panels, popovers, find bars, in-pane prompts — take
  `Sizable::small()`, and the settings `text_button` matches that scale (its
  per-row `nb_button` sits a notch below). Dialog bodies/footers and focal
  surfaces (the unlock screen, the page-title field) keep the default size.

## Tests

- Unit tests live in-file under `#[cfg(test)]`; cover non-trivial logic (import parsers,
  editor/whiteboard geometry, DB and link-rewriting).
- Live-testing the GUI: synthetic **keyboard** input does not reach a GPUI window (mouse
  does) — verify shortcuts by hand. Kill all running instances before relaunching, and
  close the app before touching its SQLite DB (it opens a real one in the platform data dir).

## Commits

Conventional commits with a scope: `fix(math): …`, `refactor: …`, `docs(changelog): …`.

## Releases

Stable = add a `## [x.y.z]` section to `CHANGELOG.md` and push an **annotated** tag
`vX.Y.Z` (no `Cargo.toml` bump — the version comes from the tag). Pre-release = a
`-suffix` tag (e.g. `v0.4.0-beta.1`). `release.yml` builds every platform; winget
submission for a stable tag is a manual `gh workflow run after_release.yml -f tag_name=vX.Y.Z`.
