# Changelog

All notable changes to **Zorite** are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Every tagged release also has a GitHub page with installers and the full commit
log: <https://github.com/packetThrower/zorite/releases>.

## [0.6.1] - 2026-07-07

Properties polish from a day of real use, plus a caret that finally knows
its size.

### Added

- **`/property`** — the `/` menu can now add a property: it inserts a fresh
  row and opens the in-place property form on it, key field ready with the
  autocomplete listing every key already in use. Escape without naming a key
  and the row cleans itself up.

### Fixed

- **Properties** — three papercuts in the in-place property editor: an empty
  value field (a template's blank `key::`) had no click target, so the caret
  couldn't enter it; clicking a field always seated the caret at the end
  instead of where you clicked (the rendered panel also always focused the
  first row, whichever row you clicked); and typing `key::` when a page named
  `key` exists auto-linked the key into `[[key]]` before the property could
  form — auto-link-as-you-type no longer treats `:` as completing a word.
- **Caret height** — on lines taller than their text (a bullet's breathing
  room, or a line grown to fit an inline formula) the caret spanned the whole
  line; it now matches the text height, vertically centered like the glyphs.
  Headings keep their proportionally taller carets.

## [0.6.0] - 2026-07-06

The Obsidian-parity release: `key:: value` properties with an in-place editor
and a vault-wide index, block references and heading links, embeds
(transclusion), foldable callouts and collapsible headings, in-flow inline
images — and a full **Import from Obsidian**, canvas boards included.

### Added

- **Collapsible headings** — hover a heading and a chevron appears past the
  text; click it to fold everything under that heading (up to the next heading
  of the same or a higher level), in both the reading view and the live editor.
  A folded heading always shows its ▸ so hidden content stays visible, nested
  folds work, and in the live editor arrowing the caret into a folded section
  reveals it while you edit and folds it back when you leave. Folds are
  per-session view state — nothing is written into the note.
- **Properties** — `key:: value` metadata lines (Obsidian/Logseq-style
  "properties"), written anywhere on a page, now render as a clean two-column
  panel in both the reading view and the live editor: a per-key icon, the key,
  and the value with `#tags` and `[[wiki-links]]` shown as clickable pills, with
  the row highlighting on hover. This generalizes the old alias-only handling —
  any key works (`attendees::`, `status::`, `time::`, …), and the panel reads the
  same whether you're viewing or editing.
- **In-place property editor** — clicking a property panel, or arrowing the
  caret into it, opens an editor seated right in the note that mirrors the
  rendered panel (icons, muted keys, value pills — the value you're editing
  reveals as raw text under the caret). Edit keys and values in place with a
  dropdown of properties already used across your notes, add or remove rows, and
  move between fields entirely from the keyboard (arrows step within and between
  fields, Tab/Shift-Tab jump field to field). Clicking away writes the
  `key:: value` lines back.
- **Properties page** — All pages → Properties lists every property key in your
  vault with its icon, page count, and values; expand a key to see each value
  and the pages carrying it (click through to open). From the same page you can
  override any key's icon from a picker (or map an icon to a key before its
  first use — the mapping shows as its own row until the key appears on a
  page), and rename a key across every page at once.
- **Embeds (transclusion)** — a line holding just `![[Note]]` renders that
  note's content right there, in a quoted box with a clickable source label;
  `![[Note#^id]]` embeds a single block and `![[Note#Heading]]` a whole section
  (an `|alias` renames the label). Both views render the real content — in the
  live editor the box sits in place of the line, shows a scrollbar on hover
  when the content overflows, and hands the wheel back to the page at its
  edges; put the caret on the line to edit the raw `![[…]]` text. Editing the
  source page updates every box embedding it, live. An unresolved target shows
  a compact `⧉` chip in the editor and stays literal text in the reading view.
- **Anchor links no longer spawn junk pages** — linking to `[[Note#^id]]` or
  `[[Note#Heading]]` used to auto-create a page literally named `Note#…` when
  links were indexed; the index now targets the real page (an existing page
  whose title genuinely contains `#` still wins).
- **Block references and heading links** — end a line with ` ^some-id` to give
  it an address, then link to it from anywhere with `[[Note#^some-id]]`; or link
  straight to a heading with `[[Note#My Heading]]` (case-insensitive). Clicking
  opens the note and jumps to that line. Either link reads as `Note → anchor` in
  both views (an `|alias` still overrides), and the `^id` anchor itself stays
  out of the way — hidden in the reading view, dimmed/hidden in the editor until
  the caret is on its line. `file.pdf#p3` keeps its page-jump meaning, and a
  page whose title literally contains `#` still opens by its full name.
  Previously an anchor link created a page literally named `Note#…`.
- **Foldable callouts** — an Obsidian-style fold char on an alert marker makes
  it collapsible: `> [!NOTE]-` starts folded (only the title shows), `> [!NOTE]+`
  starts open, and a plain `> [!NOTE]` stays as-is. A chevron joins the title in
  both views; clicking it folds/unfolds and persists the state in the note
  (the `-`/`+` flips in the source, like ticking a task checkbox). In the live
  editor, arrowing the caret into a folded callout reveals it while you edit and
  it folds back when you leave.
- **Import from Obsidian** — File → Import from Obsidian reads a vault folder:
  folders become `::` namespaces (or flatten, your choice), links and aliases
  resolve, callouts map to Zorite's alerts, frontmatter feeds aliases/tags/
  properties, `YYYY-MM-DD` notes become journal days, and images/PDFs copy into
  the managed stores. `^block-id` anchors, `[[Note#Heading]]` / `[[Note#^id]]`
  links, and `![[Note]]` transclusions come across as-is — they all work in
  Zorite now. **Canvas boards import too**: each `.canvas` becomes a
  Zorite whiteboard — text cards as labeled boxes, note cards as clickable page
  cards, image cards placed, groups as outlines, and edges as arrows with their
  labels. Anything that can't map 1:1 is downgraded and noted in the import
  summary.

## [0.5.1] - 2026-07-03

Two fixes, one urgent: Windows users could be locked out of an encrypted
database.

### Fixed

- **Windows: unlocking, Lock now, and auto-lock closed the app** — entering
  a correct password (without "Remember on this device"), pressing Lock
  now, or hitting the idle auto-lock exited Zorite entirely. The window
  handoff let the open-window count touch zero, which ends the app on
  Windows (macOS tolerates it, which hid the bug). The successor window now
  always opens before the old one closes. Your data was never at risk —
  the app just exited before showing it.
- **Selecting across a list no longer breaks its rendering** — rows kept
  their numbering (`a.`, `i.`) and indentation instead of snapping to raw
  source mid-drag; the body still reveals inline markers so the highlighted
  text is exactly what's copied. Copies now include the first item's list
  marker, and ordered items copy with the numbering the screen showed
  (still plain-markdown digits, so pastes rebuild real lists anywhere).

## [0.5.0] - 2026-07-03

The biggest Zorite release yet: encrypt your notes with a password, see them
as a graph, browse everything in one index, export any note to PDF, and make
the app yours with custom fonts and full theme control.

### Added

- **Password & encryption** — encrypt the entire database with a password
  (SQLCipher, AES-256). An unlock screen gates launch; **Remember on this
  device** keeps the password in the OS keychain (macOS Keychain / Windows
  Credential Manager; kernel keyring until reboot on Linux); an idle
  **auto-lock** (5 min – 1 hour) and a **Lock now** action re-lock a running
  app. The password itself is never stored — and never recoverable, so
  don't lose it. Settings → Security.
- **Graph view** — your pages and whiteboards as a Logseq-style map: linked
  clusters lay out by a force simulation, orphan pages ring the outside.
  Drag to pan or move nodes, pinch/⌘-scroll to zoom, click to open, hover to
  highlight a neighborhood; a panel carries a legend with live counts,
  search, and journal/orphan/whiteboard filters. Open it from All pages.
- **All pages browser** — a sidebar list icon opens an index of every page,
  whiteboard, and stored PDF: an A–Z/0–9/# strip and kind chips filter it,
  rows show created/updated dates, and the filters stay pinned while you
  scroll.
- **Export to PDF** — right-click a tab or sidebar page (or ⌘P) to write the
  note as a PDF, rendered like the reading view: styled text, tables,
  images, alerts, typeset math, and mermaid diagrams. Pure-Rust, no browser.
- **Custom fonts & full theme control** — Settings → Appearance picks the
  app font (any installed family, or import a `.ttf`/`.otf`); custom theme
  JSON can now override every color token (with `#RRGGBBAA` alpha) and name
  its own font. Plus a **text size** setting (14–20 px) that drives all
  three views.
- **GitHub alerts** — `> [!NOTE]` through `[!CAUTION]` render with icons and
  themeable colors in both the reader and WYSIWYG (which hides the marker
  and paints a label). The lenient single-line form works too.
- **Syntax highlighting** — fenced code blocks highlight ~20 common
  languages, themed to your skin, in reader, WYSIWYG, and PDF export.
- **Unlinked references** — a panel under Linked References lists pages and
  journal days that mention the open page's title without linking it; a
  one-click **Link** wraps the mentions as `[[links]]`.
- **Auto-link as you type** (opt-in, Settings → Markdown) — typing an
  existing page title wraps it as a `[[link]]` on the boundary keystroke;
  `[[` completion now suggests whiteboards too.
- **Namespace tooling** — renaming a page cascades to its `Foo::*` children
  (links rewritten, atomically); right-click → **New sub-page**; child pages
  show a clickable breadcrumb back to their ancestors.
- **Calendar entry markers** — the jump-to-date calendar dots every day
  that has a journal entry.
- **Journal back-to-top** — a floating button appears once you've scrolled
  and snaps back to today.
- **Image housekeeping** — pasting or dropping an image whose contents are
  already in the store reuses the existing file instead of duplicating it,
  and Settings → General → **Unused images** sweeps unreferenced files to
  the system trash (with a confirmation listing them).
- **Remember window position** (Settings → General) — reopen with the size
  and position you left, falling back to centered if that display is gone.
- The `/` menu now covers the whole markdown vocabulary — math, alerts,
  footnotes, strikethrough, highlights, and more.

### Changed

- **Numbered lists are Word-style** — nested levels display `1.` → `a.` →
  `i.`, every list counts from 1 regardless of the raw digits, a break
  restarts numbering, and Tab-indenting starts the nested list at 1. Editing
  a list item no longer shifts the line left: the rendered markers stay put,
  and only stepping into the marker itself reveals the raw text.
- **Reader and WYSIWYG render alike** — tables, code cards, line height, and
  list spacing/indentation now match across the two views, and both consume
  one shared link/alert/table grammar so they can't drift apart.
- **The journal feed re-renders faster** — parsed markdown is cached by
  content, so typing in one day no longer re-parses every visible day.

### Fixed

- **Opening a note containing certain non-ASCII text** (e.g. `¯\_(ツ)_/¯`)
  crashed the app; both markdown engines now scan multi-byte text safely.
- **⌘V of an image or a copied file into a page** did nothing (or pasted
  the file's name as text); images now insert at the caret, copied files
  import like a drop, and whiteboards accept pasted files at the viewport
  center.
- **Renaming a page** now also rewrites `[[ spaced ]]` and `[[aliased|label]]`
  link variants (deliberately-cased variants like `[[FOO]]` are left as
  written), and never edits links inside code blocks.
- **Auto-pair** — typing `[` over a selection that itself starts with `[`
  wraps the selection instead of deleting it.
- **The Journal tab** no longer draws a stray left border against the
  sidebar.

## [0.4.2] - 2026-07-02

A single fix for table editing.

### Fixed

- **The table add-row "+" button works along its full height** — it was
  mostly unclickable (only a sliver at its top edge responded, and clicking
  lower placed the caret instead), worst for a table at the bottom of a
  note. Every document with a table also interacted as ~22px shorter than
  it looked; clicks in that band now land correctly too. The add-column
  "+" was never affected.

## [0.4.1] - 2026-07-01

Links are clickable while editing again, plus a CRT theme and themed
widgets everywhere.

### Added

- **CRT (Green Phosphor) theme** — pure black, phosphor-green text, amber
  tags; a VT100 look, dark-only. Pick it under Settings → Appearance.

### Fixed

- **Links navigate in WYSIWYG editing** — `[[wiki-links]]` (aliases too),
  `#tags`, and `[text](url)` open on a plain click, with a hand cursor on
  hover, matching the reading view. Double-click still selects, and
  clicking beside a link still edits its text.
- **Custom themes reach every widget** — buttons, sliders, focus rings, and
  tab labels previously kept stock colors on any theme whose foreground
  isn't near-white; accent-button labels now pick black or white for
  contrast automatically.
- **Theme switches apply live to open notes** — tag/code/link colors no
  longer keep the old theme until a restart.
- **Mermaid diagram borders** are clearly visible on dark themes.

### Changed

- **GUI framework refresh** (gpui + gpui-component, ~a month of upstream
  fixes) — no user-facing changes expected; report anything that moved.

## [0.4.0] - 2026-07-01

LaTeX math — write and edit equations in your notes, inline and as display
blocks, with a two-dimensional structural editor. Images grow up too: they
behave like objects in the editor, and dropping, resizing, and deleting them
all work the way you'd expect.

### Added

- **Math, inline and display** — write `$…$` in a line or `$$…$$` as a centered
  block; both typeset to crisp equations, in the editor and the reading view.
- **A 2-D structural editor** — click or arrow into a formula to edit it visually
  (a fraction is a real stacked box you move into), not by hand-editing LaTeX:
  fractions, roots, nth-roots, super/subscripts, and matrices.
- **Symbol palette and `\command` autocomplete** — one-click structures and
  symbols, plus a scrollable menu of ~100 LaTeX commands as you type `\`.
- **Per-formula alignment** — left / center / right (centered by default), from a
  display formula's right-click menu.
- **Copy and export** — right-click a formula to copy its LaTeX or export it as a
  PNG or a self-contained SVG.
- **`/math`** inserts a math block from the command palette.

### Changed

- **Images are objects in the editor** (Word-style): moving the caret onto a
  picture no longer flips it to raw markdown — the caret parks beside it,
  Backspace/Delete removes the whole picture as one undoable edit, and
  right-click offers **Delete image**.
- **Roomier editor line height**, so stacked lines and lists read less cramped.
- **Mermaid diagrams display at half their natural size** — better proportioned
  next to note text, and pixel-crisp on Retina displays.

### Fixed

- **Math and diagrams are the same size on every display.** Formulas rendered
  twice as large on Linux (1× displays), and mermaid diagrams changed size from
  one monitor to another — both now size platform-independently.
- **Dropped and pasted images render immediately**, instead of showing a bare
  `!` until the next keystroke or tab switch.
- **Resizing works right after a drop**, and a resized image survives switching
  tabs and back (it used to come back as bare `{width=…}` text).
- **The journal rolls over midnight** — a window left open overnight now shows
  the new day without a restart.
- **Headings nested in list items** (`- ### Notes`) render as headings while
  editing, not as literal `###` text.
- **Search results reopen** when clicking back into a search box that still
  holds a query — no more editing the text to get the results back.

## [0.3.0] - 2026-06-24

WYSIWYG table editing matures, inline-formatting shortcuts, and a Windows/Linux
menu bar — plus inline HEIC/AVIF images.

### Added

- **HEIC, HEIF, and AVIF images render** inline like JPEG and PNG, on macOS,
  Windows, and Linux.
- **Edit tables like a spreadsheet** — arrow keys move cell-to-cell keeping your
  column (skipping the separator row), Enter drops to the cell below, and the
  caret enters and leaves the table cleanly at the top and bottom.
- **Table editing handles** — hover a table for "+" strips to add a row or
  column and "−" handles to delete one; a right-click menu adds insert/delete,
  per-column alignment, and "Delete table".
- **Lists continue themselves** — Enter in a list or task carries the marker to
  the next line; Enter on an empty item exits the list.
- **Inline formatting shortcuts** — ⌘B / ⌘I / ⌘E (Ctrl on Windows/Linux) toggle
  bold, italic, and code around the selection.
- **Clickable task checkboxes** — toggle ☐/☑ with a click; the cursor turns to a
  pointer over anything clickable.
- **Menu bar on Windows and Linux** — a File / Edit / View menu now lives in the
  titlebar on those platforms (macOS keeps its native menu bar).

### Changed

- **Menus follow your theme** — the table right-click menu and spell-check
  suggestions use the active theme's colors instead of a fixed dark style.

### Fixed

- **Adjacent tables stay separate** — two tables with no blank line between them
  no longer merge into one grid.
- **The caret stays in view** — arrowing up or down now scrolls the page so the
  caret never slips off-screen.

## [0.2.1] - 2026-06-22

Image fixes for the WYSIWYG editor (the single renderer since 0.2.0).

### Fixed

- **Images in lists render again** — a bulleted image (`- ![](src)`) renders as
  the image with its bullet, instead of falling back to raw source.
- **Drag-to-resize is back** — inline images have a bottom-right corner grip;
  dragging it resizes the image while the surrounding content reflows live, and
  the width persists as `{width=N}`.

### Changed

- **Photos load faster** — JPEGs decode at a reduced size directly (DCT scaling)
  instead of a full-resolution decode-then-downscale, roughly halving photo-page
  load time and cutting peak memory.
- A little vertical breathing room between stacked inline images.

## [0.2.0] - 2026-06-22

A major release: a brand-new editor with live WYSIWYG Markdown and native
spell-check, tables you edit in place, and rich text on the whiteboard.

### Added

- **WYSIWYG live editor** — the note editor renders Markdown live as you type and
  is the single renderer (the default): headings, bold / italic / strikethrough,
  inline + fenced code, links, wiki-links, tags, blockquotes, lists, task
  checkboxes, images, PDF chips, mermaid diagrams, tables, thematic breaks,
  footnotes, reference links, and `<mark>` all render formatted, with the raw
  Markdown revealed only around the caret.
- **From-scratch text editor** (`gpui-editor`) under the journal and pages —
  soft-wrap, undo / redo with coalescing, word / visual-line movement, and a
  right-click menu.
- **Native OS spell-check** — wavy underlines for misspellings and suggestions on
  right-click, via the system speller (macOS + Windows).
- **Tables, edited in place** — type directly in cells; a `/table` picker offers
  visual designs (Grid / Striped / Header / Minimal); an alignment toolbar sits
  in the header row; and a right-click menu inserts or deletes rows and columns.
- **Whiteboard — text in shapes** — double-click a closed shape for a centered
  label that auto-shrinks and wraps to fit inside the outline, editable like a
  text box and colorable on its own.
- **Whiteboard — rich text** — per-character bold, italic, underline,
  strikethrough, and highlight on any board text, via ⌘B / ⌘I / ⌘U / ⇧⌘X / ⇧⌘H
  (Ctrl on Windows / Linux), a right-click fly-out, or the toolbar.
- **Settings** — date / time formats for `/date`, `/time`, and the `{{date}}` /
  `{{time}}` placeholders, plus a filter box that narrows the panes as you type.

### Changed

- Boolean settings (live preview, update checks) are toggle switches, not
  dropdowns.
- **Enter** confirms the primary action in every dialog (Save, Insert, Create,
  …); Esc cancels.
- Logseq import stores a box's `:label` as the shape's native auto-fit label.

### Known issues

- Deleting the **last** row or column of a table can drop the caret just below
  the table; other rows and columns are unaffected.

## [0.2.0-beta.3] - 2026-06-20

### Added

- **WYSIWYG live editor** — the note editor now renders Markdown live as you type,
  and is the single renderer when enabled (the default). Headings, bold / italic /
  strikethrough, inline code, links, wiki-links, tags, blockquotes, lists, task
  checkboxes, fenced code blocks, images, PDF chips, mermaid diagrams, tables,
  thematic rules, footnotes, reference links, and `<mark>` all render formatted —
  with the raw Markdown revealed only around the caret. No more swapping between a
  rendered page and a raw-text line while editing.
- **Tables, edited in place** — type directly in cells; the `/table` picker offers
  visual designs (Grid, Striped, Header, Minimal); an alignment toolbar (left /
  center / right) appears in the header row; and a right-click menu inserts or
  deletes rows and columns.

### Known issues

- Deleting the **last** row or column of a table can drop the caret just below the
  table; other rows and columns are unaffected.

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

[0.6.1]: https://github.com/packetThrower/zorite/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/packetThrower/zorite/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/packetThrower/zorite/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/packetThrower/zorite/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/packetThrower/zorite/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/packetThrower/zorite/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/packetThrower/zorite/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/packetThrower/zorite/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/packetThrower/zorite/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/packetThrower/zorite/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/packetThrower/zorite/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/packetThrower/zorite/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/packetThrower/zorite/compare/v0.1.0-beta.2...v0.1.0
[0.1.0-beta.2]: https://github.com/packetThrower/zorite/compare/v0.1.0-beta.1...v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/packetThrower/zorite/releases/tag/v0.1.0-beta.1
