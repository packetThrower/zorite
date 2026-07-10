//! In-app PDF export (tab right-click → "Export as PDF…"). `oxidize-pdf`
//! writes the file — pure Rust, and with default features off it adds only a
//! handful of small crates — while *we* own the layout: the same mdast the
//! reader renders is walked block by block, inline runs wrap greedily using
//! the crate's font metrics, and page breaks are ours. What the reader
//! renders, the PDF renders: `$$…$$` math rasterizes through RaTeX, mermaid
//! fences through mermaid-rs + resvg (the same resvg gpui draws SVG with),
//! and images of any decodable format embed via the `image` crate. The
//! standard PDF fonts (Helvetica / Courier) keep the file embed-free; they
//! cover WinAnsi text — emoji and CJK degrade, the known tradeoff (inline
//! `$…$` math also stays as source; only block math rasterizes).

use std::path::Path;

use markdown::mdast;
use oxidize_pdf::text::measure_text;
use oxidize_pdf::{Color, Document, Font, Image, Page};

// US Letter with 0.75" margins.
const PAGE_W: f64 = 612.0;
const PAGE_H: f64 = 792.0;
const MARGIN: f64 = 54.0;
const CONTENT_W: f64 = PAGE_W - 2.0 * MARGIN;
const BODY_SIZE: f64 = 11.0;
const LINE: f64 = 1.45;
/// Indent for quote/alert bodies and per list level.
const INDENT: f64 = 16.0;
/// Block math typesets at this size (pt) and oversampling for print crispness.
const MATH_PT: f64 = 13.0;
const MATH_DPR: f32 = 4.0;
/// Mermaid / SVG rasters oversample by this much (displayed at logical size).
const SVG_SCALE: f32 = 3.0;

/// Render markdown `source` to a PDF and write it to `out`. `title` becomes
/// the document title; `base_dir` resolves relative image refs (the data dir).
pub fn export_pdf(title: &str, source: &str, base_dir: &Path, out: &Path) -> Result<(), String> {
    // Enable `$$…$$` / `$…$` math like the reader does — plain gfm() leaves
    // math off, and a Math node that never parses can never typeset.
    let mut opts = markdown::ParseOptions::gfm();
    opts.constructs.math_flow = true;
    opts.constructs.math_text = true;
    let ast = markdown::to_mdast(source, &opts).map_err(|e| e.to_string())?;
    let mdast::Node::Root(root) = &ast else {
        return Err("no document root".into());
    };

    let mut pdf = Pdf::new(title);
    for node in &root.children {
        pdf.block(node, 0.0, base_dir);
    }
    let mut doc = pdf.finish();
    doc.save(out).map_err(|e| e.to_string())
}

/// Inline styling accumulated while flattening mdast inline nodes to runs.
#[derive(Clone, Copy, Default)]
struct Style {
    bold: bool,
    italic: bool,
    code: bool,
    color: Option<Color>,
}

impl Style {
    fn font(self) -> Font {
        match (self.code, self.bold, self.italic) {
            (true, true, _) => Font::CourierBold,
            (true, _, true) => Font::CourierOblique,
            (true, false, false) => Font::Courier,
            (false, true, true) => Font::HelveticaBoldOblique,
            (false, true, false) => Font::HelveticaBold,
            (false, false, true) => Font::HelveticaOblique,
            (false, false, false) => Font::Helvetica,
        }
    }
}

/// The document builder: current page, a top-down y cursor, and page breaks.
struct Pdf {
    doc: Document,
    page: Page,
    y: f64,
    /// Unique names for embedded images (the crate keys them by name).
    image_n: usize,
    /// Horizontal alignment (0 left, 0.5 center, 1 right) set by the last
    /// `<!-- math:ALIGN -->` marker, consumed by the formula that follows.
    /// Center absent a marker — the app's and LaTeX's default.
    math_align: f64,
}

impl Pdf {
    fn new(title: &str) -> Self {
        let mut doc = Document::new();
        doc.set_title(title);
        Self {
            doc,
            page: Page::new(PAGE_W, PAGE_H),
            y: PAGE_H - MARGIN,
            image_n: 0,
            math_align: 0.5,
        }
    }

    fn finish(mut self) -> Document {
        let page = std::mem::replace(&mut self.page, Page::new(PAGE_W, PAGE_H));
        self.doc.add_page(page);
        self.doc
    }

    /// Start a new page if `height` doesn't fit above the bottom margin.
    fn ensure(&mut self, height: f64) {
        if self.y - height < MARGIN {
            let full = std::mem::replace(&mut self.page, Page::new(PAGE_W, PAGE_H));
            self.doc.add_page(full);
            self.y = PAGE_H - MARGIN;
        }
    }

    fn gap(&mut self, points: f64) {
        self.y -= points;
    }

    /// Render one block node at `indent` (points past the left margin).
    fn block(&mut self, node: &mdast::Node, indent: f64, base_dir: &Path) {
        match node {
            mdast::Node::Heading(h) => {
                let size = BODY_SIZE * f64::from(heading_scale(h.depth));
                self.gap(size * 0.8);
                let mut runs = Vec::new();
                flatten(
                    &h.children,
                    Style {
                        bold: true,
                        ..Default::default()
                    },
                    &mut runs,
                );
                self.runs(&runs, size, indent);
                self.gap(size * 0.35);
            }
            mdast::Node::Paragraph(p) => {
                // A paragraph that IS one formula renders as display math —
                // the `<!-- math:left -->` marker + `$$` layout parses as a
                // lone InlineMath this way, not as a Math block.
                if let [mdast::Node::InlineMath(m)] = p.children.as_slice()
                    && let Some(png) =
                        ratex_gpui::render::render_latex_to_png(&m.value, MATH_PT as f32, MATH_DPR)
                {
                    let align = std::mem::replace(&mut self.math_align, 0.5);
                    self.png_aligned(png, MATH_DPR as f64, indent, align);
                    return;
                }
                // Standalone images render as images; everything else flows.
                let mut runs = Vec::new();
                for child in &p.children {
                    if let mdast::Node::Image(img) = child {
                        if !runs.is_empty() {
                            self.runs(&runs, BODY_SIZE, indent);
                            runs.clear();
                        }
                        self.image(&img.url, indent, base_dir);
                    } else {
                        flatten(std::slice::from_ref(child), Style::default(), &mut runs);
                    }
                }
                if !runs.is_empty() {
                    self.runs(&runs, BODY_SIZE, indent);
                }
                self.gap(BODY_SIZE * 0.6);
            }
            mdast::Node::List(list) => {
                let start = list.start.unwrap_or(1) as usize;
                for (i, item) in list.children.iter().enumerate() {
                    let mdast::Node::ListItem(li) = item else {
                        continue;
                    };
                    let marker = match li.checked {
                        Some(true) => "[x]".to_string(),
                        Some(false) => "[ ]".to_string(),
                        None if list.ordered => format!("{}.", start + i),
                        None => "\u{2022}".to_string(),
                    };
                    self.list_item(&marker, &li.children, indent, base_dir);
                }
                if indent == 0.0 {
                    self.gap(BODY_SIZE * 0.6);
                }
            }
            mdast::Node::Code(c) if c.lang.as_deref() == Some("mermaid") => {
                // Render the diagram like the reader (mermaid → SVG → resvg),
                // on mermaid's light default theme — print is light. The code
                // shows only if the diagram fails to render.
                match mermaid_png(&c.value) {
                    Some(png) => self.png(png, SVG_SCALE as f64, indent),
                    None => self.code_block(&c.value, indent),
                }
            }
            mdast::Node::Code(c) => self.code_block(&c.value, indent),
            mdast::Node::Blockquote(b) => self.quote(b, indent, base_dir),
            mdast::Node::ThematicBreak(_) => {
                self.ensure(BODY_SIZE);
                self.gap(BODY_SIZE * 0.5);
                self.page
                    .graphics()
                    .set_fill_color(Color::gray(0.7))
                    .rect(MARGIN + indent, self.y, CONTENT_W - indent, 0.7)
                    .fill();
                self.gap(BODY_SIZE * 0.9);
            }
            mdast::Node::Table(t) => self.table(t, indent),
            // Comments never print — they carry the app's control markers
            // (`<!-- math:left -->` alignment, table styles) and are invisible
            // in every view. Other raw HTML prints literally, like the reader.
            mdast::Node::Html(h) if h.value.trim_start().starts_with("<!--") => {
                // …but a math-alignment marker steers the formula below it.
                if let Some(inner) = h
                    .value
                    .trim()
                    .strip_prefix("<!--")
                    .and_then(|v| v.strip_suffix("-->"))
                {
                    self.math_align = match inner.trim() {
                        "math:left" => 0.0,
                        "math:right" => 1.0,
                        _ => self.math_align,
                    };
                }
            }
            mdast::Node::Html(h) => {
                let runs = vec![(h.value.clone(), Style::default())];
                self.runs(&runs, BODY_SIZE, indent);
                self.gap(BODY_SIZE * 0.6);
            }
            mdast::Node::Math(m) => {
                // Typeset like the reader (RaTeX → PNG); the LaTeX source in
                // monospace only if typesetting fails.
                let align = std::mem::replace(&mut self.math_align, 0.5);
                match ratex_gpui::render::render_latex_to_png(&m.value, MATH_PT as f32, MATH_DPR) {
                    Some(png) => self.png_aligned(png, MATH_DPR as f64, indent, align),
                    None => self.code_block(&format!("$$ {} $$", m.value), indent),
                }
            }
            mdast::Node::FootnoteDefinition(fd) => {
                let mut runs = vec![(
                    format!("[^{}]: ", fd.identifier),
                    Style {
                        bold: true,
                        ..Default::default()
                    },
                )];
                for child in &fd.children {
                    if let mdast::Node::Paragraph(p) = child {
                        flatten(&p.children, Style::default(), &mut runs);
                    }
                }
                self.runs(&runs, BODY_SIZE * 0.9, indent);
                self.gap(BODY_SIZE * 0.5);
            }
            _ => {}
        }
    }

    /// A list item: marker glyph in a gutter, block children indented past it.
    fn list_item(&mut self, marker: &str, children: &[mdast::Node], indent: f64, base_dir: &Path) {
        let gutter = measure_text("[x] ", &Font::Helvetica, BODY_SIZE).max(INDENT);
        self.ensure(BODY_SIZE * LINE);
        let _ = self
            .page
            .text()
            .set_font(Font::Helvetica, BODY_SIZE)
            .at(MARGIN + indent, self.y - BODY_SIZE)
            .write(marker);
        // The first paragraph starts on the marker's line; further blocks stack.
        let mut first = true;
        for child in children {
            if first && matches!(child, mdast::Node::Paragraph(_)) {
                let mdast::Node::Paragraph(p) = child else {
                    unreachable!()
                };
                let mut runs = Vec::new();
                flatten(&p.children, Style::default(), &mut runs);
                self.runs(&runs, BODY_SIZE, indent + gutter);
                self.gap(BODY_SIZE * 0.25);
            } else {
                self.block(child, indent + gutter, base_dir);
            }
            first = false;
        }
    }

    /// A blockquote — or, when its first text opens with a `[!NOTE]`-style
    /// marker, a GitHub alert with the colored bar + bold title the app shows.
    fn quote(&mut self, b: &mdast::Blockquote, indent: f64, base_dir: &Path) {
        let alert = alert_of(b);
        let bar_top = self.y;
        let body_indent = indent + INDENT;
        match &alert {
            Some((label, color, children)) => {
                let runs = vec![(
                    label.to_string(),
                    Style {
                        bold: true,
                        color: Some(*color),
                        ..Default::default()
                    },
                )];
                self.runs(&runs, BODY_SIZE, body_indent);
                self.gap(BODY_SIZE * 0.3);
                for child in children {
                    self.block(child, body_indent, base_dir);
                }
            }
            None => {
                for child in &b.children {
                    self.block(child, body_indent, base_dir);
                }
            }
        }
        // The bar paints after so its height matches the rendered body. On a
        // page break it stops at the margin — the continuation goes unbarred,
        // a v1 simplification.
        let color = alert
            .as_ref()
            .map(|(_, c, _)| *c)
            .unwrap_or(Color::gray(0.65));
        let bottom = (self.y + BODY_SIZE * 0.4).min(bar_top);
        if bar_top > bottom && bar_top <= PAGE_H - MARGIN {
            self.page
                .graphics()
                .set_fill_color(color)
                .rect(
                    MARGIN + indent + 4.0,
                    bottom.max(MARGIN),
                    1.5,
                    bar_top - bottom.max(MARGIN),
                )
                .fill();
        }
        self.gap(BODY_SIZE * 0.4);
    }

    /// A fenced code block: light background box, Courier lines.
    fn code_block(&mut self, value: &str, indent: f64) {
        let size = BODY_SIZE * 0.9;
        let lh = size * LINE;
        let lines: Vec<&str> = value.lines().collect();
        let pad = 6.0;
        // Background per page chunk: draw before each run of lines that fits.
        let mut i = 0;
        while i < lines.len() {
            self.ensure(lh + pad * 2.0);
            let fit = (((self.y - MARGIN) / lh).floor() as usize).clamp(1, lines.len() - i);
            let chunk_h = fit as f64 * lh + pad * 2.0;
            self.page
                .graphics()
                .set_fill_color(Color::gray(0.94))
                .rect(
                    MARGIN + indent,
                    self.y - chunk_h,
                    CONTENT_W - indent,
                    chunk_h,
                )
                .fill();
            self.gap(pad);
            for line in &lines[i..i + fit] {
                self.gap(lh);
                let _ = self
                    .page
                    .text()
                    .set_font(Font::Courier, size)
                    .at(MARGIN + indent + pad, self.y + (lh - size) / 2.0)
                    .write(line);
            }
            self.gap(pad);
            i += fit;
        }
        self.gap(BODY_SIZE * 0.6);
    }

    /// A GFM table: equal-width columns, bold header, wrapped cells, hairline
    /// row rules — the reader's minimal style.
    fn table(&mut self, t: &mdast::Table, indent: f64) {
        let ncols = t
            .children
            .iter()
            .filter_map(|r| match r {
                mdast::Node::TableRow(r) => Some(r.children.len()),
                _ => None,
            })
            .max()
            .unwrap_or(1)
            .max(1);
        let col_w = (CONTENT_W - indent) / ncols as f64;
        let pad = 4.0;
        for (ri, row) in t.children.iter().enumerate() {
            let mdast::Node::TableRow(row) = row else {
                continue;
            };
            // Wrap every cell first so the row height is the tallest cell.
            let style = Style {
                bold: ri == 0,
                ..Default::default()
            };
            let cells: Vec<Vec<Vec<(String, Style)>>> = (0..ncols)
                .map(|ci| {
                    let mut runs = Vec::new();
                    if let Some(mdast::Node::TableCell(c)) = row.children.get(ci) {
                        flatten(&c.children, style, &mut runs);
                    }
                    wrap(&runs, BODY_SIZE, col_w - pad * 2.0)
                })
                .collect();
            let nlines = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
            let row_h = nlines as f64 * BODY_SIZE * LINE + pad * 2.0;
            self.ensure(row_h);
            let top = self.y;
            for (ci, lines) in cells.iter().enumerate() {
                let x = MARGIN + indent + ci as f64 * col_w + pad;
                let mut y = top - pad - BODY_SIZE;
                for line in lines {
                    let mut lx = x;
                    for (text, st) in line {
                        let _ = self
                            .page
                            .text()
                            .set_font(st.font(), BODY_SIZE)
                            .at(lx, y)
                            .write(text);
                        lx += measure_text(text, &st.font(), BODY_SIZE);
                    }
                    y -= BODY_SIZE * LINE;
                }
            }
            self.y = top - row_h;
            // Hairline under each row (stronger under the header).
            let w = if ri == 0 { 1.0 } else { 0.5 };
            self.page
                .graphics()
                .set_fill_color(Color::gray(0.75))
                .rect(MARGIN + indent, self.y, CONTENT_W - indent, w)
                .fill();
        }
        self.gap(BODY_SIZE * 0.8);
    }

    /// A standalone image, scaled to the content width, page-broken whole.
    fn image(&mut self, url: &str, indent: f64, base_dir: &Path) {
        let path = if url.starts_with("http://") || url.starts_with("https://") {
            return; // remote images aren't fetched for export
        } else {
            let p = Path::new(url);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                base_dir.join(url)
            }
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_lowercase();
        let img = match ext.as_str() {
            // Native fast paths; everything else decodes through the `image`
            // crate (webp/gif/bmp/tiff — same coverage as the reader) or
            // rasterizes through resvg, then embeds as PNG.
            "jpg" | "jpeg" => Image::from_jpeg_file(&path),
            "png" => Image::from_png_file(&path),
            "svg" => {
                let Some((png, _, _)) = std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| svg_to_png(&bytes, SVG_SCALE))
                else {
                    return;
                };
                Image::from_png_data(png)
            }
            _ => {
                let Ok(decoded) = image::open(&path) else {
                    log::warn!("export: can't decode {}", path.display());
                    return;
                };
                let mut png = Vec::new();
                if decoded
                    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .is_err()
                {
                    return;
                }
                Image::from_png_data(png)
            }
        };
        let Ok(img) = img else {
            log::warn!("export: can't embed {}", path.display());
            return;
        };
        let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        let w = (CONTENT_W - indent).min(iw * 0.75); // ~96dpi -> pt
        let h = w * ih / iw;
        let h = h.min(PAGE_H - 2.0 * MARGIN);
        let w = h * iw / ih;
        self.ensure(h);
        self.image_n += 1;
        let name = format!("img{}", self.image_n);
        self.page.add_image(&name, img);
        let _ = self
            .page
            .draw_image(&name, MARGIN + indent, self.y - h, w, h);
        self.gap(h + BODY_SIZE * 0.5);
    }

    /// Wrap styled runs to the width left of `indent` and write the lines.
    fn runs(&mut self, runs: &[(String, Style)], size: f64, indent: f64) {
        let lines = wrap(runs, size, CONTENT_W - indent);
        let lh = size * LINE;
        for line in lines {
            self.ensure(lh);
            self.gap(lh);
            let mut x = MARGIN + indent;
            for (text, style) in line {
                let t = self.page.text();
                t.set_font(style.font(), size);
                if let Some(c) = style.color {
                    t.set_fill_color(c);
                } else {
                    t.set_fill_color(Color::gray(0.1));
                }
                let _ = t.at(x, self.y + (lh - size) / 2.0).write(&text);
                x += measure_text(&text, &style.font(), size);
            }
        }
    }
}

/// Greedy word-wrap of styled runs into lines of same-style fragments.
fn wrap(runs: &[(String, Style)], size: f64, width: f64) -> Vec<Vec<(String, Style)>> {
    let width = width.max(size); // degenerate columns still make progress
    let space = |st: &Style| measure_text(" ", &st.font(), size);
    let mut lines: Vec<Vec<(String, Style)>> = Vec::new();
    let mut line: Vec<(String, Style)> = Vec::new();
    let mut x = 0.0;
    let mut flush = |line: &mut Vec<(String, Style)>, x: &mut f64| {
        lines.push(std::mem::take(line));
        *x = 0.0;
    };
    for (text, style) in runs {
        for piece in text.split('\n') {
            for word in piece.split_whitespace() {
                let word = sanitize(word);
                let w = measure_text(&word, &style.font(), size);
                let lead = if x > 0.0 { space(style) } else { 0.0 };
                if x > 0.0 && x + lead + w > width {
                    flush(&mut line, &mut x);
                }
                // Append to the previous fragment when the style matches, so a
                // line is few text ops instead of one per word.
                let sep = if x > 0.0 { " " } else { "" };
                match line.last_mut() {
                    Some((t, st)) if same(st, style) => t.push_str(&format!("{sep}{word}")),
                    _ => line.push((format!("{sep}{word}"), *style)),
                }
                x += lead + w;
            }
            if piece != text.split('\n').next_back().unwrap_or_default() || text.contains('\n') {
                // An explicit break inside the run value (soft break in mdast).
                if !line.is_empty() {
                    flush(&mut line, &mut x);
                }
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn same(a: &Style, b: &Style) -> bool {
    a.bold == b.bold && a.italic == b.italic && a.code == b.code && color_eq(a.color, b.color)
}

fn color_eq(a: Option<Color>, b: Option<Color>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => format!("{x:?}") == format!("{y:?}"),
        _ => false,
    }
}

/// The standard fonts cover WinAnsi — swap anything beyond it for a plain
/// mark so words never vanish or garble (emoji/CJK are the known tradeoff).
fn sanitize(word: &str) -> String {
    word.chars()
        .map(|c| {
            if (c as u32) < 0x2000 {
                c
            } else {
                match c {
                    '\u{2018}' | '\u{2019}' => '\'',
                    '\u{201C}' | '\u{201D}' => '"',
                    '\u{2013}' | '\u{2014}' => '-',
                    '\u{2022}' => '\u{2022}',
                    '\u{2026}' => '.',
                    _ => '\u{00BF}', // ¿ — visible placeholder, WinAnsi-safe
                }
            }
        })
        .collect()
}

/// Flatten inline nodes to styled runs (the PDF twin of the reader's
/// `build_inline`); block-irrelevant inlines print as their literal source.
fn flatten(nodes: &[mdast::Node], style: Style, out: &mut Vec<(String, Style)>) {
    for node in nodes {
        match node {
            mdast::Node::Text(t) => out.push((t.value.clone(), style)),
            mdast::Node::Strong(s) => flatten(
                &s.children,
                Style {
                    bold: true,
                    ..style
                },
                out,
            ),
            mdast::Node::Emphasis(e) => flatten(
                &e.children,
                Style {
                    italic: true,
                    ..style
                },
                out,
            ),
            mdast::Node::Delete(d) => flatten(&d.children, style, out),
            mdast::Node::InlineCode(c) => out.push((
                c.value.clone(),
                Style {
                    code: true,
                    ..style
                },
            )),
            mdast::Node::InlineMath(m) => out.push((
                format!("${}$", m.value),
                Style {
                    code: true,
                    ..style
                },
            )),
            mdast::Node::Link(l) => {
                let mut inner = Vec::new();
                flatten(
                    &l.children,
                    Style {
                        color: Some(Color::rgb(0.04, 0.41, 0.85)),
                        ..style
                    },
                    &mut inner,
                );
                out.extend(inner);
            }
            // Inline (non-standalone) images print as their alt text.
            mdast::Node::Image(img) if !img.alt.is_empty() => {
                out.push((format!("[{}]", img.alt), style));
            }
            mdast::Node::Break(_) => out.push(("\n".to_string(), style)),
            mdast::Node::FootnoteReference(r) => out.push((format!("[^{}]", r.identifier), style)),
            // `<mark>` tags vanish (the highlight has no PDF twin here) and
            // comments never print; other raw HTML prints literally.
            mdast::Node::Html(h)
                if h.value != "<mark>"
                    && h.value != "</mark>"
                    && !h.value.trim_start().starts_with("<!--") =>
            {
                out.push((h.value.clone(), style));
            }
            _ => {}
        }
    }
}

/// GitHub alert detection — recognition shared with both views
/// (`gpui_markdown::alert_children`); only the print palette (GitHub light,
/// since print is light) is this renderer's own.
fn alert_of(b: &mdast::Blockquote) -> Option<(&'static str, Color, Vec<mdast::Node>)> {
    use gpui_markdown::syntax::AlertKind;
    let (kind, children) = gpui_markdown::alert_children(b)?;
    let (r, g, bl) = match kind {
        AlertKind::Note => (0.04, 0.41, 0.85),
        AlertKind::Tip => (0.10, 0.50, 0.22),
        AlertKind::Important => (0.51, 0.31, 0.87),
        AlertKind::Warning => (0.60, 0.40, 0.00),
        AlertKind::Caution => (0.81, 0.13, 0.18),
    };
    Some((kind.label(), Color::rgb(r, g, bl), children))
}

impl Pdf {
    /// Embed pre-rendered PNG bytes at `1/oversample` of their pixel size
    /// (96dpi px → pt), capped to the content width.
    fn png(&mut self, png: Vec<u8>, oversample: f64, indent: f64) {
        self.png_aligned(png, oversample, indent, 0.0);
    }

    /// Like [`png`](Self::png), placed at `align` across the content width
    /// (0 left, 0.5 center, 1 right) — display math carries an alignment.
    fn png_aligned(&mut self, png: Vec<u8>, oversample: f64, indent: f64, align: f64) {
        let Ok(img) = Image::from_png_data(png) else {
            return;
        };
        let (iw, ih) = (f64::from(img.width()), f64::from(img.height()));
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        let w = (iw / oversample * 0.75).min(CONTENT_W - indent);
        let h = w * ih / iw;
        let h = h.min(PAGE_H - 2.0 * MARGIN);
        let w = h * iw / ih;
        self.ensure(h);
        self.image_n += 1;
        let name = format!("img{}", self.image_n);
        self.page.add_image(&name, img);
        let x = MARGIN + indent + (CONTENT_W - indent - w) * align;
        let _ = self.page.draw_image(&name, x, self.y - h, w, h);
        self.gap(h + BODY_SIZE * 0.5);
    }
}

/// Rasterize SVG bytes at `scale`× via resvg (the renderer gpui itself draws
/// SVG with) → `(png, logical width pt, height pt)`.
fn svg_to_png(bytes: &[u8], scale: f32) -> Option<(Vec<u8>, f64, f64)> {
    let mut opts = resvg::usvg::Options::default();
    // Diagram/user SVGs carry <text>; give resvg the system faces.
    opts.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(bytes, &opts).ok()?;
    let size = tree.size();
    let (w, h) = (size.width() * scale, size.height() * scale);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w.ceil() as u32, h.ceil() as u32)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png().ok()?;
    Some((
        png,
        f64::from(size.width()) * 0.75,
        f64::from(size.height()) * 0.75,
    ))
}

/// A mermaid fence → PNG, on mermaid's light default theme (print is light).
fn mermaid_png(source: &str) -> Option<Vec<u8>> {
    let options = mermaid_rs_renderer::RenderOptions::default();
    let svg = mermaid_rs_renderer::render_with_options(source, options).ok()?;
    svg_to_png(svg.as_bytes(), SVG_SCALE).map(|(png, _, _)| png)
}

use gpui_markdown::syntax::heading_scale;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_writes_a_pdf() {
        let dir = std::env::temp_dir().join("zorite-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("t.pdf");
        export_pdf(
            "Test",
            "# Hi\n\nSome **bold** and `code` text that should wrap when it gets long enough \
             to exceed the content width of a letter page with margins.\n\n\
             > [!NOTE] careful\n\n- a\n- [x] done\n\n```\nlet x = 1;\n```\n\n\
             | a | b |\n| - | - |\n| 1 | 2 |\n\n---\n\n             <!-- math:left -->\n$$\\sqrt{x^2 + 1}$$\n\n             ```mermaid\nflowchart LR\n  A --> B\n```\n",
            Path::new("/tmp"),
            &out,
        )
        .unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        // Rasterized math + mermaid make it a real multi-KB document.
        assert!(bytes.len() > 7000, "len {}", bytes.len());

        // Math alone must rasterize (not print as source): an image-bearing
        // PDF is far larger than the same doc with the formula as text.
        let math_out = dir.join("m.pdf");
        export_pdf("M", "$$\\sqrt{x^2+1}$$\n", Path::new("/tmp"), &math_out).unwrap();
        let math_bytes = std::fs::read(&math_out).unwrap();
        assert!(math_bytes.len() > 4000, "math len {}", math_bytes.len());
        let _ = std::fs::remove_file(math_out);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn wrap_breaks_long_lines_and_merges_styles() {
        let runs = vec![
            ("one two".to_string(), Style::default()),
            (
                "three".to_string(),
                Style {
                    bold: true,
                    ..Default::default()
                },
            ),
        ];
        let lines = wrap(&runs, 11.0, 500.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 2); // plain merged, bold separate
        let narrow = wrap(&runs, 11.0, 30.0);
        assert!(narrow.len() >= 2);
    }
}
