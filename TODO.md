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
- [ ] **Page hierarchy** via `[[parent::child]]` — nest a page under a parent; the parent page acts as an index (linked list) of its children
- [ ] **Page aliases** via `::alias` — resolve `[[alias]]` to the page (design note: disambiguate `::` here vs. the `parent::child` hierarchy syntax above)
- [ ] Unlinked references (mentions of a page title without `[[ ]]`)
- [ ] Journal: jump-to-date / calendar (the `Page.journal_date` field is already populated for this)

## Performance
- [ ] True **list virtualization** in the journal feed (v1 keeps all loaded days mounted)
- [ ] Move SQLite writes off the UI thread (background executor)

## App & polish
- [ ] **Visual design pass** — make the UI look professional and easy on the eyes (spacing, typography, color, density)
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
