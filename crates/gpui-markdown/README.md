# gpui-markdown

A small **Markdown renderer for [GPUI](https://www.gpui.rs/)**, built on gpui's own
`StyledText` / `InteractiveText` so paragraphs wrap properly and links are clickable
through a real **callback** — unlike renderers that only `cx.open_url` externally.

It is host-agnostic: styling comes in via [`MarkdownStyle`](#markdownstyle), and the
host supplies closures for clicking a `[[wiki-link]]`/`#tag`, rendering an image,
rendering a mermaid diagram, and click-to-caret. Standard `[text](url)` links open
externally.

## Features

- Headings, paragraphs, **bold** / *italic* / ~~strikethrough~~ / `inline code` /
  `<mark>` highlight, hard breaks
- Bullet / numbered / nested / **task** lists (`- [ ]` / `- [x]`), blockquotes,
  fenced code blocks, thematic breaks
- GFM **tables** — column alignment, plus **per-table visual designs** (striped /
  header-shaded / minimal) chosen by a hidden `<!-- table:STYLE -->` marker
- **Footnotes** and reference-style `[text][id]` links/images; raw HTML shown
  literally (never executed)
- `[[wiki-links]]` (and `[[target|label]]` aliases) and `#tags` → clickable,
  dispatched to your callback
- **Images**, **mermaid diagrams**, and **math** — `$$…$$` blocks and inline
  `$…$` formulas — rendered by host-supplied closures (the host owns loading /
  async render / interaction); each falls back gracefully (math → its raw LaTeX)
- **In-page find** — highlight matches and scroll the active one into view
  ([`search`](#in-page-find) + [`find_matches`](#in-page-find) / [`match_count`](#in-page-find))
- **Click-to-caret** — report the source offset nearest a click, for entering an
  editor at the clicked character ([`on_click_source`](#clicksourcehandler))
- `SNIPPETS` — authoring snippets a host can surface in a `/` command palette
- **Editor helpers** — pure `(text, caret)` transforms (no gpui/input dependency)
  for building a Markdown editor: list continuation, indent/outdent, and re-indent

See [`sample.md`](sample.md) for a document exercising everything.

## Quick start

```rust
use std::rc::Rc;
use gpui_markdown::{MarkdownView, MarkdownStyle};

// In a render method, returning an `impl IntoElement`:
MarkdownView::new("note-1", source_text)          // unique id + markdown source
    .style(MarkdownStyle::default())              // or map your theme onto it
    .on_wiki_link(Rc::new(|title, window, cx| {
        // navigate to page `title` in your app
    }))
    .on_image(Rc::new(|info| { /* render a real image */ todo!() }))
    .on_mermaid(Rc::new(|src| { /* render a diagram */ todo!() }))
    .on_math(Rc::new(|latex| { /* typeset a `$$…$$` block → element */ todo!() }))
    .on_inline_math(Rc::new(|latex| { /* inline `$…$` → (raster, w, h) */ None }))
```

`MarkdownView` implements `RenderOnce` (hence `IntoElement`), so it drops into any
GPUI element tree.

## API

### `MarkdownView`

The renderable element. Construct it, attach optional handlers, and place it in your
tree. All builder methods take and return `self`.

| Method | Signature | Purpose |
| --- | --- | --- |
| `new` | `fn new(id_base: impl Into<SharedString>, source: impl Into<SharedString>) -> Self` | Create a view. **`id_base` must be unique per rendered document** — it derives element ids for clickable paragraphs; reusing one across two on-screen documents collides ids. |
| `style` | `fn style(self, style: MarkdownStyle) -> Self` | Set colors/sizes. Without it, [`MarkdownStyle::default`] is used. |
| `on_wiki_link` | `fn on_wiki_link(self, handler: WikiLinkHandler) -> Self` | Handle clicks on `[[wiki-links]]` and `#tags`. Without it they render styled but inert. |
| `on_image` | `fn on_image(self, handler: ImageRenderer) -> Self` | Render standalone images. Without it, images fall back to a clickable `🖼 alt` label. |
| `on_mermaid` | `fn on_mermaid(self, handler: MermaidRenderer) -> Self` | Render ` ```mermaid ` blocks as diagrams. Without it, such a block renders as plain code. |
| `on_math` | `fn on_math(self, handler: MathRenderer) -> Self` | Render `$$…$$` math blocks as typeset images. Without it, a block renders as its raw LaTeX. |
| `on_inline_math` | `fn on_inline_math(self, handler: InlineMathRenderer) -> Self` | Render inline `$…$` formulas (raster painted over a reserved gap in the line). Without it, inline math stays literal `$…$` text. |
| `search` | `fn search(self, query: impl Into<SharedString>, current: usize) -> Self` | Highlight matches of `query`, emphasizing the `current`-th. See [In-page find](#in-page-find). |
| `track_blocks` | `fn track_blocks(self, handle: ScrollHandle) -> Self` | Track-scroll the block column so the host can scroll a match into view. See [In-page find](#in-page-find). |
| `on_click_source` | `fn on_click_source(self, handler: ClickSourceHandler) -> Self` | Report the source offset nearest a click (for click-to-caret). |

Parsing uses the [`markdown`](https://crates.io/crates/markdown) crate with
`ParseOptions::gfm()` (CommonMark + GFM). If parsing fails, the raw source is shown
as plain text.

### `MarkdownStyle`

Visual configuration (`#[derive(Clone)]`). The host typically maps its theme onto
this; `MarkdownStyle::default()` is a neutral dark palette.

```rust
pub struct MarkdownStyle {
    pub text_color: Hsla,         // body text
    pub text_size: Pixels,        // base size; headings scale from it
    pub heading_color: Hsla,      // h1–h6
    pub link_color: Hsla,         // links, footnote markers, image labels
    pub tag_color: Hsla,          // #tags
    pub code_color: Hsla,         // inline + fenced code text
    pub code_bg: Hsla,            // fenced code background; also striped/header table shade
    pub muted_color: Hsla,        // blockquotes, list markers, table borders, footnote defs, raw HTML
    pub rule_color: Hsla,         // thematic break (---)
    pub mark_bg: Hsla,            // <mark>…</mark> highlight (translucent)
    pub search_bg: Hsla,          // in-page find: every match (translucent)
    pub search_current_bg: Hsla,  // in-page find: the active match
    pub list_indent: Pixels,      // horizontal indent per nested list level
    pub mono_font: SharedString,  // monospace family for code (unknown → default font)
}
```

`list_indent` lets the host match the renderer's nesting to its editor's literal
indent so reading and editing line up. The renderer sets only `text_size`; set the
font family on a parent element if needed.

### `WikiLinkHandler`

```rust
pub type WikiLinkHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
```

Invoked with the **target name** when the user clicks:

- `[[Some Page]]` → called with `"Some Page"` (trimmed).
- `[[target|label]]` → displays `label`, called with `"target"` (e.g.
  `[[file.pdf#p3|↗]]` shows a `↗` linking to `file.pdf#p3`). An empty label falls
  back to the target.
- `#some-tag` → called with `"some-tag"` (the bare name; the displayed `#` is kept).

Standard `[text](url)` and reference-style links open externally via `cx.open_url`
and do **not** go through this handler.

### `ImageRenderer` and `ImageInfo`

```rust
pub type ImageRenderer = Rc<dyn Fn(ImageInfo) -> AnyElement>;

pub struct ImageInfo {
    pub src: SharedString,        // the URL/path exactly as written
    pub alt: SharedString,        // alt text (may be empty)
    pub width: Option<f32>,       // explicit pixels from a `{width=N}` attribute
    pub attr_target: Range<usize>,// byte range in the *source* to write `{width=N}`
}
```

When a paragraph **begins** with an image (e.g. `![alt](src)` on its own line,
optionally followed by `{width=N}` and/or caption text), the renderer calls your
`ImageRenderer` with an `ImageInfo` and renders any trailing text below. Inline
images mixed within text keep the label fallback.

Building the returned element needs no `Window`/`App` — its event handlers fire
later with their own context — so the host can return a stateful, interactive
element while this crate stays host-agnostic.

`attr_target` supports **resize-by-rewriting-the-markdown**: it's the byte span to
replace with `{width=N}` — an empty range just after the image when there's no
attribute yet, or the existing attribute's span when there is one. A host resize
handle computes a new width and rewrites `source[attr_target] = "{width=N}"`.

```rust
view.on_image(Rc::new(|info: ImageInfo| {
    let mut image = gpui::img(resolve(&info.src)); // your path/URL -> ImageSource
    if let Some(w) = info.width { image = image.w(px(w)); }
    image.into_any_element()
}))
```

`{width=N}` (or `{width=Npx}`) is this crate's convention for sizing, parsed off
the text immediately following a standalone image. See also [`images`](#images).

### `MermaidRenderer`

```rust
pub type MermaidRenderer = Rc<dyn Fn(SharedString) -> AnyElement>;
```

Renders a ` ```mermaid ` code block as a diagram, given the block's source. This
crate just detects the fence and hands the source over — the host owns the
(typically expensive, async) render and any caching, staying renderer-agnostic.
Without a handler, a mermaid block renders as an ordinary code block.

### `MathRenderer` and `InlineMathRenderer`

```rust
pub type MathRenderer = Rc<dyn Fn(SharedString) -> AnyElement>;
pub type InlineMathRenderer = Rc<dyn Fn(SharedString) -> Option<(Arc<RenderImage>, f32, f32)>>;
```

Math is parsed but **rendered by the host** (KaTeX-style typesetting is the host's
job — e.g. via [`ratex-gpui`](../ratex-gpui)), so this crate stays engine-agnostic.

- **`on_math`** (`$$…$$` blocks) — like [`MermaidRenderer`](#mermaidrenderer): given
  the block's LaTeX, return an element. Without a handler the block renders as its
  raw LaTeX.
- **`on_inline_math`** (`$…$`) — given the formula's LaTeX, return its raster plus
  its logical `(width, height)` at text size, or `None` while still rasterizing.
  The renderer reserves a non-breaking spacer of that width in the paragraph's text
  and paints the raster over it (via a `canvas` that reads the laid-out glyph
  position **in the same frame**), so the surrounding `StyledText` — and thus
  links, in-page find, and click-to-caret — is preserved and the line wraps
  normally. The paragraph's line height grows to fit a tall formula. Without a
  handler, inline `$…$` stays literal text.

Both `$$…$$` (block, `math_flow`) and `$…$` (inline, `math_text`) are enabled in the
parser; a lone `$` in prose (e.g. `it cost $5`) stays literal.

### `ClickSourceHandler`

```rust
pub type ClickSourceHandler = Rc<dyn Fn(usize, Pixels, &mut Window, &mut App)>;
```

Called when the rendered text is clicked (outside a link), with the **source** byte
offset nearest the click and the click's window **y**. A host uses it to place its
editor's caret there and keep it under the cursor when switching into edit mode.
The crate maps the click through gpui's text layout plus a source-offset map it
builds while rendering (accounting for stripped `[[ ]]` / `#` / inline-code markup).

### In-page find

The crate provides the matching + layout; the host owns the find bar. All three
report the **same matches in the same order**.

```rust
fn match_count(source: &str, query: &str) -> usize            // case-insensitive count (0 if empty)
fn find_matches(source: &str, query: &str) -> Vec<usize>      // block index of each match, in order
```

- `MarkdownView::search(query, current)` highlights every match in the rendered
  (visible) text and emphasizes the `current`-th (0-based). An empty query
  highlights nothing.
- `MarkdownView::track_blocks(handle)` track-scrolls the block column so the host
  can read a block's bounds via `ScrollHandle::bounds_for_item(find_matches(..)[current])`
  and scroll the active match into view.
- `match_count` sizes a host's "n of m" and bounds the active index.

All are pure over the source string — no I/O, no storage.

### `Snippet` and `SNIPPETS`

```rust
pub struct Snippet {
    pub label: &'static str,   // human label, e.g. "Heading 1"
    pub snippet: &'static str, // text to insert, e.g. "# "
    pub caret: usize,          // byte offset within `snippet` to place the caret
}

pub const SNIPPETS: &[Snippet];
```

Pure data (no rendering): authoring snippets for Markdown constructs (headings,
lists, to-dos, quotes, code blocks, tables, dividers, inline bold/italic/etc.). A
host can surface these in a `/` command palette to insert Markdown without
re-deriving the syntax.

### Editor helpers

Pure `(text, caret)` transforms — no gpui/input dependency — for wiring a Markdown
editor's keys. The host applies the returned edit to its own input.

```rust
pub enum ListEdit {
    Continue(String),               // insert this at the caret (e.g. "\n- ", "\n2. ", "\n- [ ] ")
    Exit { start: usize, end: usize }, // empty item: delete start..end, caret at start
}

fn list_continuation(value: &str, cursor: usize) -> Option<ListEdit>  // Enter on a list/quote
fn indent_list_line(value: &str, cursor: usize, indent: &str) -> Option<(String, usize)>  // Tab
fn outdent_line(value: &str, cursor: usize, indent: &str) -> Option<(String, usize)>      // Shift+Tab
fn reindent(content: &str, old: usize, new: usize) -> Option<String>  // re-flow nesting width
pub const INDENT: &str = "  ";      // default two-space level (fallback for indent/outdent)
```

- `list_continuation` — Enter continues a `-`/`*`/`+` / `N.` / `N)` / `- [ ]` / `>`
  item (indent preserved), or exits an empty one. `None` off a list/quote line.
- `indent_list_line` / `outdent_line` — Tab / Shift+Tab indent or outdent the
  caret's list/quote line by `indent`. `None` if the line isn't a list item (so the
  caller can insert a literal tab) / has no indent to remove.
- `reindent` — re-indent every space-indented list/quote item from `old`-space to
  `new`-space nesting units (e.g. when a list-indent setting changes). `None` when
  nothing changes.

### `images`

```rust
fn images(source: &str) -> Vec<ImageInfo>
```

Every standalone image in `source`, in document order, each with its parsed
`{width=N}` and the `attr_target` byte range to overwrite. Mirrors how the renderer
detects block images, so offsets line up with what's on screen — e.g. for a
"fit all images" command. Pure: parses the Markdown, no I/O.

### Per-table visual designs

A GFM table can carry a **hidden style marker** — an HTML comment on the line
directly above it — that the renderer honors and hides:

```markdown
<!-- table:striped -->
| Name  | Role     |
|:------|:---------|
| Ada   | Engineer |
```

| Marker | Look |
| --- | --- |
| *(none)* / `<!-- table:grid -->` | full outer box + all gridlines (default) |
| `<!-- table:striped -->` | alternate body rows shaded; a rule under the header |
| `<!-- table:header -->` | only the header row shaded |
| `<!-- table:minimal -->` | no box/gridlines; a rule under the header |

Shading uses `MarkdownStyle::code_bg`; borders use `muted_color`. Any other Markdown
viewer just ignores the comment and shows a plain table, so the marker degrades
gracefully.

## Supported syntax

Every node `ParseOptions::gfm()` produces is rendered: headings, paragraphs,
bold/italic/strikethrough/inline-code, links (inline, autolink, reference-style),
images, ordered/unordered/nested/task lists, blockquotes (nested), fenced code,
thematic breaks, tables (with alignment + the per-table designs above), footnotes
(references + definitions), and raw HTML (shown literally — except `<mark>…</mark>`,
honored as a highlight). Plus **math** — `$$…$$` blocks (`math_flow`) and inline
`$…$` (`math_text`), typeset by a host renderer (see
[`MathRenderer`](#mathrenderer-and-inlinemathrenderer)) — and Zorite-style
`[[wiki-links]]` and `#tags`.

Not handled (not enabled by `gfm()`): frontmatter (YAML/TOML) and MDX. Footnote
references render as `[label]` markers but aren't click-to-jump (that would need
anchors this text-based renderer doesn't have).

## Status

Early, but feature-complete for CommonMark + GFM. Parses with the
[`markdown`](https://crates.io/crates/markdown) crate (mdast). Not yet published to
crates.io (gpui is a git-only dependency).

## License

GPL-3.0-or-later.
