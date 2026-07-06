# gpui-editor

A from-scratch, multi-line **text editor for [GPUI](https://github.com/zed-industries/zed)** —
the basis for [Zorite](https://github.com/packetThrower/zorite)'s note editor.

Host-agnostic: it depends on `gpui` (+ `unicode-segmentation`), **not** on
`gpui-component` — plus one sibling, [`gpui-markdown`](../gpui-markdown/README.md)
with default features off, which contributes only `gpui_markdown::syntax`: the
**dependency-free** construct-recognition module (what counts as a link / alert /
table style) shared with the reader so the two views can never drift apart. It's built directly on GPUI's text primitives — an
`EntityInputHandler` for keyboard + IME input, `shape_line` for per-line text
shaping, and a custom `Element` that lays out and paints the lines, caret, and
selection.

## Overview

- **Auto-grows** to its content height (no inner scrollbar), so a host can stack
  many editors in one scroll view (e.g. a journal feed).
- **Editing:** insert / backspace / delete / newline, arrow + Home/End +
  word-wise navigation, visual-row up/down, copy / cut / paste, IME, undo / redo
  (coalesced), click + drag selection, double-click word / triple-click line.
  **Lists continue on Enter** (an empty item exits the list), and **bold /
  italic / inline-code** toggles (`cmd`/`ctrl`-`b`/`i`/`e`).
- **Soft-wrap** with content-driven height.
- **Spell-check squiggles:** the host feeds in misspelled byte ranges
  ([`Diagnostic`]); a right-click menu offers replacements via a lazy provider.
- **Live-preview Markdown ("WYSIWYG"):** with a [`SyntaxStyle`] installed, the
  editor styles its own content as you type — headings (variable line height),
  bold / italic / strikethrough, inline code, links / wiki-links / tags
  (clickable, emitting `OpenLink` / `OpenWikiLink`), blockquotes, lists,
  clickable task checkboxes, fenced code blocks, thematic rules, footnotes,
  reference links, `<mark>` — with the raw Markdown markers hidden and
  revealed only around the caret. **GitHub alerts** render with a colored bar
  and title (Obsidian's foldable `> [!NOTE]-`/`+` collapses on a chevron
  click, the flip written back to the source), and **headings fold** — hover
  one for a chevron that collapses its section (view-local state,
  reveal-on-caret while editing).
- **Anchors & properties:** trailing ` ^block-id` markers dim/hide like other
  syntax; `[[Note#Heading]]` / `[[Note#^id]]` targets display as
  `Note → anchor`; and consecutive `key:: value` lines render as a two-column
  **property panel** (per-key icons via `SyntaxStyle::property_icon`, pill
  values) — clicking or arrowing into one emits `EditProperties` so the host
  can seat an in-place property editor.
- **Block widgets:** standalone images, file chips (e.g. PDF embeds), mermaid
  diagrams, `$$…$$` math blocks, and `![[Note]]` **transclusions** (any
  `AnyView` the host resolves, in a reserved gap) render in place via
  host-supplied providers (raw source under the caret). Mid-text images flow
  as small inline thumbnails; a click emits `PreviewImage`.
- **Math:** display `$$…$$` blocks **and** inline `$…$` formulas typeset via the
  math provider; clicking or arrowing into one emits `EditMath` so the host can
  seat its own 2-D structural editor in the spot the editor reserves
  (`set_editing_block` for a block, `set_editing_inline` in-line).
- **Tables:** rendered as a grid and edited *in the cells* — arrow keys move
  cell-to-cell keeping the column and Enter drops to the row below; host-driven
  column alignment, row/column insert/delete, and whole-table delete.

## Adding the dependency

It's a path/git crate (not on crates.io — `gpui` is a git-only dependency):

```toml
[dependencies]
gpui-editor = { path = "crates/gpui-editor" }   # or a git dependency
```

> **gpui revision:** this crate takes the workspace's pinned `gpui` rev
> (`[workspace.dependencies]` — one spec, byte-for-byte). In a separate
> workspace, keep your `gpui` rev in lockstep with this crate's or you'll get
> two `gpui` versions in one build (won't compile).

## Quick start

```rust
use gpui::*;
use gpui_editor::{EditorState, EditorEvent};

// 1. Once at startup, bind the editing keys (scoped to the editor's key context).
gpui_editor::bind_keys(cx);

// 2. Create the editor entity.
let editor = cx.new(|cx| {
    EditorState::new(window, cx)
        .with_placeholder("Type here…")
        .with_text("# Hello\n\nSome **markdown**.")
});

// 3. Focus it to start editing.
editor.update(cx, |ed, cx| ed.focus(window, cx));

// 4. React to edits (e.g. save, re-run spell-check).
cx.subscribe(&editor, |_host, editor, event: &EditorEvent, cx| {
    if let EditorEvent::Changed = event {
        let text = editor.read(cx).text().to_string();
        // …save `text`…
    }
})
.detach();
```

## Rendering

`EditorState` is a GPUI entity that renders itself, so a host just renders it as
a child. The editor has **no chrome of its own** and inherits the ambient text
style (size + color) from its wrapper — set those on the parent:

```rust
div()
    .text_size(px(16.))
    .text_color(rgb(0xe6e6e6))
    .child(editor.clone())
```

## Markdown live preview — opt in at runtime, not compile time

The editor is a **text editor first**: create it, focus it, and it edits plain
text. The whole Markdown/WYSIWYG side is dormant until the host installs a
[`SyntaxStyle`] — there is deliberately **no cargo feature** for it, because
its only compile-time cost is the dependency-free `gpui_markdown::syntax`
module, and every markdown code path is dead (and dead-code-eliminated) unless
these calls are made:

```rust
editor.update(cx, |ed, cx| {
    // 1. REQUIRED for any live styling: colors + fonts, from your theme.
    ed.set_markdown_style(my_syntax_style(), cx);

    // 2. OPTIONAL, feature by feature — skip any you don't need:
    // standalone image rows render via your image pipeline…
    ed.set_block_image_provider(|src| my_images.get(src));
    // …file chips (e.g. PDF embeds) show a label and click through OpenLink…
    ed.set_block_chip_provider(|src| is_pdf(src).then(|| file_name(src).into()));
    // …standalone ![[Note]] lines render the host's transclusion view in a
    // reserved gap of the given height…
    ed.set_embed_provider(|target| my_embeds.get(target));
    // …```mermaid fences render as diagrams (raster + logical w/h)…
    ed.set_block_mermaid_provider(|src| my_mermaid.get(src));
    // …$$…$$ blocks and inline $…$ render typeset (logical size; set the em
    // the rasters were typeset at so inline formulas scale to your text)…
    ed.set_block_math_provider(|src| my_math.get(src));
    ed.set_block_math_em(22.0);
    // …and fenced code with a language tag colors its tokens.
    ed.set_code_highlighter(|lang, code| my_highlighter.highlight(lang, code));
});
```

Then handle the interaction events (see [Events](#events)): `OpenLink` /
`OpenWikiLink` for clicks on links, chips, and bare URLs; `EditMath` /
`MathMenu` if you host a math editor. Alert titles (`> [!NOTE]` …) can show
icons by supplying SVG asset paths in `SyntaxStyle::alert_icons`.

Everything renders **raw under the caret** (move onto a construct to edit its
source), and `clear_markdown_style` reverts the whole surface to plain text at
runtime — e.g. a user's "WYSIWYG off" toggle.

## Key bindings

`bind_keys(cx)` binds these in the editor's `"Editor"` key context (so they don't
shadow the host's shortcuts). `cmd-*` is macOS; `ctrl-*` is the cross-platform
equivalent.

| Keys | Action |
| --- | --- |
| typing, `backspace`, `delete`, `enter` | edit text |
| `←` `→` `↑` `↓`, `home`, `end` | move caret |
| `alt-←` / `alt-→` | word left / right |
| `shift-` + any move | extend selection |
| `cmd-a` | select all |
| `cmd-b` / `cmd-i` / `cmd-e` | bold / italic / inline code (toggle on selection) |
| `tab` / `shift-tab` | indent / outdent (list-aware) |
| `cmd-c` / `cmd-x` / `cmd-v` | copy / cut / paste |
| `cmd-z` / `cmd-shift-z` (`ctrl-y`) | undo / redo |
| `ctrl-cmd-space` | macOS character palette |
| `escape` | dismiss the right-click suggestions menu |

`tab`/`shift-tab` indent or outdent the caret's list item by [`set_tab_indent`]
spaces (or insert/remove that many spaces elsewhere), and move between cells when
the caret is in a table. **Enter** continues a list or task (an empty item exits)
and, inside a table, moves to the cell below; the **arrow keys** walk a table
cell-by-cell, keeping your column.

## Events

Subscribe with `cx.subscribe(&editor, …)`. [`EditorEvent`]:

| Variant | Meaning |
| --- | --- |
| `Changed` | The text changed via a user edit (typing, delete, paste, IME, applying a suggestion). **Not** emitted for programmatic [`set_text`]. |
| `OpenLink(SharedString)` | A file chip, `[text](url)` link, or bare URL was left-clicked — the host should open the `src`. |
| `OpenWikiLink(SharedString)` | A `[[wiki-link]]` / `#tag` (or a wiki file chip / property pill) was left-clicked, with the target name — the host navigates. The target may carry a `#Heading` / `#^id` anchor. |
| `SelectionChanged` | The caret/selection moved without a text change — for updating a caret-anchored affordance (e.g. a table-alignment toolbar). |
| `EditMath { range, source, at_end, inline }` | The caret entered a `$$…$$` block or inline `$…$` formula (click / arrow-in). The host opens a structural editor seeded from `source`, seats it (`set_editing_block` / `set_editing_inline`), and overwrites `range` on commit. `inline` distinguishes block vs in-line; `at_end` seats the caret at the formula's end vs start. |
| `MathMenu { source, position }` | A rendered formula was right-clicked — the host shows a context menu (copy LaTeX / export) at the window-space `position`. |
| `EditProperties { range, source, at_end }` | A `key:: value` property panel was clicked or arrowed into: the block's byte `range` + `source`. The host seats an in-place property editor (`set_editing_block`) and overwrites `range` on commit — the same seat/commit pattern as `EditMath`. `at_end` = entered from below (focus the last field). |
| `PreviewImage(SharedString)` | An inline (mid-text) image thumbnail was left-clicked — the host opens a full-size preview. The text is untouched. |

---

## API reference

### `fn bind_keys(cx: &mut App)`

Bind the editor's editing keys. Call once at startup. Bindings are scoped to the
`"Editor"` key context.

### `fn mermaid_sources(content: &str) -> Vec<SharedString>`

The diagram sources of every ` ```mermaid ` block in `content`, so a host can
pre-render them off-thread before the editor's mermaid provider is consulted.

### `fn math_sources(content: &str) -> Vec<SharedString>` · `fn inline_math_sources(content: &str) -> Vec<SharedString>`

The LaTeX of every `$$…$$` block / inline `$…$` formula in `content`, so a host
can pre-render them off-thread before the math provider is consulted. (Inline
formulas reuse the same store, keyed by LaTeX.)

### `struct EditorState`

The editor: text + caret/selection state, undo/redo history, and a cached layout
(the wrapped lines from the last paint, for hit-testing + IME). Implements
`Render` and `Focusable`.

#### Construction

```rust
fn new(window: &mut Window, cx: &mut Context<Self>) -> Self
fn with_text(self, text: impl Into<String>) -> Self        // builder; caret at start
fn with_placeholder(self, text: impl Into<SharedString>) -> Self  // builder; shown when empty
```

#### Content

```rust
fn text(&self) -> &str                                       // borrowed
fn value(&self) -> SharedString                              // owned
fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>)
```

`set_text` replaces the whole document, resets the caret to the start, and clears
undo history. It does **not** emit `Changed` (it's a programmatic load).

#### Caret & geometry

```rust
fn cursor(&self) -> usize                                    // caret byte offset
fn set_cursor(&mut self, offset: usize, cx: &mut Context<Self>)  // clamped to a char boundary
fn focus(&self, window: &mut Window, cx: &mut Context<Self>) // enter edit mode
fn bounds_for_offset(&self, offset: usize) -> Option<Bounds<Pixels>>  // window-space caret box
fn last_edit_was_keystroke(&self) -> bool                    // gate auto-pairing on this
```

- `set_cursor` only moves the caret; call `focus` to actually receive keyboard
  input (e.g. when entering edit mode from clicked rendered text).
- `bounds_for_offset` reads the last paint's layout — `None` before the first
  paint. Use it to anchor a popup (slash menu, toolbar) at a document offset.
- `last_edit_was_keystroke` is `true` only after a single typed character or a
  single-character backspace — not after a programmatic / multi-char edit (table
  ops, paste). A host that does its own auto-pairing should gate it on this.

#### Spell-check / diagnostics

```rust
fn set_diagnostics(&mut self, diagnostics: Vec<Diagnostic>, cx: &mut Context<Self>)
fn on_suggest(&mut self, provider: impl Fn(&str) -> Vec<String> + 'static)
```

The host computes [`Diagnostic`] spans (e.g. with the [`os-spellcheck`] crate) and
feeds them in — each underlined with a red squiggle. `on_suggest` installs the
provider consulted **only on right-click** of a flagged word (kept lazy because
the OS suggestion call can be slow); it returns replacements, best first, shown
in a popup menu that applies the chosen one on click.

#### Live Markdown styling (WYSIWYG)

```rust
fn set_markdown_style(&mut self, style: SyntaxStyle, cx: &mut Context<Self>)
fn clear_markdown_style(&mut self, cx: &mut Context<Self>)
```

With a [`SyntaxStyle`] installed, the editor renders Markdown live (markers hidden
except around the caret). `clear_markdown_style` falls back to plain text (spell
squiggles only) — e.g. when the host's WYSIWYG setting is toggled off. See
[Markdown live preview](#markdown-live-preview--opt-in-at-runtime-not-compile-time)
for the full recipe, including `set_code_highlighter` for token-colored fenced
code.

#### Block widgets

Standalone `![](src)` lines, ` ```mermaid ` blocks, and `$$…$$` math render as
widgets when the caret is elsewhere (raw source under the caret). The host owns
loading/caching/rendering and supplies a provider:

```rust
fn set_block_image_provider(&mut self, provider: impl Fn(&str) -> Option<Arc<RenderImage>> + 'static)
fn set_block_chip_provider(&mut self, provider: impl Fn(&str) -> Option<SharedString> + 'static)
fn set_embed_provider(&mut self, provider: impl Fn(&str) -> Option<(AnyView, Pixels)> + 'static)
fn set_block_mermaid_provider(&mut self, provider: impl Fn(&str) -> Option<Arc<RenderImage>> + 'static)
fn set_block_math_provider(&mut self, provider: impl Fn(&str) -> Option<Arc<RenderImage>> + 'static)
fn set_block_math_em(&mut self, em: f32)   // em the math provider rasterizes at — enables inline `$…$`
```

- **Image:** resolve `src` → a decoded `RenderImage` (or `None` while loading →
  the line shows raw `![](src)`).
- **Chip:** classify an `![](src)` as a file chip (e.g. a PDF) and return its
  label → the line renders as a clickable chip; a left-click emits
  `EditorEvent::OpenLink(src)`, a right-click places the caret to edit.
- **Embed:** resolve a standalone `![[target]]` line (Obsidian transclusion)
  to a host view + the row height to reserve → the editor reserves the gap in
  its layout and paints the `AnyView` there as an absolute overlay (skipped on
  the caret's row, where the raw `![[…]]` text shows for editing). The host
  owns resolution (fetch the target page, render it — typically with
  [`gpui-markdown`](../gpui-markdown/README.md), whose `syntax::embed_targets`
  / `extract_block` / `extract_section` slice `#^id` / `#Heading` anchors) and
  refreshing the views when a source page changes. `None` renders a compact
  `⧉ target` chip for an unresolved target.
- **Mermaid:** resolve a fenced block's source → a rendered diagram bitmap.
  Pre-render with [`mermaid_sources`].
- **Math:** resolve a formula's LaTeX → a typeset bitmap. Pre-render with
  [`math_sources`]. Calling `set_block_math_em` (with the em the provider
  rasterizes at) also turns on **inline `$…$`** — the editor reuses the block
  raster scaled to text size, painting it over a reserved gap in the line
  (pre-render those with [`inline_math_sources`]). Editing is the host's: a
  click/arrow into a formula emits `EditMath`, and the host seats its own 2-D
  editor via `set_editing_block` (a full-row gap, for `$$…$$`) or
  `set_editing_inline` (in-place, for `$…$`), then overwrites the byte range on
  commit. `set_editing_inline` / `is_inline_math_range` / `find_inline_math` are
  the inline counterparts of the block hooks.

#### Indentation

```rust
fn set_tab_indent(&mut self, spaces: usize)   // spaces per Tab / list-nesting level (min 1)
```

#### Table editing

The editor renders GFM tables as a grid and edits inside the cells. These let a
host drive column alignment and structural edits (e.g. from a toolbar or
right-click menu); each is a no-op when the caret isn't in a table.

```rust
fn caret_table_align(&self) -> Option<CellAlign>      // current column's alignment (header row only)
fn set_caret_table_align(&mut self, align: CellAlign, cx: &mut Context<Self>)  // rewrites the `|---|` separator
fn insert_table_row(&mut self, below: bool, cx: &mut Context<Self>)   // above / below the caret's row
fn delete_table_row(&mut self, cx: &mut Context<Self>)                // body rows only
fn insert_table_column(&mut self, right: bool, cx: &mut Context<Self>)  // left / right of the caret's column
fn delete_table_column(&mut self, cx: &mut Context<Self>)             // not the last column
fn delete_table(&mut self, cx: &mut Context<Self>)                    // the whole table block
```

`caret_table_align` returns `Some` only while the caret is in the **header** row
(alignment is a per-column property, set once from the header), so it doubles as
"should I show the alignment control?".

---

## Types

### `struct SyntaxStyle`

Colors + monospace font for the live-preview styling, supplied by the host so the
editor stays theme-agnostic. All fields are `gpui::Hsla` except `mono: gpui::Font`.

| Field | Styles |
| --- | --- |
| `marker` | dimmed syntax markers (`**`, `` ` ``, `[`, `](…)`, …) |
| `code` | inline `` `code` `` text |
| `code_bg` | inline-code background (also the table row-shade tint) |
| `link` | `[text](url)`, `[[wiki-links]]`, footnote/reference refs |
| `tag` | `#tags` |
| `quote` | blockquote text + left border (a muted tone) |
| `alert_note` … `alert_caution` | GitHub alert (`> [!NOTE]` …) bar + title, one per kind |
| `alert_icons` | `Option<AlertIcons>` — SVG asset paths for the alert title icons (`None` = label only) |
| `rule` | thematic break (`---`) divider |
| `mark_bg` | `<mark>` highlight background |
| `popover_*` | the built-in right-click menus (table ops, spell suggestions) |
| `mono` | monospace font for inline code + code blocks |
| `property_icon` | `Option<PropertyIconFn>` — property key → SVG asset path for the panel's per-key icons (`None` = no icons) |

### `struct Diagnostic`

```rust
pub struct Diagnostic { pub range: Range<usize> }   // byte range in the document
```

A flagged span to underline. `&text[range]` is the offending word.

### `enum EditorEvent`

`Changed` · `OpenLink(SharedString)` · `OpenWikiLink(SharedString)` ·
`SelectionChanged` · `EditMath { range, source, at_end, inline }` ·
`MathMenu { source, position }` · `EditProperties { range, source, at_end }` ·
`PreviewImage(SharedString)` — see [Events](#events).

### `enum CellAlign`

`Left` · `Center` · `Right` — a table column's text alignment, for
`caret_table_align` / `set_caret_table_align`.

---

## Demo

A standalone window wired to the real OS spell checker (via the [`os-spellcheck`]
crate), live Markdown styling, a PDF-style file chip, and styled tables:

```sh
cargo run -p gpui-editor --example demo
```

Type to watch the spell squiggles update; right-click a flagged word for
suggestions.

## Notes & caveats

- **Main thread:** like all GPUI UI, drive the editor on the main thread.
- **No styling without a `SyntaxStyle`:** absent `set_markdown_style`, the editor
  is a plain-text editor (with spell squiggles if diagnostics are fed in).
- The editor caches the **last paint's layout** for hit-testing, `bounds_for_offset`,
  and IME — those return `None`/defaults before the first paint.

## License

GPL-3.0-or-later.

[`Diagnostic`]: #struct-diagnostic
[`SyntaxStyle`]: #struct-syntaxstyle
[`EditorEvent`]: #enum-editorevent
[`set_text`]: #content
[`set_tab_indent`]: #indentation
[`mermaid_sources`]: #fn-mermaid_sourcescontent-str---vecsharedstring
[`os-spellcheck`]: ../os-spellcheck
