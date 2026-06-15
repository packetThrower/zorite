# Changelog

All notable changes to **Zorite** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Every tagged release also has a GitHub page with installers and the full commit
log: <https://github.com/packetThrower/zorite/releases>.

## [Unreleased]

## [0.1.2] - 2026-06-15

### Fixed

- **Windows: the app exited with an error where no graphics display was
  available** — headless or RDP sessions, and the winget validator's sandbox.
  GPUI couldn't initialize its DirectX renderer (the desktop compositor was
  unreachable), so the process exited non-zero. Zorite now probes for a graphics
  adapter at startup and, when none is usable, shows an explanatory dialog and
  exits cleanly instead of erroring out. No effect on a normal desktop; macOS
  and Linux were unaffected.

## [0.1.1] - 2026-06-15

### Fixed

- **Windows: the app would not launch on a clean install.** The build linked
  the Microsoft Visual C++ runtime (`VCRUNTIME140.dll`) dynamically, so on a
  machine without the VC++ Redistributable `zorite.exe` exited immediately with
  `0xC0000135` (`STATUS_DLL_NOT_FOUND`) — no window, no error dialog from the
  app. The C runtime is now statically linked (`+crt-static`), so the binary is
  self-contained. This affected every Windows install path (the installer, the
  `.msi`, Scoop, and direct downloads); macOS and Linux were unaffected.

First stable release. The highlights since `0.1.0-beta.2`:

### Added

- **Whiteboards** — a new freeform infinite-canvas surface (the
  `gpui-whiteboard` crate): pan/zoom, a freehand pen, shapes (rectangle,
  ellipse, line, arrow, diamond, triangle, rounded-rectangle, star, hexagon),
  on-canvas text that edits like a real text field, dropped/pasted images
  (rotatable in 90° steps), and **page-card embeds** that link to notes.
  Select, move, resize, and rotate one element or a multi-selection;
  per-element colour, fill, gradient, and opacity; stroke thickness; z-order
  (bring to front / send to back); snap-to-grid; and copy/cut/paste across
  boards and windows. **Reusable templates** (save a selection, stamp it from a
  modal gallery). Boards are first-class pages with their own **Whiteboards**
  sidebar section, searchable by title (`wb:` + a filter chip); the toolbar is
  movable and category-grouped, with tooltips, keyboard shortcuts, and optional
  per-board fonts.
- **Logseq import** — `File → Import from Logseq…` brings a graph's `pages/`,
  `journals/`, and assets into Zorite (namespaces, task markers, properties,
  aliases, `{{embed}}`/`((block-ref))`, and `hls__*` PDF-highlight pages all
  handled), plus **whiteboards** (tldraw boards → native Zorite boards, images
  and all) and **favorites**. Built as an extensible reader/engine split so
  other sources can be added.
- **Mermaid diagrams** — fenced `mermaid` code blocks render as themed,
  pure-Rust diagrams; click one to expand it in a lightbox.
- **Find in page** — search the rendered note text with match highlighting, a
  running count, and scroll-to-match.
- **Click-to-caret editing** — click anywhere on a rendered page (or
  right-click → **Edit**) to drop straight into edit mode with the caret at the
  click.
- **Favorites** — pin any page to a **Favorites** group in the sidebar
  (right-click → *Add to favorites*); persists across launches.
- **Tab tear-off** — drag a tab to reorder it, move it to another window, or
  tear it off into a brand-new window, with live cross-window content sync.
- **Type-aware search** — results span pages, PDF and image files, and
  whiteboards, filterable by kind.
- A **GPL-3.0** `LICENSE`.

### Changed

- The product is now styled **Zorite** (binaries and identifiers stay lowercase
  `zorite`).
- The journal feed **loads lazily** and frees off-screen image and diagram
  bitmaps, keeping long feeds responsive.
- Sidebar polish: **collapsible sections** and namespace nodes, vertical indent
  guides for nested pages, and accented section headers with a hairline rule.
- (Windows) the title-bar light/dark toggle now works and sits opposite the
  window controls.

### Fixed

- Clicking a link in a rendered note is no longer swallowed by click-to-edit.
- The slash-command menu scrolls without scrolling the page behind it, and its
  items are clickable (not Enter-only).
- Mermaid lightbox: a tighter hit-box and **Esc** to close.
- Logseq import splits glued image runs so each renders as its own block image.

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

[Unreleased]: https://github.com/packetThrower/zorite/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/packetThrower/zorite/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/packetThrower/zorite/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/packetThrower/zorite/compare/v0.1.0-beta.2...v0.1.0
[0.1.0-beta.2]: https://github.com/packetThrower/zorite/compare/v0.1.0-beta.1...v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/packetThrower/zorite/releases/tag/v0.1.0-beta.1
