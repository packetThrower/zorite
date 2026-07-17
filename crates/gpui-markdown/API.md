# gpui-markdown API

The complete public API of [`gpui-markdown`](README.md) — every exported item,
with its signature, parameters, return contract, edge cases, and cost. For the
what-and-why (the two layers, quick start, per-table designs, supported
syntax), see the [README](README.md).

The crate is two layers, and this reference mirrors that:

- **[Part I — `gpui_markdown::syntax`](#part-i--gpui_markdownsyntax)** — always
  compiled, dependency-free construct *recognition* shared by every renderer.
- **[Part II — the reader view](#part-ii--the-reader-view-feature-view)** —
  `MarkdownView` and everything around it, behind the default-on **`view`**
  feature (which owns the `gpui` + `markdown` dependencies).

## Public API at a glance

Everything below is the complete public surface — if it isn't listed here, it
isn't public. Feature `—` = always compiled (`gpui_markdown::syntax`);
`view` = behind the default-on `view` feature (crate root).

| Item | Kind | Signature | Purpose | Feature |
| --- | --- | --- | --- | --- |
| [`AlertKind`](#enum-alertkind) | enum | `Note \| Tip \| Important \| Warning \| Caution` | The five GitHub alert kinds | — |
| [`AlertKind::label`](#alertkindlabel) | method | `fn label(self) -> &'static str` | Rendered title ("Note", "Tip", …) | — |
| [`ALERT_MARKERS`](#alert_markers) | const | `[(AlertKind, &str); 5]` | `(kind, "[!NOTE]")` pairs, in matching order | — |
| [`alert_marker`](#alert_marker) | fn | `fn alert_marker(value: &str) -> Option<(AlertKind, usize, Option<bool>)>` | Match an alert marker at the start of a blockquote's text | — |
| [`alert_prefix`](#alert_prefix) | fn | `fn alert_prefix(body: &str) -> Option<(AlertKind, usize, Option<bool>)>` | Same, for a single line's body after the `>` | — |
| [`alert_fold_char`](#alert_fold_char) | fn | `fn alert_fold_char(line: &str) -> Option<(usize, bool)>` | Locate the `-`/`+` fold char on a full source line | — |
| [`toggle_alert_fold_at`](#toggle_alert_fold_at) | fn | `fn toggle_alert_fold_at(content: &str, offset: usize) -> Option<String>` | Flip a callout's fold char, returning new content | — |
| [`TableStyle`](#enum-tablestyle) | enum | `Grid \| Striped \| Header \| Minimal` | Per-table visual design (default `Grid`) | — |
| [`TableStyle::from_name`](#tablestylefrom_name) | method | `fn from_name(name: &str) -> Option<Self>` | Parse a style name (`"striped"` …) | — |
| [`table_style_marker`](#table_style_marker) | fn | `fn table_style_marker(text: &str) -> Option<TableStyle>` | Parse a `<!-- table:STYLE -->` marker comment | — |
| [`table_col_widths`](#table_col_widths) | fn | `fn table_col_widths(text: &str) -> Option<Vec<f32>>` | Explicit column widths from a marker's `cols=` | — |
| [`table_marker_text`](#table_marker_text) | fn | `fn table_marker_text(style: TableStyle, widths: Option<&[f32]>) -> Option<String>` | Write a marker line (the parsers' inverse) | — |
| [`heading_scale`](#heading_scale) | fn | `fn heading_scale(depth: u8) -> f32` | Font-size multiplier for h1–h6 | — |
| [`ordered_marker`](#ordered_marker) | fn | `fn ordered_marker(depth: usize, n: u32) -> String` | Word-style list marker (`1.` → `a.` → `i.`) | — |
| [`LinkHit`](#enum-linkhit) | enum | `Page(String) \| Url(String)` | What a clicked link-like construct targets | — |
| [`wiki_target_display`](#wiki_target_display) | fn | `fn wiki_target_display(inner: &str) -> (&str, &str)` | Split `target\|label` into `(target, display)` | — |
| [`is_tag_char`](#is_tag_char--is_word_char) | fn | `fn is_tag_char(c: u8) -> bool` | Byte valid inside a `#tag` name | — |
| [`is_word_char`](#is_tag_char--is_word_char) | fn | `fn is_word_char(c: u8) -> bool` | Word byte for boundary checks | — |
| [`url_end`](#url_end) | fn | `fn url_end(line: &str, start: usize) -> usize` | Where a bare URL ends (GFM-ish) | — |
| [`links`](#links) | fn | `fn links(line: &str) -> Vec<(Range<usize>, LinkHit)>` | Every clickable link in a line, with source ranges | — |
| [`link_at`](#link_at) | fn | `fn link_at(line: &str, col: usize) -> Option<LinkHit>` | The link under a byte column | — |
| [`block_id`](#block_id) | fn | `fn block_id(line: &str) -> Option<(usize, &str)>` | Trailing ` ^block-id` anchor on a line | — |
| [`split_block_anchor`](#split_block_anchor) | fn | `fn split_block_anchor(target: &str) -> (&str, Option<&str>)` | Split `Note#^id` into `(page, block id)` | — |
| [`split_heading_anchor`](#split_heading_anchor) | fn | `fn split_heading_anchor(target: &str) -> (&str, Option<&str>)` | Split `Note#Heading` into `(page, heading)` | — |
| [`find_heading_line`](#find_heading_line) | fn | `fn find_heading_line(content: &str, heading: &str) -> Option<usize>` | Byte offset of a matching ATX heading's line | — |
| [`find_block_line`](#find_block_line) | fn | `fn find_block_line(content: &str, id: &str) -> Option<usize>` | Byte offset of the line carrying `^id` | — |
| [`embed_line`](#embed_line) | fn | `fn embed_line(line: &str) -> Option<&str>` | Target of a standalone `![[target]]` line | — |
| [`embed_targets`](#embed_targets) | fn | `fn embed_targets(content: &str) -> Vec<String>` | Every standalone embed target, in order | — |
| [`extract_block`](#extract_block) | fn | `fn extract_block(content: &str, id: &str) -> Option<Range<usize>>` | Source range of the block carrying `^id` | — |
| [`extract_section`](#extract_section) | fn | `fn extract_section(content: &str, heading: &str) -> Option<Range<usize>>` | Source range of a heading's section | — |
| [`property`](#property) | fn | `fn property(line: &str) -> Option<(&str, &str)>` | Split a `key:: value` line into `(key, value)` | — |
| [`PropSeg`](#enum-propseg) | enum | `Text(String) \| Pill { label, target, is_tag }` | A rendered piece of a property value | — |
| [`property_value_segments`](#property_value_segments) | fn | `fn property_value_segments(value: &str) -> Vec<PropSeg>` | Split a property value into text + link pills | — |
| [`MarkdownView`](#struct-markdownview) | struct | — | The renderable reader element (`RenderOnce`) | `view` |
| [`MarkdownView::new`](#markdownviewnew) | constructor | `fn new(id_base: impl Into<SharedString>, source: impl Into<SharedString>) -> Self` | Create a view (unique id + markdown source) | `view` |
| [`MarkdownView::style`](#markdownviewstyle) | builder | `fn style(self, style: MarkdownStyle) -> Self` | Set colors/sizes | `view` |
| [`MarkdownView::on_wiki_link`](#markdownviewon_wiki_link) | builder | `fn on_wiki_link(self, handler: WikiLinkHandler) -> Self` | Handle `[[wiki-link]]` / `#tag` clicks | `view` |
| [`MarkdownView::on_image`](#markdownviewon_image) | builder | `fn on_image(self, handler: ImageRenderer) -> Self` | Render standalone images | `view` |
| [`MarkdownView::on_mermaid`](#markdownviewon_mermaid) | builder | `fn on_mermaid(self, handler: MermaidRenderer) -> Self` | Render ` ```mermaid ` blocks | `view` |
| [`MarkdownView::on_highlight`](#markdownviewon_highlight) | builder | `fn on_highlight(self, handler: CodeHighlighter) -> Self` | Syntax-highlight fenced code | `view` |
| [`MarkdownView::on_math`](#markdownviewon_math) | builder | `fn on_math(self, handler: MathRenderer) -> Self` | Render `$$…$$` math blocks | `view` |
| [`MarkdownView::on_inline_math`](#markdownviewon_inline_math) | builder | `fn on_inline_math(self, handler: InlineMathRenderer) -> Self` | Render inline `$…$` formulas | `view` |
| [`MarkdownView::on_inline_image`](#markdownviewon_inline_image) | builder | `fn on_inline_image(self, handler: InlineImageRenderer) -> Self` | Render mid-text images as thumbnails | `view` |
| [`MarkdownView::search`](#markdownviewsearch) | builder | `fn search(self, query: impl Into<SharedString>, current: usize) -> Self` | In-page find highlighting | `view` |
| [`MarkdownView::track_blocks`](#markdownviewtrack_blocks) | builder | `fn track_blocks(self, handle: ScrollHandle) -> Self` | Track-scroll the block column (scroll-to-match) | `view` |
| [`MarkdownView::on_click_source`](#markdownviewon_click_source) | builder | `fn on_click_source(self, handler: ClickSourceHandler) -> Self` | Click-to-caret (source offset of a click) | `view` |
| [`MarkdownView::on_image_preview`](#markdownviewon_image_preview) | builder | `fn on_image_preview(self, handler: ImagePreviewHandler) -> Self` | Handle inline-thumbnail clicks | `view` |
| [`MarkdownView::on_task_toggle`](#markdownviewon_task_toggle) | builder | `fn on_task_toggle(self, handler: TaskToggleHandler) -> Self` | Make task checkboxes clickable | `view` |
| [`MarkdownView::on_alert_toggle`](#markdownviewon_alert_toggle) | builder | `fn on_alert_toggle(self, handler: TaskToggleHandler) -> Self` | Handle foldable-callout title clicks | `view` |
| [`MarkdownView::on_embed`](#markdownviewon_embed) | builder | `fn on_embed(self, provider: EmbedProvider) -> Self` | Resolve standalone `![[target]]` transclusions | `view` |
| [`MarkdownView::on_embed_image`](#markdownviewon_embed_image) | builder | `fn on_embed_image(self, renderer: ImageRenderer) -> Self` | Image renderer used *inside* embeds | `view` |
| [`MarkdownView::folded_headings`](#markdownviewfolded_headings) | builder | `fn folded_headings(self, folded: HashSet<String>) -> Self` | The host-owned set of collapsed headings | `view` |
| [`MarkdownView::on_heading_toggle`](#markdownviewon_heading_toggle) | builder | `fn on_heading_toggle(self, handler: HeadingToggleHandler) -> Self` | Handle heading fold-chevron clicks | `view` |
| [`MarkdownStyle`](#struct-markdownstyle) | struct | 19 pub fields | Visual configuration (host maps its theme on) | `view` |
| `impl Default for MarkdownStyle` | trait impl | `fn default() -> Self` | Neutral dark palette | `view` |
| [`AlertColors`](#struct-alertcolors) | struct | 5 pub fields | Alert border/title colors, one per kind | `view` |
| `impl Default for AlertColors` | trait impl | `fn default() -> Self` | GitHub's dark palette | `view` |
| [`AlertIcons`](#struct-alerticons) | struct | 5 pub fields | SVG asset paths for alert title icons | `view` |
| [`PropertyIconFn`](#propertyiconfn) | type alias | `Rc<dyn Fn(&str) -> Option<SharedString>>` | Property key → icon asset path | `view` |
| [`WikiLinkHandler`](#wikilinkhandler) | type alias | `Rc<dyn Fn(SharedString, &mut Window, &mut App)>` | Wiki-link / tag click callback | `view` |
| [`ImageInfo`](#struct-imageinfo) | struct | 4 pub fields | A standalone image's src/alt/width/attr range | `view` |
| [`ImageRenderer`](#imagerenderer) | type alias | `Rc<dyn Fn(ImageInfo) -> AnyElement>` | Render a standalone image | `view` |
| [`MermaidRenderer`](#mermaidrenderer) | type alias | `Rc<dyn Fn(SharedString) -> AnyElement>` | Render a mermaid block | `view` |
| [`CodeHighlighter`](#codehighlighter) | type alias | `Rc<dyn Fn(&str, &str) -> Vec<(Range<usize>, HighlightStyle)>>` | Color fenced-code tokens | `view` |
| [`MathRenderer`](#mathrenderer) | type alias | `Rc<dyn Fn(SharedString) -> AnyElement>` | Render a `$$…$$` block | `view` |
| [`InlineMathRenderer`](#inlinemathrenderer) | type alias | `Rc<dyn Fn(SharedString) -> Option<(Arc<RenderImage>, f32, f32)>>` | Inline `$…$` → raster + logical size | `view` |
| [`InlineImageRenderer`](#inlineimagerenderer) | type alias | `Rc<dyn Fn(SharedString) -> Option<(Arc<RenderImage>, f32, f32)>>` | Mid-text image → raster + logical size | `view` |
| [`ClickSourceHandler`](#clicksourcehandler) | type alias | `Rc<dyn Fn(usize, Pixels, &mut Window, &mut App)>` | Click-to-caret callback (source offset, window y) | `view` |
| [`ImagePreviewHandler`](#imagepreviewhandler) | type alias | `Rc<dyn Fn(SharedString, &mut Window, &mut App)>` | Inline-thumbnail click callback (src) | `view` |
| [`TaskToggleHandler`](#tasktogglehandler) | type alias | `Rc<dyn Fn(usize, &mut Window, &mut App)>` | Task / callout toggle callback (source offset) | `view` |
| [`HeadingToggleHandler`](#headingtogglehandler) | type alias | `Rc<dyn Fn(&str, &mut Window, &mut App)>` | Heading fold-chevron callback (fold key) | `view` |
| [`EmbedProvider`](#embedprovider) | type alias | `Rc<dyn Fn(&str) -> Option<(SharedString, SharedString)>>` | Resolve `![[target]]` → `(label, content)` | `view` |
| [`alert_children`](#alert_children) | fn | `fn alert_children(b: &mdast::Blockquote) -> Option<(AlertKind, Vec<mdast::Node>)>` | Alert kind + marker-stripped children of a blockquote | `view` |
| [`alert_parts`](#alert_parts) | fn | `fn alert_parts(b: &mdast::Blockquote) -> Option<(AlertKind, Option<bool>, usize, Vec<mdast::Node>)>` | `alert_children` + fold state + marker offset | `view` |
| [`images`](#images) | fn | `fn images(source: &str) -> Vec<ImageInfo>` | Every standalone image, in document order | `view` |
| [`all_image_srcs`](#all_image_srcs) | fn | `fn all_image_srcs(source: &str) -> Vec<SharedString>` | Every image `src`, block *and* inline | `view` |
| [`toggle_task_at`](#toggle_task_at) | fn | `fn toggle_task_at(content: &str, offset: usize) -> Option<String>` | Flip the `[ ]`↔`[x]` on the line at `offset` | `view` |
| [`match_count`](#match_count) | fn | `fn match_count(source: &str, query: &str) -> usize` | In-page find: total matches | `view` |
| [`find_matches`](#find_matches) | fn | `fn find_matches(source: &str, query: &str) -> Vec<usize>` | In-page find: block index per match, in order | `view` |
| [`Snippet`](#struct-snippet-and-snippets) | struct | 3 pub fields | An authoring snippet (label, text, caret) | `view` |
| [`SNIPPETS`](#struct-snippet-and-snippets) | const | `&[Snippet]` | Built-in markdown snippets for a `/` palette | `view` |
| [`ListEdit`](#enum-listedit) | enum | `Continue(String) \| Exit { start, end }` | What Enter should do on a list/quote line | `view` |
| [`list_continuation`](#list_continuation) | fn | `fn list_continuation(value: &str, cursor: usize) -> Option<ListEdit>` | Enter continues / exits a list or quote item | `view` |
| [`INDENT`](#indent) | const | `&str = "  "` | Default two-space indent level | `view` |
| [`indent_list_line`](#indent_list_line) | fn | `fn indent_list_line(value: &str, cursor: usize, indent: &str) -> Option<(String, usize)>` | Tab: indent the caret's list/quote line | `view` |
| [`outdent_line`](#outdent_line) | fn | `fn outdent_line(value: &str, cursor: usize, indent: &str) -> Option<(String, usize)>` | Shift+Tab: outdent the caret's line | `view` |
| [`reindent`](#reindent) | fn | `fn reindent(content: &str, old: usize, new: usize) -> Option<String>` | Re-flow list nesting to a new indent width | `view` |

---

# Part I — `gpui_markdown::syntax`

Always compiled, **dependency-free** (no `gpui`, no `markdown`): the shared
construct recognition that this crate's reader, `gpui-editor`'s WYSIWYG view,
and the PDF exporter all consume, so *what a construct is* is defined exactly
once. Everything here is pure text over `&str` — no I/O, no allocation beyond
the returned values, no threading constraints.

Depend on it alone with `default-features = false`.

---

## `enum AlertKind`

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlertKind { Note, Tip, Important, Warning, Caution }
```

The five GitHub alert kinds (`> [!NOTE]` …), in GitHub's order.

| Variant | Marker | Label |
| --- | --- | --- |
| `Note` | `[!NOTE]` | "Note" |
| `Tip` | `[!TIP]` | "Tip" |
| `Important` | `[!IMPORTANT]` | "Important" |
| `Warning` | `[!WARNING]` | "Warning" |
| `Caution` | `[!CAUTION]` | "Caution" |

### `AlertKind::label`

```rust
pub fn label(self) -> &'static str
```

The title rendered in place of the marker ("Note", "Tip", …). Infallible,
`const`-cheap.

---

## `ALERT_MARKERS`

```rust
pub const ALERT_MARKERS: [(AlertKind, &str); 5]
```

`(kind, marker text)` for each alert — `(AlertKind::Note, "[!NOTE]")` etc.,
in matching order. The table [`alert_marker`](#alert_marker) /
[`alert_prefix`](#alert_prefix) iterate; public so another recognizer (e.g. an
importer) can share the exact marker strings.

---

## `alert_marker`

```rust
pub fn alert_marker(value: &str) -> Option<(AlertKind, usize, Option<bool>)>
```

Match an alert marker at the **start** of a blockquote's text content.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `value` | `&str` | A blockquote's text content (the mdast text node's value — `>` prefixes already stripped). |

**Returns** — `Some((kind, strip, fold))`:

- `kind` — the [`AlertKind`](#enum-alertkind).
- `strip` — how many bytes to remove from `value`'s start (marker, fold char,
  and the one newline/space separator) before rendering the body.
- `fold` — `Some(true)` = `-` (folded by default), `Some(false)` = `+` (open),
  `None` = not foldable.

`None` when `value` doesn't start with a marker.

**Guarantees & edge cases**

- The marker must be **uppercase** (`[!note]` doesn't match) and either alone
  on its first line (GitHub's form, `[!NOTE]\nbody`) or followed by a space
  and the body (`[!NOTE] like so` — the inline form).
- An Obsidian fold char directly after the `]` is consumed (`[!NOTE]- body`);
  a `-` separated by a space is body text, not a fold char.
- `[!NOTEXT]` (marker glued to more word chars with no separator) doesn't
  match.

**Example**

```rust
assert!(matches!(alert_marker("[!NOTE] inline"), Some((AlertKind::Note, 8, None))));
assert!(matches!(alert_marker("[!TIP]-\nbody"),  Some((AlertKind::Tip, 8, Some(true)))));
```

---

## `alert_prefix`

```rust
pub fn alert_prefix(body: &str) -> Option<(AlertKind, usize, Option<bool>)>
```

[`alert_marker`](#alert_marker) for a **single line's body** — the text after
a blockquote's `>` prefix, as a line-oriented editor sees it.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `body` | `&str` | One line's text after the `>` prefix. Leading spaces tolerated. |

**Returns** — `Some((kind, consumed, fold))` where `consumed` is the byte
length consumed **within `body`** — leading spaces, marker, fold char, and one
separator space — i.e. what an editor hides before painting the alert label.
Fold as in [`alert_marker`](#alert_marker). `None` when the line isn't an
alert marker.

**Guarantees & edge cases** — the marker may be alone on the line (nothing
after it) or followed by a single space and the body; anything else glued to
it fails the match. Same uppercase rule.

---

## `alert_fold_char`

```rust
pub fn alert_fold_char(line: &str) -> Option<(usize, bool)>
```

Locate the fold char of the alert marker on a **full source line** (the `>`
prefix included).

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `line` | `&str` | One full source line, e.g. `"> [!NOTE]- body"`. |

**Returns** — `Some((offset, folded))`: the fold char's byte offset within
`line` (it sits directly after the marker's closing `]`) and the current state
(`true` = `-`/folded, `false` = `+`/open). `None` when the line isn't a
**foldable** alert marker — a plain `> [!NOTE]` returns `None`.

**Example**

```rust
assert_eq!(alert_fold_char("> [!NOTE]- body"), Some((9, true)));
assert_eq!(alert_fold_char("> [!NOTE] body"), None);
```

---

## `toggle_alert_fold_at`

```rust
pub fn toggle_alert_fold_at(content: &str, offset: usize) -> Option<String>
```

Flip the fold state (`-` ↔ `+`) of the foldable alert marker on the line
containing byte `offset` — what a click on a callout's chevron persists (the
same pattern as [`toggle_task_at`](#toggle_task_at)).

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `content` | `&str` | The whole document. |
| `offset` | `usize` | Any byte offset on the marker's line — typically the marker offset reported by [`MarkdownView::on_alert_toggle`](#markdownviewon_alert_toggle). |

**Returns** — the full `content` with that one char flipped, or `None` when
`offset` is out of range or the line isn't a foldable alert marker.

**Guarantees & edge cases** — length-preserving (one ASCII byte swapped);
toggling twice restores the original; never panics (out-of-range `offset` →
`None`).

---

## `enum TableStyle`

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TableStyle { #[default] Grid, Striped, Header, Minimal }
```

Visual style of a GFM table, chosen per-table via a `<!-- table:STYLE -->`
marker comment on the line directly above it (see the
[README](README.md#per-table-visual-designs)). The renderers honor it;
standard Markdown viewers ignore the comment and show a plain table.

| Variant | Name | Look |
| --- | --- | --- |
| `Grid` *(default)* | `grid` | Full outer box + all row/column gridlines |
| `Striped` | `striped` | Alternate body rows shaded; no gridlines; a rule under the header |
| `Header` | `header` | Only the header row shaded; no gridlines |
| `Minimal` | `minimal` | No box or gridlines — just a rule under the header |

### `TableStyle::from_name`

```rust
pub fn from_name(name: &str) -> Option<Self>
```

Parse a bare style name (`"grid"`, `"striped"`, `"header"`, `"minimal"` —
exact, lowercase). `None` for anything else.

---

## `table_style_marker`

```rust
pub fn table_style_marker(text: &str) -> Option<TableStyle>
```

Parse a `<!-- table:STYLE -->` marker into its [`TableStyle`](#enum-tablestyle).

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `text` | `&str` | A whole line or an HTML comment's value. Surrounding whitespace tolerated. |

**Returns** — the style, or `None` for anything unrecognized (no comment
delimiters, no `table:` prefix, unknown style name) — so an unknown marker
stays a plain HTML comment. Attribute tokens after the style name (`cols=…`)
are tolerated and ignored here.

---

## `table_col_widths`

```rust
pub fn table_col_widths(text: &str) -> Option<Vec<f32>>
```

Explicit column widths (logical px) from a table marker's `cols=` attribute —
`<!-- table:grid cols=120,80,200 -->` — written by the editor's
drag-to-resize. Both renderers apply them over the content measurement.

**Returns** — the widths, or `None` when the attribute is absent or malformed
(non-numeric, non-positive, or empty) — the table stays content-measured.

---

## `table_marker_text`

```rust
pub fn table_marker_text(style: TableStyle, widths: Option<&[f32]>) -> Option<String>
```

The marker line for `style` plus optional explicit column widths (rounded to
whole px) — the inverse of the two parsers above. **Returns** `None` when the
marker would say nothing (Grid style, no widths): the default needs no marker.

---

## `heading_scale`

```rust
pub fn heading_scale(depth: u8) -> f32
```

Font-size multiplier for a heading of the given depth — one scale shared by
reading, editing, and export.

**Returns** — `1` → `1.8`, `2` → `1.5`, `3` → `1.3`, `4` → `1.15`,
`5` → `1.05`, anything else (including `0` and `6`) → `1.0` (body size).
Infallible.

---

## `ordered_marker`

```rust
pub fn ordered_marker(depth: usize, n: u32) -> String
```

The marker for ordered item `n` (1-based) at nesting `depth`, **Word-style**:
`1.` → `a.` → `i.`, cycling for deeper levels (`depth % 3`). Both views paint
ordered lists with this scheme — a deliberate divergence from CommonMark's
digits-everywhere — so nesting is readable at a glance.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `depth` | `usize` | Nesting level, 0-based. `0` = digits, `1` = letters, `2` = roman; then it cycles. |
| `n` | `u32` | Item number, **1-based**. Letters are bijective base 26 (`26` → `z.`, `27` → `aa.`); roman is lowercase (`4` → `iv.`). |

**Returns** — the marker with its trailing dot, e.g. `"2."`, `"aa."`, `"iv."`.
`n = 0` yields `"0."` at digit depths but a bare `"."` at letter/roman depths —
pass 1-based numbers.

---

## `enum LinkHit`

```rust
#[derive(Debug, PartialEq, Clone)]
pub enum LinkHit {
    Page(String),
    Url(String),
}
```

What a click on a link-like construct targets.

| Variant | Payload | Meaning |
| --- | --- | --- |
| `Page` | page title | A `[[wiki-link]]` target or a `#tag` name (Logseq semantics: a tag opens the page of that name) |
| `Url` | URL/path | An inline `[text](url)` or bare URL — hosts open http(s) externally and resolve file paths themselves |

---

## `wiki_target_display`

```rust
pub fn wiki_target_display(inner: &str) -> (&str, &str)
```

Split a wiki-link's inner text (between the `[[ ]]`) into `(target, display)`.

**Returns** — `target|label` yields `(target, label)`; an empty label falls
back to the target; no `|` yields the whole (trimmed) text twice. Both sides
trimmed. Infallible.

---

## `is_tag_char` / `is_word_char`

```rust
pub fn is_tag_char(c: u8) -> bool
pub fn is_word_char(c: u8) -> bool
```

Byte classifiers behind the link grammar, public so other recognizers match
it exactly.

- `is_tag_char` — can `c` appear inside a `#tag` name (after the `#`)? ASCII
  alphanumeric plus `_`, `-`, and `/` — Logseq-style namespaced tags
  (`#area/sub`) are one tag.
- `is_word_char` — a word character for boundary checks (a `#` glued to a
  word isn't a tag; a URL glued to a word isn't a link): ASCII alphanumeric
  plus `_`.

Byte-oriented by design: any non-ASCII byte is neither.

---

## `url_end`

```rust
pub fn url_end(line: &str, start: usize) -> usize
```

Where a bare URL starting at byte `start` ends: consumes to whitespace or a
wrapping delimiter (`<`, `>`, `"`, `` ` ``), then backs off trailing
punctuation (`.` `,` `;` `:` `!` `?` `)` `]`) — GFM-ish autolink trimming.

**Returns** — the end byte offset (exclusive). `start` itself if the URL is
empty. Never panics for in-range `start`.

---

## `links`

```rust
pub fn links(line: &str) -> Vec<(std::ops::Range<usize>, LinkHit)>
```

Every clickable link in `line`, as `(source byte range, target)` — **the** one
grammar behind every renderer's click hit-tests, hover cursors, and styling.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `line` | `&str` | One line (or any inline run) of markdown source. |

**Returns** — non-overlapping ranges in left-to-right order. Recognized:

- `[[target]]` / `[[target|alias]]` wiki-links → `Page(target)` — the whole
  bracketed span is the range; the alias is display-only.
- Inline `[text](url)` → `Url(url)` (the whole `[…](…)` span).
- `#tags` at a word boundary → `Page(tag)` (namespaced `#a/b` included).
- Bare `http://`/`https://` URLs at a word boundary → `Url` (trailing
  punctuation trimmed via [`url_end`](#url_end)).

**Guarantees & edge cases**

- **Opaque, never matched:** anything inside inline code (`` `https://x` `` is
  verbatim), images (`![alt](src)` — they render as widgets), footnote refs
  (`[^1]` — styled like a link but not one), and a `#`/URL glued to a word.
- Empty wiki targets (`[[]]`) and empty inline URLs are skipped.
- Byte-wise walk, safe on any UTF-8 (a regression test covers multi-byte text
  before a URL).

**Cost & threading** — one linear pass, no allocation beyond the output. Pure.

**Example**

```rust
let hits = links("see [[Page|alias]] and #tag/sub");
assert_eq!(hits[0].1, LinkHit::Page("Page".into()));
assert_eq!(hits[1].1, LinkHit::Page("tag/sub".into()));
```

---

## `link_at`

```rust
pub fn link_at(line: &str, col: usize) -> Option<LinkHit>
```

The link under byte `col` of `line`, if any — [`links`](#links) filtered to
the range containing `col`. What a mouse hit-test calls.

**Returns** — the [`LinkHit`](#enum-linkhit), or `None` when `col` isn't
inside any link's source range (ranges are end-exclusive).

---

## `block_id`

```rust
pub fn block_id(line: &str) -> Option<(usize, &str)>
```

The Obsidian block-id anchor at the end of `line` (` ^some-id`).

**Returns** — `Some((offset, id))`: the byte where the anchor's **leading
space** starts (so renderers can hide the whole tail) and the id itself
(without the `^`). `None` when there's no valid anchor.

**Guarantees & edge cases** — the id must be non-empty, made of word chars /
`-`, and sit at the line's **end** (trailing whitespace tolerated; a mid-line
`^id` doesn't count).

---

## `split_block_anchor`

```rust
pub fn split_block_anchor(target: &str) -> (&str, Option<&str>)
```

Split a wiki-link target into `(page, block id)`: `Note#^id` →
`("Note", Some("id"))`; anything else → `(target, None)`.

**Guarantees & edge cases** — only the `#^` form is an anchor. A bare `#`
stays part of the title (`C# Notes` is a page name; `file.pdf#p3` keeps its
page-jump meaning). Empty page or id → no split. Infallible (always returns
the tuple).

---

## `split_heading_anchor`

```rust
pub fn split_heading_anchor(target: &str) -> (&str, Option<&str>)
```

Split a wiki-link target into `(page, heading)`: `Note#My Heading` →
`("Note", Some("My Heading"))`.

**Guarantees & edge cases** — splits at the **first** `#` only when both sides
are non-empty, the anchor side isn't a block anchor (doesn't start with `^` —
check [`split_block_anchor`](#split_block_anchor) first), and the page part
isn't a PDF (`file.pdf#p3` keeps its page-jump meaning). A Zorite page title
may itself contain `#`, so navigation should prefer an existing
literal-titled page before splitting. Infallible.

---

## `find_heading_line`

```rust
pub fn find_heading_line(content: &str, heading: &str) -> Option<usize>
```

The byte offset of the start of the line carrying the ATX heading whose text
matches `heading` — drives navigation for `[[Note#Heading]]` links.

**Guarantees & edge cases** — case-insensitive, both sides trimmed; searches
top to bottom (first match wins); heading depth 1–6 with a space after the
`#`s; lines inside fenced code blocks are skipped. `None` when absent.

**Cost** — one linear pass; allocates only the lowercased needle/candidates.

---

## `find_block_line`

```rust
pub fn find_block_line(content: &str, id: &str) -> Option<usize>
```

The byte offset of the start of the line carrying the block anchor `^id`
(per [`block_id`](#block_id)), searching top to bottom — drives navigation
for `[[Note#^id]]` links. `None` when absent.

---

## `embed_line`

```rust
pub fn embed_line(line: &str) -> Option<&str>
```

The embed target when `line` is a **standalone transclusion** — exactly
`![[target]]` (Obsidian's embed syntax) and nothing else on the line
(surrounding whitespace tolerated).

**Returns** — the trimmed inner target. `None` for a mid-text embed (those
render as plain links), an empty target (`![[]]`), a plain `[[link]]`, or an
inner `]]`.

---

## `embed_targets`

```rust
pub fn embed_targets(content: &str) -> Vec<String>
```

Every standalone embed target in `content`, in document order — what a host
**pre-resolves** before rendering (recursing into each resolved content
itself for nested embeds) to build the map behind an
[`EmbedProvider`](#embedprovider). [`embed_line`](#embed_line) per line;
possibly empty.

---

## `extract_block`

```rust
pub fn extract_block(content: &str, id: &str) -> Option<std::ops::Range<usize>>
```

The source range of the block carrying the anchor `^id` — its **whole line**
(anchor included, newline excluded) — for embedding `![[Note#^id]]`.
`None` when the anchor is absent.

---

## `extract_section`

```rust
pub fn extract_section(content: &str, heading: &str) -> Option<std::ops::Range<usize>>
```

The source range of the section under `heading` — for embedding
`![[Note#Heading]]`.

**Returns** — from the heading line through the line before the next heading
of the **same or higher** level (so a section keeps its subsections); to the
end of `content` when no such heading follows. Heading matching as in
[`find_heading_line`](#find_heading_line) (case-insensitive, fenced code
skipped). `None` when the heading is absent.

---

## `property`

```rust
pub fn property(line: &str) -> Option<(&str, &str)>
```

Split a `key:: value` property line into `(key, value)` — one grammar for the
reader, the editor, and the importers.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `line` | `&str` | One source line. Leading indentation is ignored. |

**Returns** — `Some((key, value))` with the value trimmed. `None` when the
key doesn't look like an identifier.

**Guarantees & edge cases** — the key must start with an ASCII letter and
contain only ASCII alphanumerics / `-` `_` `.` — so prose containing `::`
(Zorite `[[Page::sub]]` links, `C++::method`) isn't mistaken for a property.
An empty key (`:: value`) or digit-led key (`1key::`) → `None`. The value may
be empty.

---

## `enum PropSeg`

```rust
pub enum PropSeg {
    Text(String),
    Pill {
        label: String,
        target: LinkHit,
        is_tag: bool,
    },
}
```

A rendered piece of a property value: literal text, or a link "pill" (a
rounded chip). Both of Zorite's property panels render values through this so
they pill-ify identically.

| Variant / field | Meaning |
| --- | --- |
| `Text(String)` | A literal run between pills |
| `Pill.label` | The chip's display text: a wiki-link's label, a tag without its `#`, or a link's text |
| `Pill.target` | Where a click goes ([`LinkHit`](#enum-linkhit)) |
| `Pill.is_tag` | `true` for a `#tag` (vs a wiki-link / URL) — panels tint tags differently |

---

## `property_value_segments`

```rust
pub fn property_value_segments(value: &str) -> Vec<PropSeg>
```

Split a property value into display segments — plain runs and link pills.

**Returns** — alternating [`PropSeg`](#enum-propseg)s in order. Wiki-links
show their label, tags drop the `#`, `[text](url)` shows its text, bare URLs
show themselves. A value with no links is a single `Text` segment.

**Guarantees & edge cases** — built on [`links`](#links), so the pill spans
match the reader's and editor's click hit-tests exactly.

---

# Part II — the reader view (feature `view`)

Everything below is behind the default-on **`view`** feature, which owns the
crate's only dependencies (`gpui`, `markdown`). It renders on the GPUI UI
thread like any element tree; every free function in this part is pure — no
I/O, no storage.

**Parsing** uses the [`markdown`](https://crates.io/crates/markdown) crate
with `ParseOptions::gfm()` plus block/inline math enabled. If parsing fails,
`MarkdownView` shows the raw source as plain text. Parses are memoized in a
thread-local, content-keyed LRU cache (64 entries), so re-rendering an
unchanged document every frame doesn't re-parse.

---

## `struct MarkdownView`

```rust
#[derive(IntoElement)]
pub struct MarkdownView { /* private */ }
```

A rendered markdown document element — the reader view of a note. Construct
with [`new`](#markdownviewnew), attach optional handlers (all builder methods
take and return `self`), and place it in a GPUI element tree. It implements
`RenderOnce` (hence `IntoElement`), and is **rebuilt every frame** — it holds
no state, which is why fold sets and find state are host-owned and passed in.

```rust
MarkdownView::new("note-1", source_text)
    .style(MarkdownStyle::default())
    .on_wiki_link(Rc::new(|title, window, cx| { /* navigate */ }))
```

### `MarkdownView::new`

```rust
pub fn new(id_base: impl Into<SharedString>, source: impl Into<SharedString>) -> Self
```

Create a view over `source`. **`id_base` must be unique per rendered
document** — it derives the element ids for clickable paragraphs; reusing one
across two on-screen documents collides ids. All handlers start unset (see
each builder for the fallback behavior); the style starts as
`MarkdownStyle::default()`.

### `MarkdownView::style`

```rust
pub fn style(self, style: MarkdownStyle) -> Self
```

Set colors/sizes (see [`MarkdownStyle`](#struct-markdownstyle)). Without it,
the default neutral dark palette is used.

### `MarkdownView::on_wiki_link`

```rust
pub fn on_wiki_link(self, handler: WikiLinkHandler) -> Self
```

Handle clicks on `[[wiki-links]]` and `#tags` (see
[`WikiLinkHandler`](#wikilinkhandler)). Without it they render styled but
inert. Also receives clicks on an embed's source label.

### `MarkdownView::on_image`

```rust
pub fn on_image(self, handler: ImageRenderer) -> Self
```

Render standalone images (see [`ImageRenderer`](#imagerenderer)). Without it,
an image falls back to a clickable `🖼 alt` text label.

### `MarkdownView::on_mermaid`

```rust
pub fn on_mermaid(self, handler: MermaidRenderer) -> Self
```

Render ` ```mermaid ` fences as diagrams (see
[`MermaidRenderer`](#mermaidrenderer)). Without it, a mermaid block renders as
a plain code block.

### `MarkdownView::on_highlight`

```rust
pub fn on_highlight(self, handler: CodeHighlighter) -> Self
```

Color the tokens of fenced code with a language tag (see
[`CodeHighlighter`](#codehighlighter)). Without it, code renders in the single
`code_color`.

### `MarkdownView::on_math`

```rust
pub fn on_math(self, handler: MathRenderer) -> Self
```

Render `$$…$$` math blocks (see [`MathRenderer`](#mathrenderer)). Without it,
a math block renders as its raw LaTeX in a code block.

### `MarkdownView::on_inline_math`

```rust
pub fn on_inline_math(self, handler: InlineMathRenderer) -> Self
```

Render inline `$…$` formulas (see
[`InlineMathRenderer`](#inlinemathrenderer)). Without it, inline math stays
literal `$…$` text.

### `MarkdownView::on_inline_image`

```rust
pub fn on_inline_image(self, handler: InlineImageRenderer) -> Self
```

Render mid-text images as small in-flow thumbnails (see
[`InlineImageRenderer`](#inlineimagerenderer)). Without it, an inline image
stays a clickable label.

### `MarkdownView::search`

```rust
pub fn search(self, query: impl Into<SharedString>, current: usize) -> Self
```

In-page find: highlight case-insensitive occurrences of `query` in the
rendered (visible) text, emphasizing the `current`-th match (0-based, document
order — `search_bg` / `search_current_bg` in the style). An empty query
highlights nothing. The host owns the find bar and the match index + total —
pair with [`match_count`](#match_count) to size "n of m" and bound `current`,
and with [`track_blocks`](#markdownviewtrack_blocks) +
[`find_matches`](#find_matches) to scroll the active match into view. Matches
inside embeds are not highlighted.

### `MarkdownView::track_blocks`

```rust
pub fn track_blocks(self, handle: ScrollHandle) -> Self
```

Track-scroll the block column with `handle` so the host can read each
top-level block's laid-out bounds via `ScrollHandle::bounds_for_item` —
indexed exactly as [`find_matches`](#find_matches) reports — and scroll a
match into view. Pair with [`search`](#markdownviewsearch).

### `MarkdownView::on_click_source`

```rust
pub fn on_click_source(self, handler: ClickSourceHandler) -> Self
```

Click-to-caret: report the **source** byte offset nearest a click on the
rendered text, outside a link (see
[`ClickSourceHandler`](#clicksourcehandler)). Suppressed inside embeds (their
offsets belong to another document).

### `MarkdownView::on_image_preview`

```rust
pub fn on_image_preview(self, handler: ImagePreviewHandler) -> Self
```

Handle a click on an inline thumbnail — open a full-size preview (see
[`ImagePreviewHandler`](#imagepreviewhandler)).

### `MarkdownView::on_task_toggle`

```rust
pub fn on_task_toggle(self, handler: TaskToggleHandler) -> Self
```

Make task checkboxes clickable: clicking a `☐`/`☑` calls the handler with the
task item's source byte offset, so the host can flip it — feed the offset to
[`toggle_task_at`](#toggle_task_at) — and persist. Without this, checkboxes
render but aren't interactive. Suppressed inside embeds.

### `MarkdownView::on_alert_toggle`

```rust
pub fn on_alert_toggle(self, handler: TaskToggleHandler) -> Self
```

Handle a foldable callout's title click, with the `[!KIND]` marker's source
byte offset — the host flips the `-`/`+` fold char with
[`syntax::toggle_alert_fold_at`](#toggle_alert_fold_at) and persists, like a
task toggle. (Same [`TaskToggleHandler`](#tasktogglehandler) alias.)
Suppressed inside embeds.

### `MarkdownView::on_embed`

```rust
pub fn on_embed(self, provider: EmbedProvider) -> Self
```

Install the embed resolver: a standalone `![[target]]` line renders the
target's content in a bordered, quoted box with a clickable source label. See
[`EmbedProvider`](#embedprovider) for the pre-resolve pattern and what's
suppressed inside an embed.

### `MarkdownView::on_embed_image`

```rust
pub fn on_embed_image(self, renderer: ImageRenderer) -> Self
```

The image renderer used **inside** embeds, replacing
[`on_image`](#markdownviewon_image) there. Hosts supply a read-only (grip-less)
variant: an embedded image's `attr_target` belongs to the *embedded* page, and
a resize written through the embedding page's handler would corrupt it. Unset
= no images render in embeds.

### `MarkdownView::folded_headings`

```rust
pub fn folded_headings(self, folded: HashSet<String>) -> Self
```

The collapsed headings — keys are **trimmed source lines** (e.g. `"## Goals"`).
A folded heading renders with a `▸` chevron and its whole section (everything
until the next heading at its level or higher) is skipped. Host-owned state,
since this view is rebuilt every frame; the line-text key is shared with
gpui-editor's WYSIWYG folds and self-heals — editing the heading drops the
fold instead of letting it drift.

### `MarkdownView::on_heading_toggle`

```rust
pub fn on_heading_toggle(self, handler: HeadingToggleHandler) -> Self
```

Handle a heading fold-chevron click (see
[`HeadingToggleHandler`](#headingtogglehandler)): the host toggles the key in
its fold set and re-renders. Without a handler headings show **no chevron** —
the switch for embeds and other read-only surfaces.

**Example** (fold wiring)

```rust
MarkdownView::new("note-1", source)
    .folded_headings(my_folds.clone())
    .on_heading_toggle(Rc::new(move |key, _window, cx| {
        // insert/remove `key` in your set, then notify to re-render
    }))
```

---

## `struct MarkdownStyle`

```rust
#[derive(Clone)]
pub struct MarkdownStyle { /* 19 pub fields, below */ }
```

Visual configuration for the renderer. The host fills this from its own
theme; `MarkdownStyle::default()` is a neutral dark palette. The renderer
sets only `text_size` — set the font family on a parent element if needed.

| Field | Type | Purpose |
| --- | --- | --- |
| `text_color` | `Hsla` | Body text |
| `text_size` | `Pixels` | Base size; headings scale from it via [`heading_scale`](#heading_scale) (default `px(15.0)`) |
| `line_height` | `f32` | Body line height as a multiple of `text_size`. Hosts with an editor match its ratio so reading and editing line up (default `1.45`, gpui-editor's) |
| `heading_color` | `Hsla` | h1–h6 |
| `link_color` | `Hsla` | Links, footnote markers, image labels |
| `tag_color` | `Hsla` | `#tags` |
| `code_color` | `Hsla` | Inline + fenced code text |
| `code_bg` | `Hsla` | Fenced-code background; also the striped/header table shade |
| `muted_color` | `Hsla` | Blockquotes, list markers, table borders, footnote definitions, raw HTML |
| `rule_color` | `Hsla` | Thematic break (`---`) divider |
| `guide_color` | `Hsla` | Nested-list indent guide — a hairline, fainter than `rule_color` |
| `mark_bg` | `Hsla` | `<mark>…</mark>` highlight background (translucent so text stays readable) |
| `search_bg` | `Hsla` | In-page find: every match (translucent) |
| `search_current_bg` | `Hsla` | In-page find: the active match |
| `list_indent` | `Pixels` | Horizontal indent per nested list level — size to your editor's literal indent so reading + editing line up (default `px(18.0)`) |
| `mono_font` | `SharedString` | Monospace family for code; an unknown family falls back to the default font (default `"monospace"`) |
| `alerts` | [`AlertColors`](#struct-alertcolors) | GitHub alert border + title colors, one per kind |
| `alert_icons` | `Option<AlertIcons>` | SVG asset paths for alert title icons, resolved through the host's `AssetSource`. `None` (default) = title without an icon, keeping the crate asset-free |
| `property_icon` | `Option<PropertyIconFn>` | Property key → icon before it in the property panel. `None` (default) = no icons |

`impl Default for MarkdownStyle` — the neutral dark palette
(`text_color: 0xE6E6E6`, `link_color: 0x4C9EFF`, `tag_color: 0x9D7CD8`, …).

---

## `struct AlertColors`

```rust
#[derive(Clone, Copy)]
pub struct AlertColors {
    pub note: Hsla,
    pub tip: Hsla,
    pub important: Hsla,
    pub warning: Hsla,
    pub caution: Hsla,
}
```

Border + title colors for the five GitHub-style alerts, one field per
[`AlertKind`](#enum-alertkind). `impl Default` is GitHub's dark palette
(`note: 0x4493F8`, `tip: 0x3FB950`, `important: 0xAB7DF8`,
`warning: 0xD29922`, `caution: 0xF85149`); the host overlays its theme.

---

## `struct AlertIcons`

```rust
#[derive(Clone)]
pub struct AlertIcons {
    pub note: SharedString,
    pub tip: SharedString,
    pub important: SharedString,
    pub warning: SharedString,
    pub caution: SharedString,
}
```

Per-kind SVG asset paths for the alert title icons, resolved through the
host's `AssetSource`. Set via `MarkdownStyle::alert_icons`; no `Default` —
supplying it is opting in to icons.

---

## `PropertyIconFn`

```rust
pub type PropertyIconFn = Rc<dyn Fn(&str) -> Option<SharedString>>;
```

Maps a property **key** (`tags`, `status`, …) to an icon asset path the host
serves, or `None` for no icon on that key. Called while rendering a property
panel, once per key row. Host-provided (via `MarkdownStyle::property_icon`) so
the crate makes no assumption about which assets exist.

---

## `WikiLinkHandler`

```rust
pub type WikiLinkHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
```

Fires when the user clicks a `[[wiki-link]]` or `#tag`, with the **target
name** (trimmed):

- `[[Some Page]]` → called with `"Some Page"`.
- `[[target|label]]` → displays `label`, called with `"target"` (e.g.
  `[[file.pdf#p3|↗]]` shows `↗` linking to `file.pdf#p3`). An empty label
  falls back to the target.
- `#some-tag` → called with `"some-tag"` (the bare name; the displayed `#` is
  kept).

Standard `[text](url)` and reference-style links open externally via
`cx.open_url` and do **not** go through this handler. Host obligation:
navigate (Logseq semantics — tags and wiki-links open pages by title); use
[`split_block_anchor`](#split_block_anchor) /
[`split_heading_anchor`](#split_heading_anchor) to peel `#^id` / `#Heading`
anchors off the target. Set via
[`on_wiki_link`](#markdownviewon_wiki_link).

---

## `struct ImageInfo`

```rust
pub struct ImageInfo {
    pub src: SharedString,
    pub alt: SharedString,
    pub width: Option<f32>,
    pub attr_target: Range<usize>,
}
```

A standalone image — a paragraph (or list item) that **begins** with
`![alt](src)`, optionally followed by a `{width=N}` attribute and/or caption
text. Handed to the host's [`ImageRenderer`](#imagerenderer); also returned by
[`images`](#images).

| Field | Type | Purpose |
| --- | --- | --- |
| `src` | `SharedString` | The image URL/path exactly as written |
| `alt` | `SharedString` | Alt text (may be empty) |
| `width` | `Option<f32>` | Explicit pixels from a `{width=N}` (or `{width=Npx}`) attribute, if present |
| `attr_target` | `Range<usize>` | Byte range in the **source** to replace with `{width=N}` when resizing: an empty range just after the image when there's no attribute yet, or the existing attribute's span when there is one |

`attr_target` supports **resize-by-rewriting-the-markdown**: a host resize
handle computes a new width and rewrites
`source[attr_target] = "{width=N}"`.

---

## `ImageRenderer`

```rust
pub type ImageRenderer = Rc<dyn Fn(ImageInfo) -> AnyElement>;
```

Renders a standalone image. Fires during render for each paragraph that leads
with an image; any trailing caption text renders below the element. Inline
images mixed within text do **not** go through this (see
[`InlineImageRenderer`](#inlineimagerenderer)).

Building the returned element needs no `Window`/`App` — its event handlers
run later with their own context — so the host can return a stateful,
interactive (draggable, resizable) element while this crate stays
host-agnostic. Set via [`on_image`](#markdownviewon_image) and, for the
read-only variant used inside embeds,
[`on_embed_image`](#markdownviewon_embed_image).

**Example**

```rust
view.on_image(Rc::new(|info: ImageInfo| {
    let mut image = gpui::img(resolve(&info.src)); // your path/URL -> ImageSource
    if let Some(w) = info.width { image = image.w(px(w)); }
    image.into_any_element()
}))
```

---

## `MermaidRenderer`

```rust
pub type MermaidRenderer = Rc<dyn Fn(SharedString) -> AnyElement>;
```

Renders a ` ```mermaid ` code block as a diagram, given the block's source.
This crate just detects the fence and hands the source over — the host owns
the (typically expensive, async) render and any caching, staying
renderer-agnostic. Called on every render pass, so the host **must** cache;
return a placeholder while a render is in flight. Set via
[`on_mermaid`](#markdownviewon_mermaid).

---

## `CodeHighlighter`

```rust
pub type CodeHighlighter = Rc<dyn Fn(&str, &str) -> Vec<(Range<usize>, HighlightStyle)>>;
```

Colors a fenced code block's tokens: `(language tag, code)` → styled ranges
(**byte offsets into the code**, sorted and non-overlapping — the host's
obligation). Supplied by the host (e.g. a tree-sitter highlighter) so the
crate stays engine-free. Fires per fenced block with a language tag; blocks
without a tag (and mermaid blocks with a renderer installed) skip it. Set via
[`on_highlight`](#markdownviewon_highlight).

---

## `MathRenderer`

```rust
pub type MathRenderer = Rc<dyn Fn(SharedString) -> AnyElement>;
```

Renders a `$$…$$` math block as a typeset element, given the block's LaTeX.
Like [`MermaidRenderer`](#mermaidrenderer), the host owns the (cached,
off-thread) render — this crate just detects the block (`math_flow` is
enabled in the parser) and hands over the source. Set via
[`on_math`](#markdownviewon_math).

---

The view wraps the returned element in a **justified row**: centered by
default (the LaTeX convention), or left/right when a `<!-- math:left -->` /
`<!-- math:right -->` marker line sits directly above the block — so the
renderer should return a content-sized element and leave alignment to the
view.

## `InlineMathRenderer`

```rust
pub type InlineMathRenderer = Rc<dyn Fn(SharedString) -> Option<(Arc<RenderImage>, f32, f32)>>;
```

Resolves an inline `$…$` formula's LaTeX to its typeset raster plus its
logical `(width, height)` in display px at text size. Return `None` while
still rasterizing — the raw `$…$` shows until then (kick off the render and
notify to re-render when it lands).

The renderer reserves a non-breaking spacer of that width in the paragraph's
text and paints the raster over it (via a `canvas` reading the laid-out glyph
position **in the same frame**), so the surrounding `StyledText` — and thus
links, in-page find, and click-to-caret — is preserved and the line wraps
normally. The paragraph's line height grows to fit a tall formula. `$…$`
parsing follows the `markdown` crate's `math_text` rules, so prose like
`it cost $5` stays literal. Set via
[`on_inline_math`](#markdownviewon_inline_math).

---

## `InlineImageRenderer`

```rust
pub type InlineImageRenderer = Rc<dyn Fn(SharedString) -> Option<(Arc<RenderImage>, f32, f32)>>;
```

Renders a **mid-text** image (`![](src)` amid text — one that doesn't lead its
paragraph) as a small in-flow thumbnail: given the `src`, return the decoded
raster plus the logical `(width, height)` to flow at, or `None` (still
decoding / remote / PDF) to keep the clickable-label fallback. Same
reserved-spacer machinery as inline math, so the line wraps normally and
grows to fit. A click on the thumbnail dispatches to
[`on_image_preview`](#markdownviewon_image_preview). Set via
[`on_inline_image`](#markdownviewon_inline_image). (Same signature as
[`InlineMathRenderer`](#inlinemathrenderer); the argument is a `src`, not
LaTeX.)

---

## `ClickSourceHandler`

```rust
pub type ClickSourceHandler = Rc<dyn Fn(usize, Pixels, &mut Window, &mut App)>;
```

Fires when the rendered text is clicked **outside a link**, with the
**source** byte offset nearest the click and the click's window **y**. A host
uses it to place its editor's caret there and keep it under the cursor when
switching into edit mode. The crate maps the click through gpui's text layout
plus a source-offset map it builds while rendering (accounting for stripped
`[[ ]]` / `#` / inline-code markup). Set via
[`on_click_source`](#markdownviewon_click_source); suppressed inside embeds.

---

## `ImagePreviewHandler`

```rust
pub type ImagePreviewHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
```

Fires when an inline thumbnail is clicked, with the image's `src` — the host
opens a full-size preview. Set via
[`on_image_preview`](#markdownviewon_image_preview).

---

## `TaskToggleHandler`

```rust
pub type TaskToggleHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;
```

Fires with a **source byte offset** the host writes back at. Used by two
builders:

- [`on_task_toggle`](#markdownviewon_task_toggle) — a `- [ ]` checkbox was
  clicked; the offset is the task item's. Flip it with
  [`toggle_task_at`](#toggle_task_at) and persist.
- [`on_alert_toggle`](#markdownviewon_alert_toggle) — a foldable callout's
  title was clicked; the offset is the `[!KIND]` marker's. Flip the `-`/`+`
  with [`syntax::toggle_alert_fold_at`](#toggle_alert_fold_at) and persist.

---

## `HeadingToggleHandler`

```rust
pub type HeadingToggleHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;
```

Fires when a heading's fold chevron is clicked, with the heading's **fold
key** — its trimmed source line (`"## Goals"`). The host owns the fold set
(this view is rebuilt every frame): toggle the key, then notify to re-render,
passing the set back via
[`folded_headings`](#markdownviewfolded_headings).

---

## `EmbedProvider`

```rust
pub type EmbedProvider = Rc<dyn Fn(&str) -> Option<(SharedString, SharedString)>>;
```

Resolves a standalone `![[target]]` line (Obsidian transclusion) to the
`(source label, content)` to render — a quoted box with a small clickable
source label (wired through [`on_wiki_link`](#markdownviewon_wiki_link))
above the target's content, rendered like any note. Set via
[`on_embed`](#markdownviewon_embed).

**Host obligations — the pre-resolve pattern.** Render-time closures can't
query a database, so the host pre-resolves: collect the targets with
[`syntax::embed_targets(source)`](#embed_targets) (recursing into resolved
content for nested embeds), fetch each page — slicing by
[`syntax::extract_block`](#extract_block) /
[`extract_section`](#extract_section) for `#^id` / `#Heading` anchors — and
hand the map's `get` in as the provider.

**Guarantees & edge cases**

- `None` (or a missing target) leaves the line as literal text.
- Inside an embed, **write-back handlers are suppressed** — click-to-caret,
  task and callout toggles, heading folds — because those source offsets
  belong to the *embedding* page, not the embedded one. In-page find is
  suppressed too.
- Images inside an embed render through
  [`on_embed_image`](#markdownviewon_embed_image) (supply a grip-less
  read-only variant), never `on_image`.
- Nesting is capped at **depth 3**, which also breaks embed cycles.

---

## `alert_children`

```rust
pub fn alert_children(b: &mdast::Blockquote) -> Option<(AlertKind, Vec<mdast::Node>)>
```

If blockquote `b` is a GitHub alert, return its kind and a **copy** of its
children with the marker stripped. Public so other renderers of the same
construct (e.g. a PDF exporter) share the exact recognition — note the
`markdown` crate's `mdast::Blockquote` in the signature; this is the one
place its types surface in the public API.

**Returns** — `None` when `b` isn't an alert (recognition per
[`syntax::alert_marker`](#alert_marker) on the first paragraph's first text
node). On a match, the first text's source offset is advanced by the stripped
length, so a rendered→source click map stays aligned; a marker that was the
whole text node is dropped (along with a following hard `Break`).

---

## `alert_parts`

```rust
pub fn alert_parts(
    b: &mdast::Blockquote,
) -> Option<(AlertKind, Option<bool>, usize, Vec<mdast::Node>)>
```

[`alert_children`](#alert_children) plus the callout's fold state
(`Some(true)` = folded, `Some(false)` = open, `None` = not foldable) and the
marker's **source byte offset** — what a foldable callout's chevron click
reports so the host can flip the `-`/`+` in the source (via
[`toggle_alert_fold_at`](#toggle_alert_fold_at)). Same `None` contract and
child-stripping behavior as `alert_children`.

---

## `images`

```rust
pub fn images(source: &str) -> Vec<ImageInfo>
```

Every standalone image in `source` — a paragraph or list item that **begins**
with `![alt](src)` — in document order, each with its parsed `{width=N}` (if
any) and the [`attr_target`](#struct-imageinfo) byte range to overwrite to
set or replace that width.

**Guarantees & edge cases** — mirrors exactly how `MarkdownView` detects
block images, so the offsets line up with what's on screen (e.g. for a "fit
all images" command). Unparseable source → empty vec.

**Cost & threading** — pure; parses the markdown (uncached), no I/O or
storage.

---

## `all_image_srcs`

```rust
pub fn all_image_srcs(source: &str) -> Vec<SharedString>
```

Every image `src` in `source` — block (leading) **and** inline, in document
order, recursing through paragraphs, headings, blockquotes, lists, emphasis,
links, and tables — so the host can pre-decode them all (inline images render
as rasters too, not just leading ones). Pure; parses the markdown, no I/O.

---

## `toggle_task_at`

```rust
pub fn toggle_task_at(content: &str, offset: usize) -> Option<String>
```

Toggle the GFM task checkbox on the source line containing byte `offset` (a
task item's offset, as reported by
[`on_task_toggle`](#markdownviewon_task_toggle)).

**Returns** — the full `content` with that one checkbox flipped
(`[ ]`↔`[x]`), or `None` if there's no task checkbox on that line or `offset`
is out of range.

**Guarantees & edge cases** — length is unchanged (one ASCII byte swapped);
the checkbox is the **first** `[ ]`/`[x]`/`[X]` on the line (it precedes any
body text); `[X]` flips to `[ ]`; never panics.

---

## `match_count`

```rust
pub fn match_count(source: &str, query: &str) -> usize
```

Count case-insensitive matches of `query` in the **rendered (visible)** text
of `source` — the same matches [`search`](#markdownviewsearch) highlights and
[`find_matches`](#find_matches) indexes, in the same order. Empty query → 0.
Use it to size a host find bar's "n of m" and bound the active-match index.
Pure; parses the markdown (uncached), no I/O or storage.

---

## `find_matches`

```rust
pub fn find_matches(source: &str, query: &str) -> Vec<usize>
```

The **block index** (top-level column-child index, as rendered) of each match
of `query` in `source`, in document order — one entry per match, so
`len()` = [`match_count`](#match_count).

**Guarantees & edge cases**

- Pair with [`track_blocks`](#markdownviewtrack_blocks): the host reads
  `handle.bounds_for_item(find_matches(..)[current])` to scroll the active
  match's block into view — the indexing is kept in sync with what renders to
  a column child (a hidden `<!-- table:STYLE -->` marker doesn't get an
  index).
- Matching is over the rendered text (markers stripped — e.g. an alert's
  `[!NOTE]` doesn't match; a `[[wiki|alias]]`'s alias does), case-insensitive.
- Empty query → empty vec.

**Cost & threading** — pure; parses the markdown (uncached), no I/O.

---

## `struct Snippet` and `SNIPPETS`

```rust
pub struct Snippet {
    pub label: &'static str,
    pub snippet: &'static str,
    pub caret: usize,
}

pub const SNIPPETS: &[Snippet];
```

An authoring snippet for a markdown construct — pure data, no rendering.
Exposed so a host's `/` command palette can offer markdown commands without
re-deriving the syntax.

| Field | Type | Purpose |
| --- | --- | --- |
| `label` | `&'static str` | Human label, e.g. `"Heading 1"` |
| `snippet` | `&'static str` | Text to insert, e.g. `"# "` or `"```\n\n```"` |
| `caret` | `usize` | Byte offset within `snippet` to place the caret after inserting |

`SNIPPETS` currently holds 26 entries: block constructs (headings 1–3, bullet
/ numbered / to-do lists, quote, the five alerts, code block, mermaid, math,
table, divider) then inline ones (bold, italic, strikethrough, inline code,
inline math, highlight, link, wiki link, image — caret landing between the
markers).

---

## `enum ListEdit`

```rust
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ListEdit {
    Continue(String),
    Exit { start: usize, end: usize },
}
```

What pressing Enter should do on a markdown list / blockquote line — returned
by [`list_continuation`](#list_continuation); the host applies it to its own
input.

| Variant | Meaning |
| --- | --- |
| `Continue(String)` | Insert this text at the caret — a newline plus the continued marker (e.g. `"\n- "`, `"\n2. "`, `"\n> "`, `"\n- [ ] "`), indent preserved |
| `Exit { start, end }` | The current item is empty (just a marker); delete the byte range `start..end` and leave the caret at `start` (an empty line) |

---

## `list_continuation`

```rust
pub fn list_continuation(value: &str, cursor: usize) -> Option<ListEdit>
```

Decide how Enter continues a markdown list/quote at `cursor` in `value` —
pure `(text, caret)` in, [`ListEdit`](#enum-listedit) out; no gpui/input
dependency.

**Guarantees & edge cases**

- Recognizes `-`/`*`/`+` bullets, `N.`/`N)` ordered items (continues with
  `N+1`), `- [ ]`/`- [x]` task items (new items start **unchecked**), and `>`
  blockquotes — leading indent preserved on the continued line.
- A non-empty item continues with the next marker; an **empty** item (marker
  only) exits the list via `Exit`.
- `None` when the caret's line isn't a list/quote item (the host inserts a
  plain newline). `cursor` is clamped to `value.len()` — never panics.

---

## `INDENT`

```rust
pub const INDENT: &str = "  ";
```

The default indent level (two spaces) for Tab / Shift+Tab on list items. The
host passes its configured indent to
[`indent_list_line`](#indent_list_line) / [`outdent_line`](#outdent_line);
this is just the fallback for callers without a setting.

---

## `indent_list_line`

```rust
pub fn indent_list_line(value: &str, cursor: usize, indent: &str) -> Option<(String, usize)>
```

Tab: if the caret's line is a list/quote item, indent it one level (insert
`indent` at the line start).

**Returns** — `(new text, shifted caret)`. `None` when the line isn't a
list/quote item — so the caller can insert a literal tab instead. `cursor` is
clamped; never panics.

---

## `outdent_line`

```rust
pub fn outdent_line(value: &str, cursor: usize, indent: &str) -> Option<(String, usize)>
```

Shift+Tab: outdent the caret's line one level — remove up to `indent`'s width
of leading spaces, or one leading tab.

**Returns** — `(new text, new caret)`; the caret never moves before the line
start. `None` if the line has no leading indent to remove. Unlike
[`indent_list_line`](#indent_list_line) it doesn't require a list marker —
any indented line outdents.

---

## `reindent`

```rust
pub fn reindent(content: &str, old: usize, new: usize) -> Option<String>
```

Re-indent every space-indented list / quote item in `content` from
`old`-space nesting units to `new`-space units — e.g. when a list-indent
setting changes, so existing nesting matches the new width.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `content` | `&str` | The whole document. |
| `old` | `usize` | The previous spaces-per-level. Each item's level is its leading spaces ÷ `old` (integer division). |
| `new` | `usize` | The new spaces-per-level. |

**Returns** — the rewritten content, or `None` when nothing changes
(including `old == new` or `old == 0`).

**Guarantees & edge cases** — only lines that are list/quote items (per the
same marker grammar as [`list_continuation`](#list_continuation)) with
leading **spaces** are touched; non-list lines, top-level items, and
tab-indented lines pass through unchanged.
