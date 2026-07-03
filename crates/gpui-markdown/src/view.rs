//! The reader view itself — everything behind the default-on `view` feature:
//! [`MarkdownView`], its styles, and the render tree. `lib.rs` holds the
//! crate docs; `syntax` (always compiled, dependency-free) holds the shared
//! construct recognition.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, Corners, ElementId, FontStyle, FontWeight, HighlightStyle, Hsla,
    InteractiveElement, InteractiveText, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Pixels, RenderImage, RenderOnce, ScrollHandle, SharedString, StatefulInteractiveElement,
    StrikethroughStyle, Styled, StyledText, TextRun, Window, canvas, div, point, px, relative, rgb,
    rgba, size, svg,
};
use markdown::mdast;

use crate::syntax::{AlertKind, TableStyle, alert_marker, heading_scale, table_style_marker};

/// Visual configuration for the renderer. The host fills this from its
/// own theme; defaults are a neutral dark palette.
#[derive(Clone)]
pub struct MarkdownStyle {
    pub text_color: Hsla,
    pub text_size: Pixels,
    /// Body line height as a multiple of `text_size`. Hosts with an editor
    /// match it to the editor's ratio so reading and editing line up (Zorite
    /// passes gpui-editor's 1.45); the default follows suit.
    pub line_height: f32,
    pub heading_color: Hsla,
    pub link_color: Hsla,
    pub tag_color: Hsla,
    pub code_color: Hsla,
    pub code_bg: Hsla,
    pub muted_color: Hsla,
    /// Thematic break (`---`) divider color.
    pub rule_color: Hsla,
    /// Nested-list indent guide — a hairline, fainter than `rule_color`.
    pub guide_color: Hsla,
    /// Background for `<mark>…</mark>` highlighted text. Translucent so the body text
    /// stays readable over it in any theme.
    pub mark_bg: Hsla,
    /// Backgrounds for in-page search: every match (`search_bg`) and the current /
    /// active one (`search_current_bg`). Painted only when a query is set via
    /// [`MarkdownView::search`]; the host owns the find UI. Translucent so text
    /// stays readable.
    pub search_bg: Hsla,
    pub search_current_bg: Hsla,
    /// Horizontal indent per nested list level. The host sizes this to match its
    /// editor's literal indent (so reading + editing line up).
    pub list_indent: Pixels,
    /// Monospace font family for code blocks + inline code. The host picks one that
    /// exists on the platform; an unknown family just falls back to the default font.
    pub mono_font: SharedString,
    /// GitHub-style alert (`> [!NOTE]` …) border + title colors.
    pub alerts: AlertColors,
    /// SVG asset paths for the alert title icons, resolved through the host's
    /// `AssetSource`. `None` (the default) renders the title without an icon,
    /// keeping the crate asset-free.
    pub alert_icons: Option<AlertIcons>,
}

/// Per-kind SVG asset paths for the alert title icons.
#[derive(Clone)]
pub struct AlertIcons {
    pub note: SharedString,
    pub tip: SharedString,
    pub important: SharedString,
    pub warning: SharedString,
    pub caution: SharedString,
}

impl Default for MarkdownStyle {
    fn default() -> Self {
        Self {
            text_color: rgb(0xE6E6E6).into(),
            text_size: px(15.0),
            line_height: 1.45,
            heading_color: rgb(0xFFFFFF).into(),
            link_color: rgb(0x4C9EFF).into(),
            tag_color: rgb(0x9D7CD8).into(),
            code_color: rgb(0xD7BA7D).into(),
            code_bg: rgba(0xFFFFFF14).into(),
            muted_color: rgb(0x9AA0A6).into(),
            rule_color: rgba(0xFFFFFF22).into(),
            guide_color: rgba(0xFFFFFF14).into(),
            mark_bg: rgba(0xFFD60066).into(),
            search_bg: rgba(0xFFD60055).into(),
            search_current_bg: rgba(0xFF9500DD).into(),
            list_indent: px(18.0),
            mono_font: "monospace".into(),
            alerts: AlertColors::default(),
            alert_icons: None,
        }
    }
}

/// Border + title colors for the five GitHub-style alerts (`> [!NOTE]` …).
/// Defaults are GitHub's dark palette; the host overlays its theme.
#[derive(Clone, Copy)]
pub struct AlertColors {
    pub note: Hsla,
    pub tip: Hsla,
    pub important: Hsla,
    pub warning: Hsla,
    pub caution: Hsla,
}

impl Default for AlertColors {
    fn default() -> Self {
        Self {
            note: rgb(0x4493F8).into(),
            tip: rgb(0x3FB950).into(),
            important: rgb(0xAB7DF8).into(),
            warning: rgb(0xD29922).into(),
            caution: rgb(0xF85149).into(),
        }
    }
}

/// View-side styling for the shared [`syntax::AlertKind`].
trait AlertKindExt {
    fn color(self, c: &AlertColors) -> Hsla;
    fn icon(self, i: &AlertIcons) -> SharedString;
}

impl AlertKindExt for AlertKind {
    fn color(self, c: &AlertColors) -> Hsla {
        match self {
            Self::Note => c.note,
            Self::Tip => c.tip,
            Self::Important => c.important,
            Self::Warning => c.warning,
            Self::Caution => c.caution,
        }
    }

    fn icon(self, i: &AlertIcons) -> SharedString {
        match self {
            Self::Note => i.note.clone(),
            Self::Tip => i.tip.clone(),
            Self::Important => i.important.clone(),
            Self::Warning => i.warning.clone(),
            Self::Caution => i.caution.clone(),
        }
    }
}

/// If blockquote `b` is a GitHub alert, return its kind and a copy of its
/// children with the marker stripped (the first text's source offset advances
/// by the stripped length, so the rendered→source click map stays aligned).
/// Public so other renderers of the same construct (e.g. a PDF exporter)
/// share the exact recognition.
pub fn alert_children(b: &mdast::Blockquote) -> Option<(AlertKind, Vec<mdast::Node>)> {
    let Some(mdast::Node::Paragraph(p)) = b.children.first() else {
        return None;
    };
    let Some(mdast::Node::Text(t)) = p.children.first() else {
        return None;
    };
    let (kind, strip) = alert_marker(&t.value)?;
    let mut children = b.children.clone();
    if let Some(mdast::Node::Paragraph(p)) = children.first_mut() {
        if strip >= t.value.len() {
            // The marker was the whole text node: drop it (and a following
            // hard Break, if the marker line ended with one).
            p.children.remove(0);
            if matches!(p.children.first(), Some(mdast::Node::Break(_))) {
                p.children.remove(0);
            }
            if p.children.is_empty() {
                children.remove(0);
            }
        } else if let Some(mdast::Node::Text(t)) = p.children.first_mut() {
            t.value.drain(..strip);
            if let Some(pos) = &mut t.position {
                pos.start.offset += strip;
            }
        }
    }
    Some((kind, children))
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
        label: "Note alert",
        snippet: "> [!NOTE] ",
        caret: 10,
    },
    Snippet {
        label: "Tip alert",
        snippet: "> [!TIP] ",
        caret: 9,
    },
    Snippet {
        label: "Important alert",
        snippet: "> [!IMPORTANT] ",
        caret: 15,
    },
    Snippet {
        label: "Warning alert",
        snippet: "> [!WARNING] ",
        caret: 13,
    },
    Snippet {
        label: "Caution alert",
        snippet: "> [!CAUTION] ",
        caret: 13,
    },
    Snippet {
        label: "Code block",
        snippet: "```\n\n```",
        caret: 4,
    },
    Snippet {
        label: "Mermaid diagram",
        snippet: "```mermaid\n\n```",
        caret: 11,
    },
    Snippet {
        label: "Math",
        snippet: "$$\n\n$$",
        caret: 3,
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
        label: "Inline math",
        snippet: "$$",
        caret: 1,
    },
    Snippet {
        label: "Highlight",
        snippet: "<mark></mark>",
        caret: 6,
    },
    Snippet {
        label: "Link",
        snippet: "[]()",
        caret: 1,
    },
    Snippet {
        label: "Wiki link",
        snippet: "[[]]",
        caret: 2,
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

/// Renders a ` ```mermaid ` code block as a diagram, given the block's source. The
/// host owns the (expensive, async) render — this crate just detects the fence and
/// hands the source over, staying renderer-agnostic. Set via
/// [`MarkdownView::on_mermaid`].
pub type MermaidRenderer = Rc<dyn Fn(SharedString) -> AnyElement>;

/// Colors a fenced code block's tokens: `(language tag, code) → sorted,
/// non-overlapping styled ranges` (byte offsets into the code). Supplied by
/// the host (e.g. a tree-sitter highlighter) so the crate stays engine-free;
/// absent it, code renders in the single `code_color`.
pub type CodeHighlighter = Rc<dyn Fn(&str, &str) -> Vec<(Range<usize>, HighlightStyle)>>;

/// Renders a `$$…$$` math block as a typeset image, given the block's LaTeX. Like
/// [`MermaidRenderer`], the host owns the (cached, off-thread) render — this crate just
/// detects the block and hands over the source. Set via [`MarkdownView::on_math`].
pub type MathRenderer = Rc<dyn Fn(SharedString) -> AnyElement>;

/// Resolves an inline `$…$` formula's LaTeX to its typeset raster + logical (display) px size
/// at text size, so the renderer can reserve a gap in the line and paint the image over it. The
/// host owns the (cached, off-thread) render; `None` while it's still rasterizing (the raw
/// `$…$` shows until then). Set via [`MarkdownView::on_inline_math`].
pub type InlineMathRenderer = Rc<dyn Fn(SharedString) -> Option<(Arc<RenderImage>, f32, f32)>>;

/// Called when the rendered text is clicked (outside a link), with the **source**
/// byte offset nearest the click and the click's window **y** — so the host can
/// place its editor caret there and keep it under the cursor when switching into
/// edit mode. Set via [`MarkdownView::on_click_source`].
pub type ClickSourceHandler = Rc<dyn Fn(usize, Pixels, &mut Window, &mut App)>;

/// Toggle the task checkbox of a clicked list item — the argument is the source
/// byte offset of that task item (feed it to [`toggle_task_at`]). Set via
/// [`MarkdownView::on_task_toggle`].
pub type TaskToggleHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// A rendered markdown document element — the reader view of a note.
#[derive(IntoElement)]
pub struct MarkdownView {
    id_base: SharedString,
    source: SharedString,
    style: MarkdownStyle,
    on_wiki_link: Option<WikiLinkHandler>,
    on_image: Option<ImageRenderer>,
    on_mermaid: Option<MermaidRenderer>,
    on_highlight: Option<CodeHighlighter>,
    on_math: Option<MathRenderer>,
    on_inline_math: Option<InlineMathRenderer>,
    /// In-page search query (non-empty when `Some`) + the active match index.
    query: Option<SharedString>,
    current_match: usize,
    /// When set, the block column is `track_scroll`ed with this handle, so the host
    /// can read each block's bounds (`bounds_for_item`) to scroll a match into view.
    block_scroll: Option<ScrollHandle>,
    /// Click-to-caret: maps a click on the rendered text to its source offset.
    on_click_source: Option<ClickSourceHandler>,
    /// Click a task checkbox to toggle it (the host applies + persists).
    on_task_toggle: Option<TaskToggleHandler>,
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
            on_mermaid: None,
            on_highlight: None,
            on_math: None,
            on_inline_math: None,
            query: None,
            current_match: 0,
            block_scroll: None,
            on_click_source: None,
            on_task_toggle: None,
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

    /// Supply a renderer for ` ```mermaid ` code blocks. Without one, a mermaid
    /// block renders as a plain code block.
    pub fn on_mermaid(mut self, handler: MermaidRenderer) -> Self {
        self.on_mermaid = Some(handler);
        self
    }

    /// Set the fenced-code syntax highlighter (see [`CodeHighlighter`]).
    pub fn on_highlight(mut self, handler: CodeHighlighter) -> Self {
        self.on_highlight = Some(handler);
        self
    }

    /// Supply a renderer for inline `$…$` formulas (raster + size). Without one, inline math
    /// stays literal `$…$` text.
    pub fn on_inline_math(mut self, handler: InlineMathRenderer) -> Self {
        self.on_inline_math = Some(handler);
        self
    }

    /// Supply a renderer for `$$…$$` math blocks. Without one, a math block renders as
    /// its raw LaTeX in a code block.
    pub fn on_math(mut self, handler: MathRenderer) -> Self {
        self.on_math = Some(handler);
        self
    }

    /// Highlight case-insensitive occurrences of `query` in the rendered (visible)
    /// text, emphasizing the `current`-th match (0-based, document order). An empty
    /// query highlights nothing. The host owns the find bar and the match index +
    /// total — pair this with [`match_count`] to size "n of m" and bound `current`.
    /// gpui-markdown only paints: no I/O, no storage, just the source string.
    pub fn search(mut self, query: impl Into<SharedString>, current: usize) -> Self {
        let q = query.into();
        self.query = (!q.is_empty()).then_some(q);
        self.current_match = current;
        self
    }

    /// Track-scroll the block column with `handle` so the host can read each block's
    /// laid-out bounds via [`ScrollHandle::bounds_for_item`] — indexed exactly as
    /// [`find_matches`] reports — and scroll a match into view. Pair with [`search`].
    ///
    /// [`search`]: MarkdownView::search
    pub fn track_blocks(mut self, handle: ScrollHandle) -> Self {
        self.block_scroll = Some(handle);
        self
    }

    /// Report the **source** byte offset nearest a click on the rendered text
    /// (outside a link), so the host can place its editor's caret there. Maps the
    /// click through gpui's text layout + a source-offset map built while rendering.
    pub fn on_click_source(mut self, handler: ClickSourceHandler) -> Self {
        self.on_click_source = Some(handler);
        self
    }

    /// Make task checkboxes clickable: clicking a `☐`/`☑` calls `handler` with the
    /// task item's source byte offset, so the host can flip it (see [`toggle_task_at`])
    /// and persist. Without this, checkboxes render but aren't interactive.
    pub fn on_task_toggle(mut self, handler: TaskToggleHandler) -> Self {
        self.on_task_toggle = Some(handler);
        self
    }
}

/// Content-keyed parse cache. The host's journal feed re-renders every
/// visible day on any interaction, and re-running `to_mdast` for every
/// non-editing day was the dominant per-frame cost (O(days × content)).
/// Keyed by the exact source string (no hash collisions to reason about;
/// the cap bounds memory to ~a few dozen notes of text), LRU-evicted, and
/// thread-local — gpui renders every window on the one UI thread, so all
/// windows share hits and there's no locking.
const PARSE_CACHE_CAP: usize = 64;

thread_local! {
    #[allow(clippy::type_complexity)]
    static PARSE_CACHE: RefCell<HashMap<String, (Arc<mdast::Node>, u64)>> =
        RefCell::new(HashMap::new());
    static PARSE_TICK: Cell<u64> = const { Cell::new(0) };
}

/// Parse `source` with the view's options (GFM + `$…$`/`$$…$$` math),
/// memoized. `None` when the parser errors (not cached — the caller falls
/// back to plain text).
fn parse_cached(source: &str) -> Option<Arc<mdast::Node>> {
    PARSE_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        let tick = PARSE_TICK.with(|t| {
            let v = t.get() + 1;
            t.set(v);
            v
        });
        if let Some((node, last_used)) = map.get_mut(source) {
            *last_used = tick;
            return Some(node.clone());
        }
        // Enable block math (`$$…$$` -> a Math node) and inline `$…$`
        // (`math_text` -> an InlineMath node). markdown's `math_text` already
        // follows the sensible rules (a `$` followed/preceded by a non-space,
        // etc.), so prose like "it cost $5" stays literal.
        let mut opts = markdown::ParseOptions::gfm();
        opts.constructs.math_flow = true;
        opts.constructs.math_text = true;
        let node = Arc::new(markdown::to_mdast(source, &opts).ok()?);
        if map.len() >= PARSE_CACHE_CAP {
            // Evict the least-recently-used entry (the cap is small; a linear
            // scan is cheaper than an ordered structure).
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (_, last))| *last)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(source.to_string(), (node.clone(), tick));
        Some(node)
    })
}

impl RenderOnce for MarkdownView {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let source = self.source;
        let block_scroll = self.block_scroll;
        let root_id: SharedString = format!("{}-md-root", self.id_base).into();
        let mut ctx = Ctx {
            style: self.style,
            on_wiki_link: self.on_wiki_link,
            on_image: self.on_image,
            on_mermaid: self.on_mermaid,
            on_highlight: self.on_highlight,
            on_math: self.on_math,
            on_inline_math: self.on_inline_math,
            id_base: self.id_base,
            counter: 0,
            definitions: HashMap::new(),
            query: self.query,
            current_match: self.current_match,
            match_ix: 0,
            on_click_source: self.on_click_source,
            on_task_toggle: self.on_task_toggle,
            suppress_heading_top: false,
        };

        let mut col = div()
            .id(root_id)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .text_color(ctx.style.text_color)
            .text_size(ctx.style.text_size)
            // Body line height matches the host's editor (gpui's default is
            // the taller phi), so reading and editing space text identically.
            .line_height(relative(ctx.style.line_height));

        let parsed = parse_cached(&source);
        match parsed.as_deref() {
            Some(mdast::Node::Root(root)) => {
                for node in &root.children {
                    collect_definitions(node, &mut ctx.definitions);
                }
                // A `<!-- table:STYLE -->` comment styles the next table and is
                // itself hidden; everything else renders normally.
                let mut pending_style = None;
                for node in &root.children {
                    if let mdast::Node::Html(h) = node
                        && let Some(style) = table_style_marker(&h.value)
                    {
                        pending_style = Some(style);
                        continue;
                    }
                    if let mdast::Node::Table(t) = node {
                        col = col.child(render_table(
                            t,
                            &mut ctx,
                            pending_style.take().unwrap_or_default(),
                            window,
                        ));
                        continue;
                    }
                    pending_style = None;
                    if let Some(el) = render_block(node, &mut ctx, window) {
                        col = col.child(el);
                    }
                }
            }
            _ => col = col.child(StyledText::new(source)),
        }
        // Track each block's bounds so the host can scroll a search match into view.
        if let Some(handle) = &block_scroll {
            col = col.track_scroll(handle);
        }
        col
    }
}

struct Ctx {
    style: MarkdownStyle,
    on_wiki_link: Option<WikiLinkHandler>,
    on_image: Option<ImageRenderer>,
    on_mermaid: Option<MermaidRenderer>,
    on_highlight: Option<CodeHighlighter>,
    on_math: Option<MathRenderer>,
    on_inline_math: Option<InlineMathRenderer>,
    id_base: SharedString,
    counter: usize,
    /// `[id] -> url` from reference definitions (`[id]: url`), collected up
    /// front so `[text][id]` references resolve regardless of definition order.
    definitions: HashMap<String, String>,
    /// In-page search: the active query (non-empty when `Some`), the current/active
    /// match index, and a running counter that assigns each match its document-order
    /// index as blocks render — so it stays in step with [`match_count`].
    query: Option<SharedString>,
    current_match: usize,
    match_ix: usize,
    on_click_source: Option<ClickSourceHandler>,
    on_task_toggle: Option<TaskToggleHandler>,
    /// Set while rendering a list item's first block: drops a leading heading's
    /// top margin so the bullet marker lines up with the heading text instead of
    /// floating above it.
    suppress_heading_top: bool,
}

// --- Block rendering ---

fn render_block(node: &mdast::Node, ctx: &mut Ctx, window: &mut Window) -> Option<AnyElement> {
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
            let scale = heading_scale(h.depth);
            let size = px(f32::from(ctx.style.text_size) * scale);
            let color = ctx.style.heading_color;
            // Extra room above a heading so a new section separates from the text
            // before it (on top of the inter-block gap); bigger headings get more.
            let top = if ctx.suppress_heading_top {
                px(0.0)
            } else {
                px(match h.depth {
                    1 => 16.0,
                    2 => 12.0,
                    3 => 8.0,
                    _ => 6.0,
                })
            };
            Some(
                div()
                    .mt(top)
                    .text_size(size)
                    .text_color(color)
                    .font_weight(FontWeight::BOLD)
                    .child(inline_element(&h.children, ctx))
                    .into_any_element(),
            )
        }
        mdast::Node::List(list) => Some(render_list(list, ctx, 0, window)),
        mdast::Node::Code(c) => {
            // A ```mermaid fence renders as a diagram when the host supplies a
            // renderer; otherwise it falls through to a normal code block.
            if c.lang.as_deref() == Some("mermaid")
                && let Some(renderer) = ctx.on_mermaid.clone()
            {
                return Some(renderer(c.value.clone().into()));
            }
            let bg = ctx.style.code_bg;
            let color = ctx.style.code_color;
            // With a host highlighter and a language tag, color the tokens
            // (the ranges come back sorted + non-overlapping, as
            // `with_highlights` requires).
            let text = match (&ctx.on_highlight, c.lang.as_deref()) {
                (Some(hl), Some(lang)) if !lang.is_empty() => {
                    StyledText::new(c.value.clone()).with_highlights(hl(lang, &c.value))
                }
                _ => StyledText::new(c.value.clone()),
            };
            // Size the card to its widest line, like WYSIWYG's code box (a
            // full-width card left most of the page as empty gray). Capped to
            // the column; a longer line wraps inside.
            let mut mono = window.text_style().font();
            mono.family = ctx.style.mono_font.clone();
            // Measure at bold: highlighted keywords render bold, and a
            // regular-weight measurement under-sizes the card into wrapping.
            mono.weight = FontWeight::BOLD;
            let widest = c
                .value
                .lines()
                .map(|l| {
                    let run = TextRun {
                        len: l.len(),
                        font: mono.clone(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    window
                        .text_system()
                        .shape_line(
                            SharedString::from(l.to_string()),
                            ctx.style.text_size,
                            &[run],
                            None,
                        )
                        .width()
                })
                .fold(px(0.0), Pixels::max);
            Some(
                div()
                    .w(widest + px(26.0))
                    .max_w_full()
                    .rounded(px(6.0))
                    .bg(bg)
                    .px(px(12.0))
                    .py(px(8.0))
                    .font_family(ctx.style.mono_font.clone())
                    .text_color(color)
                    .child(text)
                    .into_any_element(),
            )
        }
        mdast::Node::Math(m) => {
            // A `$$…$$` block renders as a typeset image when the host supplies a
            // renderer; otherwise it falls back to its raw LaTeX in a code block.
            if let Some(renderer) = ctx.on_math.clone() {
                return Some(renderer(m.value.clone().into()));
            }
            let bg = ctx.style.code_bg;
            let color = ctx.style.code_color;
            Some(
                div()
                    .w_full()
                    .rounded(px(6.0))
                    .bg(bg)
                    .px(px(12.0))
                    .py(px(8.0))
                    .font_family(ctx.style.mono_font.clone())
                    .text_color(color)
                    .child(StyledText::new(m.value.clone()))
                    .into_any_element(),
            )
        }
        mdast::Node::Blockquote(b) => {
            // A GitHub alert (`> [!NOTE]` …): colored border + bold title, body
            // in the normal text color (unlike a plain quote's muted tone).
            if let Some((kind, children)) = alert_children(b) {
                let color = kind.color(&ctx.style.alerts);
                let mut title = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(color);
                if let Some(icons) = &ctx.style.alert_icons {
                    let sz = px(f32::from(ctx.style.text_size));
                    title = title.child(
                        svg()
                            .path(kind.icon(icons))
                            .text_color(color)
                            .w(sz)
                            .h(sz)
                            .flex_shrink_0(),
                    );
                }
                title = title.child(kind.label());
                let mut q = div()
                    .border_l_2()
                    .border_color(color)
                    .pl(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(title);
                for child in &children {
                    if let Some(el) = render_block(child, ctx, window) {
                        q = q.child(el);
                    }
                }
                return Some(q.into_any_element());
            }
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
                if let Some(el) = render_block(child, ctx, window) {
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
        // Top-level tables get their style from a preceding marker (see the root
        // loop); a nested/standalone table here renders as the default Grid.
        mdast::Node::Table(t) => Some(render_table(t, ctx, TableStyle::default(), window)),
        // A footnote definition: `[label] <content>`, rendered muted/smaller
        // where it sits (authors put these at the bottom).
        mdast::Node::FootnoteDefinition(f) => {
            let label = f.label.clone().unwrap_or_else(|| f.identifier.clone());
            let muted = ctx.style.muted_color;
            let mut body = div().flex().flex_col().gap(px(4.0));
            for child in &f.children {
                if let Some(el) = render_block(child, ctx, window) {
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
        // Comments never render — they carry control markers (math alignment,
        // table styles) and are invisible in every view.
        mdast::Node::Html(h) if h.value.trim_start().starts_with("<!--") => None,
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

/// Every standalone image in `source` (a paragraph or list item that begins with
/// `![alt](src)`), in document order — each with its parsed `{width=N}` (if any)
/// and the `attr_target` byte range to overwrite to set or replace that width.
/// Mirrors how the renderer detects block images, so the offsets line up with
/// what's on screen. Pure: parses the markdown, no I/O or storage.
pub fn images(source: &str) -> Vec<ImageInfo> {
    let mut out = Vec::new();
    if let Ok(mdast::Node::Root(root)) = markdown::to_mdast(source, &markdown::ParseOptions::gfm())
    {
        collect_images(&root.children, &mut out);
    }
    out
}

/// Recurse paragraphs and list items, pushing each leading-image's [`ImageInfo`].
fn collect_images(nodes: &[mdast::Node], out: &mut Vec<ImageInfo>) {
    for node in nodes {
        match node {
            mdast::Node::Paragraph(p) => {
                if let Some((info, _rest)) = leading_image(&p.children) {
                    out.push(info);
                }
            }
            mdast::Node::List(l) => collect_images(&l.children, out),
            mdast::Node::ListItem(li) => collect_images(&li.children, out),
            _ => {}
        }
    }
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

fn render_list(list: &mdast::List, ctx: &mut Ctx, depth: usize, window: &mut Window) -> AnyElement {
    let nested = depth > 0;
    let mut col = div().flex().flex_col().gap(px(4.0)).pl(if nested {
        ctx.style.list_indent
    } else {
        px(2.0)
    });
    for (i, item) in list.children.iter().enumerate() {
        let mdast::Node::ListItem(li) = item else {
            continue;
        };
        // GFM task items (`- [ ]` / `- [x]`) carry `checked`; render a box
        // instead of a bullet/number.
        let marker = if let Some(done) = li.checked {
            (if done { "☑" } else { "☐" }).to_string()
        } else if list.ordered {
            // Word-style depth markers, counted from 1 — the WYSIWYG editor
            // numbers the same way, and source digits are display-irrelevant.
            crate::syntax::ordered_marker(depth, i as u32 + 1)
        } else {
            "•".to_string()
        };

        let mut content = div().flex().flex_col().gap(px(4.0));
        for (ci, child) in li.children.iter().enumerate() {
            match child {
                mdast::Node::List(sub) => {
                    content = content.child(render_list(sub, ctx, depth + 1, window))
                }
                other => {
                    // Drop a leading heading's top margin so the bullet lines up
                    // with the heading; later blocks in the item keep theirs.
                    let prev = ctx.suppress_heading_top;
                    ctx.suppress_heading_top = ci == 0;
                    if let Some(el) = render_block(other, ctx, window) {
                        content = content.child(el);
                    }
                    ctx.suppress_heading_top = prev;
                }
            }
        }

        // If the item leads with a heading, nudge the bullet down to the
        // heading's optical center. Both lines use gpui's default phi line
        // height, so the heading's line is taller by base * phi * (scale - 1);
        // half that gap re-centers the (top-aligned) bullet on the heading.
        let lead_scale = match li.children.first() {
            Some(mdast::Node::Heading(h)) => heading_scale(h.depth),
            _ => 1.0,
        };
        let marker_top = px(f32::from(ctx.style.text_size) * (lead_scale - 1.0) * 1.618_034 / 2.0);

        // The marker is a plain glyph, except a task's ☐/☑ is clickable: a click
        // calls back with the item's source offset so the host can flip + persist.
        let mut marker_el = div()
            .flex_shrink_0()
            .pt(marker_top)
            .text_color(ctx.style.muted_color)
            .child(marker);
        if li.checked.is_some()
            && let (Some(off), Some(toggle)) = (
                li.position.as_ref().map(|p| p.start.offset),
                ctx.on_task_toggle.clone(),
            )
        {
            marker_el = marker_el.cursor_pointer().on_mouse_down(
                MouseButton::Left,
                move |_: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    toggle(off, window, cx);
                },
            );
        }
        // An item with a nested list hangs a faint vertical guide from its
        // bullet down the sub-items (Logseq-style) — under the bullet itself,
        // not at the nested block's edge.
        let has_sub = li
            .children
            .iter()
            .any(|c| matches!(c, mdast::Node::List(_)));
        let marker_col = if has_sub {
            div()
                .flex()
                .flex_col()
                .items_center()
                .flex_shrink_0()
                .child(marker_el)
                .child(div().w(px(1.0)).flex_1().bg(ctx.style.guide_color))
                .into_any_element()
        } else {
            marker_el.into_any_element()
        };
        col = col.child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(marker_col)
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
    /// `(rendered byte offset, source byte offset)` checkpoints, in increasing
    /// rendered order, recorded as text is appended — so a click on the rendered
    /// text maps back to a source offset (see [`map_to_source`]).
    source_map: Vec<(usize, usize)>,
    /// Inline `$…$` formulas: `(rendered byte offset of the spacer, raster, logical w, h)`. The
    /// spacer (non-breaking spaces) reserves the width in the text; a canvas paints the raster
    /// over it at the laid-out position (see [`inline_element`]).
    math: Vec<(usize, Arc<RenderImage>, f32, f32)>,
}

impl Inline {
    /// Record that the text appended next maps to source byte offset `src`.
    fn map(&mut self, src: usize) {
        self.source_map.push((self.text.len(), src));
    }
}

fn inline_element(nodes: &[mdast::Node], ctx: &mut Ctx) -> AnyElement {
    let mut inl = Inline::default();
    build_inline(
        nodes,
        HighlightStyle::default(),
        &ctx.style,
        &ctx.definitions,
        ctx.on_inline_math.as_ref(),
        &mut inl,
    );

    // In-page search: overlay a background on each match in this block's visible
    // text (document order, a stronger colour for the active match), merged into
    // the existing formatting runs so the result stays sorted + non-overlapping
    // (which `with_highlights` / `compute_runs` requires).
    let highlights = if let Some(query) = ctx.query.clone() {
        let search: Vec<(Range<usize>, Hsla)> = scan_matches(&inl.text, &query)
            .into_iter()
            .map(|r| {
                let bg = if ctx.match_ix == ctx.current_match {
                    ctx.style.search_current_bg
                } else {
                    ctx.style.search_bg
                };
                ctx.match_ix += 1;
                (r, bg)
            })
            .collect();
        overlay_search(inl.highlights, &search)
    } else {
        inl.highlights
    };

    let math = std::mem::take(&mut inl.math);
    let styled = StyledText::new(inl.text).with_highlights(highlights);
    // Capture the text layout (a shared handle, populated on paint) so a click can
    // be mapped to a rendered byte index, then to a source offset (and so a canvas can paint
    // inline formulas over their spacers).
    let layout = styled.layout().clone();
    // Rendered-text ranges of this block's links, so the click-to-caret handler
    // below can ignore a click that lands on a link. A link's own `on_click`
    // fires on mouse-*up*; the caret handler fires on mouse-*down*, so without
    // this it would enter the editor first and swallow the link click.
    let link_ranges: Vec<Range<usize>> = inl.links.iter().map(|(r, _)| r.clone()).collect();

    let inner = if inl.links.is_empty() {
        styled.into_any_element()
    } else {
        ctx.counter += 1;
        let id = ElementId::Name(format!("{}-{}", ctx.id_base, ctx.counter).into());
        let targets: Vec<LinkTarget> = inl.links.into_iter().map(|(_, t)| t).collect();
        let on_wiki = ctx.on_wiki_link.clone();
        InteractiveText::new(id, styled)
            .on_click(link_ranges.clone(), move |ix, window, cx| {
                // The click was on a link range; consume it so it doesn't also reach
                // a surrounding host handler (e.g. the click-to-caret below).
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
    };

    // Click-to-caret: outside a link (link clicks `stop_propagation` above), map the
    // click to a source offset and report it so the host can place its editor caret
    // there. No handler (e.g. the journal feed) → just the inner element.
    let el = match ctx.on_click_source.clone() {
        None => inner,
        Some(on_click_source) => {
            let source_map = inl.source_map;
            let click_layout = layout.clone();
            div()
                .child(inner)
                .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                    let rendered = click_layout
                        .index_for_position(ev.position)
                        .unwrap_or_else(|e| e);
                    // A click on a link belongs to the link (its on_click fires on
                    // mouse-up) — don't hijack it for the caret.
                    if link_ranges.iter().any(|r| r.contains(&rendered)) {
                        return;
                    }
                    if let Some(src) = map_to_source(&source_map, rendered) {
                        // Consume so the host's surrounding click-to-edit doesn't also fire;
                        // pass the click's y so the host can keep the caret under the cursor.
                        cx.stop_propagation();
                        on_click_source(src, ev.position.y, window, cx);
                    }
                })
                .into_any_element()
        }
    };
    if math.is_empty() {
        return el;
    }
    // A paragraph with inline formulas: paint each raster over its spacer via a canvas painted
    // AFTER the text (so the text layout is populated + gives the spacer's window position), and
    // grow the line height so a tall formula (a fraction) doesn't overlap the neighbouring line.
    let tallest = math.iter().fold(0f32, |a, (.., h)| a.max(*h));
    let line_h = px((f32::from(ctx.style.text_size) * 1.4).max(tallest + 6.0));
    div()
        .relative()
        .line_height(line_h)
        .child(el)
        .child(
            canvas(
                |_, _, _| {},
                move |_bounds, _: (), window, _cx| {
                    let row_h = layout.line_height();
                    for (off, img, w, h) in &math {
                        if let Some(p) = layout.position_for_index(*off) {
                            let y = p.y + (row_h - px(*h)) / 2.0;
                            let b = Bounds::new(point(p.x, y), size(px(*w), px(*h)));
                            let _ =
                                window.paint_image(b, Corners::default(), img.clone(), 0, false);
                        }
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
}

fn build_inline(
    nodes: &[mdast::Node],
    cur: HighlightStyle,
    style: &MarkdownStyle,
    defs: &HashMap<String, String>,
    im: Option<&InlineMathRenderer>,
    out: &mut Inline,
) {
    // Mutable so `<mark>` / `</mark>` — flat sibling HTML tags, not a wrapping node —
    // can toggle the highlight on the runs between them.
    let mut cur = cur;
    for node in nodes {
        match node {
            mdast::Node::Text(t) => push_text(&t.value, node_src(node), cur, style, out),
            mdast::Node::Strong(s) => {
                let mut c = cur;
                c.font_weight = Some(FontWeight::BOLD);
                build_inline(&s.children, c, style, defs, im, out);
            }
            mdast::Node::Emphasis(e) => {
                let mut c = cur;
                c.font_style = Some(FontStyle::Italic);
                build_inline(&e.children, c, style, defs, im, out);
            }
            mdast::Node::InlineCode(ic) => {
                let mut c = cur;
                c.color = Some(style.code_color);
                // A subtle chip background sets inline code apart from prose. (A
                // monospace font can't be applied per text-run — `HighlightStyle` has no
                // font field — so inline code keeps the body font but gets the tint.)
                c.background_color = Some(style.code_bg);
                out.map(node_src(node) + 1); // +1 past the opening backtick
                push_run(&ic.value, c, out);
            }
            mdast::Node::InlineMath(m) => {
                // A ready formula reserves a non-breaking spacer (≈ its width) that a canvas
                // paints the raster over; until it's ready (or with no renderer) the raw
                // `$latex$` shows so the source is never lost.
                out.map(node_src(node));
                match im.and_then(|f| f(m.value.clone().into())) {
                    Some((img, w, h)) => {
                        let space_w = (f32::from(style.text_size) * 0.26).max(1.0);
                        let n = ((w / space_w).ceil() as usize).max(1);
                        out.math.push((out.text.len(), img, w, h));
                        out.text.extend(std::iter::repeat_n('\u{00A0}', n));
                    }
                    None => push_run(&format!("${}$", m.value), cur, out),
                }
            }
            mdast::Node::Link(l) => {
                let mut c = cur;
                c.color = Some(style.link_color);
                let start = out.text.len();
                build_inline(&l.children, c, style, defs, im, out);
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
                build_inline(&d.children, c, style, defs, im, out);
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
                    build_inline(&l.children, c, style, defs, im, out);
                    let end = out.text.len();
                    if start < end {
                        out.links.push((start..end, LinkTarget::Url(url.into())));
                    }
                } else {
                    build_inline(&l.children, cur, style, defs, im, out);
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
            // Inline raw HTML stays literal (never executed) — except `<mark>`, a
            // safe highlight tag: toggle a background on the runs it wraps.
            mdast::Node::Html(h) => {
                let tag = h.value.trim().to_ascii_lowercase();
                if tag == "<mark>" || tag.starts_with("<mark ") {
                    cur.background_color = Some(style.mark_bg);
                } else if tag == "</mark>" {
                    cur.background_color = None;
                } else {
                    push_run(&h.value, cur, out);
                }
            }
            // Recurse into any other container node; ignore leaves we
            // don't special-case.
            other => {
                if let Some(children) = node_children(other) {
                    build_inline(children, cur, style, defs, im, out);
                }
            }
        }
    }
}

/// Push plain text, splitting out `[[wiki-links]]` and `#tags` into
/// clickable runs. Both navigate to a page; a tag keeps its `#` in the
/// display text but targets the bare name.
fn push_text(
    value: &str,
    src_base: usize,
    cur: HighlightStyle,
    style: &MarkdownStyle,
    out: &mut Inline,
) {
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
                let (target, display) = crate::syntax::wiki_target_display(inner);
                if !target.is_empty() {
                    out.map(src_base + plain_start);
                    push_run(&value[plain_start..i], cur, out);
                    out.map(src_base + i + 2); // the display text sits just past `[[`
                    push_link(display, target, style.link_color, cur, out);
                    i += 2 + close + 2;
                    plain_start = i;
                    continue;
                }
            }
            i += 1; // not a valid link; the '[' stays plain
            continue;
        }
        // #tag — at a word boundary, followed by tag characters (the shared
        // grammar: namespaced `#a/b` included, boundary = any non-word char).
        if bytes[i] == b'#' && (i == 0 || !crate::syntax::is_word_char(bytes[i - 1])) {
            let mut j = i + 1;
            while j < value.len() && crate::syntax::is_tag_char(bytes[j]) {
                j += 1;
            }
            if j > i + 1 {
                let name = &value[i + 1..j];
                out.map(src_base + plain_start);
                push_run(&value[plain_start..i], cur, out);
                out.map(src_base + i); // the tag (with its `#`) is verbatim in the source
                push_link(&value[i..j], name, style.tag_color, cur, out);
                i = j;
                plain_start = i;
                continue;
            }
        }
        i += value[i..].chars().next().map_or(1, |c| c.len_utf8());
    }
    out.map(src_base + plain_start);
    push_run(&value[plain_start..], cur, out);
}

/// Source byte offset where `node` begins (0 if the parser recorded none).
fn node_src(node: &mdast::Node) -> usize {
    node.position().map_or(0, |p| p.start.offset)
}

/// Toggle the GFM task checkbox on the source line containing byte `offset` (a task
/// item's offset, as reported by [`MarkdownView::on_task_toggle`]). Returns the full
/// `content` with that one checkbox flipped (`[ ]`↔`[x]`), or `None` if there's no
/// task checkbox on that line. Length is unchanged (one ASCII byte swapped).
pub fn toggle_task_at(content: &str, offset: usize) -> Option<String> {
    if offset > content.len() {
        return None;
    }
    let line_start = content[..offset].rfind('\n').map_or(0, |p| p + 1);
    let line_end = content[offset..]
        .find('\n')
        .map_or(content.len(), |p| offset + p);
    let line = &content.as_bytes()[line_start..line_end];
    // The checkbox is the first `[ ]`/`[x]` on the line (it precedes any body text).
    let lb = line.iter().position(|&b| b == b'[')?;
    if lb + 2 < line.len() && line[lb + 2] == b']' && matches!(line[lb + 1], b' ' | b'x' | b'X') {
        let box_byte = line_start + lb + 1; // the status char, in `content`
        let flipped = if matches!(line[lb + 1], b'x' | b'X') {
            " "
        } else {
            "x"
        };
        let mut out = content.to_string();
        out.replace_range(box_byte..box_byte + 1, flipped);
        return Some(out);
    }
    None
}

/// Map a rendered byte index to a source byte offset via the checkpoints recorded
/// while building the inline text. `None` when there's no checkpoint at/before the
/// index (the host then falls back to plain enter-edit).
fn map_to_source(map: &[(usize, usize)], rendered: usize) -> Option<usize> {
    map.iter()
        .rev()
        .find(|(r, _)| *r <= rendered)
        .map(|(r, s)| s + (rendered - r))
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
/// Per-table visual style, from a `<!-- table:STYLE -->` marker comment on the
/// line above the table (mirrors the editor; the style names are the shared
/// contract). `Grid` is the default (no marker).
fn render_table(
    table: &mdast::Table,
    ctx: &mut Ctx,
    style: TableStyle,
    window: &mut Window,
) -> AnyElement {
    let border = ctx.style.muted_color;
    let shade = ctx.style.code_bg;
    // Content-measured column widths, like WYSIWYG's `table_column_widths`
    // (the old equal-width `flex_1` columns stretched every table to the full
    // content width). Cells measure at their rendered size — bold header —
    // with the same 10px cell pad and 48px floor; an over-wide table scrolls
    // horizontally in its own row (the wide-image pattern) instead of
    // squeezing.
    let cell_pad = px(10.0);
    let ncols = table
        .children
        .iter()
        .filter_map(|r| match r {
            mdast::Node::TableRow(r) => Some(r.children.len()),
            _ => None,
        })
        .max()
        .unwrap_or(1)
        .max(1);
    let base_font = window.text_style().font();
    let mut widths = vec![px(0.0); ncols];
    for (ri, row) in table.children.iter().enumerate() {
        let mdast::Node::TableRow(r) = row else {
            continue;
        };
        for (ci, cell) in r.children.iter().enumerate().take(ncols) {
            let mdast::Node::TableCell(c) = cell else {
                continue;
            };
            let text = inline_text(&c.children, &ctx.style, &ctx.definitions);
            if text.is_empty() {
                continue;
            }
            let mut font = base_font.clone();
            if ri == 0 {
                font.weight = FontWeight::BOLD;
            }
            let run = TextRun {
                len: text.len(),
                font,
                color: ctx.style.text_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let w = window
                .text_system()
                .shape_line(SharedString::from(text), ctx.style.text_size, &[run], None)
                .width();
            widths[ci] = widths[ci].max(w + cell_pad * 2.0);
        }
    }
    for w in &mut widths {
        *w = (*w).max(px(48.0));
    }
    // The grid must be sized explicitly: gpui's default stretch alignment
    // would otherwise fill the parent's full width, leaving a void after the
    // last column (borders: one vline per cell + the outer box).
    let total: Pixels = widths.iter().copied().sum::<Pixels>() + px((ncols + 2) as f32);
    let boxed = matches!(style, TableStyle::Grid);
    let vlines = matches!(style, TableStyle::Grid);
    let row_lines = matches!(style, TableStyle::Grid);
    // A single rule under the header instead of a divider between every row.
    let header_rule = matches!(style, TableStyle::Striped | TableStyle::Minimal);

    let mut grid = div().flex().flex_col().w(total).flex_shrink_0();
    if boxed {
        grid = grid
            .border_1()
            .border_color(border)
            .rounded(px(6.0))
            .overflow_hidden();
    }

    for (ri, row) in table.children.iter().enumerate() {
        let mdast::Node::TableRow(r) = row else {
            continue;
        };
        let is_header = ri == 0;
        // The mdast table has no separator child: row 0 is the header, row 1 the
        // first body row (body_index 0).
        let body_index = ri.checked_sub(1);
        let mut row_el = div().flex().flex_row();
        // Top divider: under every row (Grid), just under the header
        // (Striped/Minimal → the first body row's top), or never (Header).
        let top_divider = if row_lines {
            !is_header
        } else {
            header_rule && body_index == Some(0)
        };
        if top_divider {
            row_el = row_el.border_t_1().border_color(border);
        }
        // Row shading: the header (Header style) or alternate body rows (Striped).
        let shaded = match style {
            TableStyle::Header => is_header,
            TableStyle::Striped => body_index.is_some_and(|b| b % 2 == 1),
            _ => false,
        };
        if shaded {
            row_el = row_el.bg(shade);
        }
        for (ci, cell) in r.children.iter().enumerate() {
            let mdast::Node::TableCell(c) = cell else {
                continue;
            };
            let mut cell_el = div()
                .w(widths.get(ci).copied().unwrap_or(px(48.0)))
                .flex_shrink_0()
                .px(px(10.0))
                // WYSIWYG's row height (text x 1.45 + 12); wrapped cells grow.
                .min_h(px(f32::from(ctx.style.text_size) * 1.45 + 12.0))
                .flex()
                .items_center();
            if vlines && ci + 1 < r.children.len() {
                cell_el = cell_el.border_r_1().border_color(border);
            }
            // Honor the column's GFM alignment (`:---:` / `---:`).
            match table.align.get(ci) {
                Some(mdast::AlignKind::Center) => cell_el = cell_el.text_center(),
                Some(mdast::AlignKind::Right) => cell_el = cell_el.text_right(),
                _ => {}
            }
            if is_header {
                cell_el = cell_el.font_weight(FontWeight::BOLD);
            }
            row_el = row_el.child(cell_el.child(inline_element(&c.children, ctx)));
        }
        grid = grid.child(row_el);
    }
    // Content-sized: the grid hugs its columns; a table wider than the note
    // column scrolls horizontally in its own row (like oversized images)
    // while the text around it keeps wrapping normally.
    ctx.counter += 1;
    div()
        .id(("md-table", ctx.counter))
        // WYSIWYG indents tables into a 22px gutter (its row handles live
        // there); mirror it so the grid sits at the same x in both views.
        .ml(px(22.0))
        .max_w_full()
        .overflow_x_scroll()
        .child(grid)
        .into_any_element()
}

// --- In-page search ---
//
// Pure, host-agnostic search over the *rendered* (visible) text — markup already
// stripped, the same text the reader sees. `match_count` sizes a host find bar's
// "n of m"; `MarkdownView::search` paints the matches. Both walk the inline blocks
// in the same document order, so the running match index lines up. No I/O, no
// storage — only the source string, so any app can reuse it (db or not).

/// Case-insensitive (ASCII-folded), non-overlapping byte ranges of `query` in
/// `text`. Ranges land on char boundaries, so they're safe for `StyledText`.
/// Non-ASCII is matched exactly (no Unicode case-folding — a rare nicety).
fn scan_matches(text: &str, query: &str) -> Vec<Range<usize>> {
    let (t, q) = (text.as_bytes(), query.as_bytes());
    let mut out = Vec::new();
    if q.is_empty() || q.len() > t.len() {
        return out;
    }
    let mut i = 0;
    while i + q.len() <= t.len() {
        if t[i..i + q.len()].eq_ignore_ascii_case(q)
            && text.is_char_boundary(i)
            && text.is_char_boundary(i + q.len())
        {
            out.push(i..i + q.len());
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Merge `search` backgrounds into a block's existing (sorted, non-overlapping)
/// `formatting` runs, producing a sorted, non-overlapping set — splitting at every
/// boundary and OR-ing the search background onto any overlapping formatting run.
/// `with_highlights` / `compute_runs` require that ordering, so a match landing on
/// a bold/italic/link span can't just be appended.
fn overlay_search(
    formatting: Vec<(Range<usize>, HighlightStyle)>,
    search: &[(Range<usize>, Hsla)],
) -> Vec<(Range<usize>, HighlightStyle)> {
    if search.is_empty() {
        return formatting;
    }
    let mut points: Vec<usize> = Vec::with_capacity((formatting.len() + search.len()) * 2);
    for (r, _) in &formatting {
        points.push(r.start);
        points.push(r.end);
    }
    for (r, _) in search {
        points.push(r.start);
        points.push(r.end);
    }
    points.sort_unstable();
    points.dedup();

    let mut out = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        // Boundaries split at every range edge, so each segment is fully inside or
        // fully outside every range — containment is a simple test.
        let fmt = formatting
            .iter()
            .find(|(r, _)| r.start <= a && b <= r.end)
            .map(|(_, s)| *s);
        let bg = search
            .iter()
            .find(|(r, _)| r.start <= a && b <= r.end)
            .map(|(_, c)| *c);
        match (fmt, bg) {
            (None, None) => {} // plain run — compute_runs fills the gap with the default
            (Some(s), None) => out.push((a..b, s)),
            (None, Some(c)) => out.push((
                a..b,
                HighlightStyle {
                    background_color: Some(c),
                    ..Default::default()
                },
            )),
            (Some(mut s), Some(c)) => {
                s.background_color = Some(c);
                out.push((a..b, s));
            }
        }
    }
    out
}

/// The visible text of an inline run (markup stripped), exactly as `inline_element`
/// renders it. Style/definitions don't affect the *text*, so defaults are fine.
fn inline_text(
    nodes: &[mdast::Node],
    style: &MarkdownStyle,
    defs: &HashMap<String, String>,
) -> String {
    let mut inl = Inline::default();
    build_inline(
        nodes,
        HighlightStyle::default(),
        style,
        defs,
        None,
        &mut inl,
    );
    inl.text
}

/// Visit each block's visible inline text in document order, mirroring exactly the
/// nodes `render_block` sends through `inline_element` — so search counts and
/// indices match what's painted. Code blocks / raw HTML render text directly and
/// aren't searched.
fn for_each_inline_text(
    nodes: &[mdast::Node],
    style: &MarkdownStyle,
    defs: &HashMap<String, String>,
    f: &mut impl FnMut(&str),
) {
    for node in nodes {
        match node {
            mdast::Node::Paragraph(p) => f(&inline_text(&p.children, style, defs)),
            mdast::Node::Heading(h) => f(&inline_text(&h.children, style, defs)),
            mdast::Node::TableCell(c) => f(&inline_text(&c.children, style, defs)),
            mdast::Node::List(l) => for_each_inline_text(&l.children, style, defs, f),
            mdast::Node::ListItem(li) => for_each_inline_text(&li.children, style, defs, f),
            mdast::Node::Blockquote(b) => {
                // Mirror the alert-marker strip in `render_block`, so search
                // match indices line up with what's painted.
                if let Some((_, children)) = alert_children(b) {
                    for_each_inline_text(&children, style, defs, f);
                } else {
                    for_each_inline_text(&b.children, style, defs, f);
                }
            }
            mdast::Node::Table(t) => for_each_inline_text(&t.children, style, defs, f),
            mdast::Node::TableRow(r) => for_each_inline_text(&r.children, style, defs, f),
            mdast::Node::FootnoteDefinition(fd) => {
                for_each_inline_text(&fd.children, style, defs, f)
            }
            _ => {}
        }
    }
}

/// Count case-insensitive matches of `query` in the rendered (visible) text of
/// `source` — the same matches [`MarkdownView::search`] highlights, in the same
/// order. Pure: parses the markdown, no I/O or storage. Empty query → 0. Use it to
/// size a host find bar's "n of m" and to bound the active-match index.
pub fn match_count(source: &str, query: &str) -> usize {
    find_matches(source, query).len()
}

/// The **block index** (top-level column-child index, as rendered) of each match of
/// `query` in `source`, in document order. Pair with [`MarkdownView::track_blocks`]:
/// the host reads `bounds_for_item(find_matches(..)[current])` to scroll the active
/// match's block into view. Pure — parses the markdown, no I/O or storage.
pub fn find_matches(source: &str, query: &str) -> Vec<usize> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let style = MarkdownStyle::default();
    let defs = HashMap::new();
    if let Ok(mdast::Node::Root(root)) = markdown::to_mdast(source, &markdown::ParseOptions::gfm())
    {
        // Walk top-level blocks in render order, assigning each a column-child index
        // (only blocks that render to a child get one — same as `render`), and push
        // that index once per match found inside it (recursing through inline text).
        let mut block_ix = 0usize;
        for node in &root.children {
            if !renders_to_block(node) {
                continue;
            }
            let mut n = 0;
            for_each_inline_text(std::slice::from_ref(node), &style, &defs, &mut |t| {
                n += scan_matches(t, query).len();
            });
            out.extend(std::iter::repeat_n(block_ix, n));
            block_ix += 1;
        }
    }
    out
}

/// Whether a top-level node renders to a column child (mirrors `render_block`
/// returning `Some`). Kept in sync with `render_block` so `find_matches`' block
/// indices line up with the `track_blocks` handle's `bounds_for_item`.
fn renders_to_block(node: &mdast::Node) -> bool {
    match node {
        // A `<!-- table:STYLE -->` marker is hidden (folded into the next table),
        // so it isn't a column child; other HTML renders as a muted block.
        mdast::Node::Html(h) => table_style_marker(&h.value).is_none(),
        _ => matches!(
            node,
            mdast::Node::Paragraph(_)
                | mdast::Node::Heading(_)
                | mdast::Node::List(_)
                | mdast::Node::Code(_)
                | mdast::Node::Blockquote(_)
                | mdast::Node::ThematicBreak(_)
                | mdast::Node::Table(_)
                | mdast::Node::FootnoteDefinition(_)
                | mdast::Node::Text(_)
        ),
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn scan_is_ascii_case_insensitive_and_nonoverlapping() {
        assert_eq!(
            scan_matches("Hello hello HELLO", "hello"),
            vec![0..5, 6..11, 12..17]
        );
        assert_eq!(scan_matches("aaaa", "aa"), vec![0..2, 2..4]); // non-overlapping
        assert!(scan_matches("abc", "").is_empty());
        assert!(scan_matches("abc", "xyz").is_empty());
    }

    #[test]
    fn table_style_marker_parses() {
        assert_eq!(
            table_style_marker("<!-- table:striped -->"),
            Some(TableStyle::Striped)
        );
        assert_eq!(
            table_style_marker("<!--table:header-->"),
            Some(TableStyle::Header)
        );
        assert_eq!(table_style_marker("<!-- table:nope -->"), None);
        assert_eq!(table_style_marker("<!-- ordinary -->"), None);
    }

    #[test]
    fn match_count_searches_visible_text_not_markup() {
        // Markup is stripped: the word is found, the syntax isn't.
        assert_eq!(match_count("**bold** and _italic_", "bold"), 1);
        assert_eq!(match_count("**bold**", "*"), 0);
        // Across blocks, case-insensitive (heading + paragraph).
        assert_eq!(match_count("# Title\n\nthe title here", "title"), 2);
        // List items + table cells are searched; nested too.
        assert_eq!(match_count("- one\n- two one", "one"), 2);
        assert_eq!(match_count("| a | one |\n|---|---|\n| one | b |", "one"), 2);
        assert_eq!(match_count("", "x"), 0);
        // A fenced code block is NOT searched (renders text directly).
        assert_eq!(match_count("```\nsecret\n```", "secret"), 0);
    }

    #[test]
    fn find_matches_reports_block_indices() {
        // Heading is block 0 (1 match), paragraph is block 1 (2 matches).
        assert_eq!(
            find_matches("# title here\n\nthe title and a title", "title"),
            vec![0, 1, 1]
        );
        // A code block holds block index 0 (no inline text), shifting the paragraph
        // with the matches to block 1 — so the host scrolls to the right child.
        assert_eq!(
            find_matches("```\ncode\n```\n\nfind me, find", "find"),
            vec![1, 1]
        );
        assert!(find_matches("x", "").is_empty());
    }

    fn first_paragraph_inline(source: &str) -> Inline {
        let style = MarkdownStyle::default();
        let defs = HashMap::new();
        let mdast::Node::Root(root) =
            markdown::to_mdast(source, &markdown::ParseOptions::gfm()).unwrap()
        else {
            panic!("not a root")
        };
        let mdast::Node::Paragraph(p) = &root.children[0] else {
            panic!("not a paragraph")
        };
        let mut inl = Inline::default();
        build_inline(
            &p.children,
            HighlightStyle::default(),
            &style,
            &defs,
            None,
            &mut inl,
        );
        inl
    }

    #[test]
    fn source_map_maps_rendered_clicks_back_to_source() {
        // "See [[Foo]] now" renders "See Foo now"; clicking past the (stripped) link
        // must land on the matching source byte.
        let src = "See [[Foo]] now";
        let inl = first_paragraph_inline(src);
        assert_eq!(inl.text, "See Foo now");
        let rendered_now = inl.text.find("now").unwrap(); // 8
        let s = map_to_source(&inl.source_map, rendered_now).unwrap();
        assert_eq!(&src[s..s + 3], "now");
        // Plain head maps 1:1.
        assert_eq!(map_to_source(&inl.source_map, 1), Some(1));
    }

    #[test]
    fn map_to_source_interpolates_and_handles_empty() {
        let map = vec![(0, 0), (4, 6), (7, 11)];
        assert_eq!(map_to_source(&map, 2), Some(2)); // 0 + 2
        assert_eq!(map_to_source(&map, 5), Some(7)); // 6 + (5-4)
        assert_eq!(map_to_source(&map, 9), Some(13)); // 11 + (9-7)
        assert!(map_to_source(&[], 3).is_none());
    }

    #[test]
    fn overlay_splits_and_merges_overlap() {
        // A match (2..6) overlapping a bold run (0..4) → three sorted, non-overlapping
        // segments: bold-only, bold+bg, bg-only.
        let fmt = vec![(
            0..4,
            HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            },
        )];
        let search = vec![(2..6, rgba(0xFF0000FF).into())];
        let out = overlay_search(fmt, &search);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].0, 0..2);
        assert!(out[0].1.background_color.is_none() && out[0].1.font_weight.is_some());
        assert_eq!(out[1].0, 2..4);
        assert!(out[1].1.background_color.is_some() && out[1].1.font_weight.is_some());
        assert_eq!(out[2].0, 4..6);
        assert!(out[2].1.background_color.is_some() && out[2].1.font_weight.is_none());
    }
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

/// The default indent level (two spaces) for Tab / Shift+Tab on list items. The
/// host passes its configured indent to [`indent_list_line`] / [`outdent_line`];
/// this is just the fallback / a convenience for callers without a setting.
pub const INDENT: &str = "  ";

/// If the caret's line is a list/quote item, indent it one level (insert `indent`
/// at the line start), returning the new text and shifted caret. `None` when the
/// line isn't a list item, so the caller can insert a literal tab instead.
pub fn indent_list_line(value: &str, cursor: usize, indent: &str) -> Option<(String, usize)> {
    let cursor = cursor.min(value.len());
    let line_start = value[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let line_end = value[cursor..]
        .find('\n')
        .map_or(value.len(), |i| cursor + i);
    let line = &value[line_start..line_end];
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    parse_list_marker(&line[indent_len..])?; // only list / quote lines
    let new = format!("{}{indent}{}", &value[..line_start], &value[line_start..]);
    Some((new, cursor + indent.len()))
}

/// Re-indent every space-indented list / quote item in `content` from `old`-space
/// nesting units to `new`-space units (e.g. when the list-indent setting changes),
/// so existing nesting matches the new width. Each item's level is its leading
/// spaces ÷ `old`. Non-list lines, top-level items, and tab-indented lines are
/// left untouched. `None` when nothing changes.
pub fn reindent(content: &str, old: usize, new: usize) -> Option<String> {
    if old == 0 || old == new {
        return None;
    }
    let mut changed = false;
    let out = content
        .split('\n')
        .map(|line| {
            let ws = line.len() - line.trim_start_matches(' ').len();
            if ws > 0 && parse_list_marker(&line[ws..]).is_some() {
                let new_ws = (ws / old) * new;
                if new_ws != ws {
                    changed = true;
                }
                format!("{}{}", " ".repeat(new_ws), &line[ws..])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    changed.then_some(out)
}

/// Outdent the caret's line one level: remove up to `indent`'s width of leading
/// spaces (or one leading tab). Returns the new text and caret, or `None` if the
/// line has no leading indent to remove.
pub fn outdent_line(value: &str, cursor: usize, indent: &str) -> Option<(String, usize)> {
    let cursor = cursor.min(value.len());
    let line_start = value[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let line = &value[line_start..];
    let removed = if line.starts_with('\t') {
        1
    } else {
        line.bytes()
            .take(indent.len().max(1))
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

    #[test]
    fn parse_cache_hits_and_evicts() {
        // Same source → the same cached tree (pointer-equal Arc).
        let a1 = parse_cached("# cache me\n\n- item").unwrap();
        let a2 = parse_cached("# cache me\n\n- item").unwrap();
        assert!(Arc::ptr_eq(&a1, &a2));
        // Changed content is a different key — never a stale tree.
        let b = parse_cached("# cache me\n\n- item edited").unwrap();
        assert!(!Arc::ptr_eq(&a1, &b));
        // Filling past the cap evicts the least-recently used, not the
        // just-touched entry.
        let keep = parse_cached("keep me").unwrap();
        for i in 0..PARSE_CACHE_CAP {
            let _ = parse_cached(&format!("filler {i}"));
            let again = parse_cached("keep me").unwrap(); // stays hot
            assert!(Arc::ptr_eq(&keep, &again));
        }
        // And math stays enabled through the cached options.
        let math = parse_cached("$$\nx^2\n$$").unwrap();
        let mdast::Node::Root(root) = &*math else {
            panic!("root")
        };
        assert!(matches!(root.children.first(), Some(mdast::Node::Math(_))));
    }

    #[test]
    fn alert_marker_and_strip() {
        assert!(matches!(
            alert_marker("[!NOTE]\nbody"),
            Some((AlertKind::Note, 8))
        ));
        assert!(matches!(
            alert_marker("[!WARNING]"),
            Some((AlertKind::Warning, 10))
        ));
        // Lenient form: body text on the marker line (strip includes the space).
        assert!(matches!(
            alert_marker("[!NOTE] trailing"),
            Some((AlertKind::Note, 8))
        ));
        // Wrong case / glued text → plain blockquote.
        assert!(alert_marker("[!note]\nbody").is_none());
        assert!(alert_marker("[!NOTEXT]").is_none());

        // End-to-end through mdast: the marker strips, its text's source
        // offset advances, and the body survives.
        let ast =
            markdown::to_mdast("> [!TIP]\n> body here", &markdown::ParseOptions::gfm()).unwrap();
        let mdast::Node::Root(root) = &ast else {
            panic!("no root")
        };
        let mdast::Node::Blockquote(b) = &root.children[0] else {
            panic!("no blockquote")
        };
        let (kind, children) = alert_children(b).expect("alert detected");
        assert_eq!(kind.label(), "Tip");
        let mdast::Node::Paragraph(p) = &children[0] else {
            panic!("no paragraph")
        };
        let mdast::Node::Text(t) = &p.children[0] else {
            panic!("no text")
        };
        assert_eq!(t.value, "body here");
        // Source offset advanced past "[!TIP]\n" (starts at "> " = 2, +7).
        assert_eq!(t.position.as_ref().unwrap().start.offset, 9);
    }

    fn cont(s: &str) -> Option<ListEdit> {
        list_continuation(s, s.len())
    }

    #[test]
    fn reindent_converts_list_widths() {
        let content = "- a\n    - b\n        - c\nplain\n    not a list";
        // 4 → 2 spaces/level: nested list items shrink; non-list + top-level stay.
        assert_eq!(
            reindent(content, 4, 2).unwrap(),
            "- a\n  - b\n    - c\nplain\n    not a list"
        );
        // 4 → 8: nested items grow.
        assert_eq!(reindent("    - b", 4, 8).unwrap(), "        - b");
        // No-op when unchanged or nothing nested.
        assert!(reindent(content, 4, 4).is_none());
        assert!(reindent("- a\n- b", 4, 8).is_none());
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
        assert_eq!(indent_list_line("- a", 3, "  "), Some(("  - a".into(), 5)));
        assert_eq!(indent_list_line("* x", 1, "  "), Some(("  * x".into(), 3)));
        assert_eq!(
            indent_list_line("1. y", 4, "  "),
            Some(("  1. y".into(), 6))
        );
        // The indent unit is the caller's: a 4-space setting inserts four.
        assert_eq!(
            indent_list_line("- a", 3, "    "),
            Some(("    - a".into(), 7))
        );
        // Only the caret's line is indented.
        assert_eq!(
            indent_list_line("- a\n- b", 7, "  "),
            Some(("- a\n  - b".into(), 9))
        );
    }

    #[test]
    fn tab_ignores_non_list_lines() {
        assert_eq!(indent_list_line("hello", 5, "  "), None);
    }

    #[test]
    fn shift_tab_outdents() {
        assert_eq!(outdent_line("  - a", 5, "  "), Some(("- a".into(), 3)));
        assert_eq!(outdent_line("    x", 5, "  "), Some(("  x".into(), 3)));
        assert_eq!(outdent_line("- a", 3, "  "), None);
        // A 4-space unit removes up to four leading spaces in one outdent.
        assert_eq!(outdent_line("    - a", 7, "    "), Some(("- a".into(), 3)));
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
            None,
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
    fn mark_tag_highlights_text_other_html_stays_literal() {
        let html = |v: &str| {
            mdast::Node::Html(mdast::Html {
                value: v.into(),
                position: None,
            })
        };
        let text = |v: &str| {
            mdast::Node::Text(mdast::Text {
                value: v.into(),
                position: None,
            })
        };
        let style = MarkdownStyle::default();

        // `<mark>hi</mark>`: the tags are consumed and `hi` carries the mark background.
        let mut inl = Inline::default();
        build_inline(
            &[html("<mark>"), text("hi"), html("</mark>")],
            HighlightStyle::default(),
            &style,
            &HashMap::new(),
            None,
            &mut inl,
        );
        assert_eq!(inl.text, "hi");
        assert!(
            inl.highlights
                .iter()
                .any(|(r, s)| &inl.text[r.clone()] == "hi"
                    && s.background_color == Some(style.mark_bg)),
            "wrapped text should carry the mark background"
        );

        // Any other inline HTML is still shown literally.
        let mut inl2 = Inline::default();
        build_inline(
            &[html("<br>")],
            HighlightStyle::default(),
            &style,
            &HashMap::new(),
            None,
            &mut inl2,
        );
        assert_eq!(inl2.text, "<br>");
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

    #[test]
    fn images_enumerates_list_items() {
        // The real journal/page format: images live in bullet list items.
        let src = "- ![](a.jpg){width=2000}\n- ![](b.jpg)\n- text only\n- ![](c.jpg)";
        let imgs = images(src);
        assert_eq!(imgs.len(), 3); // the text-only item is skipped
        // The explicit width is parsed; its attr_target spans the {width=N}.
        assert_eq!(imgs[0].width, Some(2000.0));
        assert!(imgs[0].attr_target.start < imgs[0].attr_target.end);
        // A width-less image reports None and an empty (insertion-point) range
        // that sits right after its `)` — so inserting `{width=N}` there is valid.
        assert_eq!(imgs[1].width, None);
        assert_eq!(imgs[1].attr_target.start, imgs[1].attr_target.end);
        assert_eq!(
            &src[imgs[1].attr_target.start..imgs[1].attr_target.start + 1],
            "\n"
        );
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
            None,
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
            None,
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
                    None,
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
