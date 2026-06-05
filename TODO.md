# zorite — TODO

Roadmap / known follow-ups. Roughly priority-ordered within each section.

## Editor & rendering
- [ ] Real inline **image rendering** (images currently render as clickable links; needs async image loading)
- [ ] Slash menu: **click-to-insert** a command (keyboard-only today; needs to avoid blurring the editor on click)
- [ ] **Task-list checkboxes** (`- [ ]` / `- [x]`) — the `markdown` 1.0 crate's `ListItem` has no `checked` field, so this needs a different parser or post-processing
- [ ] Broaden `gpui-markdown` coverage: footnotes, nested-list edge cases
- [ ] Place the caret at the click point when entering edit mode (currently keeps the last position)
- [ ] `/time` and `/date` slash commands — insert the current time/date directly (distinct from the `{{time}}` / `{{date}}` *template* placeholders, which only expand inside a template)

## Notes & navigation
- [ ] **Page rename** (and rewrite `[[links]]` pointing at it)
- [ ] **Page hierarchy** via `[[parent::child]]` — nest a page under a parent; the parent page acts as an index (linked list) of its children
- [ ] **Page aliases** via `::alias` — resolve `[[alias]]` to the page (design note: disambiguate `::` here vs. the `parent::child` hierarchy syntax above)
- [ ] Unlinked references (mentions of a page title without `[[ ]]`)
- [ ] Journal: jump-to-date / calendar

## Performance
- [ ] True **list virtualization** in the journal feed (v1 keeps all loaded days mounted)
- [ ] Move SQLite writes off the UI thread (background executor)

## App & polish
- [ ] **Visual design pass** — make the UI look professional and easy on the eyes (spacing, typography, color, density)
- [ ] Window-bounds persistence (reopen where you left off)
- [ ] Light theme / theme switching
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
