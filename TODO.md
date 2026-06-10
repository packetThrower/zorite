# zorite — TODO

Roadmap / known follow-ups. Roughly priority-ordered within each section. Finished
work is collected under [Completed](#completed) at the bottom.

## Contents

- [Editor & rendering](#editor--rendering)
- [Notes & navigation](#notes--navigation)
- [Performance](#performance)
- [App & polish](#app--polish)
- [Import & export](#import--export)
- [gpui-markdown crate](#gpui-markdown-crate)
- [Completed](#completed)

## Editor & rendering
- [ ] Images: **orphan GC** (delete `images/` files no page references) + optional content-addressed names (dedupe identical pastes)
- [ ] Images: **AVIF** isn't decodable by gpui (jpg/png/webp/gif/bmp/tiff/svg work) — convert on import, or surface a clearer message
- [ ] Slash menu: **click-to-insert** a command (keyboard-only today; needs to avoid blurring the editor on click)

## Notes & navigation
- [ ] Rename: also rewrite case/whitespace link variants (`[[ Foo ]]`, `[[FOO]]`) — v1 rewrites the exact stored title only
- [ ] Hierarchy follow-ups: collapsible namespace nodes in the sidebar; cascade-rename a namespace (rename `Foo` → rewrite `Foo::*` children + their `[[links]]`); a "New sub-page" action
- [ ] Aliases: offer a page's aliases as suggestions in `[[` autocomplete
- [ ] Unlinked references (mentions of a page title without `[[ ]]`)
- [ ] **Favorites section in the sidebar** — pin chosen pages to a "Favorites" group above the page list (e.g. right-click → Favorite); persists across launches
- [ ] Sidebar: a "show all pages" affordance (browse the full tree, not just recent)
- [ ] Calendar: mark/indicate days that already have entries (would read `Page.journal_date`, which is populated for exactly this)

## Performance
- [ ] True **list virtualization** in the journal feed (v1 keeps all loaded days mounted)
- [ ] Move SQLite writes off the UI thread (background executor) — **fsync stall handled** for now via WAL + `synchronous = NORMAL` in `Db::open` (per-keystroke autosave no longer fsyncs on the UI thread; measured worst case ~1.2 ms at a 50k-char page, well within a frame). The full off-thread refactor is now a lower-priority fast-follow (pathological pages / slow or contended disks)

## App & polish
- [ ] **Visual design pass** — make the UI look professional and easy on the eyes (spacing, typography, color, density)
- [ ] Sidebar: remember the collapsed state across launches, and add a keyboard shortcut to toggle it
- [ ] Multi-window: **drag a tab between existing windows** — gpui's drag-and-drop is per-window (the OS captures the mouse to the source window), so a tab can't be dragged from one window's strip into another's. Cross-window *moves* already work via the right-click "Open in new window" menu; true cross-window dragging is possible but clunky (done in Baudrun) — revisit
- [ ] Multi-window: same-page **concurrent edits** are last-write-wins — editing the *same* page/day in two windows at once can drop one side's changes. True resolution needs a CRDT/OT layer (out of scope for a single-user app); revisit only if real-time collaboration is ever wanted
- [ ] Window-bounds persistence (reopen where you left off)
- [ ] Add a `LICENSE` file (Cargo.toml already declares `GPL-3.0-or-later`)

## Import & export
- [ ] Logseq import follow-ups: an in-progress indicator with real progress (it's a bare "may take a minute" dialog today); surface imported pages in the sidebar right away (a fresh DB shows "No recent pages" until things are visited); consider importing whiteboards/draws as attachments
- [ ] **Print / PDF export** — generate a PDF from a note (`oxidize-pdf` can generate; or a typeset path like typst/`printpdf`)
- [ ] PDF: **fit-width / fit-page** zoom modes (zoom is free-scale only today)
- [ ] PDF: **area (image-region) highlights** — only text-anchored highlights exist so far; a box-drag over a scanned region would cover figures / pages with no text layer
- [ ] PDF: **garbled quotes from decorative fonts** — some heading fonts decode to shifted/garbled unicode (e.g. a −29 glyph shift), so a highlight on them stores garbled text (it still re-locates, since garbled matches garbled); body text is correct. Upstream hayro limitation
- [ ] PDF: **graceful fallback for unsupported files** — encrypted PDFs now open behind a password prompt (RC4 / AES-128 / AES-256), but hayro can still fail on an *unsupported* encryption algorithm (e.g. a public-key / certificate handler) or exotic transparency / blend modes; on such a load/parse failure, show an "Open in default app" affordance (hand off to the OS viewer) instead of a blank pane
- [ ] PDF: **AcroForm + annotations** — no pure-Rust crate does a full interactive forms/annotation engine. Heavy options reintroduce a native dep: `pdfium-render` (PDFium — full forms/annotations/render, permissive license) or `mupdf-rs` (full, but AGPL + native). Pure-Rust path: a targeted subset on `lopdf` — read `/AcroForm /Fields`, fill text fields/checkboxes via `/V` (+ `/NeedAppearances`), and render existing annotation appearance streams (`/AP /N`, which are XObjects hayro may already rasterize). First check whether hayro already composites `/AP` streams

## gpui-markdown crate
- [ ] Extract editor features (e.g. the slash menu) into a reusable crate if they generalize
- [ ] Publish to crates.io once the API is stable
- [ ] **Split the reusable crates (`gpui-markdown`, `gpui-pdf`) into their own repos** so outside contributors don't have to fork/clone all of zorite to contribute — **defer until the first stable release**. Gotcha to plan for: both pin `gpui = { git = ".../zed" }` with no rev and rely on the *workspace's single lockfile* to unify everything to one gpui; in separate repos each gets its own lockfile, and a gpui-rev mismatch puts two gpui versions in one build (won't compile), so the revs must be kept in lockstep. Extraction is cheap and lossless when the time comes — `git subtree split -P crates/<name>` carries each crate's history into the new repo. (crates.io publishing stays blocked regardless, since gpui is a git-only dep.)

## Completed

### Editor & rendering
- [x] **Click-to-caret** — clicking the rendered page **or a journal day** enters edit mode with the caret on the clicked character (empty space → end of the nearest line). gpui-markdown records a rendered→source byte-offset map while rendering (handling stripped `[[ ]]` / `#` / inline-code markup) and resolves a click via gpui's text layout (`index_for_position`); the host places the editor caret (`set_cursor_position`). To keep the clicked line under the cursor (the source layout is more compact than the rendered one), gpui-markdown reports the click's window-y and the host *predicts* the caret's row with the same `LineWrapper` soft-wrap math the editor uses (`predict_caret_row` — mirrors the input's 1.25 rem line height + paddings), jumping the scroll in the same frame so the editor's first paint is already aligned; a stability-gated verify pass (`align_caret_to_click`) mops up drift and rejects the editor's stale first-paint bounds. Near the document top the jump clamps to 0 — the page stays put and the caret just lands visibly. See `crates/gpui-markdown` (`on_click_source`) + `AppView::edit_page_at_offset` / `edit_day_at_offset`
- [x] **Find in page** (`⌘F`) — a find bar above a named page searches the **rendered** text (not the editor, which clashes with click-to-edit): every match highlights, the active one emphasized, with an *n / m* count; Enter/⇧Enter or ↑/↓ step (scrolling the match into view), Esc closes. `⌘⇧F` focuses the global note search; the journal feed defers to it. The search core lives in **`gpui-markdown`** — a reusable, db-free `find_matches` + `MarkdownView::search` / `track_blocks` (operates only on the source string) — with the find bar, shortcuts, and scroll in the host. See `src/ui/page_view.rs`, `crates/gpui-markdown/src/lib.rs`
- [x] **Configurable date/time format** — a **Settings → General** pane chooses the date (ISO / US / European / long / day-month-year) and time (24-hour / 12-hour) styles used by `/date`, `/time`, and the `{{date}}` / `{{time}}` placeholders; persisted, ISO + 24-hour by default. Date helpers consolidated into `src/dates.rs`; journal day headers keep their own long format. See `src/dates.rs`, `src/settings.rs`
- [x] **As-you-type completion** — `[[` (pages, with a "Create" entry), `#` (tags), and `{{` (template placeholders); reuses the slash popup, ranks matches, and caps the list so it stays usable with many pages
- [x] **Auto-pair brackets/quotes** (`()` `[]` `{}` `<>` `""` `''`) with type-over and prose-safe guards (contraction-aware quotes, `<` only after a word); confirming a `[[`/`{{` completion absorbs the auto-inserted closer
- [x] Auto-pair: **wrap the selection** — typing an opener with text selected wraps it (`foo` → `(foo)`); done in the change handler by diffing against the prior text, no key-level interception needed
- [x] Auto-pair: **backspace deletes an empty pair** (`(|)` + backspace → remove both)
- [x] **Inline image rendering** — standalone `![](path-or-url)` images render for real (async, aspect-ratio preserved, capped to content width); an image at the start of a paragraph renders with trailing text as a caption below
- [x] **Note-image memory** — local images go through `images::ImageStore`: decoded **downscaled to display size** (`DynamicImage::thumbnail`, longest edge ≤ 2048 — a 12 MP phone photo is ~12 MB of RGBA instead of ~47 MB) into a GPU-ready `RenderImage`, decoded **one at a time** (a serialized queue, so only one full-res decode buffer is ever alive), and **freed on view change** (`cx.drop_image` for CPU + GPU atlas, like the PDF viewer — gpui never auto-evicts a `RenderImage`). Before this, a note's photos decoded at full native resolution and were never released, so RAM climbed without bound as you browsed photo pages (the synthetic perf DBs had no real images, so it only surfaced after the Logseq import). Remaining: `image` 0.25 can't DCT-decode JPEGs at reduced size, so a full-res buffer is still briefly allocated and macOS's allocator keeps it as a bounded ~50 MB reclaimable cache (`MALLOC_LARGE (empty)`); a frugal/zune decoder path could remove even that. See `src/images.rs`, `AppView::ensure_image_loaded` / `pump_image_decodes` / `release_images`
- [x] Image **resize** — drag the corner handle (live preview); persists as `![](src){width=N}` in the markdown
- [x] Image **insert** — paste from clipboard (`Cmd+V`) or drag-and-drop a file; copied into the data-dir `images/` folder and referenced relatively
- [x] **Task-list checkboxes** (`- [ ]` / `- [x]`) — rendered via mdast `ListItem.checked` (the field does exist after all)
- [x] `gpui-markdown` now covers CommonMark + GFM: footnotes, reference-style `[text][id]` links/images, and raw HTML (shown literally)
- [x] `/time` and `/date` slash commands — insert the current time/date directly (distinct from the `{{time}}` / `{{date}}` *template* placeholders, which only expand inside a template)

### Notes & navigation
- [x] **Page rename** (and rewrite `[[links]]` pointing at it) — right-click → Rename page → dialog; `db.rename_page` rewrites links in a transaction
- [x] **Page hierarchy** via `[[parent::child]]` — Logseq-style: the `::` path *is* the page title, so the sidebar tree and each page's "Sub-pages" index are derived from titles (no parent column). Intermediate namespace segments show as virtual nodes and materialize on click. See `src/hierarchy.rs`
- [x] **Page aliases** — a subdued `alias::` field under the page title takes a comma list of alternate names; `[[name]]` then resolves to that page (exact title wins). Stored in a `page_aliases` table; resolution lives in `get_or_create_page`, so links and backlinks follow it
- [x] **Sidebar shows recent pages** — the page tree is capped to the last 10 *viewed* named pages (persisted in `settings`; seeded from the most-recently-edited pages on first run). Reach the rest via search
- [x] **Type-aware search** — the global search returns the PDF and image *files* referenced in notes, not just pages. A `pdf:` / `img:` / `page:` prefix (or a results-pane chip with a live per-kind count) filters by kind; `pdf:` / `img:` with no term browses every file in the managed `pdf/` / `images/` store. A PDF hit opens the viewer, an image opens the page showing it, a page opens the page. Files are extracted from the FTS-matched pages (`gpui_markdown::images` + the wiki-link index) rather than a separate file index. See `src/search.rs`, `src/ui/search.rs`
- [x] Journal: jump-to-date — a sidebar calendar date picker opens any day (creating it if needed)

### Performance
- [x] **Lighter `list_pages`** — the page list loads `id`/`title` only (not content): ~4× faster and memory-flat at scale (50k pages: 103 ms → 28 ms; RAM ~flat 10k→50k). See the [Performance](README.md#performance) section
- [x] **Full-text search index** — a trigram FTS5 index over page title + content (external-content, kept in sync by triggers) replaces the old `LIKE` table scan: same case-insensitive *substring* matching, now indexed so it scales. Migration `v4→v5` populates existing pages; queries < 3 chars (trigram's minimum) fall back to LIKE. See `src/db.rs`

### Data & migrations
- [x] **Back up before migrating** — `Db::open` snapshots the database to `zorite.db.bak-v<N>` (WAL-checkpointed first, so the copy is complete) before any schema upgrade, so a buggy migration is recoverable. One snapshot per source version. See `Db::backup_before_migration`
- [x] **Transactional migrations** — every step (`v0→2`, `v2→3`, `v3→4`, `v4→5`) now runs inside a transaction, so a mid-migration failure rolls back cleanly instead of leaving a half-migrated DB
- [x] **Friendlier migration failure** — a failed open no longer silently drops the user into blank notes: it falls back to a temporary in-memory store and shows a one-time dialog explaining what happened, with **Reveal Backup** (points at the `.bak-v<N>` snapshot) and **Quit**. See `AppView::show_db_error_dialog`

### App & polish
- [x] **Collapsible sidebar** — a `<` caret collapses it to a thin icon rail (`>` to expand, plus the calendar/settings icons); the content area reclaims the space
- [x] **Multi-window** — right-click a sidebar page or a tab → "Open in new window" opens a full, independent second window focused on that page (`AppView::open_in_new_window`). Each window is its own `AppView` with its own SQLite connection to the same file. See `src/app.rs`, `src/ui/tab_bar.rs`
- [x] Multi-window: **drag a tab out to tear off a new window** + reorder within the strip — both work (`tear_off_tab` on a drop in the content area, `reorder_tab` on a drop over a tab). Right-click "Open in new window" *moves* the tab (tears it off) rather than duplicating. Wayland: the compositor controls new-window placement
- [x] Multi-window: **live cross-window sync** — a shared `DocSignal` (gpui global, one per process) is emitted on content saves (`save_page_content`) AND structural changes (create / rename / delete + the blur link re-index); other windows run `apply_external_edit`, reloading changed journal days, the active page (content + backlinks), and the sidebar page-list (via value-comparison, so only stale data is touched and the editing window never clobbers itself)
- [x] **App icon + packaging** — cargo-packager builds `.app`/`.dmg`, NSIS `.exe`, `.deb`/`.AppImage`, `.rpm`, etc.; a custom app icon ships (Windows PE icon embedded via `build.rs`; `.icns`/`.ico`/`.png`). See `Cargo.toml` `[package.metadata.packager]`
- [x] **CI** — GitHub Actions builds + `cargo test --workspace` across macOS / Windows / Linux (5 runners) and publishes a prerelease on `vX.Y.Z-beta.N` tags. See `.github/workflows/`
- [x] **Keyboard shortcuts + menu bar** — standard cross-OS shortcuts via gpui's `secondary-` (Cmd on macOS / Ctrl elsewhere): New Tab `⌘T`, New Window `⌘N`, Close Tab `⌘W`, Settings `⌘,`, Quit `⌘Q`, Next/Prev Tab `Ctrl+Tab` / `Ctrl+Shift+Tab`. Native macOS menu bar (zorite / File / Edit / View) via `cx.set_menus`; the Edit menu reuses gpui-component's input actions. A read-only **Settings → Keyboard** section lists them all. `⌘F` finds in the current page and `⌘⇧F` focuses the global search (see **Find in page**). See `src/actions.rs`, `src/main.rs`

### Import & export
- [x] **Logseq import** — `File → Import from Logseq…` picks a graph folder and imports `pages/` + `journals/` (whiteboards, draws, config, `bak`/`.recycle` are skipped). Namespaces map to zorite's (`Budget___2024.md` files and `[[Budget/2024]]` links → `Budget::2024`); the all-bullets outline converts per a user choice — **flatten** (top-level bullets → paragraphs/headings, children stay lists) or **keep bullets**; `TODO`/`DONE`/`CANCELED` → task checkboxes; Logseq-internal properties (`id::`, `collapsed::`, …) drop while user properties stay as text; `title::`/`alias::` feed the page title and alias table; `{{video}}`/`{{embed}}`/`((block-ref))` resolve and queries stay visible as inline code; glued code fences (` ```cfg… `) are normalized; assets copy into `images/` + `pdf/` (percent-decoded, sanitized names) with refs rewritten (`[[pdf/x.pdf]]` chips); Logseq `hls__*` PDF-highlight pages convert to zorite's `<name>.pdf (highlights)` format. Existing pages/days keep their content (import appends below, reported in the summary). Runs on a background thread against its own DB connection; `ZORITE_DATA` env (new) isolates a whole test data set. **Extensible by design**: each source is a *reader* module producing a source-agnostic `ImportBundle` (pages, journal days, asset copies), and one engine (`import::write_bundle`) owns the collision policy, link/alias indexing, and asset copying — adding Obsidian/Notion/… is a new reader plus a menu entry (see the "Adding an importer" doc in `src/import/mod.rs`). See `src/import/` (engine + `logseq.rs` reader, both unit-tested end to end), `AppView::on_import_logseq`
- [x] In-app **PDF viewer** — `[[file.pdf]]` / `![](file.pdf)` open a dedicated viewer tab (`ui::pdf_view`); pages are sized from `render_dimensions()` for instant layout. Closing the tab frees both the CPU images and their **GPU atlas textures** (`cx.drop_image` — raw `RenderImage`s are never auto-evicted; this was an ~140 MB/open leak). See `src/pdf.rs`
- [x] PDF: **viewport virtualization** — only the on-screen pages (±2) are rasterized; far ones are evicted (image + GPU texture), so memory is bounded by the viewport, not the page count (`AppView::ensure_pdf_window` + `pdf_view::keep_window`). Verified: scrolling a 32-page PDF end-to-end holds ~178 MB vs 403 MB before
- [x] PDF: **DPI-aware render scale** — pages rasterize at display pixel-ratio × zoom × a host **quality** multiplier (no longer a fixed 1.5×); a render-quality slider in Settings trades sharpness for speed (default 75%), read reactively so open viewers re-render live
- [x] PDF: **zoom + page navigation** — − / + / reset (⌘= / ⌘- / ⌘0) and ‹ / › with a click-to-edit page-number input (+ PageUp / PageDown / Home / End); no blank on zoom/quality change (the old bitmap stays, rescaled, until the crisp one lands)
- [x] PDF: **extracted to a reusable [`gpui-pdf`](crates/gpui-pdf/README.md) crate** — host-agnostic primitives + a self-contained `PdfView` component; markup is behind an optional `markup` feature
- [x] PDF: **Logseq-style text markup** — drag-to-highlight in the viewer writes a reference block (`- pN: quote {color} [[file.pdf#pN|↗]]`) on a per-PDF "(highlights)" page; clicking the ↗ opens the PDF and scrolls to + **flashes** the highlight. Done **dep-free** — a custom hayro `Device` extracts text + glyph rects (only `kurbo`, *not* oxidize-pdf). Has a **color picker** (yellow/green/blue/pink/orange) and header **tooltips**
- [x] PDF: **find-in-PDF** — a browser-style find bar (🔍 / ⌘F) over the text layer: type to search the whole document, matches boxed + a focused one outlined, `n / N` count, Enter/⇧Enter to step (scroll-to), Esc to close. Behind a `search` feature (= `["markup"]`, shares the text layer). See `gpui-pdf` `find_matches` + `src/pdf.rs`
- [x] PDF: **outline / table-of-contents** — reads `/Catalog /Outlines` via `hayro-syntax` into `gpui_pdf::outline()`; a toggle (≡, by the title) opens a navigable, level-indented TOC panel that jumps to each page (hidden when there's no outline). Also extracts `/Link` annotations (`page_links`) → clickable in-page links (internal jumps + external URLs), and bulleted highlight selections store as a nested markdown list. See `crates/gpui-pdf/src/outline.rs`
- [x] PDF: **password-protected files** — an encrypted PDF opens behind a prompt (masked field, Enter or a button to submit, "incorrect password" on a wrong try) instead of a blank pane; hayro 0.7.2 decrypts RC4 / AES-128 / AES-256 (standard security handler) via `Pdf::new_with_password`. The prompt UI lives in the app so `gpui-pdf` stays gpui-component-free: `PdfView` keeps the bytes, reports `is_locked()` / `unlock_failed()` and retries via `unlock(password)`, emitting `PdfEvent::LockChanged` so the host swaps prompt ↔ viewer. Unsupported handlers (public-key / certificate) still need the fallback above. See `crates/gpui-pdf` (README + `lib.rs`), `src/app.rs`
