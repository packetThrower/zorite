# gpui-markdown

A small **Markdown renderer for [GPUI](https://www.gpui.rs/)**, built on gpui's own
`StyledText` / `InteractiveText` so paragraphs wrap properly and links are clickable
through a real **callback** — unlike renderers that only `cx.open_url` externally.

It is host-agnostic: styling comes in via `MarkdownStyle`, and the host supplies
closures for clicking a `[[wiki-link]]`/`#tag`, rendering an image, rendering a
mermaid diagram, syntax-highlighting code, and click-to-caret. Standard
`[text](url)` links open externally.

**📖 Full reference:** every public item, with signatures, parameter tables,
return contracts, edge cases, and cost notes, lives in [API.md](API.md).

This is **the** markdown crate of the Zorite workspace, in two layers:

- **`gpui_markdown::syntax`** — always compiled, **dependency-free**: the shared
  construct *recognition* (linkables, GitHub alert kinds + fold chars, table
  styles, heading scales, `key:: value` properties, ` ^block-id` anchors,
  `#Heading` / `#^id` link-target splitting, and `![[embed]]` lines with
  block/section extraction) that this reader, the
  [`gpui-editor`](../gpui-editor/README.md) WYSIWYG
  view, and the PDF exporter all consume, so what a construct IS is defined once.
- **The reader view** — `MarkdownView` and everything around it, behind the
  default-on **`view`** feature, which owns the `gpui` + `markdown` dependencies.
  Consumers that only need recognition (like gpui-editor) depend with
  `default-features = false`.

## Features

- Headings, paragraphs, **bold** / *italic* / ~~strikethrough~~ / `inline code` /
  `<mark>` highlight, hard breaks
- Bullet / numbered / nested / **task** lists (`- [ ]` / `- [x]`), blockquotes,
  fenced code blocks, thematic breaks
- GFM **tables** — content-measured columns, column alignment, plus **per-table
  visual designs** (striped / header-shaded / minimal) chosen by a hidden
  `<!-- table:STYLE -->` marker
- **GitHub alerts** — `> [!NOTE]` / `[!TIP]` / `[!IMPORTANT]` / `[!WARNING]` /
  `[!CAUTION]` blockquotes render with a colored bar, bold title, and optional
  host-supplied icons; the natural inline form (`> [!NOTE] like so`) works too.
  Obsidian's **fold char** makes a callout collapsible — `> [!NOTE]-` renders
  folded (title + chevron only), `+` open; clicking the title dispatches to
  `on_alert_toggle` so the host can flip the char in the source
- **Collapsible headings** — every heading gets a hover-revealed fold chevron;
  a folded heading's whole section is skipped. The fold set is host-owned
  (`folded_headings` + `on_heading_toggle`) since this view is rebuilt every frame
- **Properties** — consecutive `key:: value` lines render as a two-column
  panel: per-key icons (via `MarkdownStyle::property_icon`), muted keys, and
  values with `#tag` / `[[wiki-link]]` segments as clickable pills
- **Block ids, anchors & embeds** — a trailing ` ^block-id` marker hides from
  the rendered text; `[[Note#Heading]]` / `[[Note#^id]]` link targets display
  as `Note → anchor`; and a standalone `![[Note]]` line **transcludes** the
  target's content in a quoted box via a host resolver (`on_embed`), nested
  embeds included
- **Inline (in-flow) images** — an image that doesn't lead its paragraph
  renders as a small in-flow thumbnail via `on_inline_image`, wrapping with
  the text; a click dispatches to `on_image_preview`
- **Clickable task checkboxes** — a `- [ ]` box click dispatches its source
  offset to `on_task_toggle` so the host can flip `[ ]`↔`[x]` and persist
- **Syntax highlighting** — fenced code with a language tag colors its tokens via
  a host-supplied `on_highlight` closure (bring your own engine; Zorite passes
  gpui-component's tree-sitter highlighter)
- **Footnotes** and reference-style `[text][id]` links/images; raw HTML shown
  literally (never executed)
- `[[wiki-links]]` (and `[[target|label]]` aliases) and `#tags` → clickable,
  dispatched to your callback
- **Images**, **mermaid diagrams**, and **math** — `$$…$$` blocks and inline
  `$…$` formulas — rendered by host-supplied closures (the host owns loading /
  async render / interaction); each falls back gracefully (math → its raw LaTeX)
- **In-page find** — highlight matches and scroll the active one into view
  (`search` + `find_matches` / `match_count`)
- **Click-to-caret** — report the source offset nearest a click, for entering an
  editor at the clicked character (`on_click_source`)
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
GPUI element tree. The full builder surface — embeds, folds, find, click-to-caret,
task toggles — is in [API.md](API.md).

## Per-table visual designs

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
`$…$` (`math_text`), typeset by a host renderer — and Zorite-style
`[[wiki-links]]` and `#tags`.

Not handled (not enabled by `gfm()`): frontmatter (YAML/TOML) and MDX. Footnote
references render as `[label]` markers but aren't click-to-jump (that would need
anchors this text-based renderer doesn't have).

Also rendered: **GitHub alerts** on blockquotes (both marker forms, plus the
foldable `-`/`+` variant), Zorite-style `[[wiki-links]]` and `#tags`
(namespaced `#a/b` included — the grammar is the shared `syntax` module's),
`[[Note#Heading]]` / `[[Note#^id]]` anchors (displayed as `Note → anchor`),
trailing ` ^block-id` markers (hidden), `key:: value` **property panels**,
standalone `![[Note]]` **embeds**, and table-style / math-alignment control
comments, which — like all HTML comments — never render.

The `syntax` module's recognizers back all of it and are public — every one is
documented in [API.md](API.md).

## Status

Feature-complete for CommonMark + GFM. The view parses with the
[`markdown`](https://crates.io/crates/markdown) crate (mdast); `syntax` is pure
text and dependency-free. Not yet published to crates.io (gpui is a git-only
dependency).

## License

GPL-3.0-or-later.
