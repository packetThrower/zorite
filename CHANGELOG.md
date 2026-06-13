# Changelog

All notable changes to **Zorite** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Every tagged release also has a GitHub page with installers and the full commit
log: <https://github.com/packetThrower/zorite/releases>.

## [Unreleased]

## [0.1.0-beta.2] - 2026-06-08

### Added

- **Full-text search** — a trigram FTS5 index over page titles and content:
  the same case-insensitive substring matching as before, now indexed so it
  scales to many pages.
- **Keyboard shortcuts and menus** — standard cross-platform shortcuts (New Tab
  `⌘/Ctrl+T`, New Window `⌘/Ctrl+N`, Close Tab `⌘/Ctrl+W`, Settings `⌘/Ctrl+,`,
  Quit `⌘/Ctrl+Q`, and `Ctrl+Tab` / `Ctrl+Shift+Tab` to switch tabs), a native
  macOS menu bar, and a **Settings → Keyboard** reference that lists them all.
- **PDF table of contents** — detects a PDF's outline and shows a navigable TOC
  panel; in-page links inside a PDF are now clickable.
- **Database safety** — a schema upgrade now snapshots the database to
  `zorite.db.bak-v<N>` first, runs each step inside a transaction, and surfaces
  a clear dialog (pointing at the backup) on failure instead of silently opening
  to blank notes.
- **Configurable list indentation** for markdown (the editor and the rendered
  view use the same width), and `<mark>` text renders as a highlight.
- The new-page **+** button now also appears in the collapsed sidebar rail.

### Changed

- Per-keystroke autosave now uses SQLite's **WAL** journal — smoother writes and
  better multi-window concurrency.
- **Open in new window** now *moves* the tab instead of duplicating it.
- PDF highlights require a **drag** (not a click) to create, and a multi-bullet
  selection is captured as a nested markdown list.
- **Esc** exits markdown edit mode, returning to the rendered view.
- Markdown rendering polish: monospace code spans, roomier heading spacing, and
  nested-list guide lines.
- Settings: the *Installed themes* list no longer includes built-ins, and PDF
  rendering moved to its own category.

### Fixed

- Backspace inside a doubled bracket or quote pair (e.g. `[[ ]]`, `( )`)
  duplicated the pair instead of deleting it.
- PDF viewer: draggable scrollbar behaviour, highlight row-drift deep in long
  documents, and the sidebar **+** button across platforms.

## [0.1.0-beta.1] - 2026-06-08

First cross-platform beta.

### Added

- **Local-first journal and notes** — a daily-journal feed where each day and
  page is a single markdown document; click any day or page's open area to edit.
- **Linking and structure** — `[[wiki-links]]` with backlinks, `#tags`, page
  aliases (`alias::`), and Logseq-style `Foo::Bar` namespace hierarchy in the
  sidebar.
- **Editing** — a slash-command menu with templates (`/date`, `/time`),
  autocomplete for `[[` links, `#` tags and `{{` placeholders, auto-pairing
  brackets and quotes (wrap-selection, type-over, smart backspace),
  `Tab`/`Shift+Tab` list indenting, and auto-continued lists on Enter.
- **Markdown** — full CommonMark + GFM rendering (the `gpui-markdown` crate) and
  inline images (render, paste and drag-drop, drag-to-resize).
- **In-app PDF viewer** (the `gpui-pdf` crate) — virtualized pages, drag-drop
  import, zoom and navigation, find-in-PDF, and drag-to-highlight markup with a
  colour picker and note↔PDF jump links.
- **Multi-window** — open a page or tab in a new window, drag tabs to reorder or
  tear off, with live cross-window content and backlink sync.
- **Sidebar** — a collapsible rail, a jump-to-date calendar, a recently-viewed
  page tree, and search.
- **Theming and settings**, plus cross-platform **packaging** (`.app`/`.dmg`,
  `.exe`, `.deb`/`.AppImage`/`.rpm`) with an app icon, and cross-platform CI.

[Unreleased]: https://github.com/packetThrower/zorite/compare/v0.1.0-beta.2...HEAD
[0.1.0-beta.2]: https://github.com/packetThrower/zorite/compare/v0.1.0-beta.1...v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/packetThrower/zorite/releases/tag/v0.1.0-beta.1
