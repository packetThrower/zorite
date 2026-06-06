# gpui-markdown

A small **Markdown renderer for [GPUI](https://www.gpui.rs/)**, built on gpui's own
`StyledText` / `InteractiveText` so paragraphs wrap properly and links are clickable
through a real **callback** — unlike renderers that only `cx.open_url` externally.

It is host-agnostic: styling comes in via [`MarkdownStyle`](#markdownstyle), clicking a
`[[wiki-link]]` or `#tag` invokes a closure you provide, and images are rendered by a
closure you provide. Standard `[text](url)` links open externally.

## Features

- Headings, paragraphs, **bold** / *italic* / ~~strikethrough~~ / `inline code`, hard breaks
- Bullet / numbered / nested / **task** lists (`- [ ]` / `- [x]`), blockquotes,
  fenced code blocks, thematic breaks
- GFM **tables** (with column alignment), **footnotes**, and reference-style
  `[text][id]` links/images
- Raw HTML shown literally (never executed)
- `[[wiki-links]]` and `#tags` → clickable, dispatched to your callback
- **Images** rendered by a host-supplied closure (so the host owns loading and any
  interaction); falls back to a clickable label otherwise
- `SNIPPETS` — authoring snippets a host can surface in a `/` command palette

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
```

`MarkdownView` implements `RenderOnce` (hence `IntoElement`), so it drops into any
GPUI element tree.

## API

### `MarkdownView`

The renderable element. Construct it, attach optional handlers, and place it in your
tree. All builder methods take and return `self`.

```rust
MarkdownView::new(id_base, source)
    .style(markdown_style)     // optional; defaults to a neutral dark palette
    .on_wiki_link(handler)     // optional; enables [[wiki-link]] / #tag clicks
    .on_image(renderer)        // optional; enables real image rendering
```

| Method | Signature | Purpose |
| --- | --- | --- |
| `new` | `fn new(id_base: impl Into<SharedString>, source: impl Into<SharedString>) -> Self` | Create a view. **`id_base` must be unique per rendered document** — it derives element ids for clickable paragraphs; reusing one across two documents on screen causes id collisions. |
| `style` | `fn style(self, style: MarkdownStyle) -> Self` | Set colors/sizes. Without it, [`MarkdownStyle::default`] is used. |
| `on_wiki_link` | `fn on_wiki_link(self, handler: WikiLinkHandler) -> Self` | Handle clicks on `[[wiki-links]]` and `#tags`. Without it, those render styled but inert. |
| `on_image` | `fn on_image(self, handler: ImageRenderer) -> Self` | Render standalone images. Without it, images fall back to a clickable `🖼 alt` label. |

Parsing uses the [`markdown`](https://crates.io/crates/markdown) crate with
`ParseOptions::gfm()` (CommonMark + GFM). If parsing fails, the raw source is shown
as plain text.

### `MarkdownStyle`

Visual configuration (`#[derive(Clone)]`). The host typically maps its theme onto
this; `MarkdownStyle::default()` is a neutral dark palette.

```rust
pub struct MarkdownStyle {
    pub text_color: Hsla,    // body text
    pub text_size: Pixels,   // base size; headings scale from it
    pub heading_color: Hsla, // h1–h6
    pub link_color: Hsla,    // links, footnote markers, image labels
    pub tag_color: Hsla,     // #tags
    pub code_color: Hsla,    // inline + fenced code text
    pub code_bg: Hsla,       // fenced code block background
    pub muted_color: Hsla,   // blockquotes, list markers, table borders, footnote defs, raw HTML
    pub rule_color: Hsla,    // thematic break (---)
}
```

The renderer does not set the surrounding text size itself beyond `text_size`; set
font family on a parent element if needed.

### `WikiLinkHandler`

```rust
pub type WikiLinkHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
```

Invoked with the **target name** when the user clicks:

- `[[Some Page]]` → called with `"Some Page"` (trimmed).
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
`ImageRenderer` with an `ImageInfo` and renders any trailing text below the element.
Inline images mixed within text keep the label fallback.

Building the returned element needs no `Window`/`App` — its event handlers fire later
with their own context — so the host can return a stateful, interactive element while
this crate stays host-agnostic.

`attr_target` supports **resize-by-rewriting-the-markdown**: it is the byte span to
replace with `{width=N}` — an empty range just after the image when there's no
attribute yet, or the existing attribute's span when there is one. A host resize
handle computes a new width and rewrites `source[attr_target] = "{width=N}"`.

```rust
view.on_image(Rc::new(|info: ImageInfo| {
    let mut image = gpui::img(resolve(&info.src)); // your path/URL -> ImageSource
    if let Some(w) = info.width {
        image = image.w(px(w));
    }
    image.into_any_element()
}))
```

The `{width=N}` (or `{width=Npx}`) attribute is this crate's convention for sizing,
parsed off the text immediately following a standalone image.

### `Snippet` and `SNIPPETS`

```rust
pub struct Snippet {
    pub label: &'static str,   // human label, e.g. "Heading 1"
    pub snippet: &'static str, // text to insert, e.g. "# "
    pub caret: usize,          // byte offset within `snippet` to place the caret
}

pub const SNIPPETS: &[Snippet];
```

Pure data (no rendering): authoring snippets for markdown constructs (headings,
lists, to-dos, quotes, code blocks, tables, dividers, and inline bold/italic/etc.).
A host can surface these in a `/` command palette to insert markdown without
re-deriving the syntax.

## Supported syntax

Every node `ParseOptions::gfm()` produces is rendered: headings, paragraphs,
bold/italic/strikethrough/inline-code, links (inline, autolink, reference-style),
images, ordered/unordered/nested/task lists, blockquotes (nested), fenced code,
thematic breaks, tables (with alignment), footnotes (references + definitions), and
raw HTML (shown literally). Plus zorite-style `[[wiki-links]]` and `#tags`.

Not handled (not enabled by `gfm()`): math (`$x$`), frontmatter (YAML/TOML), and MDX.
Footnote references render as `[label]` markers but are not click-to-jump (that would
need anchors this text-based renderer doesn't have).

## Status

Early, but feature-complete for CommonMark + GFM. Parses with the
[`markdown`](https://crates.io/crates/markdown) crate (mdast). Not yet published to
crates.io.

## License

GPL-3.0-or-later.
