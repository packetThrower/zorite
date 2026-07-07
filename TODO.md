# Zorite — TODO

Roadmap / known follow-ups. Roughly priority-ordered within each section. Finished
work is collected under [Completed](#completed) at the bottom.

## Contents

- [Notes & navigation](#notes--navigation)
- [Notebooks (multiple data folders)](#notebooks-multiple-data-folders)
- [Performance](#performance)
- [App & polish](#app--polish)
- [Settings window](#settings-window)
- [Import & export](#import--export)
- [Crates](#crates)
- [Maybe](#maybe)
- [Completed](#completed)

## Notes & navigation
- [ ] Aliases: offer a page's aliases as suggestions in `[[` autocomplete
- [ ] Block references: **"Copy block link"** — auto-generate a ` ^id` on a line
  (right-click / command) and put `[[Page#^id]]` on the clipboard, so linking to
  a block doesn't require inventing an id by hand
- [ ] Embeds: the box **height estimate undershoots** for image/math/mermaid-heavy
  content (it's a line-count heuristic — `ensure_content_embeds`), so those boxes
  scroll more than they should; measure or estimate rendered heights instead
- [ ] Properties: **typed values** (list / date / number) — today every value is
  text; types would enable sorting/filtering on the Properties page and smarter
  pills

## Notebooks (multiple data folders)

Obsidian-style multiple "vaults" — separate, self-contained data sets the user
switches between (work / personal / a shared folder in Dropbox). **Not called
vaults**; working name **Notebooks** (alternatives considered: Spaces,
Workspaces, Collections). **Held for 0.7.0** (planned 2026-07-06; not in the
next release).

**Why this is cheaper than it sounds — what already exists:**
- A data dir is already a fully self-contained bundle: `zorite.db` + `images/`
  + `pdf/` + `themes/` + `fonts/` + the window-bounds sidecar. Nothing lives
  outside it except the location-pointer file (`data_location.json`, fixed home
  = the OS-default dir). Settings, favorites, recents, theme — all in the DB,
  so they're per-notebook for free.
- `paths.rs` already points the app at an arbitrary dir: `plan_relocation`
  distinguishes **Switch** (target already holds a `zorite.db` → point at it in
  place) from **Move** (relocate current data) — so "open a different data set"
  exists internally today; it lacks only a registry, a switcher UI, and a
  restart hook. `ZORITE_DATA` proves the isolation (it's how all live testing
  runs).
- gpui has `cx.restart()` (Zed's updater uses it) — a clean relaunch is
  available for switch-by-restart.

**Phase 1 — registry + switcher, switch = relaunch:**
- [ ] Extend `data_location.json` into a registry: `{active, notebooks:
  [{name, dir}]}`. Serde compat both ways (old builds ignore unknown fields;
  `#[serde(default)]` reads old files). First launch after the update
  auto-registers the current dir as **"Main"**.
- [ ] **Switcher at the bottom of the sidebar** (user-picked spot): a compact
  chip showing the active notebook's name; clicking opens a popover — the
  notebook list (✓ on active, click to switch), **New notebook…** (name + a
  folder picker; seeds a fresh empty dir), **Add existing…** (pick a folder
  that holds a `zorite.db`), right-click → rename / **remove from list**
  (forgets the entry, never deletes files) / Reveal in Finder. Hide the chip
  (or show "Main" quietly) when only one notebook is registered.
- [ ] Switch = write `active` to the pointer file + `cx.restart()`. An
  encrypted target notebook lands on its unlock screen naturally, and restart
  sidesteps the Windows zero-window-exit gotcha entirely.
- [ ] Window title gains the notebook name when more than one is registered.
- [ ] Settings → General's existing "data location" pane folds into this
  (Move becomes a per-notebook action; Switch is superseded by the registry).
- [ ] `ZORITE_DATA` keeps top precedence (dev/test), bypassing the registry.

**Phase 2 — restartless switching / notebooks open side-by-side (only if
Phase 1 proves demand):**
- The crux: `paths::data_dir()` is a process-wide `OnceLock` and every store
  resolves through it — per-window notebooks means threading a data-dir
  context through every `paths::*` call site (the big refactor).
- Per-notebook `DocSignal` (today one process-global would cross-sync
  different notebooks' windows), block cross-notebook tab drags
  (`GlobalAppWindows`), audit per-`AppView` caches (image/mermaid/highlight
  stores are per-window already; the parse cache is content-keyed, safe).

**Out of scope** (matching Obsidian): cross-notebook links/search/embeds,
per-notebook settings sync.

## Performance
- [ ] Move SQLite writes off the UI thread (background executor) — **fsync stall handled** for now via WAL + `synchronous = NORMAL` in `Db::open` (per-keystroke autosave no longer fsyncs on the UI thread; measured worst case ~1.2 ms at a 50k-char page, well within a frame). The full off-thread refactor is now a lower-priority fast-follow (pathological pages / slow or contended disks)

## App & polish
- [ ] **Visual design pass** — make the UI look professional and easy on the eyes (spacing, typography, color, density)
- [ ] Sidebar: remember the collapsed state across launches, and add a keyboard shortcut to toggle it
- [ ] Multi-window: same-page **concurrent edits** are last-write-wins — editing the *same* page/day in two windows at once can drop one side's changes. True resolution needs a CRDT/OT layer (out of scope for a single-user app); revisit only if real-time collaboration is ever wanted

## Settings window
- [ ] Use small versions of components

## Import & export
- [ ] Logseq import follow-ups: an in-progress indicator with real progress (it's a bare "may take a minute" dialog today); surface imported pages in the sidebar right away (a fresh DB shows "No recent pages" until things are visited)
- [ ] PDF: **fit-width / fit-page** zoom modes (zoom is free-scale only today)
- [ ] PDF: **area (image-region) highlights** — only text-anchored highlights exist so far; a box-drag over a scanned region would cover figures / pages with no text layer
- [ ] PDF: **garbled quotes from decorative fonts** — some heading fonts decode to shifted/garbled unicode (e.g. a −29 glyph shift), so a highlight on them stores garbled text (it still re-locates, since garbled matches garbled); body text is correct. Upstream hayro limitation
- [ ] PDF: **graceful fallback for unsupported files** — encrypted PDFs now open behind a password prompt (RC4 / AES-128 / AES-256), but hayro can still fail on an *unsupported* encryption algorithm (e.g. a public-key / certificate handler) or exotic transparency / blend modes; on such a load/parse failure, show an "Open in default app" affordance (hand off to the OS viewer) instead of a blank pane
- [ ] PDF: **a failed load is silent and permanent** (found in the 2026-07-06 API audit) — an unreadable file or malformed PDF only `log::error!`s and `PdfView` sits on the "Loading PDF…" placeholder forever, with no error state, event, or retry; and a retry-unlock failing with `LoadError::Other` (e.g. unsupported encryption discovered at unlock time) is logged but **eventless**, so the password prompt gets no signal. Both want an explicit error state + `PdfEvent`; overlaps with the graceful-fallback item above
- [ ] PDF: `is_pdf` misses query-string refs (`report.pdf?v=2`) — it only checks `ends_with(".pdf")` after trimming trailing whitespace (API audit)
- [ ] PDF forms, follow-ups — the AcroForm feature SHIPPED 2026-07-06 (see
  Completed): remaining niceties are **choice-field dropdowns** (Ch fields
  edit as free text today; `FormField::options` already carries `/Opt`),
  synthesized-appearance fidelity (`/DA` fonts, `/Q` quadding, comb fields,
  multiline), and **filing the two hayro gaps upstream** (state-dict `/AP /N`
  selected by `/AS`; `NeedAppearances` synthesis) so the lopdf normalization
  pass can eventually retire.

## Crates
Crate-internal defects and API hygiene, mostly surfaced by the 2026-07-06
public-API audit (every crate now carries a complete `API.md`; these are the
findings worth fixing rather than just documenting):
- [ ] `ratex-gpui`: **duplicate `"angle"` command** in the `input.rs` `COMMANDS`
  table — the ⟨⟩ delimiter pair shadows the `\angle` symbol entry (first-match
  lookup), so the symbol is unreachable by name; rename one (e.g. `langle`
  `rangle` for the delimiters, matching LaTeX)
- [ ] `ratex-gpui`: `MathEditor` rasterizes at a **hard-coded `dpr: 2.0`**
  (`view.rs` `with_root`) instead of the window's scale factor — slightly soft
  on 1× displays, wasteful on 3×
- [ ] `os-spellcheck`: the Windows backend creates its checker for a
  **hardcoded `en-US`** — follow the system UI language
  (`GetUserDefaultLocaleName`), falling back gracefully when unsupported
- [ ] `gpui-whiteboard`: `Font::layout_wrapped` / `layout_styled` are `pub` but
  return **crate-private types** (unnameable outside — `layout_styled` is
  effectively uncallable externally); demote to `pub(crate)` or export the types
- [ ] `gpui-whiteboard`: the `PasteFn` doc comment in `lib.rs` says keyboard ⌘V
  is handled internally, but ⌘V actually routes through `on_paste` (with `None`
  deliberately propagating so a clipboard image can paste) — fix the comment to
  match `API.md`
- [ ] Extract editor features (e.g. the slash menu) into a reusable crate if they generalize
- [ ] Publish to crates.io once the API is stable
- [ ] **Split the reusable crates (`gpui-markdown`, `gpui-pdf`) into their own repos** so outside contributors don't have to fork/clone all of Zorite to contribute — **defer until the first stable release**. Gotcha to plan for: the crates use the workspace's pinned gpui rev (`[workspace.dependencies]`, one spec byte-for-byte); in separate repos each picks its own rev, and a mismatch puts two gpui versions in one consumer's build (won't compile), so the revs must be kept in lockstep. Extraction is cheap and lossless when the time comes — `git subtree split -P crates/<name>` carries each crate's history into the new repo. (crates.io publishing stays blocked regardless, since gpui is a git-only dep.)

## Maybe

Ideas worth keeping, not yet committed to.

- [ ] **nixpkgs submission IN REVIEW** — NixOS/nixpkgs#539400 (opened
  2026-07-07): `pkgs/by-name/zo/zorite` at 0.6.0, single `cargoHash`
  (fetchCargoVendor covers the git deps — NOT per-source outputHashes),
  `nix-update-script` so r-ryantm auto-bumps releases, maintainer entry
  for packetThrower, AI-policy disclosure included. Both platforms
  CI-verified pre-submission. When it MERGES: delete `nixpkgs-staging/`
  + the temporary `nixpkgs-staging` CI job from nix.yml, and respond to
  reviewer feedback in the meantime.

- [ ] **MCP server** — let Claude (Desktop / Code) read and eventually write the
  journal. An external agent's draft prompt proposed a standalone binary doing
  direct SQLite writes — analyzed 2026-07-06 and rejected as-written: the app
  autosaves per keystroke from an in-memory copy, so an external write to an
  open day is silently clobbered (no conflict detection; `DocSignal` is
  in-process); external writes also skip the `page_links` reindex, alias/
  collision handling, and the `kind` column; a SQLCipher-encrypted DB can't be
  opened at all; and hardcoded platform paths ignore `data_location.json` +
  `ZORITE_DATA`. The FTS index alone would survive (trigger-maintained).
  **Phase 1 (safe): read-only sidecar.** **Phase 2: writes only via the running
  app** (in-process MCP endpoint or a stdio shim over a local socket, so saves
  run `save_page_content` → link reindex → `DocSignal`). Corrected prompt for
  Phase 1:

  > Add a `zorite-mcp` binary to the Zorite workspace (a new workspace member
  > following AGENTS.md — fmt/clippy `-D warnings`/test gate, cross-platform,
  > no native deps): a **read-only** MCP server over **stdio** using the
  > official Rust SDK (`rmcp` — modelcontextprotocol/rust-sdk) and `rusqlite`.
  >
  > Open the database read-only (`mode=ro`, `busy_timeout`); WAL makes
  > cross-process reads safe (the app itself opens one connection per window).
  > Resolve the data dir exactly like `src/paths.rs`: `ZORITE_DATA` env →
  > the `data_location.json` pointer in the OS-default dir → platform default.
  > If the file starts with the SQLCipher header (not `SQLite format 3\0`),
  > return a clear "database is encrypted — the MCP server can't read it"
  > error; likewise clean JSON-RPC errors for a missing file.
  >
  > Use the real schema (see `src/db.rs`, schema v9): `pages(id, title,
  > is_journal, journal_date, content, created_at, updated_at, kind)`,
  > `page_links(source_id, target_id)`, `page_aliases`, and the
  > external-content trigram FTS5 table `pages_fts`. No placeholders.
  >
  > Tools (all read-only):
  > - `list_pages` — id/title/kind/updated_at only (never bodies); filter
  >   `kind = 'page'` by default, flag whiteboards.
  > - `get_page` — body by title (case-insensitive, alias-aware via
  >   `page_aliases`) or by `journal_date` for a day; label whiteboard JSON
  >   rather than returning it as markdown.
  > - `search` — `pages_fts` MATCH for queries ≥ 3 chars (trigram minimum),
  >   `LIKE` fallback below that, exactly like the app's `search_pages`.
  > - `get_backlinks` — join `page_links` (the indexed table, not a markdown
  >   scan); note it reflects the last app-side save.
  >
  > Resources: `journal://today` (and `journal://YYYY-MM-DD`) → that day's
  > markdown; `journal://tags` → distinct `#tags` extracted from content with
  > the shared `gpui_markdown::syntax::links` grammar (tags are inline, not
  > "properties").
  >
  > **No write tools in this phase** — writes are unsafe while the app runs
  > (per-keystroke autosave clobbers external edits) and belong to a later
  > in-app MCP endpoint. Ship with compile instructions plus
  > `claude_desktop_config.json` / `claude mcp add` snippets, and a README +
  > API.md per the crate-docs convention.

## Completed

### Obsidian parity (0.6.0)
- [x] **Properties (`key:: value` anywhere)** (PR #32) — any-line properties render as a two-column panel (per-key icons, `#tag` / `[[link]]` values as clickable pills, hover highlight) in BOTH views; an **in-place property editor** seated in the note (click or arrow in; key dropdown fed by every key in the vault; full keyboard nav; writes `key:: value` back on blur); and a **Properties index page** (All pages → Properties): every key with its values + pages, icon overrides / pre-mapping from a picker, and rename-a-key-across-the-vault. Recognition shared in `gpui_markdown::syntax`; `alias::` keeps its `page_aliases` DB resolution
- [x] **Block references & heading anchors** (PR #34) — ` ^block-id` gives a line an address; `[[Note#^id]]` and `[[Note#Heading]]` (case-insensitive) open the note scrolled to that line, in both views. Links read as `Note → anchor` (the raw `#^` / `#` renders as ` → `), the trailing `^id` marker hides outside the caret's line, `file.pdf#p3` and literal `#`-titled pages keep their meaning, and the link reindexer no longer spawns junk `Note#…` pages
- [x] **Transclusion / embeds (`![[note]]`)** (PR #34) — a standalone `![[Note]]` / `![[Note#Heading]]` / `![[Note#^id]]` line renders the target's content in a quoted box with a clickable source label, in BOTH views: hover scrollbar + wheel hand-off at the edges, live updates when the source page changes, full inner rendering (images read-only, math, mermaid, highlighted code, nested embeds capped at depth 3), `|alias` renames the label; caret on the line edits the raw text
- [x] **Foldable callouts** — Obsidian's fold char on an alert marker: `> [!NOTE]-` starts folded, `+` open; a chevron joins the title in both views, clicking folds/unfolds and persists the flip in the source (like a task checkbox), and the editor reveals a folded callout while the caret is inside
- [x] **Collapsible headings** (a8da34c) — hover a heading → chevron; click folds its section (to the next same-or-higher heading, fence-aware) in both views. Session-local view state (markdown has no fold syntax); keyed by heading text (self-heals on rename; duplicate headings fold together — known ceiling); editor reveals a folded section while the caret is inside its body
- [x] **Inline (in-flow) images** (PR #30) — an image that doesn't lead its line renders as a small in-flow thumbnail (height-capped, width-capped) instead of vanishing, in BOTH views; click opens a full-size preview, hover shows a hand
- [x] **Obsidian importer** (PR #31) — File → Import from Obsidian… reads a vault: folders → `::` namespaces (or flatten), links + aliases resolve through a name→title map, ~13 callout types → Zorite's 5 alerts, frontmatter → aliases/tags/`key:: value` properties, `YYYY-MM-DD` notes → journal days, assets copied into the managed stores. Block ids, `#Heading`/`#^id` anchors, and `![[embeds]]` come across **as-is** (they all work in Zorite now). **`.canvas` boards → whiteboards**: text cards as labeled boxes (colors mapped), note cards as clickable page cards (ids resolved at write time), image cards placed, groups as outlines, edges as arrows with labels; every 1:1 gap is warned in the import summary

### Export (unreleased)
- [x] **Export Notebook as Markdown** (2026-07-06) — `src/export_md.rs`, the
  importers' mirror: a pure `plan_export` (paths sanitized + case-insensitively
  uniquified, `::` → folders, map-driven link/embed rewriting preserving
  anchors + aliases, fence-aware; frontmatter aliases YAML-quoted; referenced
  assets collected via `all_image_srcs` + pdf-chip links) and a `write_export`
  that refuses non-empty destinations. Kept deliberately portable: `<mark>`,
  `{width=N}`, properties, callouts, block ids all pass through — no
  Obsidian-only conversions. Round-trip tested through our own Obsidian
  importer (`import(export(x)) ≈ x`); live-verified against a seeded coverage
  page.
- [x] **Whiteboards → `.canvas`** (2026-07-06) — the canvas importer's reverse:
  box shapes flatten to text cards (stroke color kept), page cards → file
  nodes at the exported note's path (board-to-board cards keep `.canvas`),
  images → file nodes (assets copied), lines/arrows → edges anchored to the
  nearest node side (24 px snap; arrowheads honored via `toEnd`). Freehand
  strokes and unanchored lines are counted in the summary. Round-trip tested
  back through `read_vault`.

### PDF forms (unreleased)
- [x] **AcroForm display + filling** (2026-07-06, spike -> M1 -> M2 -> M3 in a day) —
  the spike proved hayro already composites `/AP /N` appearance streams, leaving two
  gaps: state-dict appearances (checkboxes/radios) and `NeedAppearances` fields.
  **M1**: `forms` feature in gpui-pdf — a lopdf pass before parse resolves `/AS`
  state dicts and synthesizes missing text appearances (idempotent, encrypted
  files untouched, end-to-end render-tested). **M2**: `form_fields(bytes)`
  (qualified names, kinds, pages, ordered rects, values, options — verified on a
  real 54-field government form) + `set_form_value` (writes `/V` on the
  /FT-owning dict, per-widget `/AS` for button groups, regenerated appearances
  so output renders in every viewer; refuses read-only/signature). **M3**:
  fillable in the viewer — widgets overlay like link annotations (hover tint,
  pointer), `PdfEvent::FieldClicked` carries field + window bounds, checkboxes
  toggle instantly, text fields seat an app-side input BELOW the widget (field
  stays readable; caption with name + key hints), Enter/click-away commits, Esc
  cancels, Tab/Shift-Tab hops fields with `reveal_field` scrolling; writes go
  fs::read -> set_form_value -> fs::write -> `replace_bytes` (no-blanking hot
  swap, the zoom-change pattern — blanking flashed the window black on every
  toggle). Esc/Tab route through the app's existing Input-context actions
  (SlashCancel / InsertTab / Outdent), the same mechanism as the property editor.

### Editor & rendering (0.5.x)
- [x] **gpui-markdown becomes THE markdown crate; gpui-editor consumes it**
  (design agreed 2026-07-02, replacing the earlier third-crate idea). The two
  views recognize every construct separately and drift — links (fixed 0.4.1),
  alerts (recognized in 3 places incl. PDF export), math parse options. Plan:
  1. gpui-markdown owns *recognition* — construct detection + payloads
     (wiki/tag/url linkables, alert kinds + palette, table styles, heading
     scales) — exposed BOTH as mdast helpers and as line-level recognizers
     (the editor can't afford full parses per keystroke). The reader view
     moves behind a default-on `view` feature.
  2. gpui-editor depends on gpui-markdown (recognition only,
     default-features = false) — `markdown_syntax.rs` keeps the scanning
     shape but consumes shared definitions. AGENTS.md's "crates depend on
     gpui only" gains this one sibling exception.
  3. gpui-editor's whole markdown/WYSIWYG side moves behind a default-on
     `markdown` feature — it's a text editor first (ratex-gpui's `editor`
     feature is the precedent).
  **DONE** (2026-07-02) except one deliberate cut: `gpui_markdown::syntax`
  holds alerts, table styles, heading scales, AND the linkables (one grammar —
  unification caught live tag-rule drift and gave WYSIWYG bare-URL autolinks);
  the `view` feature ships (recognition-only builds are dependency-free, the
  editor consumes `default-features = false`). **Cut: the gpui-editor
  `markdown` feature** — ~102 integration points would need cfg or a 30-item
  stub mirror, while the benefit evaporated once recognition became a
  dependency-free module (unused markdown paths are dead-code-eliminated for
  consumers that never call `set_markdown_style`). "Text editor first" is
  documented in the crate README instead. Parity rules live in AGENTS.md.
- [x] Images: **orphan GC** — Settings → General → "Unused images" deletes `images/` files no page, whiteboard, or template references (substring scan of all content; files under an hour old kept for the autosave race)
- [x] Images: import dedupe — pastes AND dragged files reuse any existing store file with identical contents (size-narrowed byte compare), whatever its name; new pastes get content-addressed names (`pasted-<sha256/64bit>.<ext>`). Pre-existing duplicates aren't retro-deduped (would need reference rewriting)
- [x] Images: **AVIF/HEIC/HEIF** decode via the pure-Rust `heic_decoder` (commit 28a5ebd, PR #15) — EXIF orientation applied, rav1d runs on a big-stack thread. Known gap: grid-tiled primary items fall back to a placeholder

### Editor & rendering (0.5.0)
- [x] **WYSIWYG table: delete last row/column caret drop** — no longer reproduces (user-verified 2026-07-02); most likely resolved by the table measure/hit-box overhaul (shared `line_pads`, always-committed strip rects) that fixed the add-row "+" strip
- [x] **Shared construct recognition** (`gpui_markdown::syntax`) — alerts, table styles, heading scales recognized in ONE place; reader, WYSIWYG, and PDF export all consume it (phase 1 of the restructure above)
- [x] **View parity rounds** — reader ↔ WYSIWYG converged per the AGENTS.md parity rules: body line height 1.45 both (reader had gpui's phi default), content-hugging tables (WYSIWYG's measured columns, 22px gutter, row metric) and code cards (widest line, bold-measured for highlight runs), list spacing + indentation (reader's roomier look, WYSIWYG adopts), under-bullet nested-list guides (reader), HTML comments render nowhere
- [x] **GitHub alerts** (`> [!NOTE]` …) in both views + slash menu + PDF export, five themeable palette tokens, Lucide icons; lenient inline form accepted
- [x] **Syntax highlighting** for fenced code blocks in both views — gpui-component's tree-sitter highlighter (already in the binary), 22 grammars as Cargo features, one app-side cache, themes recolor live
- [x] **Custom fonts** — Settings → Appearance → Font (any installed family or an imported .ttf/.otf, persisted + re-registered at startup); themes can name a `"font"`; full per-token theme overrides (19 palette tokens, #RRGGBBAA)
- [x] **Text size setting** — 14–20px, one value drives all three views; exposed latent size-mismatch bugs (measure vs paint, table hit-testing) now fixed structurally

### Import & export (0.5.0)
- [x] **PDF export** — tab / sidebar right-click "Export as PDF…", File menu, and ⌘P for the active tab; a native save dialog, then `src/export.rs` renders the note's mdast straight to PDF via `oxidize-pdf` (pure Rust; ~10 small new crates with default features off). We own the layout: wrapped styled runs (bold/italic/code) with real font metrics, headings/lists/tasks/quotes/tables/code/footnotes/dividers, page breaks. Render-view fidelity: `$$` math rasterizes through RaTeX, mermaid through mermaid-rs + resvg (gpui's own SVG rasterizer), images of any decodable format via the `image` crate; control comments (`<!-- math:left -->`) never print. Journal tab exports its loaded days under date headings. Known v1 gaps (documented in `export.rs`): inline `$…$` stays as source unless it's a whole paragraph, emoji/CJK degrade under the standard PDF fonts, remote images skipped, quote bars don't span page breaks

### Editor & rendering (v0.4.1)
- [x] **Links in WYSIWYG** — wiki-links (`[[page]]`), tags (`#tag`), inline URLs (`[text](url)`), and PDF references (`[[file.pdf]]`) are now clickable in the WYSIWYG view, matching the reader. Recognition via `markdown_syntax::links()` + `LinkHit` enum (wiki / URL / email); on-click handlers (`EditorEvent::OpenLink` / `OpenWikiLink`) route to navigation or PDF open. Visual affordance: **hover hand cursor** over clickable regions (draw link-grip hitboxes during prepaint, set cursor on paint). See commits 35a95ad (link navigation + cursor), 4c35a09 (caret/image fixes)
- [x] **Images as Word-style atomic objects** — in WYSIWYG, images no longer expose markdown when the caret moves across them; instead, the caret parks **beside** the image (visually like a single unit). Backspace or Delete removes the image + its markdown row, and right-click opens a context menu with **"Delete image"**. Internally: `EditorEvent::DeleteImage` triggers `delete_image_row` (removes the image line), and caret positioning after drop places the caret on the line *below* the markdown to avoid the resize-grip interaction. See commits 4c35a09, 7d0d04e
- [x] **Images: on-drop rendering** — fixed the "hit Enter to make image render" bug. Root cause: `set_text` doesn't emit `EditorEvent::Changed`, so the image wasn't being scanned for initial load. Fixed by having `insert_image_markdown` call `ensure_image_loaded` directly, then the `EditorEvent::Changed` subscriptions handle subsequent rescans. See commit 4c35a09
- [x] **Caret placement after image drop** — caret lands on the line *below* the image markdown to avoid colliding with the resize grip during immediate resize operations. Computed as `pos + snippet.len() + (trailing space offset)`. See commit 4c35a09
- [x] **LaTeX sizing on Linux** — root-cause: WYSIWYG sized math via `texture_px ÷ window_scale_factor` while rasters are fixed DPR=2.0 — cancels only on Retina. Fixed by having math + mermaid providers return logical sizes `(Arc<RenderImage>, f32, f32)` so rendering engines receive the correct size independent of platform DPI. Verified: math now renders at the same size on macOS and Linux X11. See commits 7a5d4f3, 935ba9a
- [x] **Mermaid sizing** — tuned to user preference: `RASTER_SCALE=1.0`, `DISPLAY_SCALE=0.5` (renders at full resolution, displays at half-natural size). Easier to read and consistent with the "smaller" diagrams the user preferred. See commit 935ba9a
- [x] **Search: refocus reopens results** — when a search box already has a query and regains focus (e.g., after navigating), results now reopen automatically instead of requiring a re-edit. Handled via `InputEvent::Focus` subscription (separate from `InputEvent::Change`). See commit 3e0e36d
- [x] **CRT (Green Phosphor) builtin theme** — a new skin inspired by classic CRT monitors (green-on-black aesthetics). Created as a builtin in `skins.rs` (no longer a JSON file) with palette `(bg=#000000, surface=#030703, accent=#33FF33, …)`. Theme tokens (button/slider/ring families) now respect the custom accent color via luminance-aware foreground selection. All gpui-component widget families (tab, button, slider, ring) now properly theme to custom skins. See commits caf3c8d, 87abe5c, 3e0e36d

### Editor & rendering (older)
- [x] **Click-to-caret** — clicking the rendered page **or a journal day** enters edit mode with the caret on the clicked character (empty space → end of the nearest line). gpui-markdown records a rendered→source byte-offset map while rendering (handling stripped `[[ ]]` / `#` / inline-code markup) and resolves a click via gpui's text layout (`index_for_position`); the host places the editor caret (`set_cursor_position`). To keep the clicked line under the cursor (the source layout is more compact than the rendered one), gpui-markdown reports the click's window-y and the host *predicts* the caret's row with the same `LineWrapper` soft-wrap math the editor uses (`predict_caret_row` — mirrors the input's 1.25 rem line height + paddings), jumping the scroll in the same frame so the editor's first paint is already aligned; a stability-gated verify pass (`align_caret_to_click`) mops up drift and rejects the editor's stale first-paint bounds. Near the document top the jump clamps to 0 — the page stays put and the caret just lands visibly. See `crates/gpui-markdown` (`on_click_source`) + `AppView::edit_page_at_offset` / `edit_day_at_offset`
- [x] Slash menu: **click-to-insert** — slash-menu rows are now mouse-driven as well as keyboard-driven: hovering a row moves the selection to it (one highlight shared with the arrow keys) and clicking accepts it like Enter (inserts the snippet or opens a category). Driven from the row's `on_mouse_down` with `stop_propagation` so it fires before the press can blur the editor — the insertion lands and focus stays put. See `src/ui/slash_menu.rs`, `AppView::click_slash` / `slash_hover`
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
- [x] Image **fit-to-view** (`⌘⇧I`) — shrink every image in the active page / journal that renders wider than ~half the content column back down to that size, so an image dragged, pasted, or **imported with no `{width}`** stops dominating the page. Width-less images are handled too: the size comes from the painted measurement (`image_widths`), not just an explicit `{width=N}`, and all images are enumerated via a new `gpui_markdown::images()`. Until fit, an over-wide image **scrolls horizontally within its own row** (keeping its resize grip reachable) instead of running off the page, while sibling text keeps wrapping at the normal width. See `AppView::on_fit_images` / `apply_fit`, `src/ui/image.rs`
- [x] **Task-list checkboxes** (`- [ ]` / `- [x]`) — rendered via mdast `ListItem.checked` (the field does exist after all)
- [x] `gpui-markdown` now covers CommonMark + GFM: footnotes, reference-style `[text][id]` links/images, and raw HTML (shown literally)
- [x] **Mermaid diagrams** — a ` ```mermaid ` block renders as a diagram (flowchart / sequence / class / state). Pure-Rust, **no JS**: the [`mermaid-rs-renderer`](https://github.com/zed-industries/mermaid-rs-renderer) crate (the one Zed's markdown preview uses) lays it out to SVG, then gpui's built-in SVG rasterizer turns that into a `RenderImage`. Rendered off-thread and cached by source text (mirroring `ImageStore`), with a "Rendering…" placeholder and a fall-back to the code on failure. **Themed to the live skin** — `mermaid::current_theme()` maps Zorite's palette onto the diagram theme (translucent tokens composited over the page background so colours land right), and the cache is dropped in `apply_theme` so diagrams re-colour when you switch skin / light-dark. `gpui-markdown` stays renderer-agnostic via an `on_mermaid` hook (sibling of `on_image`). **Click a diagram to expand it** in a full-window, scrollable lightbox (dimmed backdrop or × to close). Follow-ups: match the UI font, render at display DPI for crispness. See `src/mermaid.rs`, `src/ui/mermaid.rs`, `crates/gpui-markdown` (`on_mermaid`)
- [x] `/time` and `/date` slash commands — insert the current time/date directly (distinct from the `{{time}}` / `{{date}}` *template* placeholders, which only expand inside a template)
- [x] **Headings nested in list items** — markdown like `- # Heading` now renders the heading with proper size/weight in WYSIWYG (previously displayed as plain text). Parser recognizes heading markers post-list-marker via shared `apply_heading` function. See commit 7d81518
- [x] **Line height tuning** — adjusted `LINE_HEIGHT_RATIO` to 1.45 (from 1.35) for better readability in normal text while maintaining good visual balance. See commit e928638
- [x] **Journal midnight-rollover fix** — journal entries now correctly appear for a new day after an overnight absence. Root cause: `ensure_feed_loaded` wasn't checking if today's date changed; the feed was cached with yesterday's bounds. Fixed by adding a `day_editors.contains_key(&date_for_offset(0))` guard to refresh the day if the offset has rolled. See commit 59b8add

### Notes & navigation
- [x] **Page rename** (and rewrite `[[links]]` pointing at it) — right-click → Rename page → dialog; `db.rename_page` rewrites links in a transaction
- [x] **Page hierarchy** via `[[parent::child]]` — Logseq-style: the `::` path *is* the page title, so the sidebar tree and each page's "Sub-pages" index are derived from titles (no parent column). Intermediate namespace segments show as virtual nodes and materialize on click. See `src/hierarchy.rs`
- [x] **Page aliases** — a subdued `alias::` field under the page title takes a comma list of alternate names; `[[name]]` then resolves to that page (exact title wins). Stored in a `page_aliases` table; resolution lives in `get_or_create_page`, so links and backlinks follow it
- [x] **Sidebar shows recent pages** — the page tree is capped to the last 10 *viewed* named pages (persisted in `settings`; seeded from the most-recently-edited pages on first run). Reach the rest via search
- [x] **Type-aware search** — the global search returns the PDF and image *files* referenced in notes, not just pages. A `pdf:` / `img:` / `page:` prefix (or a results-pane chip with a live per-kind count) filters by kind; `pdf:` / `img:` with no term browses every file in the managed `pdf/` / `images/` store. A PDF hit opens the viewer, an image opens the page showing it, a page opens the page. Files are extracted from the FTS-matched pages (`gpui_markdown::images` + the wiki-link index) rather than a separate file index. See `src/search.rs`, `src/ui/search.rs`
- [x] Journal: jump-to-date — a sidebar calendar date picker opens any day (creating it if needed)
- [x] Rename: whitespace + alias-label link variants rewrite (2026-07-03) — `[[ Foo ]]` and `[[Foo|nick]]` follow a rename (fenced code untouched; `mentions::rewrite_wiki_links` on the shared links grammar). **Case variants (`[[FOO]]`) deliberately left alone** — that casing reads as the writer's choice, and links resolve case-insensitively anyway
- [x] Hierarchy follow-ups: cascade-rename a namespace (renaming `Foo` retitles `Foo::*` children and rewrites their exact `[[links]]`, atomically — any child collision aborts the whole rename); sidebar right-click → "New sub-page" (the New-page dialog pre-filled with `Parent::`)
- [x] Unlinked references (2026-07-03) — an "UNLINKED REFERENCES" panel under Linked References: word-bounded, case-insensitive title mentions outside links/tags/code (`src/mentions.rs`, built on the shared `syntax::links` grammar; whiteboard JSON excluded), each row with a one-click **Link** that wraps that source's mentions as `[[links]]` and re-indexes
- [x] **Auto-link existing page titles as you type** (2026-07-03) — a completed word or trailing phrase (up to 4 words) matching an existing page title (case-insensitive, 3+ chars) wraps as `[[Canonical Title]]` on the boundary keystroke. Settings → Markdown toggle (default off, persisted); one undo step reverts a wrap; never fires inside code, `[[ ]]`, tags, or `[text](` syntax. Editor side is a generic `set_auto_replace` hook; the matcher + title cache live in the app
- [x] Sidebar: **"All pages" browser** (2026-07-03) — a list-icon in the sidebar toolbar opens an index tab of every named page and whiteboard: an A–Z / 0–9 / `#` strip filters by first character (letters with no matches dim; clicking the active letter clears), kind chips (All / Pages / Whiteboards) compose with it, each row shows a type badge, and clicking opens the page or canvas. Journal days excluded by design (the calendar is their browser). See `src/ui/all_pages.rs`
- [x] Calendar: entry markers (2026-07-03) — the jump-to-date overlay is a hand-rolled month grid (`src/ui/month_cal.rs`; gpui-component's Calendar has no per-day decoration hook): an accent dot + brighter number on days with non-empty entries, today outlined, ‹ › month nav, click any day to jump

### Whiteboards
The freeform `gpui-whiteboard` canvas — a reusable, host-agnostic crate (like `gpui-markdown` / `gpui-pdf`) for an infinite, pannable/zoomable board of shapes, arrows, freehand, text, images, and page-cards, linkable to pages. A distinct surface from the text journal. Design: [docs/whiteboard-architecture.md](docs/whiteboard-architecture.md). **Feature-complete** — the milestones:
- [x] **Pan mode** — a dedicated pan tool (✋) that's the default tool; left-drag pans with a grab cursor (double-click recenters, middle-drag still pans)
- [x] **Multiple page management** — boards are first-class pages now: "New" makes a distinct board (`create_whiteboard`), a "Whiteboards" sidebar section lists them (open / rename / delete / favorite), and they're searchable by title (`wb:` + a Whiteboards chip)
- [x] **Keyboard shortcuts** — tool keys (H/V/P/R/O/L/A/T), ⌫/Del to delete the selection, ⌘Z / ⌘⇧Z undo-redo, Esc to deselect; the board takes focus on a canvas click, and tooltips show the keys
- [x] **Toolbar tooltips** — hover label on every tool / action / color button (a self-rendered themed `Tip`, since the bar is icon-only)
- [x] **Organize the toolbar** — categories on the main bar (`Pan · Select · Color │ Shapes & Text ▾ · Pages & Images ▾ │ Undo · Redo · Delete`); each category button shows its group's active/representative tool + a `▾` and opens a click-to-toggle flyout of that group's tools (picking one activates it + closes); flyout also closes on a canvas press or when the color picker opens
- [x] **More shapes** — diamond, triangle, rounded rectangle, hexagon, 5-point star. All ride the shared `box_like` machinery (bounds / select / resize / rotate / fill) like rect & ellipse; polygons via a generic `paint_box_polygon`, rounded-rect via `paint_round_rect`. New flyout entries + shortcuts (D / G / U / S / X)
- [x] **Snap to grid** — hold ⌥ Option while dragging to snap to the 24px dot grid: create snaps both box corners / line endpoints, move snaps the primary element's top-left (the rest follow), resize / group-resize snap the dragged corner, and line/arrow endpoint-drag snaps the endpoint. Freehand and rotation are unaffected (`snap_grid` helper)
- [x] **Picture / image uploads** — add images via the image tool (Pages & Images → file picker), paste (⌘V on a board), or drag-drop from Finder. Stored in the managed `images/` dir and decoded by the shared `ImageStore` (off-thread, downscaled-to-display, GPU-texture-managed). Image element behaves like a page-card (move + aspect-locked resize), rendered as an overlay from a host `ImageFn` callback. Crate stays host-agnostic via `ImageFn`/`PlaceImageFn`/`DropFilesFn`
- [x] **Image rotation** — images rotate in **90° steps** (the rotate handle snaps to quarter turns). gpui can't transform a raster sprite, so the host pre-rotates the pixels (`imageops::rotate90/180/270` — exact, no resampling or bounding-box growth) and caches one bitmap per `(src, quarter-turn)`, freed on view change — bounded RAM, vs. the per-degree texture churn an arbitrary angle would cause. The element box + selection snap to 90° to track the bitmap, and the `img` is sized by width only so it fills the rotated AABB without gpui's forced `aspect_ratio` overflow-clipping it
- [x] **Templates** — reusable groups of elements. Multi-select → right-click → "Save as template" (a self-rendered canvas menu) → name dialog → stored in a global `whiteboard_templates` DB table (schema v7). A dedicated **Templates** toolbar button (2×2-grid icon, separate from the tool icons) opens a **modal gallery**: a scrim + centered panel of large preview cards (scrollable grid, hover highlight, empty state); click a card to stamp it (centered in the viewport, selected), right-click to delete (with confirm); Esc / scrim / ✕ close it. Crate stays host-agnostic via `Template` + `set_templates`/`on_save_template`/`on_delete_template`
- [x] **Z-order** — Bring to Front / Bring Forward / Send Backward / Send to Back, via a right-click menu (any selection) and shortcuts (⌘] / ⌘[ one step, ⌘⇧] / ⌘⇧[ all the way). The board paints as a true z-ordered stack — canvas "bands" (shapes / lines / pen / text) interleaved with image and page-card overlay divs in element order — so a shape can sit above *or* below an image or card (you can finally draw over an image). Reorder is a stable partition (to front/back) or a neighbour swap (one step), moving a multi-selection as a block, with undo + persist
- [x] **Copy / cut / paste** — ⌘C / ⌘X / ⌘V and a right-click Copy / Cut / Paste, through the system clipboard so a selection moves across boards and windows. Copy writes a tagged `zorite-whiteboard-v1` string (the template serialization); paste prefers it over a clipboard image and stamps the group centered + selected with fresh ids. Crate stays host-agnostic via `CopyFn` / `PasteFn`; ⌘C/⌘X/⌘V are handled in the crate (not a host action, which the board's key context wouldn't fire)
- [x] **Stroke thickness** — a thickness control next to the color swatch: a flyout of preset weights (1 / 2.5 / 4 / 6 / 9 px) over a drag slider for any custom width (reusing the color picker's drag-strip machinery). Sets the weight for new elements and applies to the selection, stored zoom-independently (`active_width / zoom`) so it stays consistent on screen
- [x] **Saved-color palette** — a "Saved" column in the color picker (beside the gradient controls): `+` saves the current color, swatches apply on click / remove on right-click. Global, persisted in `settings` and synced across open boards; host-agnostic via `SavedColorsFn` + `set_saved_colors`. Sized to the space the one-line theme-swatch row leaves, so the panel crops to that row and saved colors wrap rather than run off the edge
- [x] **Per-axis resize** — edge-midpoint handles (between the corners) *stretch* one axis, on a single element **or** a multi-selection; the corners still scale proportionally. Each element scales from its grab-time geometry about the opposite edge: axis-aligned shapes / pen / cards are exact, an image distorts (its corners stay aspect-locked, so corner = safe, edge = free-stretch), and a rotated box scales along its own axes — the pragmatic tradeoff, since a true world-axis shear isn't representable. Text and rotated elements keep corners only (a single font size can't stretch one axis; a rotated box's edges aren't world-axis-aligned). Crate-only (`ResizeHandle` + `axis_scale`, feeding the existing `resize_about`)
- [x] **Custom / uploaded fonts** — a per-board text face: an **"Aa"** toolbar button opens a flyout to *upload* a `.ttf`/`.otf` (validated as a real face, copied into a managed `fonts/` dir, persisted per board in `settings`) or *revert to default* (bundled JetBrains Mono). The chosen face is restored when the board reopens. The crate stays host-agnostic via `FontPick` + `set_on_pick_font` (it already consumed font bytes through `set_font` / `Font::from_bytes`)
- [x] **Text boxes edit like a real text field** — click any letter to place the caret, click-drag / double-click to select (word), ⇧ + arrows / click to extend, arrows / Home / End / ⌘A to navigate, and type / Backspace / Delete + ⌘C / ⌘X / ⌘V relative to the caret/selection. Built from scratch on the vector-font layout (no text-input widget): `font.rs` gained per-char caret positions (`caret_pos`), click→offset hit-testing (`index_at`), and selection rects; the view carries a caret + selection-anchor model wired through click / drag / keyboard, with the caret + a translucent highlight rendered inline (and following the text's rotation). Replaces the old append/backspace-only buffer
- [x] **Movable toolbar** — a dotted grip at the left of the tool pill drags the whole toolbar anywhere on the canvas (clamped on-board; double-click the grip resets it to top-center). Tapping **`R`** mid-drag flips the bar between a row and a column (dividers reorient, flyouts/picker move to the bar's right). Flyouts and the color picker follow it, and the position + orientation persist globally in `settings` (synced across open boards). The pill is no longer occluded — a press routes through the canvas by captured bounds (the color picker's pattern), so the buttons still click. Crate stays host-agnostic via `MoveToolbarFn` + `set_toolbar_pos` / `set_toolbar_vertical`
- [x] **Text in shapes** — double-click any closed shape (rect / ellipse / diamond / triangle / rounded-rect / hexagon / star) to add a centered label that **auto-shrinks + word-wraps to fit inside the outline** (per-shape inscribed-rectangle factors, à la Excalidraw, with padding), editable with the full caret / selection / clipboard like a text box. It inks with the shape's stroke unless recolored via a new **Text** category in the color picker. Closes #10. See `crates/gpui-whiteboard` (`shape_label_block`, `EditTarget`)
- [x] **Rich text formatting** — per-character **bold / italic / underline / strikethrough / highlight** on any board text (free text *or* shape labels), over a selection or armed for typing with none. Three entry points share one ✓-marked panel: keyboard (⌘B / ⌘I / ⌘U / ⇧⌘X / ⇧⌘H), a right-click **Text ▸** fly-out, and a toolbar **A** fly-out. Stored as style runs in the scene; italic + bold are synthetic (a shear + a stroke over the solid fill) so they work with any uploaded face. See `crates/gpui-whiteboard` (`RunStyle` / `StyleSpan`, `font::layout_styled`)

### Performance
- [x] **Journal feed cost at scale** — DONE (2026-07-03): the content-keyed parse cache shipped in `gpui-markdown` (`parse_cached`: exact-source key, Arc'd mdast, 64-entry LRU, thread-local so all windows share it) — feed re-renders now cache-hit every non-editing day. True windowing remains unexplored (and likely unneeded). Original analysis: — lower priority now that lazy-load bounds the common case: the feed starts at 14 days and only grows by `FEED_CHUNK` (7) on scroll-to-bottom / "Load older days", capped at `FEED_MAX_DAYS` (3650), so a typical session mounts only a couple dozen days. The latent issue is the *per-day* cost, not the day count: `journal::render` mounts every loaded day in a plain `for i in 0..loaded_days` loop (gpui lays out all children — Taffy doesn't virtualize a plain div — and culls only *paint*), and `MarkdownView` is `RenderOnce` with **no parse cache**, so every feed re-render re-parses every non-editing day's markdown (`to_mdast`), i.e. O(loaded_days × content). Fine at tens of days; a heavy scroller (hundreds of days) who then interacts pays a full re-parse each render. **Cheaper first step than true virtualization:** a content-keyed parse cache in `gpui-markdown` (memoize the mdast / built element by source hash) kills the dominant cost without touching the scroll model. True windowing is a poorer fit — days are variable-height, so gpui's `uniform_list` doesn't apply; it'd need `gpui::list` or a custom windowing scheme.
- [x] **Lighter `list_pages`** — the page list loads `id`/`title` only (not content): ~4× faster and memory-flat at scale (50k pages: 103 ms → 28 ms; RAM ~flat 10k→50k). See the [Performance](README.md#performance) section
- [x] **Full-text search index** — a trigram FTS5 index over page title + content (external-content, kept in sync by triggers) replaces the old `LIKE` table scan: same case-insensitive *substring* matching, now indexed so it scales. Migration `v4→v5` populates existing pages; queries < 3 chars (trigram's minimum) fall back to LIKE. See `src/db.rs`

### Data & migrations
- [x] **Back up before migrating** — `Db::open` snapshots the database to `zorite.db.bak-v<N>` (WAL-checkpointed first, so the copy is complete) before any schema upgrade, so a buggy migration is recoverable. One snapshot per source version. See `Db::backup_before_migration`
- [x] **Transactional migrations** — every step (`v0→2`, `v2→3`, `v3→4`, `v4→5`) now runs inside a transaction, so a mid-migration failure rolls back cleanly instead of leaving a half-migrated DB
- [x] **Friendlier migration failure** — a failed open no longer silently drops the user into blank notes: it falls back to a temporary in-memory store and shows a one-time dialog explaining what happened, with **Reveal Backup** (points at the `.bak-v<N>` snapshot) and **Quit**. See `AppView::show_db_error_dialog`

### App & polish
- [x] **Settings: switches + a filter box** — the Updates pane's "Automatically check for updates" / "Include pre-releases" are toggle switches (matching the WYSIWYG one), not On/Off dropdowns. A header filter box narrows the panes Baudrun-style: cards and nav tabs that don't match the typed text fade to ~0.3 (but stay interactive), with a × to clear; matching is each card's title + a keyword index (`SECTIONS`). See `src/settings.rs`
- [x] **Collapsible sidebar** — a `<` caret collapses it to a thin icon rail (`>` to expand, plus the calendar/settings icons); the content area reclaims the space
- [x] Sidebar: **truncate over-long titles to the rail width** — a title wider than the rail is clipped with an ellipsis (rows stretch to the rail and `.truncate()`), so a row and its selection highlight never run past the sidebar edge; the full title shows in a tooltip on hover (measured per row via `text_system().layout_line`, so the tooltip appears only when actually clipped). Replaces the earlier horizontal-scroll-on-overflow approach. See `src/ui/sidebar.rs`
- [x] Sidebar: **Favorites group** — right-click a page → **Add/Remove from favorites** pins it to a "Favorites" section above "Recent", shown by **full title** (so `Foo::Bar` is unambiguous, unlike the leaf-nested recent tree). Persists across launches as a comma-separated id list in `settings` (mirrors `recent_pages`, no migration); the section hides when empty, and a deleted page drops out of favorites. Recent and favorites rows share one `page_row` builder. See `src/ui/sidebar.rs`, `AppView::toggle_favorite`
- [x] Sidebar: **collapsible namespace nodes** — a node with children shows a disclosure chevron (`▸`/`⌄`) in a gutter at the start of every tree row; clicking it hides/shows the subtree (the click stops propagation so it doesn't also open the page). Collapsed paths persist across launches (newline-separated in `settings`). The indent guides from the previous item align under each parent's chevron. See `src/ui/sidebar.rs`, `AppView::toggle_collapsed`
- [x] Sidebar: **collapsible sections** — each section header (Favorites / Whiteboards / Recent) is clickable, with a disclosure chevron at the right end of its rule; collapsing hides that section's rows. Collapsed sections persist across launches (newline-separated keys in `settings`, mirroring the node-collapse plumbing). The "new whiteboard" action moved out of the Whiteboards header to a `Frame`-icon button in the sidebar's top toolbar (next to new-page / calendar / settings, both labelled by tooltips). See `src/ui/sidebar.rs` (`section_header`), `AppView::toggle_section`
- [x] **Multi-window** — right-click a sidebar page or a tab → "Open in new window" opens a full, independent second window focused on that page (`AppView::open_in_new_window`). Each window is its own `AppView` with its own SQLite connection to the same file. See `src/app.rs`, `src/ui/tab_bar.rs`
- [x] Multi-window: **drag a tab anywhere** — reorder within the strip (drop over a tab), **move it to another open window** by dropping on *that window's tab bar* (it's added there, activated, and the window is focused), or **tear it into a new window** by releasing on content / outside any window, *including onto the desktop* (the new window opens under the cursor). gpui drag-and-drop is per-window, so the source window — which keeps OS mouse capture for the whole drag — owns the release: a strip drop reorders via the tab's `on_drop`; otherwise the root's `on_mouse_up` / `on_mouse_up_out` runs `on_tab_drag_release`, which reads the release point in global coords (`window.bounds().origin + mouse_position`) and finds a target via `GlobalAppWindows`. **Detection is per-tab-bar, not per-window**: each window records its strip's rect each paint (`tab_strip_bounds`, captured by a `canvas`), and a move only fires when the cursor is over another window's *strip* — so a window hidden behind the source isn't picked off its overlapping area, and releasing over your own strip is a "keep here" no-op (drag back to the strip to cancel). It's a gpui-internal drag (a `DraggingTab`, never a native file promise), so releasing on the desktop **never writes a file there** — the trap hit in Baudrun. While dragging, the source's `on_drag_move` lights up the strip under the cursor (`GlobalDropTarget`), which shows a translucent **ghost tab** where the tab would land (repainted cross-window via `cx.notify`). Right-click "Open in new window" still *moves* the tab. Wayland: the compositor controls new-window placement. Known edge: with no window z-order from gpui, a target window's strip that's *hidden behind* the source's content can still match — drop on a visible tab bar. See `src/app.rs`, `src/ui/tab_bar.rs`
- [x] Multi-window: **live cross-window sync** — a shared `DocSignal` (gpui global, one per process) is emitted on content saves (`save_page_content`) AND structural changes (create / rename / delete + the blur link re-index); other windows run `apply_external_edit`, reloading changed journal days, the active page (content + backlinks), and the sidebar page-list (via value-comparison, so only stale data is touched and the editing window never clobbers itself)
- [x] **App icon + packaging** — cargo-packager builds `.app`/`.dmg`, NSIS `.exe`, `.deb`/`.AppImage`, `.rpm`, etc.; a custom app icon ships (Windows PE icon embedded via `build.rs`; `.icns`/`.ico`/`.png`). See `Cargo.toml` `[package.metadata.packager]`
- [x] **CI** — GitHub Actions builds + `cargo test --workspace` across macOS / Windows / Linux (5 runners) and publishes a prerelease on `vX.Y.Z-beta.N` tags. See `.github/workflows/`
- [x] **Keyboard shortcuts + menu bar** — standard cross-OS shortcuts via gpui's `secondary-` (Cmd on macOS / Ctrl elsewhere): New Tab `⌘T`, New Window `⌘N`, Close Tab `⌘W`, Settings `⌘,`, Quit `⌘Q`, Next/Prev Tab `Ctrl+Tab` / `Ctrl+Shift+Tab`. Native macOS menu bar (Zorite / File / Edit / View) via `cx.set_menus`; the Edit menu reuses gpui-component's input actions. A read-only **Settings → Keyboard** section lists them all. `⌘F` finds in the current page and `⌘⇧F` focuses the global search (see **Find in page**). See `src/actions.rs`, `src/main.rs`
- [x] Window-bounds persistence (2026-07-03) — Settings → General → "Remember window position": live-saved on move/resize (maximized tracked, fullscreen skipped) to a DB-free sidecar file (`window-bounds` — needed before an encrypted DB unlocks; file presence = on/off), restored at launch when the saved rect still touches a connected display
- [x] **`LICENSE` file** — the full GNU GPL-3.0 text at the repo root, matching the `GPL-3.0-or-later` already declared in `Cargo.toml`

### Import & export
- [x] **Logseq import** — `File → Import from Logseq…` picks a graph folder and imports `pages/` + `journals/` + `whiteboards/` (draws, config, `bak`/`.recycle` are skipped). Namespaces map to Zorite's (`Budget___2024.md` files and `[[Budget/2024]]` links → `Budget::2024`); the all-bullets outline converts per a user choice — **flatten** (top-level bullets → paragraphs/headings, children stay lists) or **keep bullets**; `TODO`/`DONE`/`CANCELED` → task checkboxes; Logseq-internal properties (`id::`, `collapsed::`, …) drop while user properties stay as text; `title::`/`alias::` feed the page title and alias table; `{{video}}`/`{{embed}}`/`((block-ref))` resolve and queries stay visible as inline code; glued code fences (` ```cfg… `) are normalized; assets copy into `images/` + `pdf/` (percent-decoded, sanitized names) with refs rewritten (`[[pdf/x.pdf]]` chips); Logseq `hls__*` PDF-highlight pages convert to Zorite's `<name>.pdf (highlights)` format. **Whiteboards** (`whiteboards/*.edn`, tldraw EDN — a minimal EDN reader in `src/import/edn.rs`) convert to native Zorite boards: text, boxes (with their `:label` as the shape's native auto-fit label), ellipses, lines, **arrows** (`:decorations {:end/:start "arrow"}`), freehand, and embedded base64 **images** (under `:pages → :logseq.tldraw.page → :assets`, decoded into `images/wb-*.png`); web-embeds and page-portal cards have no equivalent and are skipped (warned per board). **Favorites** (`config.edn` `:favorites`) map to the sidebar Favorites list. Existing pages/days keep their content (import appends below, reported in the summary). Runs on a background thread against its own DB connection; `ZORITE_DATA` env (new) isolates a whole test data set. **Extensible by design**: each source is a *reader* module producing a source-agnostic `ImportBundle` (pages, journal days, asset copies + decoded byte-assets, whiteboards, favorites), and one engine (`import::write_bundle`) owns the collision policy, link/alias indexing, and asset copying — adding Obsidian/Notion/… is a new reader plus a menu entry (see the "Adding an importer" doc in `src/import/mod.rs`). See `src/import/` (engine + `logseq.rs` reader, both unit-tested end to end), `AppView::on_import_logseq`
- [x] In-app **PDF viewer** — `[[file.pdf]]` / `![](file.pdf)` open a dedicated viewer tab (`ui::pdf_view`); pages are sized from `render_dimensions()` for instant layout. Closing the tab frees both the CPU images and their **GPU atlas textures** (`cx.drop_image` — raw `RenderImage`s are never auto-evicted; this was an ~140 MB/open leak). See `src/pdf.rs`
- [x] PDF: **viewport virtualization** — only the on-screen pages (±2) are rasterized; far ones are evicted (image + GPU texture), so memory is bounded by the viewport, not the page count (`AppView::ensure_pdf_window` + `pdf_view::keep_window`). Verified: scrolling a 32-page PDF end-to-end holds ~178 MB vs 403 MB before
- [x] PDF: **DPI-aware render scale** — pages rasterize at display pixel-ratio × zoom × a host **quality** multiplier (no longer a fixed 1.5×); a render-quality slider in Settings trades sharpness for speed (default 75%), read reactively so open viewers re-render live
- [x] PDF: **zoom + page navigation** — − / + / reset (⌘= / ⌘- / ⌘0) and ‹ / › with a click-to-edit page-number input (+ PageUp / PageDown / Home / End); no blank on zoom/quality change (the old bitmap stays, rescaled, until the crisp one lands)
- [x] PDF: **extracted to a reusable [`gpui-pdf`](crates/gpui-pdf/README.md) crate** — host-agnostic primitives + a self-contained `PdfView` component; markup is behind an optional `markup` feature
- [x] PDF: **Logseq-style text markup** — drag-to-highlight in the viewer writes a reference block (`- pN: quote {color} [[file.pdf#pN|↗]]`) on a per-PDF "(highlights)" page; clicking the ↗ opens the PDF and scrolls to + **flashes** the highlight. Done **dep-free** — a custom hayro `Device` extracts text + glyph rects (only `kurbo`, *not* oxidize-pdf). Has a **color picker** (yellow/green/blue/pink/orange) and header **tooltips**
- [x] PDF: **find-in-PDF** — a browser-style find bar (🔍 / ⌘F) over the text layer: type to search the whole document, matches boxed + a focused one outlined, `n / N` count, Enter/⇧Enter to step (scroll-to), Esc to close. Behind a `search` feature (= `["markup"]`, shares the text layer). See `gpui-pdf` `find_matches` + `src/pdf.rs`
- [x] PDF: **outline / table-of-contents** — reads `/Catalog /Outlines` via `hayro-syntax` into `gpui_pdf::outline()`; a toggle (≡, by the title) opens a navigable, level-indented TOC panel that jumps to each page (hidden when there's no outline). Also extracts `/Link` annotations (`page_links`) → clickable in-page links (internal jumps + external URLs), and bulleted highlight selections store as a nested markdown list. See `crates/gpui-pdf/src/outline.rs`
- [x] PDF: **password-protected files** — an encrypted PDF opens behind a prompt (masked field, Enter or a button to submit, "incorrect password" on a wrong try) instead of a blank pane; hayro 0.7.2 decrypts RC4 / AES-128 / AES-256 (standard security handler) via `Pdf::new_with_password`. The prompt UI lives in the app so `gpui-pdf` stays gpui-component-free: `PdfView` keeps the bytes, reports `is_locked()` / `unlock_failed()` and retries via `unlock(password)`, emitting `PdfEvent::LockChanged` so the host swaps prompt ↔ viewer. Unsupported handlers (public-key / certificate) still need the fallback above. See `crates/gpui-pdf` (README + `lib.rs`), `src/app.rs`
