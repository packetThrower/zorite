# zorite — TODO

Roadmap / known follow-ups. Roughly priority-ordered within each section.

## Contents

- [Editor & rendering](#editor--rendering)
- [Notes & navigation](#notes--navigation)
- [Performance](#performance)
- [Data & migrations](#data--migrations)
- [App & polish](#app--polish)
- [Import & export](#import--export)
- [gpui-markdown crate](#gpui-markdown-crate)

## Editor & rendering
- [x] **As-you-type completion** — `[[` (pages, with a "Create" entry), `#` (tags), and `{{` (template placeholders); reuses the slash popup, ranks matches, and caps the list so it stays usable with many pages
- [x] **Auto-pair brackets/quotes** (`()` `[]` `{}` `<>` `""` `''`) with type-over and prose-safe guards (contraction-aware quotes, `<` only after a word); confirming a `[[`/`{{` completion absorbs the auto-inserted closer
- [x] Auto-pair: **wrap the selection** — typing an opener with text selected wraps it (`foo` → `(foo)`); done in the change handler by diffing against the prior text, no key-level interception needed
- [x] Auto-pair: **backspace deletes an empty pair** (`(|)` + backspace → remove both)
- [x] **Inline image rendering** — standalone `![](path-or-url)` images render for real (async, aspect-ratio preserved, capped to content width); an image at the start of a paragraph renders with trailing text as a caption below
- [x] Image **resize** — drag the corner handle (live preview); persists as `![](src){width=N}` in the markdown
- [x] Image **insert** — paste from clipboard (`Cmd+V`) or drag-and-drop a file; copied into the data-dir `images/` folder and referenced relatively
- [ ] Images: **orphan GC** (delete `images/` files no page references) + optional content-addressed names (dedupe identical pastes)
- [ ] Images: **AVIF** isn't decodable by gpui (jpg/png/webp/gif/bmp/tiff/svg work) — convert on import, or surface a clearer message
- [ ] Slash menu: **click-to-insert** a command (keyboard-only today; needs to avoid blurring the editor on click)
- [x] **Task-list checkboxes** (`- [ ]` / `- [x]`) — rendered via mdast `ListItem.checked` (the field does exist after all)
- [x] `gpui-markdown` now covers CommonMark + GFM: footnotes, reference-style `[text][id]` links/images, and raw HTML (shown literally)
- [ ] Place the caret at the click point when entering edit mode (currently keeps the last position)
- [x] `/time` and `/date` slash commands — insert the current time/date directly (distinct from the `{{time}}` / `{{date}}` *template* placeholders, which only expand inside a template)
- [ ] **Setting: configurable date format** — let the user choose the date (and time) format used by `/date` / `/time` and the `{{date}}` / `{{time}}` placeholders (currently hardcoded to `YYYY-MM-DD` / `HH:MM` in `slash::current_date`/`current_time`)

## Notes & navigation
- [x] **Page rename** (and rewrite `[[links]]` pointing at it) — right-click → Rename page → dialog; `db.rename_page` rewrites links in a transaction
- [ ] Rename: also rewrite case/whitespace link variants (`[[ Foo ]]`, `[[FOO]]`) — v1 rewrites the exact stored title only
- [x] **Page hierarchy** via `[[parent::child]]` — Logseq-style: the `::` path *is* the page title, so the sidebar tree and each page's "Sub-pages" index are derived from titles (no parent column). Intermediate namespace segments show as virtual nodes and materialize on click. See `src/hierarchy.rs`
- [ ] Hierarchy follow-ups: collapsible namespace nodes in the sidebar; cascade-rename a namespace (rename `Foo` → rewrite `Foo::*` children + their `[[links]]`); a "New sub-page" action
- [x] **Page aliases** — a subdued `alias::` field under the page title takes a comma list of alternate names; `[[name]]` then resolves to that page (exact title wins). Stored in a `page_aliases` table; resolution lives in `get_or_create_page`, so links and backlinks follow it
- [ ] Aliases: offer a page's aliases as suggestions in `[[` autocomplete
- [ ] Unlinked references (mentions of a page title without `[[ ]]`)
- [x] **Sidebar shows recent pages** — the page tree is capped to the last 10 *viewed* named pages (persisted in `settings`; seeded from the most-recently-edited pages on first run). Reach the rest via search
- [ ] **Favorites section in the sidebar** — pin chosen pages to a "Favorites" group above the page list (e.g. right-click → Favorite); persists across launches
- [ ] Sidebar: a "show all pages" affordance (browse the full tree, not just recent)
- [x] Journal: jump-to-date — a sidebar calendar date picker opens any day (creating it if needed)
- [ ] Calendar: mark/indicate days that already have entries (would read `Page.journal_date`, which is populated for exactly this)

## Performance
- [x] **Lighter `list_pages`** — the page list loads `id`/`title` only (not content): ~4× faster and memory-flat at scale (50k pages: 103 ms → 28 ms; RAM ~flat 10k→50k). See the [Performance](README.md#performance) section
- [ ] **Full-text search index** — search is a `LIKE` scan (~23 ms/keystroke at 50k pages); an FTS5 index would scale it
- [ ] True **list virtualization** in the journal feed (v1 keeps all loaded days mounted)
- [ ] Move SQLite writes off the UI thread (background executor)

## Data & migrations
- [ ] **Back up before migrating** — copy the DB to `zorite.db.bak-v<N>` before applying schema migrations on launch, so a bad migration is recoverable
- [ ] **Transactional migrations** — wrap each migration step (especially any data transform) in a transaction; today only `v1→v2` is, so a mid-way failure can leave a half-migrated DB
- [ ] **Friendlier migration failure** — a failed migration currently falls back to an empty in-memory DB (the user opens to blank notes); surface the error and offer to restore the backup instead of silently showing emptiness

## App & polish
- [ ] **Visual design pass** — make the UI look professional and easy on the eyes (spacing, typography, color, density)
- [x] **Collapsible sidebar** — a `<` caret collapses it to a thin icon rail (`>` to expand, plus the calendar/settings icons); the content area reclaims the space
- [ ] Sidebar: remember the collapsed state across launches, and add a keyboard shortcut to toggle it
- [x] **Multi-window** — right-click a sidebar page or a tab → "Open in new window" opens a full, independent second window focused on that page (`AppView::open_in_new_window`). Each window is its own `AppView` with its own SQLite connection to the same file. See `src/app.rs`, `src/ui/tab_bar.rs`
- [ ] Multi-window: **drag a tab out to tear off a new window** (browser-style) + reorder tabs within the strip. No library support (gpui-component's dock only rearranges within one window); custom-build on `on_drag_move` + `mouse_position` vs `window.bounds()` → `open_in_new_window`. Wayland: the compositor controls new-window placement
- [x] Multi-window: **live cross-window sync** — a shared `DocSignal` (gpui global, one per process) is emitted on content saves (`save_page_content`) AND structural changes (create / rename / delete + the blur link re-index); other windows run `apply_external_edit`, reloading changed journal days, the active page (content + backlinks), and the sidebar page-list (via value-comparison, so only stale data is touched and the editing window never clobbers itself)
- [ ] Multi-window: same-page **concurrent edits** are last-write-wins — editing the *same* page/day in two windows at once can drop one side's changes. True resolution needs a CRDT/OT layer (out of scope for a single-user app); revisit only if real-time collaboration is ever wanted
- [ ] Window-bounds persistence (reopen where you left off)
- [ ] App icon + packaging (cargo-packager: `.dmg` / `.msi` / `.deb`)
- [ ] Add a `LICENSE` file (Cargo.toml already declares `GPL-3.0-or-later`)
- [ ] CI (build + `cargo test --workspace`)

## Import & export
- [ ] **Logseq import** — bring in existing Logseq pages/journals (markdown)
- [ ] **Print / PDF export** — generate a PDF from a note (`oxidize-pdf` can generate; or a typeset path like typst/`printpdf`)
- [x] In-app **PDF viewer** — `[[file.pdf]]` / `![](file.pdf)` open a dedicated viewer tab (`ui::pdf_view`); pages are sized from `render_dimensions()` for instant layout. Closing the tab frees both the CPU images and their **GPU atlas textures** (`cx.drop_image` — raw `RenderImage`s are never auto-evicted; this was an ~140 MB/open leak). See `src/pdf.rs`
- [x] PDF: **viewport virtualization** — only the on-screen pages (±2) are rasterized; far ones are evicted (image + GPU texture), so memory is bounded by the viewport, not the page count (`AppView::ensure_pdf_window` + `pdf_view::keep_window`). Verified: scrolling a 32-page PDF end-to-end holds ~178 MB vs 403 MB before
- [x] PDF: **DPI-aware render scale** — pages rasterize at display pixel-ratio × zoom × a host **quality** multiplier (no longer a fixed 1.5×); a render-quality slider in Settings trades sharpness for speed (default 75%), read reactively so open viewers re-render live
- [x] PDF: **zoom + page navigation** — − / + / reset (⌘= / ⌘- / ⌘0) and ‹ / › with a click-to-edit page-number input (+ PageUp / PageDown / Home / End); no blank on zoom/quality change (the old bitmap stays, rescaled, until the crisp one lands)
- [ ] PDF: **fit-width / fit-page** zoom modes (zoom is free-scale only today)
- [x] PDF: **extracted to a reusable [`gpui-pdf`](crates/gpui-pdf/README.md) crate** — host-agnostic primitives + a self-contained `PdfView` component; markup is behind an optional `markup` feature
- [x] PDF: **Logseq-style text markup** — drag-to-highlight in the viewer writes a reference block (`- pN: quote {color} [[file.pdf#pN|↗]]`) on a per-PDF "(highlights)" page; clicking the ↗ opens the PDF and scrolls to + **flashes** the highlight. Done **dep-free** — a custom hayro `Device` extracts text + glyph rects (only `kurbo`, *not* oxidize-pdf). Has a **color picker** (yellow/green/blue/pink/orange) and header **tooltips**
- [ ] PDF: **area (image-region) highlights** — only text-anchored highlights exist so far; a box-drag over a scanned region would cover figures / pages with no text layer
- [x] PDF: **find-in-PDF** — a browser-style find bar (🔍 / ⌘F) over the text layer: type to search the whole document, matches boxed + a focused one outlined, `n / N` count, Enter/⇧Enter to step (scroll-to), Esc to close. Behind a `search` feature (= `["markup"]`, shares the text layer). See `gpui-pdf` `find_matches` + `src/pdf.rs`
- [ ] PDF: **garbled quotes from decorative fonts** — some heading fonts decode to shifted/garbled unicode (e.g. a −29 glyph shift), so a highlight on them stores garbled text (it still re-locates, since garbled matches garbled); body text is correct. Upstream hayro limitation
- [ ] PDF: **graceful fallback for unsupported files** — hayro can't open encrypted / password-protected PDFs (a stated limitation) and may stumble on exotic transparency / blend modes; on a load/parse failure, show an "Open in default app" affordance (hand off to the OS viewer) instead of a blank pane
- [ ] PDF: **outline / table-of-contents detection** — read the document outline (`/Catalog /Outlines` bookmarks) via `hayro-syntax` (or `lopdf`) and show a navigable TOC panel that jumps to each destination's page; hide it when the PDF has no outline
- [ ] PDF: **AcroForm + annotations** — no pure-Rust crate does a full interactive forms/annotation engine. Heavy options reintroduce a native dep: `pdfium-render` (PDFium — full forms/annotations/render, permissive license) or `mupdf-rs` (full, but AGPL + native). Pure-Rust path: a targeted subset on `lopdf` — read `/AcroForm /Fields`, fill text fields/checkboxes via `/V` (+ `/NeedAppearances`), and render existing annotation appearance streams (`/AP /N`, which are XObjects hayro may already rasterize). First check whether hayro already composites `/AP` streams

## gpui-markdown crate
- [ ] Extract editor features (e.g. the slash menu) into a reusable crate if they generalize
- [ ] Publish to crates.io once the API is stable
