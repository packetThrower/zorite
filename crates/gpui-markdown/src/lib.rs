//! A small markdown renderer for GPUI.
//!
//! It parses markdown to an AST (via the `markdown` crate) and renders
//! it with gpui's own `StyledText` / `InteractiveText`, so paragraphs
//! wrap properly and links are clickable through a real callback — not
//! `cx.open_url`, which only opens externally.
//!
//! It is deliberately host-agnostic: styling comes in via [`MarkdownStyle`],
//! and clicking a `[[wiki-link]]` invokes a caller-supplied closure
//! (`on_wiki_link`) rather than knowing anything about the host app.
//! Standard `[text](url)` links open externally via `cx.open_url`.
//!
//! Covers CommonMark + GFM: headings, paragraphs, bold/italic/strikethrough/
//! inline-code, fenced code blocks, ordered/unordered/nested and task lists,
//! blockquotes, thematic breaks, hard breaks, tables, links (inline and
//! reference-style), images (rendered by the host via `on_image`), footnotes,
//! and raw HTML (shown literally, never executed). `[[wiki-links]]` and
//! `#tags` become clickable via caller callbacks.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText,
    IntoElement, ParentElement, Pixels, RenderOnce, SharedString, StrikethroughStyle, Styled,
    StyledText, Window, div, px, rgb, rgba,
};
use markdown::mdast;

/// Visual configuration for the renderer. The host fills this from its
/// own theme; defaults are a neutral dark palette.
#[derive(Clone)]
pub struct MarkdownStyle {
    pub text_color: Hsla,
    pub text_size: Pixels,
    pub heading_color: Hsla,
    pub link_color: Hsla,
    pub tag_color: Hsla,
    pub code_color: Hsla,
    pub code_bg: Hsla,
    pub muted_color: Hsla,
    pub rule_color: Hsla,
}

impl Default for MarkdownStyle {
    fn default() -> Self {
        Self {
            text_color: rgb(0xE6E6E6).into(),
            text_size: px(15.0),
            heading_color: rgb(0xFFFFFF).into(),
            link_color: rgb(0x4C9EFF).into(),
            tag_color: rgb(0x9D7CD8).into(),
            code_color: rgb(0xD7BA7D).into(),
            code_bg: rgba(0xFFFFFF14).into(),
            muted_color: rgb(0x9AA0A6).into(),
            rule_color: rgba(0xFFFFFF22).into(),
        }
    }
}

/// An authoring snippet for a markdown construct: a label, the text to
/// insert, and the caret offset (bytes) within that text. Exposed so a
/// host's command palette can offer markdown commands without re-deriving
/// the syntax. Pure data — no rendering involved.
pub struct Snippet {
    pub label: &'static str,
    pub snippet: &'static str,
    pub caret: usize,
}

/// Built-in markdown authoring snippets (for a `/` command palette).
pub const SNIPPETS: &[Snippet] = &[
    // Blocks
    Snippet {
        label: "Heading 1",
        snippet: "# ",
        caret: 2,
    },
    Snippet {
        label: "Heading 2",
        snippet: "## ",
        caret: 3,
    },
    Snippet {
        label: "Heading 3",
        snippet: "### ",
        caret: 4,
    },
    Snippet {
        label: "Bullet list",
        snippet: "- ",
        caret: 2,
    },
    Snippet {
        label: "Numbered list",
        snippet: "1. ",
        caret: 3,
    },
    Snippet {
        label: "To-do",
        snippet: "- [ ] ",
        caret: 6,
    },
    Snippet {
        label: "Quote",
        snippet: "> ",
        caret: 2,
    },
    Snippet {
        label: "Code block",
        snippet: "```\n\n```",
        caret: 4,
    },
    Snippet {
        label: "Table",
        snippet: "|  |  |\n| --- | --- |\n|  |  |\n",
        caret: 2,
    },
    Snippet {
        label: "Divider",
        snippet: "---\n",
        caret: 4,
    },
    // Inline (caret lands between the markers)
    Snippet {
        label: "Bold",
        snippet: "****",
        caret: 2,
    },
    Snippet {
        label: "Italic",
        snippet: "**",
        caret: 1,
    },
    Snippet {
        label: "Strikethrough",
        snippet: "~~~~",
        caret: 2,
    },
    Snippet {
        label: "Inline code",
        snippet: "``",
        caret: 1,
    },
    Snippet {
        label: "Link",
        snippet: "[]()",
        caret: 1,
    },
    Snippet {
        label: "Image",
        snippet: "![]()",
        caret: 4,
    },
];

/// Called when a `[[wiki-link]]` is clicked, with the trimmed title.
pub type WikiLinkHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

/// A standalone image (a paragraph that is just `![alt](src)`, optionally
/// followed by a `{width=N}` attribute). Handed to the host's [`ImageRenderer`]
/// so it can render a real, possibly interactive, image element.
pub struct ImageInfo {
    /// The image URL/path as written in the markdown.
    pub src: SharedString,
    /// The alt text (may be empty).
    pub alt: SharedString,
    /// An explicit width in pixels from a `{width=N}` attribute, if present.
    pub width: Option<f32>,
    /// The byte range in the source to replace with `{width=N}` when resizing:
    /// an empty range (just after the image) when there's no attribute yet, or
    /// the existing attribute's span when there is one.
    pub attr_target: Range<usize>,
}

/// Renders a standalone image. The element's event handlers run later (with
/// their own context), so building it needs no window/app — letting the host
/// supply a stateful, draggable image while this crate stays host-agnostic.
pub type ImageRenderer = Rc<dyn Fn(ImageInfo) -> AnyElement>;

/// A rendered markdown document element.
#[derive(IntoElement)]
pub struct MarkdownView {
    id_base: SharedString,
    source: SharedString,
    style: MarkdownStyle,
    on_wiki_link: Option<WikiLinkHandler>,
    on_image: Option<ImageRenderer>,
}

impl MarkdownView {
    /// `id_base` must be unique per rendered document (used to derive
    /// element ids for clickable paragraphs).
    pub fn new(id_base: impl Into<SharedString>, source: impl Into<SharedString>) -> Self {
        Self {
            id_base: id_base.into(),
            source: source.into(),
            style: MarkdownStyle::default(),
            on_wiki_link: None,
            on_image: None,
        }
    }

    pub fn style(mut self, style: MarkdownStyle) -> Self {
        self.style = style;
        self
    }

    pub fn on_wiki_link(mut self, handler: WikiLinkHandler) -> Self {
        self.on_wiki_link = Some(handler);
        self
    }

    /// Supply a renderer for standalone images. Without one, images fall back
    /// to a clickable "🖼 alt" text label.
    pub fn on_image(mut self, handler: ImageRenderer) -> Self {
        self.on_image = Some(handler);
        self
    }
}

impl RenderOnce for MarkdownView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let source = self.source;
        let mut ctx = Ctx {
            style: self.style,
            on_wiki_link: self.on_wiki_link,
            on_image: self.on_image,
            id_base: self.id_base,
            counter: 0,
            definitions: HashMap::new(),
        };

        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .text_color(ctx.style.text_color)
            .text_size(ctx.style.text_size);

        match markdown::to_mdast(&source, &markdown::ParseOptions::gfm()) {
            Ok(mdast::Node::Root(root)) => {
                for node in &root.children {
                    collect_definitions(node, &mut ctx.definitions);
                }
                for node in &root.children {
                    if let Some(el) = render_block(node, &mut ctx) {
                        col = col.child(el);
                    }
                }
            }
            _ => col = col.child(StyledText::new(source)),
        }
        col
    }
}

struct Ctx {
    style: MarkdownStyle,
    on_wiki_link: Option<WikiLinkHandler>,
    on_image: Option<ImageRenderer>,
    id_base: SharedString,
    counter: usize,
    /// `[id] -> url` from reference definitions (`[id]: url`), collected up
    /// front so `[text][id]` references resolve regardless of definition order.
    definitions: HashMap<String, String>,
}

// --- Block rendering ---

fn render_block(node: &mdast::Node, ctx: &mut Ctx) -> Option<AnyElement> {
    match node {
        mdast::Node::Paragraph(p) => {
            // A paragraph that *starts* with `![alt](src)` (optionally
            // `{width=N}`) renders as a real image via the host. Any text that
            // follows on the same line (a caption typed right under it) renders
            // below the image rather than reverting the whole thing to text.
            if let Some(renderer) = ctx.on_image.clone()
                && let Some((info, rest)) = leading_image(&p.children)
            {
                let image = renderer(info);
                if rest.is_empty() {
                    return Some(image);
                }
                return Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(image)
                        .child(inline_element(&rest, ctx))
                        .into_any_element(),
                );
            }
            Some(inline_element(&p.children, ctx))
        }
        mdast::Node::Heading(h) => {
            let scale = match h.depth {
                1 => 1.8,
                2 => 1.5,
                3 => 1.3,
                4 => 1.15,
                5 => 1.05,
                _ => 1.0,
            };
            let size = px(f32::from(ctx.style.text_size) * scale);
            let color = ctx.style.heading_color;
            Some(
                div()
                    .text_size(size)
                    .text_color(color)
                    .font_weight(FontWeight::BOLD)
                    .child(inline_element(&h.children, ctx))
                    .into_any_element(),
            )
        }
        mdast::Node::List(list) => Some(render_list(list, ctx, 0)),
        mdast::Node::Code(c) => {
            let bg = ctx.style.code_bg;
            let color = ctx.style.code_color;
            Some(
                div()
                    .w_full()
                    .rounded(px(6.0))
                    .bg(bg)
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_color(color)
                    .child(StyledText::new(c.value.clone()))
                    .into_any_element(),
            )
        }
        mdast::Node::Blockquote(b) => {
            let muted = ctx.style.muted_color;
            let mut q = div()
                .border_l_2()
                .border_color(muted)
                .pl(px(12.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .text_color(muted);
            for child in &b.children {
                if let Some(el) = render_block(child, ctx) {
                    q = q.child(el);
                }
            }
            Some(q.into_any_element())
        }
        mdast::Node::ThematicBreak(_) => Some(
            div()
                .w_full()
                .h(px(1.0))
                .my(px(6.0))
                .bg(ctx.style.rule_color)
                .into_any_element(),
        ),
        mdast::Node::Table(t) => Some(render_table(t, ctx)),
        // A footnote definition: `[label] <content>`, rendered muted/smaller
        // where it sits (authors put these at the bottom).
        mdast::Node::FootnoteDefinition(f) => {
            let label = f.label.clone().unwrap_or_else(|| f.identifier.clone());
            let muted = ctx.style.muted_color;
            let mut body = div().flex().flex_col().gap(px(4.0));
            for child in &f.children {
                if let Some(el) = render_block(child, ctx) {
                    body = body.child(el);
                }
            }
            Some(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .text_size(px(f32::from(ctx.style.text_size) * 0.9))
                    .text_color(muted)
                    .child(div().flex_shrink_0().child(format!("[{label}]")))
                    .child(body)
                    .into_any_element(),
            )
        }
        // Raw HTML block: show the literal source (muted), never executed.
        mdast::Node::Html(h) => Some(
            div()
                .text_color(ctx.style.muted_color)
                .child(StyledText::new(h.value.clone()))
                .into_any_element(),
        ),
        // Stray inline content at block level, or unsupported blocks:
        // render whatever text we can.
        mdast::Node::Text(t) => Some(StyledText::new(t.value.clone()).into_any_element()),
        _ => None,
    }
}

/// If a paragraph *begins* with an image (ignoring leading whitespace), return
/// it as an [`ImageInfo`] — picking up a `{width=N}` attribute typed right after
/// it — along with the remaining inline nodes (e.g. a caption on the next line)
/// to render below the image. `None` if the paragraph doesn't start with an
/// image.
fn leading_image(children: &[mdast::Node]) -> Option<(ImageInfo, Vec<mdast::Node>)> {
    let mut iter = children.iter();
    let mut first = iter.next()?;
    // Skip a purely-whitespace leading text node.
    if let mdast::Node::Text(t) = first
        && t.value.trim().is_empty()
    {
        first = iter.next()?;
    }
    let mdast::Node::Image(img) = first else {
        return None;
    };
    let img_end = img.position.as_ref()?.end.offset;

    let rest: Vec<&mdast::Node> = iter.collect();
    let mut width = None;
    let mut attr_end = img_end;
    let mut out: Vec<mdast::Node> = Vec::new();

    // The text immediately after the image may begin with `{width=N}`.
    if let Some((mdast::Node::Text(t), tail)) = rest.split_first() {
        let remainder = if let Some((w, after)) = parse_leading_width(&t.value) {
            width = Some(w);
            let consumed = t.value.len() - after.len();
            attr_end = t
                .position
                .as_ref()
                .map_or(img_end, |p| p.start.offset + consumed);
            after
        } else {
            &t.value
        };
        let remainder = remainder.trim_start();
        if !remainder.is_empty() {
            out.push(text_node(remainder));
        }
        out.extend(tail.iter().map(|n| (*n).clone()));
    } else {
        out.extend(rest.iter().map(|n| (*n).clone()));
    }

    let attr_target = if width.is_some() {
        img_end..attr_end
    } else {
        img_end..img_end
    };
    Some((
        ImageInfo {
            src: img.url.clone().into(),
            alt: img.alt.clone().into(),
            width,
            attr_target,
        },
        out,
    ))
}

/// Parse a leading `{width=320}` / `{width=320px}` from `s`, returning the width
/// and the text after the closing `}`.
fn parse_leading_width(s: &str) -> Option<(f32, &str)> {
    let rest = s.strip_prefix("{width=")?;
    let close = rest.find('}')?;
    let num = rest[..close].strip_suffix("px").unwrap_or(&rest[..close]);
    let w = num.trim().parse::<f32>().ok().filter(|w| *w > 0.0)?;
    Some((w, &rest[close + 1..]))
}

fn text_node(value: &str) -> mdast::Node {
    mdast::Node::Text(mdast::Text {
        value: value.to_string(),
        position: None,
    })
}

fn render_list(list: &mdast::List, ctx: &mut Ctx, depth: usize) -> AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .pl(px(if depth > 0 { 18.0 } else { 2.0 }));
    let start = list.start.unwrap_or(1) as usize;

    for (i, item) in list.children.iter().enumerate() {
        let mdast::Node::ListItem(li) = item else {
            continue;
        };
        // GFM task items (`- [ ]` / `- [x]`) carry `checked`; render a box
        // instead of a bullet/number.
        let marker = if let Some(done) = li.checked {
            (if done { "☑" } else { "☐" }).to_string()
        } else if list.ordered {
            format!("{}.", start + i)
        } else {
            "•".to_string()
        };

        let mut content = div().flex().flex_col().gap(px(4.0));
        for child in &li.children {
            match child {
                mdast::Node::List(sub) => content = content.child(render_list(sub, ctx, depth + 1)),
                other => {
                    if let Some(el) = render_block(other, ctx) {
                        content = content.child(el);
                    }
                }
            }
        }

        col = col.child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .items_start()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(ctx.style.muted_color)
                        .child(marker),
                )
                .child(div().flex_1().min_w_0().child(content)),
        );
    }
    col.into_any_element()
}

// --- Inline rendering ---

enum LinkTarget {
    Wiki(SharedString),
    Url(SharedString),
}

#[derive(Default)]
struct Inline {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    links: Vec<(Range<usize>, LinkTarget)>,
}

fn inline_element(nodes: &[mdast::Node], ctx: &mut Ctx) -> AnyElement {
    let mut inl = Inline::default();
    build_inline(
        nodes,
        HighlightStyle::default(),
        &ctx.style,
        &ctx.definitions,
        &mut inl,
    );

    let styled = StyledText::new(inl.text).with_highlights(inl.highlights);
    if inl.links.is_empty() {
        return styled.into_any_element();
    }

    ctx.counter += 1;
    let id = ElementId::Name(format!("{}-{}", ctx.id_base, ctx.counter).into());
    let ranges: Vec<Range<usize>> = inl.links.iter().map(|(r, _)| r.clone()).collect();
    let targets: Vec<LinkTarget> = inl.links.into_iter().map(|(_, t)| t).collect();
    let on_wiki = ctx.on_wiki_link.clone();

    InteractiveText::new(id, styled)
        .on_click(ranges, move |ix, window, cx| {
            // The click was on a link range; consume it so it doesn't also reach
            // a surrounding host handler (e.g. a click-to-edit area).
            cx.stop_propagation();
            match targets.get(ix) {
                Some(LinkTarget::Wiki(title)) => {
                    if let Some(handler) = &on_wiki {
                        handler(title.clone(), window, cx);
                    }
                }
                Some(LinkTarget::Url(url)) => cx.open_url(url),
                None => {}
            }
        })
        .into_any_element()
}

fn build_inline(
    nodes: &[mdast::Node],
    cur: HighlightStyle,
    style: &MarkdownStyle,
    defs: &HashMap<String, String>,
    out: &mut Inline,
) {
    for node in nodes {
        match node {
            mdast::Node::Text(t) => push_text(&t.value, cur, style, out),
            mdast::Node::Strong(s) => {
                let mut c = cur;
                c.font_weight = Some(FontWeight::BOLD);
                build_inline(&s.children, c, style, defs, out);
            }
            mdast::Node::Emphasis(e) => {
                let mut c = cur;
                c.font_style = Some(FontStyle::Italic);
                build_inline(&e.children, c, style, defs, out);
            }
            mdast::Node::InlineCode(ic) => {
                let mut c = cur;
                c.color = Some(style.code_color);
                push_run(&ic.value, c, out);
            }
            mdast::Node::Link(l) => {
                let mut c = cur;
                c.color = Some(style.link_color);
                let start = out.text.len();
                build_inline(&l.children, c, style, defs, out);
                let end = out.text.len();
                if start < end {
                    out.links
                        .push((start..end, LinkTarget::Url(l.url.clone().into())));
                }
            }
            mdast::Node::Break(_) => push_run("\n", cur, out),
            mdast::Node::Delete(d) => {
                let mut c = cur;
                c.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: None,
                });
                build_inline(&d.children, c, style, defs, out);
            }
            mdast::Node::Image(img) => {
                // Render as a clickable label opening the URL (real image
                // rendering is a follow-up).
                let label = if img.alt.is_empty() {
                    "🖼 image".to_string()
                } else {
                    format!("🖼 {}", img.alt)
                };
                let mut c = cur;
                c.color = Some(style.link_color);
                let start = out.text.len();
                push_run(&label, c, out);
                let end = out.text.len();
                out.links
                    .push((start..end, LinkTarget::Url(img.url.clone().into())));
            }
            mdast::Node::FootnoteReference(f) => {
                // Render `[label]` as a marker (not clickable — jumping would
                // need anchors this text renderer doesn't have).
                let label = f.label.clone().unwrap_or_else(|| f.identifier.clone());
                let mut c = cur;
                c.color = Some(style.link_color);
                push_run(&format!("[{label}]"), c, out);
            }
            mdast::Node::LinkReference(l) => {
                // `[text][id]` resolved against the collected definitions; if
                // unresolved, the text still renders (just not linked).
                if let Some(url) = defs.get(&l.identifier).cloned() {
                    let mut c = cur;
                    c.color = Some(style.link_color);
                    let start = out.text.len();
                    build_inline(&l.children, c, style, defs, out);
                    let end = out.text.len();
                    if start < end {
                        out.links.push((start..end, LinkTarget::Url(url.into())));
                    }
                } else {
                    build_inline(&l.children, cur, style, defs, out);
                }
            }
            mdast::Node::ImageReference(img) => {
                let label = if img.alt.is_empty() {
                    "🖼 image".to_string()
                } else {
                    format!("🖼 {}", img.alt)
                };
                let mut c = cur;
                c.color = Some(style.link_color);
                let start = out.text.len();
                push_run(&label, c, out);
                let end = out.text.len();
                if let Some(url) = defs.get(&img.identifier) {
                    out.links
                        .push((start..end, LinkTarget::Url(url.clone().into())));
                }
            }
            // Inline raw HTML: render the literal source, never executed.
            mdast::Node::Html(h) => push_run(&h.value, cur, out),
            // Recurse into any other container node; ignore leaves we
            // don't special-case.
            other => {
                if let Some(children) = node_children(other) {
                    build_inline(children, cur, style, defs, out);
                }
            }
        }
    }
}

/// Push plain text, splitting out `[[wiki-links]]` and `#tags` into
/// clickable runs. Both navigate to a page; a tag keeps its `#` in the
/// display text but targets the bare name.
fn push_text(value: &str, cur: HighlightStyle, style: &MarkdownStyle, out: &mut Inline) {
    let bytes = value.as_bytes();
    let mut plain_start = 0;
    let mut i = 0;
    while i < value.len() {
        // [[wiki-link]]
        if value[i..].starts_with("[[") {
            if let Some(close) = value[i + 2..].find("]]") {
                let inner = &value[i + 2..i + 2 + close];
                // `[[target|label]]` shows `label` but links to `target`; `[[name]]`
                // uses the name for both. An empty label falls back to the target.
                let (target, display) = match inner.split_once('|') {
                    Some((t, l)) if !l.trim().is_empty() => (t.trim(), l.trim()),
                    Some((t, _)) => (t.trim(), t.trim()),
                    None => (inner.trim(), inner.trim()),
                };
                if !target.is_empty() {
                    push_run(&value[plain_start..i], cur, out);
                    push_link(display, target, style.link_color, cur, out);
                    i += 2 + close + 2;
                    plain_start = i;
                    continue;
                }
            }
            i += 1; // not a valid link; the '[' stays plain
            continue;
        }
        // #tag — at a word boundary, followed by tag characters
        if bytes[i] == b'#' && (i == 0 || is_boundary(bytes[i - 1])) {
            let mut j = i + 1;
            while j < value.len() && is_tag_char(bytes[j]) {
                j += 1;
            }
            if j > i + 1 {
                let name = &value[i + 1..j];
                push_run(&value[plain_start..i], cur, out);
                push_link(&value[i..j], name, style.tag_color, cur, out);
                i = j;
                plain_start = i;
                continue;
            }
        }
        i += value[i..].chars().next().map_or(1, |c| c.len_utf8());
    }
    push_run(&value[plain_start..], cur, out);
}

/// Push `display` as a clickable run that navigates to page `target`.
fn push_link(display: &str, target: &str, color: Hsla, cur: HighlightStyle, out: &mut Inline) {
    let mut c = cur;
    c.color = Some(color);
    let start = out.text.len();
    push_run(display, c, out);
    let end = out.text.len();
    out.links
        .push((start..end, LinkTarget::Wiki(target.into())));
}

fn is_boundary(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'(' | b'[')
}

fn is_tag_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn push_run(s: &str, style: HighlightStyle, out: &mut Inline) {
    if s.is_empty() {
        return;
    }
    let start = out.text.len();
    out.text.push_str(s);
    out.highlights.push((start..out.text.len(), style));
}

/// Children of a container mdast node we don't explicitly handle, so we
/// can still surface their inline text.
fn node_children(node: &mdast::Node) -> Option<&[mdast::Node]> {
    match node {
        mdast::Node::Paragraph(n) => Some(&n.children),
        _ => None,
    }
}

/// Walk the whole tree collecting reference definitions (`[id]: url`) so that
/// `[text][id]` / `![alt][id]` resolve no matter where the definition appears.
fn collect_definitions(node: &mdast::Node, out: &mut HashMap<String, String>) {
    if let mdast::Node::Definition(d) = node {
        out.entry(d.identifier.clone())
            .or_insert_with(|| d.url.clone());
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_definitions(child, out);
        }
    }
}

/// Render a GFM table as a bordered grid; the first row is the header.
fn render_table(table: &mdast::Table, ctx: &mut Ctx) -> AnyElement {
    let border = ctx.style.muted_color;
    let mut grid = div()
        .flex()
        .flex_col()
        .border_1()
        .border_color(border)
        .rounded(px(6.0))
        .overflow_hidden();

    for (ri, row) in table.children.iter().enumerate() {
        let mdast::Node::TableRow(r) = row else {
            continue;
        };
        let mut row_el = div().flex().flex_row();
        if ri > 0 {
            row_el = row_el.border_t_1().border_color(border);
        }
        for (ci, cell) in r.children.iter().enumerate() {
            let mdast::Node::TableCell(c) = cell else {
                continue;
            };
            let mut cell_el = div()
                .flex_1()
                .min_w_0()
                .px(px(10.0))
                .py(px(6.0))
                .border_r_1()
                .border_color(border);
            // Honor the column's GFM alignment (`:---:` / `---:`).
            match table.align.get(ci) {
                Some(mdast::AlignKind::Center) => cell_el = cell_el.text_center(),
                Some(mdast::AlignKind::Right) => cell_el = cell_el.text_right(),
                _ => {}
            }
            if ri == 0 {
                cell_el = cell_el.font_weight(FontWeight::BOLD);
            }
            row_el = row_el.child(cell_el.child(inline_element(&c.children, ctx)));
        }
        grid = grid.child(row_el);
    }
    grid.into_any_element()
}

// --- Editor helpers: markdown list / quote continuation + indent ---
//
// Pure, host-agnostic transforms over `(text, caret)` — no gpui or editor
// dependency. A markdown editor built on this crate wires Enter to
// `list_continuation` and Tab / Shift+Tab to `indent_list_line` / `outdent_line`,
// then applies the returned edit to its own input.

/// What pressing Enter should do on a markdown list / blockquote line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ListEdit {
    /// Insert this text at the caret — a newline plus the continued marker
    /// (e.g. `"\n- "`, `"\n2. "`, `"\n> "`, `"\n- [ ] "`), indent preserved.
    Continue(String),
    /// The current item is empty (just a marker); remove it. Delete the byte
    /// range `start..end` and leave the caret at `start` (an empty line).
    Exit { start: usize, end: usize },
}

/// Decide how Enter continues a markdown list/quote at `cursor` in `value`.
/// Recognizes `-`/`*`/`+` bullets, `N.`/`N)` ordered items, `- [ ]` task items,
/// and `>` blockquotes (leading indent preserved). A non-empty item continues
/// with the next marker; an empty item exits the list. `None` when the current
/// line isn't a list/quote item.
pub fn list_continuation(value: &str, cursor: usize) -> Option<ListEdit> {
    let cursor = cursor.min(value.len());
    let line_start = value[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let line_end = value[cursor..]
        .find('\n')
        .map_or(value.len(), |i| cursor + i);
    let line = &value[line_start..line_end];
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let (indent, rest) = line.split_at(indent_len);
    let (marker, content) = parse_list_marker(rest)?;
    if content.trim().is_empty() {
        Some(ListEdit::Exit {
            start: line_start,
            end: line_end,
        })
    } else {
        Some(ListEdit::Continue(format!("\n{indent}{marker}")))
    }
}

/// Parse a list/quote marker at the start of `rest` (after indent). Returns the
/// marker to begin the *next* line with, plus the content after this line's
/// marker. Task items are checked before plain bullets.
fn parse_list_marker(rest: &str) -> Option<(String, &str)> {
    let bullet = rest.chars().next().filter(|c| matches!(c, '-' | '*' | '+'));
    if let Some(b) = bullet
        && let Some(after) = rest[1..].strip_prefix(' ')
    {
        // Task item: `<bullet> [ ] content` (the box char is ignored — new items
        // start unchecked).
        if after.len() >= 3
            && after.starts_with('[')
            && after.as_bytes()[2] == b']'
            && let Some(content) = after[3..].strip_prefix(' ')
        {
            return Some((format!("{b} [ ] "), content));
        }
        // Plain bullet: `<bullet> content`.
        return Some((format!("{b} "), after));
    }
    // Ordered: `N. content` or `N) content` — continue with the next number.
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0
        && let Ok(n) = rest[..digits].parse::<u64>()
    {
        let after_num = &rest[digits..];
        for sep in ['.', ')'] {
            if let Some(after_sep) = after_num.strip_prefix(sep)
                && let Some(content) = after_sep.strip_prefix(' ')
            {
                return Some((format!("{}{sep} ", n + 1), content));
            }
        }
    }
    // Blockquote: `> content` (or `>content`).
    if let Some(after) = rest.strip_prefix('>') {
        let content = after.strip_prefix(' ').unwrap_or(after);
        return Some(("> ".to_string(), content));
    }
    None
}

/// One indent level for Tab / Shift+Tab on list items — two spaces.
pub const INDENT: &str = "  ";

/// If the caret's line is a list/quote item, indent it one level (insert
/// [`INDENT`] at the line start), returning the new text and shifted caret.
/// `None` when the line isn't a list item, so the caller can insert a literal
/// tab instead.
pub fn indent_list_line(value: &str, cursor: usize) -> Option<(String, usize)> {
    let cursor = cursor.min(value.len());
    let line_start = value[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let line_end = value[cursor..]
        .find('\n')
        .map_or(value.len(), |i| cursor + i);
    let line = &value[line_start..line_end];
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    parse_list_marker(&line[indent_len..])?; // only list / quote lines
    let new = format!("{}{INDENT}{}", &value[..line_start], &value[line_start..]);
    Some((new, cursor + INDENT.len()))
}

/// Outdent the caret's line one level: remove up to [`INDENT`] of leading spaces
/// (or one leading tab). Returns the new text and caret, or `None` if the line
/// has no leading indent to remove.
pub fn outdent_line(value: &str, cursor: usize) -> Option<(String, usize)> {
    let cursor = cursor.min(value.len());
    let line_start = value[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let line = &value[line_start..];
    let removed = if line.starts_with('\t') {
        1
    } else {
        line.bytes()
            .take(INDENT.len())
            .take_while(|b| *b == b' ')
            .count()
    };
    if removed == 0 {
        return None;
    }
    let new = format!("{}{}", &value[..line_start], &value[line_start + removed..]);
    let caret = cursor.saturating_sub(removed).max(line_start);
    Some((new, caret))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cont(s: &str) -> Option<ListEdit> {
        list_continuation(s, s.len())
    }

    #[test]
    fn list_continues_bullets() {
        assert_eq!(cont("- a"), Some(ListEdit::Continue("\n- ".into())));
        assert_eq!(cont("* a"), Some(ListEdit::Continue("\n* ".into())));
        assert_eq!(cont("+ a"), Some(ListEdit::Continue("\n+ ".into())));
    }

    #[test]
    fn list_continues_ordered_incrementing() {
        assert_eq!(cont("1. a"), Some(ListEdit::Continue("\n2. ".into())));
        assert_eq!(cont("9. x"), Some(ListEdit::Continue("\n10. ".into())));
        assert_eq!(cont("3) y"), Some(ListEdit::Continue("\n4) ".into())));
    }

    #[test]
    fn list_continues_task_unchecked() {
        assert_eq!(
            cont("- [x] done"),
            Some(ListEdit::Continue("\n- [ ] ".into()))
        );
        assert_eq!(
            cont("- [ ] todo"),
            Some(ListEdit::Continue("\n- [ ] ".into()))
        );
    }

    #[test]
    fn list_continues_blockquote_and_preserves_indent() {
        assert_eq!(cont("> hi"), Some(ListEdit::Continue("\n> ".into())));
        assert_eq!(cont("  - a"), Some(ListEdit::Continue("\n  - ".into())));
    }

    #[test]
    fn list_empty_item_exits() {
        assert_eq!(cont("- "), Some(ListEdit::Exit { start: 0, end: 2 }));
        assert_eq!(cont("1. "), Some(ListEdit::Exit { start: 0, end: 3 }));
        assert_eq!(cont("> "), Some(ListEdit::Exit { start: 0, end: 2 }));
        assert_eq!(cont("- [ ] "), Some(ListEdit::Exit { start: 0, end: 6 }));
    }

    #[test]
    fn list_continuation_ignores_non_lists() {
        assert_eq!(cont("hello"), None);
        assert_eq!(cont(""), None);
        assert_eq!(cont("-nospace"), None);
    }

    #[test]
    fn list_continuation_uses_the_caret_line() {
        // Cursor at the end of the second (list) line continues that line.
        let v = "intro\n- one";
        assert_eq!(
            list_continuation(v, v.len()),
            Some(ListEdit::Continue("\n- ".into()))
        );
    }

    #[test]
    fn tab_indents_list_lines() {
        assert_eq!(indent_list_line("- a", 3), Some(("  - a".into(), 5)));
        assert_eq!(indent_list_line("* x", 1), Some(("  * x".into(), 3)));
        assert_eq!(indent_list_line("1. y", 4), Some(("  1. y".into(), 6)));
        // Only the caret's line is indented.
        assert_eq!(
            indent_list_line("- a\n- b", 7),
            Some(("- a\n  - b".into(), 9))
        );
    }

    #[test]
    fn tab_ignores_non_list_lines() {
        assert_eq!(indent_list_line("hello", 5), None);
    }

    #[test]
    fn shift_tab_outdents() {
        assert_eq!(outdent_line("  - a", 5), Some(("- a".into(), 3)));
        assert_eq!(outdent_line("    x", 5), Some(("  x".into(), 3)));
        assert_eq!(outdent_line("- a", 3), None);
    }

    fn inline_of(text: &str) -> Inline {
        let mut inl = Inline::default();
        let nodes = vec![mdast::Node::Text(mdast::Text {
            value: text.into(),
            position: None,
        })];
        build_inline(
            &nodes,
            HighlightStyle::default(),
            &MarkdownStyle::default(),
            &HashMap::new(),
            &mut inl,
        );
        inl
    }

    #[test]
    fn wikilinks_become_clickable_runs_without_brackets() {
        let inl = inline_of("see [[Foo]] and [[Bar]] ok");
        assert_eq!(inl.text, "see Foo and Bar ok");
        assert_eq!(inl.links.len(), 2);
        let titles: Vec<&str> = inl
            .links
            .iter()
            .map(|(r, t)| match t {
                LinkTarget::Wiki(_) => &inl.text[r.clone()],
                _ => "",
            })
            .collect();
        assert_eq!(titles, vec!["Foo", "Bar"]);
    }

    #[test]
    fn aliased_wikilink_shows_label_links_target() {
        let inl = inline_of("jump [[file.pdf#p3|\u{2197}]] here");
        assert_eq!(inl.text, "jump \u{2197} here");
        assert_eq!(inl.links.len(), 1);
        let (range, target) = &inl.links[0];
        assert_eq!(&inl.text[range.clone()], "\u{2197}"); // displayed label
        match target {
            LinkTarget::Wiki(t) => assert_eq!(t.as_ref(), "file.pdf#p3"), // link target
            _ => panic!("expected wiki link"),
        }
    }

    #[test]
    fn hashtags_become_clickable_links_targeting_bare_name() {
        let inl = inline_of("a #foo and #bar-baz end");
        assert_eq!(inl.text, "a #foo and #bar-baz end"); // display keeps the '#'
        assert_eq!(inl.links.len(), 2);
        assert_eq!(&inl.text[inl.links[0].0.clone()], "#foo");
        match (&inl.links[0].1, &inl.links[1].1) {
            (LinkTarget::Wiki(a), LinkTarget::Wiki(b)) => {
                assert_eq!(a.as_ref(), "foo");
                assert_eq!(b.as_ref(), "bar-baz");
            }
            _ => panic!("expected wiki targets"),
        }
    }

    #[test]
    fn heading_hash_with_space_is_not_a_tag() {
        let inl = inline_of("# not a tag");
        assert!(inl.links.is_empty());
    }

    #[test]
    fn plain_text_has_no_links() {
        let inl = inline_of("just text");
        assert_eq!(inl.text, "just text");
        assert!(inl.links.is_empty());
    }

    #[test]
    fn empty_brackets_are_literal() {
        let inl = inline_of("a [[]] b");
        assert_eq!(inl.text, "a [[]] b");
        assert!(inl.links.is_empty());
    }

    #[test]
    fn parse_leading_width_variants() {
        assert_eq!(parse_leading_width("{width=320}"), Some((320.0, "")));
        assert_eq!(
            parse_leading_width("{width=200px}\nHere"),
            Some((200.0, "\nHere"))
        );
        assert_eq!(parse_leading_width("{width=0}"), None);
        assert_eq!(parse_leading_width("just text"), None);
    }

    #[test]
    fn leading_image_detection() {
        fn para_children(md: &str) -> Vec<mdast::Node> {
            let tree = markdown::to_mdast(md, &markdown::ParseOptions::gfm()).unwrap();
            if let mdast::Node::Root(root) = tree {
                for n in root.children {
                    if let mdast::Node::Paragraph(p) = n {
                        return p.children;
                    }
                }
            }
            vec![]
        }
        let (info, rest) = leading_image(&para_children("![alt](/x.png)")).unwrap();
        assert_eq!(info.src.as_ref(), "/x.png");
        assert_eq!(info.alt.as_ref(), "alt");
        assert_eq!(info.width, None);
        assert!(rest.is_empty());

        let (sized, rest) = leading_image(&para_children("![](/x.png){width=200}")).unwrap();
        assert_eq!(sized.width, Some(200.0));
        // The attribute span is non-empty so a resize replaces it in place.
        assert!(sized.attr_target.start < sized.attr_target.end);
        assert!(rest.is_empty());

        // A caption typed on the next line: the image still renders, and the
        // text is returned as `rest` to show below it (the reported bug).
        let (capt, rest) = leading_image(&para_children("![](/x.png){width=200}\nHere")).unwrap();
        assert_eq!(capt.width, Some(200.0));
        assert!(!rest.is_empty());

        // Text before the image isn't a leading-image block.
        assert!(leading_image(&para_children("see ![](/x.png) here")).is_none());
    }

    fn first_para(md: &str) -> Vec<mdast::Node> {
        let tree = markdown::to_mdast(md, &markdown::ParseOptions::gfm()).unwrap();
        if let mdast::Node::Root(root) = tree {
            for n in root.children {
                if let mdast::Node::Paragraph(p) = n {
                    return p.children;
                }
            }
        }
        vec![]
    }

    #[test]
    fn reference_link_resolves_against_definition() {
        let md = "[the docs][d]\n\n[d]: https://example.com";
        let tree = markdown::to_mdast(md, &markdown::ParseOptions::gfm()).unwrap();
        let mut defs = HashMap::new();
        if let mdast::Node::Root(root) = &tree {
            for n in &root.children {
                collect_definitions(n, &mut defs);
            }
        }
        assert_eq!(
            defs.get("d").map(String::as_str),
            Some("https://example.com")
        );

        let mut inl = Inline::default();
        build_inline(
            &first_para(md),
            HighlightStyle::default(),
            &MarkdownStyle::default(),
            &defs,
            &mut inl,
        );
        assert_eq!(inl.text, "the docs");
        assert_eq!(inl.links.len(), 1);
        assert!(
            matches!(&inl.links[0].1, LinkTarget::Url(u) if u.as_ref() == "https://example.com")
        );
    }

    #[test]
    fn footnote_reference_renders_marker() {
        let mut inl = Inline::default();
        build_inline(
            &first_para("text[^1]\n\n[^1]: the note"),
            HighlightStyle::default(),
            &MarkdownStyle::default(),
            &HashMap::new(),
            &mut inl,
        );
        assert_eq!(inl.text, "text[1]");
    }

    #[test]
    fn document_extracts_inline_text_from_blocks() {
        // Walk representative markdown the way `render_block` does: pull
        // inline text out of heading/paragraph blocks.
        let md = "# Title\n\nSome **bold** and *italic* and `code` with [[Link]].\n\n- a\n- b\n";
        let tree = markdown::to_mdast(md, &markdown::ParseOptions::default()).unwrap();
        let mut text = String::new();
        if let mdast::Node::Root(root) = tree {
            for n in &root.children {
                let children = match n {
                    mdast::Node::Heading(h) => &h.children,
                    mdast::Node::Paragraph(p) => &p.children,
                    _ => continue,
                };
                let mut inl = Inline::default();
                build_inline(
                    children,
                    HighlightStyle::default(),
                    &MarkdownStyle::default(),
                    &HashMap::new(),
                    &mut inl,
                );
                text.push_str(&inl.text);
                text.push('\n');
            }
        }
        assert!(text.contains("Title"), "got: {text:?}");
        assert!(text.contains("bold"));
        assert!(text.contains("Link")); // [[Link]] rendered as "Link"
    }
}
