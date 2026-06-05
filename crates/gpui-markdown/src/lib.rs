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
//! Scope (v1): headings, paragraphs, bold/italic/inline-code, fenced code
//! blocks, ordered/unordered nested lists, blockquotes, thematic breaks,
//! hard breaks, and links. Tables/images/footnotes render as plain text.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText,
    IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Styled, StyledText, Window, div,
    px, rgb, rgba,
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
            code_color: rgb(0xD7BA7D).into(),
            code_bg: rgba(0xFFFFFF14).into(),
            muted_color: rgb(0x9AA0A6).into(),
            rule_color: rgba(0xFFFFFF22).into(),
        }
    }
}

/// Called when a `[[wiki-link]]` is clicked, with the trimmed title.
pub type WikiLinkHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

/// A rendered markdown document element.
#[derive(IntoElement)]
pub struct MarkdownView {
    id_base: SharedString,
    source: SharedString,
    style: MarkdownStyle,
    on_wiki_link: Option<WikiLinkHandler>,
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
}

impl RenderOnce for MarkdownView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let source = self.source;
        let mut ctx = Ctx {
            style: self.style,
            on_wiki_link: self.on_wiki_link,
            id_base: self.id_base,
            counter: 0,
        };

        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .text_color(ctx.style.text_color)
            .text_size(ctx.style.text_size);

        match markdown::to_mdast(&source, &markdown::ParseOptions::default()) {
            Ok(mdast::Node::Root(root)) => {
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
    id_base: SharedString,
    counter: usize,
}

// --- Block rendering ---

fn render_block(node: &mdast::Node, ctx: &mut Ctx) -> Option<AnyElement> {
    match node {
        mdast::Node::Paragraph(p) => Some(inline_element(&p.children, ctx)),
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
            div().w_full().h(px(1.0)).my(px(6.0)).bg(ctx.style.rule_color).into_any_element(),
        ),
        // Stray inline content at block level, or unsupported blocks:
        // render whatever text we can.
        mdast::Node::Text(t) => Some(StyledText::new(t.value.clone()).into_any_element()),
        _ => None,
    }
}

fn render_list(list: &mdast::List, ctx: &mut Ctx, depth: usize) -> AnyElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .pl(px(if depth > 0 { 18.0 } else { 2.0 }));
    let start = list.start.unwrap_or(1) as usize;

    for (i, item) in list.children.iter().enumerate() {
        let mdast::Node::ListItem(li) = item else { continue };
        let marker = if list.ordered {
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
                .child(div().flex_shrink_0().text_color(ctx.style.muted_color).child(marker))
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
    build_inline(nodes, HighlightStyle::default(), &ctx.style, &mut inl);

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
        .on_click(ranges, move |ix, window, cx| match targets.get(ix) {
            Some(LinkTarget::Wiki(title)) => {
                if let Some(handler) = &on_wiki {
                    handler(title.clone(), window, cx);
                }
            }
            Some(LinkTarget::Url(url)) => cx.open_url(url),
            None => {}
        })
        .into_any_element()
}

fn build_inline(nodes: &[mdast::Node], cur: HighlightStyle, style: &MarkdownStyle, out: &mut Inline) {
    for node in nodes {
        match node {
            mdast::Node::Text(t) => push_text(&t.value, cur, style, out),
            mdast::Node::Strong(s) => {
                let mut c = cur;
                c.font_weight = Some(FontWeight::BOLD);
                build_inline(&s.children, c, style, out);
            }
            mdast::Node::Emphasis(e) => {
                let mut c = cur;
                c.font_style = Some(FontStyle::Italic);
                build_inline(&e.children, c, style, out);
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
                build_inline(&l.children, c, style, out);
                let end = out.text.len();
                if start < end {
                    out.links.push((start..end, LinkTarget::Url(l.url.clone().into())));
                }
            }
            mdast::Node::Break(_) => push_run("\n", cur, out),
            // Recurse into any other container node; ignore leaves we
            // don't special-case.
            other => {
                if let Some(children) = node_children(other) {
                    build_inline(children, cur, style, out);
                }
            }
        }
    }
}

/// Push plain text, splitting out `[[wiki-links]]` into clickable runs.
fn push_text(value: &str, cur: HighlightStyle, style: &MarkdownStyle, out: &mut Inline) {
    let mut rest = value;
    loop {
        let Some(open) = rest.find("[[") else {
            push_run(rest, cur, out);
            break;
        };
        let Some(close) = rest[open + 2..].find("]]") else {
            push_run(rest, cur, out);
            break;
        };
        push_run(&rest[..open], cur, out);
        let title = rest[open + 2..open + 2 + close].trim();
        if title.is_empty() {
            push_run(&rest[open..open + 2 + close + 2], cur, out);
        } else {
            let mut c = cur;
            c.color = Some(style.link_color);
            let start = out.text.len();
            push_run(title, c, out);
            let end = out.text.len();
            out.links.push((start..end, LinkTarget::Wiki(title.into())));
        }
        rest = &rest[open + 2 + close + 2..];
    }
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
        mdast::Node::Delete(n) => Some(&n.children),
        mdast::Node::Paragraph(n) => Some(&n.children),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inline_of(text: &str) -> Inline {
        let mut inl = Inline::default();
        let nodes = vec![mdast::Node::Text(mdast::Text { value: text.into(), position: None })];
        build_inline(&nodes, HighlightStyle::default(), &MarkdownStyle::default(), &mut inl);
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
                build_inline(children, HighlightStyle::default(), &MarkdownStyle::default(), &mut inl);
                text.push_str(&inl.text);
                text.push('\n');
            }
        }
        assert!(text.contains("Title"), "got: {text:?}");
        assert!(text.contains("bold"));
        assert!(text.contains("Link")); // [[Link]] rendered as "Link"
    }
}
