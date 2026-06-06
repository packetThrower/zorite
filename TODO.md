# zorite — TODO

Roadmap / known follow-ups. Roughly priority-ordered within each section.

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
- [ ] Window-bounds persistence (reopen where you left off)
- [ ] App icon + packaging (cargo-packager: `.dmg` / `.msi` / `.deb`)
- [ ] Add a `LICENSE` file (Cargo.toml already declares `GPL-3.0-or-later`)
- [ ] CI (build + `cargo test --workspace`)

## Import & export
- [ ] **Logseq import** — bring in existing Logseq pages/journals (markdown)
- [ ] **Print / PDF export** — generate a PDF (e.g. a markdown-to-PDF crate)
- [ ] In-app **PDF rendering / embedding** (large project)

## gpui-markdown crate
- [ ] Extract editor features (e.g. the slash menu) into a reusable crate if they generalize
- [ ] Publish to crates.io once the API is stable
