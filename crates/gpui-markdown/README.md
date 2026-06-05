# gpui-markdown

A small **Markdown renderer for [GPUI](https://www.gpui.rs/)**, built on gpui's own
`StyledText` / `InteractiveText` so paragraphs wrap properly and links are clickable
through a real **callback** — unlike renderers that only `cx.open_url` externally.

It is host-agnostic: styling comes in via `MarkdownStyle`, and clicking a
`[[wiki-link]]` or `#tag` invokes a closure you provide (navigate however you like).
Standard `[text](url)` links open externally.

## Features

- Headings, paragraphs, **bold** / *italic* / `inline code`
- Bullet / numbered / nested lists, blockquotes, fenced code blocks, thematic breaks
- GFM **tables** and ~~strikethrough~~
- `[[wiki-links]]` and `#tags` → clickable, dispatched to your callback
- Images rendered as clickable links (real inline image rendering is a TODO)

## Usage

```rust
use std::rc::Rc;
use gpui_markdown::{MarkdownView, MarkdownStyle};

// In a render method:
MarkdownView::new("note-1", source_text)         // unique id + markdown source
    .style(MarkdownStyle::default())             // or map your theme onto it
    .on_wiki_link(Rc::new(|title, window, cx| {
        // navigate to page `title` in your app
    }))
```

`MarkdownView` implements `IntoElement`, so it drops into any GPUI element tree.
Set the surrounding text color/size on a parent element; the renderer styles
headings, code, links, and tags relative to that via `MarkdownStyle`.

## Status

Early. Parses with the [`markdown`](https://crates.io/crates/markdown) crate (mdast,
GFM). Not yet published to crates.io.

## License

GPL-3.0-or-later.
