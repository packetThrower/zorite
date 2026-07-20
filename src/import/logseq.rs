//! The Logseq reader: turns a graph folder (`pages/`, `journals/`,
//! `assets/`) into an [`ImportBundle`] for [`super::write_bundle`]. Pure
//! filesystem + string work — no database access. The interesting part is
//! translating Logseq's conventions into Zorite's:
//!
//! - **Namespaces** — `Budget___2024.md` files and `[[Budget/2024]]` links
//!   both become Zorite's `Budget::2024`.
//! - **Outliner** — Logseq makes every line a bullet. `Options::flatten`
//!   turns top-level blocks into paragraphs/headings (children stay nested
//!   lists); otherwise every block stays a list item.
//! - **Tasks** — `TODO`/`DOING`/… → `- [ ]`, `DONE` → `- [x]`,
//!   `CANCELED` → struck-through `- [x]`.
//! - **Properties** — Logseq-internal metadata (`id::`, `collapsed::`,
//!   `query-table::`, …) is dropped; `title::`/`alias::` feed the page title
//!   and Zorite's alias table; anything else (`subject::`, `attendees::`, …)
//!   is kept as plain text.
//! - **Macros** — `{{video url}}` → the url, `{{embed [[X]]}}` → `[[X]]`,
//!   `((block-ref))` → the referenced block's text; queries and unknown
//!   macros are kept visible as inline code.
//! - **Assets** — image links are rewritten to `images/<name>` and PDFs to
//!   `[[pdf/<name>]]` chips, with the files queued for copying.
//! - **PDF highlights** — Logseq's `hls__*` pages become Zorite's
//!   `<name>.pdf (highlights)` pages (`- p<N>: quote [[pdf/…#pN|↗]]`).
//!
//! Whiteboards, draws, `logseq/` config, and `bak`/`.recycle` folders are
//! skipped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;

use gpui_whiteboard::{
    BoxGeom, Element, ElementKind, ImageGeom, Scene, SegGeom, SegmentStyle, Stroke, TextGeom,
};

use super::edn::{self, Edn};
use super::{AssetBytes, AssetCopy, ImportBundle, ImportDay, ImportPage, ImportWhiteboard};

/// Importer choices the user makes up front.
pub struct Options {
    /// Convert top-level blocks to paragraphs/headings (children become
    /// markdown lists). `false` keeps every block a list item, Logseq-style.
    pub flatten: bool,
}

// --- Scanning ---

/// What a source file becomes in Zorite.
enum Kind {
    Page(String),
    Journal(String),
    /// A Logseq `hls__*` PDF-highlight page.
    Highlights,
}

struct SourceFile {
    path: PathBuf,
    kind: Kind,
}

/// Find the importable markdown files. Only `pages/` and `journals/` matter —
/// config, whiteboards, draws, and Logseq's `bak`/`.recycle` are skipped.
fn scan(root: &Path) -> Result<Vec<SourceFile>, String> {
    let pages_dir = root.join("pages");
    let journals_dir = root.join("journals");
    if !pages_dir.is_dir() && !journals_dir.is_dir() {
        return Err(format!(
            "{} doesn't look like a Logseq graph (no pages/ or journals/ folder)",
            root.display()
        ));
    }
    let mut files = Vec::new();
    for dir in [&journals_dir, &pages_dir] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        paths.sort();
        for path in paths {
            let stem = match path.file_stem() {
                Some(s) => s.to_string_lossy().into_owned(),
                None => continue,
            };
            let kind = if *dir == journals_dir {
                match journal_date(&stem) {
                    Some(date) => Kind::Journal(date),
                    // An oddly-named journal file still imports, as a page.
                    None => Kind::Page(title_from_stem(&stem)),
                }
            } else if stem.starts_with("hls__") {
                Kind::Highlights
            } else {
                Kind::Page(title_from_stem(&stem))
            };
            files.push(SourceFile { path, kind });
        }
    }
    Ok(files)
}

/// `2024_02_07` → `2024-02-07`, or `None` if it isn't a Logseq journal name.
fn journal_date(stem: &str) -> Option<String> {
    let parts: Vec<&str> = stem.split('_').collect();
    let [y, m, d] = parts.as_slice() else {
        return None;
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    if !parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let (month, day) = (m.parse::<u32>().ok()?, d.parse::<u32>().ok()?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{y}-{m}-{d}"))
}

/// File stem → Zorite page title: percent-decode (Logseq encodes special
/// characters, sometimes repeatedly) and turn `___` namespaces into `::`.
fn title_from_stem(stem: &str) -> String {
    let decoded = percent_decode_repeated(stem);
    decoded
        .split("___")
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("::")
}

/// Decode `%XX` escapes, repeating while it keeps changing (Logseq filenames
/// in the wild are sometimes encoded more than once), capped to stay sane.
fn percent_decode_repeated(s: &str) -> String {
    let mut cur = s.to_string();
    for _ in 0..3 {
        let next = percent_decode(&cur);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A Logseq page name → Zorite title: `/` namespaces become `::`, matching how
/// `[[a/b]]` links and `a___b.md` filenames are converted.
fn name_to_title(name: &str) -> String {
    name.split('/')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("::")
}

/// The favorited page names from `logseq/config.edn` (an EDN `:favorites ["A"
/// "B/C" …]` vector), as Zorite titles. Empty if the file or key is absent. A
/// favorite that wasn't imported (e.g. a whiteboard) simply won't match a page
/// later and is skipped by the writer.
fn read_favorites(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("logseq").join("config.edn"))
        .map(|raw| parse_favorites(&raw))
        .unwrap_or_default()
}

/// Parse the `:favorites ["A" "B/C" …]` vector out of a `config.edn` string,
/// returning the names as Zorite titles. Empty if the key is absent.
fn parse_favorites(edn: &str) -> Vec<String> {
    edn::parse(edn)
        .as_ref()
        .and_then(|e| e.get("favorites"))
        .and_then(Edn::as_seq)
        .map(|items| {
            items
                .iter()
                .filter_map(Edn::as_str)
                .map(name_to_title)
                .collect()
        })
        .unwrap_or_default()
}

// --- Whiteboards (best-effort tldraw-EDN → Zorite scene) ---

/// Convert each `whiteboards/*.edn` (Logseq's tldraw format) into a Zorite
/// whiteboard, best-effort: text / box / ellipse / line / freehand shapes map to
/// Zorite elements; images, embeds, and unknown shapes are skipped (and counted
/// in a per-board warning). Empty or unreadable files are skipped.
fn read_whiteboards(
    root: &Path,
    warnings: &mut Vec<String>,
    images: &mut Vec<AssetBytes>,
) -> Vec<ImportWhiteboard> {
    let Ok(entries) = std::fs::read_dir(root.join("whiteboards")) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "edn"))
        .collect();
    paths.sort();

    let mut out = Vec::new();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(parsed) = edn::parse(&text) else {
            warnings.push(format!("whiteboard {}: unreadable", path.display()));
            continue;
        };
        let Some(blocks) = parsed.get("blocks").and_then(Edn::as_seq) else {
            warnings.push(format!("whiteboard {}: unreadable", path.display()));
            continue;
        };

        // Asset registry: each page's `:logseq.tldraw.page {:assets [{:id … :src
        // "data:…"}]}` → assetId → data URI (merged across the file's pages).
        let assets: HashMap<&str, &str> = parsed
            .get("pages")
            .and_then(Edn::as_seq)
            .into_iter()
            .flatten()
            .filter_map(|page| {
                page.get("block/properties")?
                    .get("logseq.tldraw.page")?
                    .get("assets")?
                    .as_seq()
            })
            .flatten()
            .filter_map(|a| Some((a.get("id")?.as_str()?, a.get("src")?.as_str()?)))
            .collect();

        // Name: the whiteboard-page block's original name, else the filename stem.
        let title = blocks
            .iter()
            .find_map(|b| {
                let is_page = b.get("block/properties").and_then(|p| p.get("ls-type"))
                    == Some(&Edn::Keyword("whiteboard-page".into()));
                is_page
                    .then(|| b.get("block/original-name").and_then(Edn::as_str))
                    .flatten()
            })
            .map(str::to_string)
            .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Imported Whiteboard".to_string());

        let mut elements = Vec::new();
        let mut skipped = 0usize;
        for b in blocks {
            let Some(shape) = b
                .get("block/properties")
                .and_then(|p| p.get("logseq.tldraw.shape"))
            else {
                continue;
            };
            let id = elements.len() as u64 + 1;
            let els = if shape.get("type").and_then(Edn::as_str) == Some("image") {
                image_element(shape, id, &assets, images)
            } else {
                shape_to_element(shape, id)
            };
            if els.is_empty() {
                skipped += 1;
            } else {
                elements.extend(els);
            }
        }
        if skipped > 0 {
            warnings.push(format!(
                "whiteboard \"{title}\": {skipped} shape(s) skipped (embeds / portals / unsupported)"
            ));
        }
        let scene = Scene {
            elements,
            ..Default::default()
        };
        out.push(ImportWhiteboard {
            title,
            scene_json: scene.to_json(),
        });
    }
    out
}

/// A tldraw `image` shape → a Zorite image element, decoding its embedded asset
/// into `images` (deduped by destination). Empty if the asset is missing or not
/// an inline `data:` image (e.g. a remote URL).
fn image_element(
    shape: &Edn,
    id: u64,
    assets: &HashMap<&str, &str>,
    images: &mut Vec<AssetBytes>,
) -> Vec<Element> {
    let Some(asset_id) = shape.get("assetId").and_then(Edn::as_str) else {
        return Vec::new();
    };
    let Some((name, bytes)) = assets
        .get(asset_id)
        .and_then(|src| decode_data_uri(asset_id, src))
    else {
        return Vec::new();
    };
    let managed = format!("images/{name}");
    if !images.iter().any(|a| a.managed == managed) {
        images.push(AssetBytes {
            bytes,
            managed: managed.clone(),
        });
    }
    let (px, py) = xy(shape.get("point"));
    let (w, h) = xy(shape.get("size"));
    vec![Element {
        id,
        kind: ElementKind::Image(ImageGeom {
            src: managed,
            x: px as f32,
            y: py as f32,
            w: w as f32,
            h: h as f32,
            rotation: 0.0,
        }),
        stroke: None,
        fill: None,
        label: None,
        label_color: None,
        styles: Vec::new(),
        mindmap: None,
    }]
}

/// Decode a `data:image/<ext>;base64,<data>` URI into `(filename, bytes)`, naming
/// the file `wb-<asset_id>.<ext>`. `None` for a non-data / non-raster / undecodable
/// URI (the shape is then skipped).
fn decode_data_uri(asset_id: &str, src: &str) -> Option<(String, Vec<u8>)> {
    let (mime, data) = src.strip_prefix("data:image/")?.split_once(";base64,")?;
    let ext = match mime {
        "png" | "gif" | "webp" | "bmp" => mime,
        "jpeg" | "jpg" => "jpg",
        _ => return None, // svg / unknown → host image decode is raster-only
    };
    // Tolerate any whitespace the base64 payload may carry across EDN lines.
    let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .ok()?;
    Some((format!("wb-{asset_id}.{ext}"), bytes))
}

/// `[x y]` from an EDN vector (missing entries default to 0).
fn xy(edn: Option<&Edn>) -> (f64, f64) {
    let s = edn.and_then(Edn::as_seq).unwrap_or(&[]);
    let n = |i: usize| s.get(i).and_then(Edn::as_f64).unwrap_or(0.0);
    (n(0), n(1))
}

/// One tldraw shape → Zorite [`Element`]s, ids starting at `id`. Usually one,
/// but a labeled box/ellipse also yields a centered text element (tldraw keeps a
/// box's text in `:label` on the shape, not as a separate text shape). Empty for
/// an unsupported type or a degenerate/empty shape.
fn shape_to_element(shape: &Edn, id: u64) -> Vec<Element> {
    let Some(ty) = shape.get("type").and_then(Edn::as_str) else {
        return Vec::new();
    };
    let (px, py) = xy(shape.get("point"));
    let (w, h) = xy(shape.get("size"));
    let stroke_w = shape
        .get("strokeWidth")
        .and_then(Edn::as_f64)
        .unwrap_or(2.0) as f32;
    let stroke = shape
        .get("stroke")
        .and_then(Edn::as_str)
        .and_then(parse_color);
    let fill = shape
        .get("fill")
        .and_then(Edn::as_str)
        .and_then(parse_color);
    let font_size = shape.get("fontSize").and_then(Edn::as_f64).unwrap_or(20.0) as f32;

    let kind = match ty {
        "text" => {
            let content = shape.get("text").and_then(Edn::as_str).unwrap_or("").trim();
            if content.is_empty() {
                return Vec::new();
            }
            ElementKind::Text(text_geom(px as f32, py as f32, content, font_size))
        }
        "box" => ElementKind::Rect(box_geom(px, py, w, h, stroke_w)),
        "ellipse" | "circle" => ElementKind::Ellipse(box_geom(px, py, w, h, stroke_w)),
        "line" => {
            let (x1, y1, x2, y2) = line_endpoints(shape, px, py, w, h);
            line_or_arrow(shape, x1, y1, x2, y2, stroke_w)
        }
        "highlighter" | "pencil" | "draw" => {
            let points = freehand_points(shape, px, py);
            if points.len() < 2 {
                return Vec::new();
            }
            ElementKind::Draw(Stroke {
                points,
                width: stroke_w,
            })
        }
        _ => return Vec::new(), // image / iframe / youtube / …
    };
    // Closed shapes carry a fill; everything else just an ink/stroke color.
    let closed = matches!(ty, "box" | "ellipse" | "circle");
    let mut out = vec![Element {
        id,
        kind,
        stroke,
        fill: closed.then_some(fill).flatten(),
        label: None,
        label_color: None,
        styles: Vec::new(),
        mindmap: None,
    }];
    // A box/ellipse keeps its text in `:label` → the shape's native label, which
    // the renderer centers and auto-shrinks to fit inside the outline.
    if closed
        && let Some(label) = shape.get("label").and_then(Edn::as_str)
        && !label.trim().is_empty()
    {
        out[0].label = Some(label.trim().to_string());
    }
    out
}

/// A [`TextGeom`] at a top-left anchor; `measured_*` start unmeasured (the render
/// font fills them in).
fn text_geom(x: f32, y: f32, content: &str, size: f32) -> TextGeom {
    TextGeom {
        x,
        y,
        content: content.to_string(),
        size,
        rotation: 0.0,
        measured_w: 0.0,
        measured_h: 0.0,
    }
}

fn box_geom(x: f64, y: f64, w: f64, h: f64, width: f32) -> BoxGeom {
    BoxGeom {
        x: x as f32,
        y: y as f32,
        w: w as f32,
        h: h as f32,
        width,
        rotation: 0.0,
    }
}

/// A tldraw `line` becomes a Zorite arrow when it carries an arrowhead
/// (`:decorations {:end "arrow"}` / `{:start "arrow"}`), else a plain line.
/// Zorite paints the head at the segment's *end*, so a start-only arrow is
/// emitted with its endpoints swapped; a double-headed line keeps the end head.
fn line_or_arrow(shape: &Edn, x1: f32, y1: f32, x2: f32, y2: f32, width: f32) -> ElementKind {
    let arrow_at = |name| {
        shape
            .get("decorations")
            .and_then(|d| d.get(name))
            .and_then(Edn::as_str)
            == Some("arrow")
    };
    let seg = |x1, y1, x2, y2| SegGeom {
        x1,
        y1,
        x2,
        y2,
        width,
        style: SegmentStyle::Solid,
        start_anchor: None,
        end_anchor: None,
    };
    if arrow_at("end") {
        ElementKind::Arrow(seg(x1, y1, x2, y2))
    } else if arrow_at("start") {
        ElementKind::Arrow(seg(x2, y2, x1, y1))
    } else {
        ElementKind::Line(seg(x1, y1, x2, y2))
    }
}

/// A line's world endpoints: tldraw `:handles {:start … :end …}` points are
/// relative to the shape origin; without handles, fall back to the box diagonal.
fn line_endpoints(shape: &Edn, px: f64, py: f64, w: f64, h: f64) -> (f32, f32, f32, f32) {
    if let Some(handles) = shape.get("handles") {
        let pt = |name| xy(handles.get(name).and_then(|hp| hp.get("point")));
        let (sx, sy) = pt("start");
        let (ex, ey) = pt("end");
        if handles.get("start").is_some() && handles.get("end").is_some() {
            return (
                (px + sx) as f32,
                (py + sy) as f32,
                (px + ex) as f32,
                (py + ey) as f32,
            );
        }
    }
    (px as f32, py as f32, (px + w) as f32, (py + h) as f32)
}

/// Freehand `:points` (each `[x y …]`, relative to the shape origin) → absolute
/// Zorite stroke points.
fn freehand_points(shape: &Edn, px: f64, py: f64) -> Vec<[f32; 2]> {
    shape
        .get("points")
        .and_then(Edn::as_seq)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| {
            let (x, y) = xy(Some(p));
            p.as_seq()
                .filter(|s| s.len() >= 2)
                .map(|_| [(px + x) as f32, (py + y) as f32])
        })
        .collect()
}

/// A Logseq/CSS color → Zorite packed `0xRRGGBBAA`. Handles `#rgb` / `#rrggbb` /
/// `#rrggbbaa`, `var(--x, #hex)` (uses the fallback), and a few names. Anything
/// else (empty, a var with no fallback, an unknown name) → `None` (theme ink /
/// unfilled).
fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("var(") {
        return rest
            .split(',')
            .nth(1)
            .and_then(|f| parse_color(f.trim().trim_end_matches(')').trim()));
    }
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    match s {
        "black" => Some(0x0000_00ff),
        "white" => Some(0xffff_ffff),
        "gray" | "grey" => Some(0x8080_80ff),
        "red" => Some(0xff00_00ff),
        "green" => Some(0x00ff_00ff),
        "blue" => Some(0x0000_ffff),
        _ => None,
    }
}

fn parse_hex(hex: &str) -> Option<u32> {
    let h = hex.trim();
    let b = |s: &str| u8::from_str_radix(s, 16).ok();
    match h.len() {
        3 => Some(u32::from_be_bytes([
            b(&h[0..1].repeat(2))?,
            b(&h[1..2].repeat(2))?,
            b(&h[2..3].repeat(2))?,
            0xff,
        ])),
        6 => Some(u32::from_be_bytes([
            b(&h[0..2])?,
            b(&h[2..4])?,
            b(&h[4..6])?,
            0xff,
        ])),
        8 => Some(u32::from_be_bytes([
            b(&h[0..2])?,
            b(&h[2..4])?,
            b(&h[4..6])?,
            b(&h[6..8])?,
        ])),
        _ => None,
    }
}

// --- Outline parsing ---

/// One Logseq block: its indent depth and raw lines (first line is the bullet
/// text, the rest are its continuation lines, prefix-stripped).
struct Block {
    depth: usize,
    lines: Vec<String>,
}

/// Parse Logseq's bullet outline. Lines before any bullet (the page-property
/// preamble some files have) become a depth-0 block.
fn parse_outline(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut in_preamble = false;
    for raw in text.lines() {
        let stripped = raw.trim_start_matches(['\t', ' ']);
        if stripped == "-" || stripped.starts_with("- ") {
            let prefix = &raw[..raw.len() - stripped.len()];
            let content = stripped.strip_prefix("- ").unwrap_or("").to_string();
            blocks.push(Block {
                depth: depth_of(prefix),
                lines: vec![content],
            });
            in_preamble = false;
        } else if let Some(last) = blocks.last_mut().filter(|_| !in_preamble) {
            last.lines.push(strip_continuation(raw, last.depth));
        } else if let Some(last) = blocks.last_mut() {
            last.lines.push(raw.to_string());
        } else {
            in_preamble = true;
            blocks.push(Block {
                depth: 0,
                lines: vec![raw.to_string()],
            });
        }
    }
    blocks
}

/// Indent depth of a bullet's whitespace prefix: a tab is one level, as is
/// each pair of spaces.
fn depth_of(prefix: &str) -> usize {
    let mut depth = 0;
    let mut spaces = 0;
    for c in prefix.chars() {
        match c {
            '\t' => {
                depth += 1;
                spaces = 0;
            }
            ' ' => {
                spaces += 1;
                if spaces == 2 {
                    depth += 1;
                    spaces = 0;
                }
            }
            _ => {}
        }
    }
    depth
}

/// Strip a continuation line's prefix: the block's indent, then the two
/// spaces Logseq aligns continuations with. Anything beyond that (e.g. code
/// indentation inside a fence) is content and stays.
fn strip_continuation(raw: &str, depth: usize) -> String {
    let mut rest = raw;
    let mut stripped = 0;
    while stripped < depth {
        if let Some(r) = rest.strip_prefix('\t') {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("  ") {
            rest = r;
        } else {
            break;
        }
        stripped += 1;
    }
    let rest = rest
        .strip_prefix("  ")
        .or_else(|| rest.strip_prefix('\t'))
        .unwrap_or(rest);
    rest.to_string()
}

// --- Block conversion ---

/// A block after conversion: Zorite-markdown lines plus rendering flags.
struct ConvBlock {
    depth: usize,
    lines: Vec<String>,
    /// Logseq `logseq.order-list-type:: number` → render as `1.`-style item.
    numbered: bool,
    /// Starts with a task checkbox (`[ ] …`) — always rendered as a list item.
    task: bool,
}

/// Properties Logseq manages internally — dropped on import. Anything not
/// listed here is user data and is kept as a plain `key:: value` text line.
fn is_internal_prop(key: &str) -> bool {
    matches!(
        key,
        "id" | "collapsed"
            | "heading"
            | "icon"
            | "file"
            | "file-path"
            | "ls-type"
            | "hl-page"
            | "hl-color"
            | "hl-type"
            | "hl-stamp"
            | "background-color"
            | "background-image"
            | "created-at"
            | "updated-at"
            | "exclude-from-graph-view"
    )
}

use gpui_markdown::syntax::property as parse_prop;

/// The import-side identity of a block carrying an `id::` property.
struct BlockRef {
    /// The page title / journal date the block lands in — the `[[target#^id]]`
    /// link target. Empty when unknown (highlights pages): refs fall back to
    /// the inlined text.
    target: String,
    /// The block's first content line — the inline fallback.
    text: String,
}

/// Per-import conversion state: the block-ref map and pending asset copies.
struct Converter {
    /// `id:: <uuid>` → its block's identity, for `((ref))`s (issue #53:
    /// refs become real block LINKS, not inlined text).
    id_map: HashMap<String, BlockRef>,
    /// Ids some `((…))` references anywhere in the vault — their blocks get
    /// a ` ^short` anchor on import so the links have a destination.
    referenced: std::collections::HashSet<String>,
    /// Asset files to copy: (source path, managed ref like `images/x.png`).
    copies: Vec<(PathBuf, String)>,
    assets_dir: PathBuf,
    warnings: Vec<String>,
}

/// Page-level properties pulled out of a page's first block.
#[derive(Default)]
struct PageProps {
    title: Option<String>,
    aliases: Vec<String>,
}

impl Converter {
    fn new(root: &Path) -> Self {
        Self {
            id_map: HashMap::new(),
            referenced: std::collections::HashSet::new(),
            copies: Vec::new(),
            assets_dir: root.join("assets"),
            warnings: Vec::new(),
        }
    }

    /// Pass 1: collect `id::` properties so `((block-ref))`s can resolve.
    /// `target` is the page title / journal date the block will land in.
    fn collect_ids(&mut self, blocks: &[Block], target: Option<&str>) {
        for b in blocks {
            let Some(id) = b
                .lines
                .iter()
                .find_map(|l| parse_prop(l).and_then(|(k, v)| (k == "id").then(|| v.to_string())))
            else {
                continue;
            };
            let text = b
                .lines
                .iter()
                .find(|l| !l.trim().is_empty() && parse_prop(l).is_none())
                .map(|l| l.trim().to_string())
                .unwrap_or_default();
            self.id_map.insert(
                id,
                BlockRef {
                    target: target.unwrap_or_default().to_string(),
                    text,
                },
            );
        }
    }

    /// Pass 1: every `((id))` the raw text references, so pass 2 can anchor
    /// exactly the blocks that need it.
    fn collect_refs(&mut self, text: &str) {
        let mut rest = text;
        while let Some(p) = rest.find("((") {
            rest = &rest[p + 2..];
            if let Some(end) = rest.find("))") {
                self.referenced.insert(rest[..end].trim().to_string());
            }
        }
    }

    /// Convert one block's lines. `page_props` is `Some` for a page's first
    /// block, where Logseq keeps page-level properties.
    fn convert_block(
        &mut self,
        block: &Block,
        mut page_props: Option<&mut PageProps>,
    ) -> Option<ConvBlock> {
        let mut lines: Vec<String> = Vec::new();
        let mut numbered = false;
        let mut in_logbook = false;
        let mut in_fence = false;
        for raw in &block.lines {
            let t = raw.trim();
            if !in_fence {
                if t == ":LOGBOOK:" {
                    in_logbook = true;
                    continue;
                }
                if in_logbook {
                    if t == ":END:" {
                        in_logbook = false;
                    }
                    continue;
                }
                if let Some((key, value)) = parse_prop(raw) {
                    if key == "logseq.order-list-type" {
                        numbered = value == "number";
                        continue;
                    }
                    if is_internal_prop(key)
                        || key.starts_with("logseq.")
                        || key.starts_with("query-")
                        || key.starts_with("card-")
                    {
                        continue;
                    }
                    if let Some(props) = page_props.as_deref_mut() {
                        match key {
                            "title" => {
                                props.title = Some(name_to_title(value));
                                continue;
                            }
                            "alias" => {
                                props.aliases.extend(
                                    value
                                        .split(',')
                                        .map(|a| {
                                            a.trim().trim_matches(['[', ']']).trim().to_string()
                                        })
                                        .filter(|a| !a.is_empty()),
                                );
                                continue;
                            }
                            _ => {}
                        }
                    }
                    lines.push(format!("{key}:: {}", self.convert_inline(value)));
                    continue;
                }
            }
            // Logseq glues fences onto content (`- ```interface 2/1/44` to
            // open, `…3600``` ` to close), which markdown reads as an info
            // string / an unterminated fence. Normalize both onto their own
            // lines so the fence opens and closes cleanly.
            if !in_fence {
                if let Some(info) = t.strip_prefix("```") {
                    in_fence = !info.contains("```"); // ```x``` inline stays closed
                    if in_fence && info.contains(char::is_whitespace) {
                        lines.push("```".to_string());
                        lines.push(info.to_string());
                    } else {
                        lines.push(raw.clone());
                    }
                    continue;
                }
                lines.push(self.convert_inline(raw));
                continue;
            }
            if let Some(body) = t.strip_suffix("```").filter(|b| !b.is_empty()) {
                let indent = &raw[..raw.len() - raw.trim_start().len()];
                lines.push(format!("{indent}{body}"));
                lines.push("```".to_string());
                in_fence = false;
                continue;
            }
            if t.starts_with("```") {
                in_fence = false;
            }
            lines.push(raw.clone());
        }
        // Trim blank edges; a block with nothing left disappears.
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        while lines.first().is_some_and(|l| l.trim().is_empty()) {
            lines.remove(0);
        }
        if lines.is_empty() {
            return None;
        }
        // A block whose `id::` some `((ref))` targets gets a ` ^short` anchor
        // on its first plain content line, so the imported `[[page#^short]]`
        // links land (issue #53). Fence/table lines can't carry anchors — if
        // the block has no plain line, the link degrades to page navigation.
        if let Some(id) = block.lines.iter().find_map(|l| {
            parse_prop(l).and_then(|(k, v)| (k == "id").then(|| v.trim().to_string()))
        }) && self.referenced.contains(&id)
            && let Some(first) = lines.iter_mut().find(|l| {
                let t = l.trim_start();
                !t.is_empty()
                    && parse_prop(l).is_none()
                    && !t.starts_with('|')
                    && !t.starts_with("```")
            })
        {
            let anchor = format!(" ^{}", short_block_id(&id));
            if !first.ends_with(&anchor) {
                first.push_str(&anchor);
            }
        }
        let task = convert_task(&mut lines[0]);
        Some(ConvBlock {
            depth: block.depth,
            lines,
            numbered,
            task,
        })
    }

    /// All inline conversions for one line of prose (not inside a code fence).
    fn convert_inline(&mut self, line: &str) -> String {
        let line = convert_macros(line, &self.id_map);
        let line = convert_block_refs(&line, &self.id_map);
        let line = convert_wiki_links(&line);
        self.convert_assets(&line)
    }

    /// Rewrite `../assets/…` links: images → `images/<name>`, PDFs → a
    /// `[[pdf/<name>]]` chip, anything else a plain link into `images/`.
    /// Queues the copy for each referenced file.
    fn convert_assets(&mut self, line: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        loop {
            // Find the next markdown link destination that points into assets/.
            let Some((before, alt, path, after, bang)) = next_md_link(rest) else {
                out.push_str(rest);
                break;
            };
            let rel = path
                .strip_prefix("../assets/")
                .or_else(|| path.strip_prefix("assets/"));
            let Some(rel) = rel else {
                // Not an asset link — keep verbatim and continue after it.
                out.push_str(before);
                if bang {
                    out.push('!');
                }
                out.push_str(&format!("[{alt}]({path})"));
                rest = after;
                continue;
            };
            out.push_str(before);
            let decoded = percent_decode_repeated(rel);
            let name = sanitize_name(decoded.rsplit('/').next().unwrap_or(&decoded));
            let src = self.find_asset(rel, &decoded);
            let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            let is_image = matches!(
                ext.as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tiff" | "tif" | "svg"
            );
            let managed = if ext == "pdf" {
                format!("pdf/{name}")
            } else {
                format!("images/{name}")
            };
            match src {
                Some(src) => self.copies.push((src, managed.clone())),
                None => self.warnings.push(format!("asset not found: {rel}")),
            }
            if ext == "pdf" {
                out.push_str(&format!("[[{managed}]]"));
            } else if is_image {
                out.push_str(&format!("![{alt}]({managed})"));
            } else {
                let label = if alt.is_empty() { &name } else { &alt };
                out.push_str(&format!("[{label}]({managed})"));
            }
            rest = after;
        }
        out
    }

    /// Locate an asset on disk, trying the raw reference and progressively
    /// percent-decoded forms (Logseq stores some names encoded).
    fn find_asset(&self, raw: &str, decoded: &str) -> Option<PathBuf> {
        let mut candidates = vec![raw.to_string(), decoded.to_string()];
        candidates.push(percent_decode(raw));
        candidates.push(percent_decode(&percent_decode(raw)));
        for c in candidates {
            let p = self.assets_dir.join(&c);
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
}

/// `TODO …` → `[ ] …` etc. on a block's first line. Returns whether the
/// block is a task (so rendering makes it a list item).
fn convert_task(first: &mut String) -> bool {
    let (marker, rest) = match first.split_once(' ') {
        Some((m, r)) => (m, r),
        None => (first.as_str(), ""),
    };
    let checked = match marker {
        "TODO" | "LATER" | "NOW" | "DOING" | "WAITING" | "IN-PROGRESS" => false,
        "DONE" => true,
        "CANCELED" | "CANCELLED" => {
            *first = format!("[x] ~~{}~~", strip_priority(rest));
            return true;
        }
        _ => return false,
    };
    let rest = strip_priority(rest);
    *first = format!("[{}] {rest}", if checked { "x" } else { " " });
    true
}

/// Drop a Logseq priority cookie (`[#A] `) from a task's text.
fn strip_priority(text: &str) -> &str {
    let t = text.trim_start();
    if t.len() >= 4 && t.starts_with("[#") && t.as_bytes()[3] == b']' {
        t[4..].trim_start()
    } else {
        text
    }
}

/// `{{video url}}` → url, `{{embed [[X]]}}` → `[[X]]`, `{{embed ((id))}}` →
/// the block's text; queries and anything unrecognized stay visible as
/// inline code so nothing is silently lost.
fn convert_macros(line: &str, id_map: &HashMap<String, BlockRef>) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start..].find("}}") else {
            break;
        };
        let inner = rest[start + 2..start + end].trim();
        out.push_str(&rest[..start]);
        if let Some(url) = inner.strip_prefix("video ") {
            out.push_str(url.trim());
        } else if let Some(target) = inner.strip_prefix("embed ") {
            let target = target.trim();
            if let Some(id) = target.strip_prefix("((").and_then(|t| t.strip_suffix("))")) {
                match id_map.get(id.trim()) {
                    Some(r) => out.push_str(&r.text),
                    None => out.push_str(target),
                }
            } else {
                out.push_str(target);
            }
        } else {
            out.push_str(&format!("`{{{{{inner}}}}}`"));
        }
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

/// `((uuid))` → the referenced block's text (left as-is when unknown).
fn convert_block_refs(line: &str, id_map: &HashMap<String, BlockRef>) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find("((") {
        let Some(end) = rest[start..].find("))") else {
            break;
        };
        let inner = rest[start + 2..start + end].trim();
        out.push_str(&rest[..start]);
        match id_map.get(inner) {
            // A known block on a known page: a real block link (issue #53) —
            // the target block gets a matching ` ^short` anchor on import.
            Some(r) if !r.target.is_empty() => {
                out.push_str(&format!("[[{}#^{}]]", r.target, short_block_id(inner)));
            }
            // Known block, unknown page (highlights): the old inline text.
            Some(r) => out.push_str(&r.text),
            None => out.push_str(&rest[start..start + end + 2]),
        }
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

/// The imported anchor id for a Logseq block uuid: its first 8 hex chars —
/// short enough to live in the source, unique enough for a vault.
fn short_block_id(uuid: &str) -> &str {
    &uuid[..8.min(uuid.len())]
}

/// `[[A/B]]` → `[[A::B]]` (segments trimmed) and `#[[multi word]]` →
/// `[[multi word]]`. URLs inside `[[…]]` are left alone.
fn convert_wiki_links(line: &str) -> String {
    let line = line.replace("#[[", "[[");
    let mut out = String::new();
    let mut rest = line.as_str();
    while let Some(start) = rest.find("[[") {
        let Some(end) = rest[start..].find("]]") else {
            break;
        };
        let inner = &rest[start + 2..start + end];
        out.push_str(&rest[..start]);
        if inner.contains("://") || inner.starts_with("pdf/") || inner.starts_with("images/") {
            out.push_str(&format!("[[{inner}]]"));
        } else {
            out.push_str(&format!("[[{}]]", name_to_title(inner)));
        }
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

/// Find the next markdown link in `s`: returns
/// `(text-before, alt, destination, text-after, was-image)`. Destinations may
/// contain balanced parentheses (Logseq writes them unescaped).
fn next_md_link(s: &str) -> Option<(&str, String, String, &str, bool)> {
    let mut search_from = 0;
    loop {
        let open = s[search_from..].find("](")? + search_from;
        // Walk back to the matching `[`.
        let mut depth = 0;
        let mut label_start = None;
        for (i, c) in s[..open].char_indices().rev() {
            match c {
                ']' => depth += 1,
                '[' => {
                    if depth == 0 {
                        label_start = Some(i);
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        let Some(label_start) = label_start else {
            search_from = open + 2;
            continue;
        };
        // Walk forward to the matching `)`, balancing parens.
        let dest_start = open + 2;
        let mut depth = 0;
        let mut dest_end = None;
        for (i, c) in s[dest_start..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        dest_end = Some(dest_start + i);
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        let Some(dest_end) = dest_end else {
            search_from = open + 2;
            continue;
        };
        let bang = label_start > 0 && s.as_bytes()[label_start - 1] == b'!';
        let before_end = if bang { label_start - 1 } else { label_start };
        return Some((
            &s[..before_end],
            s[label_start + 1..open].to_string(),
            s[dest_start..dest_end].to_string(),
            &s[dest_end + 1..],
            bang,
        ));
    }
}

/// Make an asset filename safe inside a markdown destination (spaces and
/// parentheses break `(…)` targets).
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Whether a line is, on its own, a single markdown image (`![alt](src)`).
/// Logseq writes one image per line; this doesn't try to handle several images
/// (or trailing text) on one line.
fn is_standalone_image(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("![") && t.ends_with(')') && t.contains("](")
}

/// Logseq glues several images onto one bullet's continuation lines, so a single
/// block can hold a run of standalone `![](…)` lines. Rendered together only the
/// first becomes a real (block) image and the rest collapse to inline — so split
/// each standalone image onto its own block. The original block's list semantics
/// (numbered / task) stay on its first piece.
fn split_standalone_images(block: ConvBlock) -> Vec<ConvBlock> {
    if block.lines.len() < 2 || !block.lines.iter().any(|l| is_standalone_image(l)) {
        return vec![block];
    }
    let (depth, numbered, task) = (block.depth, block.numbered, block.task);
    let plain = |lines: Vec<String>| ConvBlock {
        depth,
        lines,
        numbered: false,
        task: false,
    };
    let mut out: Vec<ConvBlock> = Vec::new();
    let mut text: Vec<String> = Vec::new();
    for line in block.lines {
        if is_standalone_image(&line) {
            if !text.is_empty() {
                out.push(plain(std::mem::take(&mut text)));
            }
            out.push(plain(vec![line]));
        } else {
            text.push(line);
        }
    }
    if !text.is_empty() {
        out.push(plain(text));
    }
    if let Some(first) = out.first_mut() {
        first.numbered = numbered;
        first.task = task;
    }
    out
}

// --- Rendering ---

/// Render converted blocks as Zorite markdown. With `flatten`, top-level
/// blocks become paragraphs/headings and only nested blocks stay list items;
/// otherwise everything is a list item at its depth. List indentation is two
/// spaces per level; multi-line blocks keep their extra lines aligned under
/// the item text.
fn render(blocks: &[ConvBlock], flatten: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut counters: HashMap<usize, usize> = HashMap::new();
    // Whether the current top-level subtree's root rendered as a list item
    // (then children indent one level deeper than under a paragraph root).
    let mut root_is_item = !flatten;
    for b in blocks {
        // Numbering survives a detour into deeper children, but resets when a
        // non-numbered sibling (or anything shallower) interrupts the run.
        counters.retain(|&d, _| d <= b.depth);
        let as_item = !flatten || b.depth > 0 || b.task || b.numbered;
        if b.depth == 0 {
            root_is_item = as_item;
        }
        if !as_item {
            counters.clear();
            // Paragraph/heading: blank line before when following other text.
            if !out.is_empty() {
                out.push(String::new());
            }
            out.extend(b.lines.iter().cloned());
            out.push(String::new());
            continue;
        }
        let units = if flatten && !root_is_item {
            b.depth.saturating_sub(1)
        } else {
            b.depth
        };
        let indent = "  ".repeat(units);
        let marker = if b.numbered {
            let n = counters.entry(b.depth).or_insert(0);
            *n += 1;
            format!("{n}. ")
        } else {
            counters.remove(&b.depth);
            "- ".to_string()
        };
        out.push(format!("{indent}{marker}{}", b.lines[0]));
        let cont = " ".repeat(indent.len() + marker.len());
        for line in &b.lines[1..] {
            if line.is_empty() {
                out.push(String::new());
            } else {
                out.push(format!("{cont}{line}"));
            }
        }
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

// --- PDF-highlight pages ---

/// Convert a Logseq `hls__*` page into Zorite's `<name>.pdf (highlights)`
/// page. Returns `(title, content, pdf-copy)` or `None` when the source PDF
/// can't be determined.
fn convert_highlights(conv: &mut Converter, blocks: &[Block]) -> Option<(String, String)> {
    // The PDF lives in the page properties: `file-path:: ../assets/X.pdf`.
    let mut pdf_rel: Option<String> = None;
    for b in blocks {
        for line in &b.lines {
            if let Some(("file-path", v)) = parse_prop(line) {
                pdf_rel = Some(
                    v.trim_start_matches("../assets/")
                        .trim_start_matches("assets/")
                        .to_string(),
                );
            }
        }
        if pdf_rel.is_some() {
            break;
        }
    }
    let rel = pdf_rel?;
    let decoded = percent_decode_repeated(&rel);
    let name = sanitize_name(decoded.rsplit('/').next().unwrap_or(&decoded));
    match conv.find_asset(&rel, &decoded) {
        Some(src) => conv.copies.push((src, format!("pdf/{name}"))),
        None => conv.warnings.push(format!("asset not found: {rel}")),
    }
    let title = crate::pdf::highlights_title(Path::new(&name));
    let mut lines = Vec::new();
    for b in blocks {
        let mut page: Option<u32> = None;
        let mut color = String::new();
        let mut quote = String::new();
        for line in &b.lines {
            if let Some((k, v)) = parse_prop(line) {
                match k {
                    "hl-page" => page = v.parse().ok(),
                    "hl-color" => color = map_color(v),
                    _ => {}
                }
            } else if quote.is_empty() {
                let t = line.trim();
                if !t.is_empty() && t != "[:span]" {
                    quote = t.to_string();
                }
            }
        }
        let Some(page) = page else { continue };
        let quote = if quote.is_empty() {
            "(area highlight)".to_string()
        } else {
            quote
        };
        let mut meta = String::new();
        if !color.is_empty() && color != "yellow" {
            meta.push_str(&format!(" {{{color}}}"));
        }
        meta.push_str(&format!(" [[pdf/{name}#p{page}|↗]]"));
        lines.push(format!("- p{page}: {quote}{meta}"));
    }
    Some((title, lines.join("\n")))
}

/// Map a Logseq highlight color onto Zorite's palette (yellow is the default
/// and is omitted from the stored line).
fn map_color(logseq: &str) -> String {
    match logseq.to_ascii_lowercase().as_str() {
        "green" => "green",
        "blue" => "blue",
        "orange" => "orange",
        "red" | "purple" | "pink" => "pink",
        _ => "",
    }
    .to_string()
}

// --- Driving the read ---

/// Read a Logseq graph into an [`ImportBundle`] (write it with
/// [`super::write_bundle`]).
pub fn read_graph(root: &Path, opts: &Options) -> Result<ImportBundle, String> {
    let files = scan(root)?;
    let mut bundle = ImportBundle::default();
    let mut conv = Converter::new(root);

    // Pass 1: parse everything and collect block ids for `((ref))`s.
    let mut parsed: Vec<(Kind, Vec<Block>)> = Vec::new();
    for file in files {
        let text = match std::fs::read_to_string(&file.path) {
            Ok(t) => t,
            Err(e) => {
                bundle
                    .warnings
                    .push(format!("{}: {e}", file.path.display()));
                continue;
            }
        };
        let blocks = parse_outline(&text);
        // The link target a block on this file resolves to: the page's final
        // title (a first-block `title::` overrides the filename) or the
        // journal date. Highlights pages derive titles later — their ids fall
        // back to inlined text.
        let target = match &file.kind {
            Kind::Journal(date) => Some(date.clone()),
            Kind::Page(title_guess) => Some(
                blocks
                    .first()
                    .and_then(|b| {
                        b.lines.iter().find_map(|l| {
                            parse_prop(l)
                                .and_then(|(k, v)| (k == "title").then(|| name_to_title(v)))
                        })
                    })
                    .unwrap_or_else(|| title_guess.clone()),
            ),
            Kind::Highlights => None,
        };
        conv.collect_ids(&blocks, target.as_deref());
        conv.collect_refs(&text);
        parsed.push((file.kind, blocks));
    }

    // Pass 2: convert. Empty results (a graph has plenty of stub files)
    // simply don't land in the bundle.
    for (kind, blocks) in parsed {
        match kind {
            Kind::Highlights => {
                let Some((title, content)) = convert_highlights(&mut conv, &blocks) else {
                    bundle
                        .warnings
                        .push("hls page without a file-path:: — skipped".to_string());
                    continue;
                };
                if content.is_empty() {
                    continue;
                }
                bundle.pages.push(ImportPage {
                    title,
                    content,
                    aliases: Vec::new(),
                    is_highlights: true,
                });
            }
            Kind::Page(title_guess) => {
                let mut props = PageProps::default();
                let mut conv_blocks = Vec::new();
                for (bi, b) in blocks.iter().enumerate() {
                    let props_slot = (bi == 0).then_some(&mut props);
                    if let Some(cb) = conv.convert_block(b, props_slot) {
                        conv_blocks.extend(split_standalone_images(cb));
                    }
                }
                let content = render(&conv_blocks, opts.flatten);
                if content.trim().is_empty() && props.aliases.is_empty() {
                    continue;
                }
                bundle.pages.push(ImportPage {
                    title: props.title.unwrap_or(title_guess),
                    content,
                    aliases: props.aliases,
                    is_highlights: false,
                });
            }
            Kind::Journal(date) => {
                let mut conv_blocks = Vec::new();
                for b in &blocks {
                    if let Some(cb) = conv.convert_block(b, None) {
                        conv_blocks.extend(split_standalone_images(cb));
                    }
                }
                let content = render(&conv_blocks, opts.flatten);
                if content.trim().is_empty() {
                    continue;
                }
                bundle.days.push(ImportDay { date, content });
            }
        }
    }

    bundle.assets = conv
        .copies
        .into_iter()
        .map(|(src, managed)| AssetCopy { src, managed })
        .collect();
    bundle.warnings.extend(conv.warnings);
    bundle.favorites = read_favorites(root);
    bundle.whiteboards = read_whiteboards(root, &mut bundle.warnings, &mut bundle.asset_bytes);
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_refs_import_as_links_with_anchors() {
        // A synthetic vault: a target block with an id, a ref to it, a
        // title::-renamed page, a journal block, and an unresolvable ref.
        let root = std::env::temp_dir().join(format!("zorite-logseq-refs-{}", std::process::id()));
        let pages = root.join("pages");
        let journals = root.join("journals");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::create_dir_all(&journals).unwrap();
        std::fs::write(
            pages.join("Target.md"),
            "- first line\n  id:: 638b7565-ac9b-4d71-ac77-758bc6ceec42\n- other\n",
        )
        .unwrap();
        std::fs::write(
            pages.join("Renamed.md"),
            "- title:: Actual Name\n- kept\n  id:: 12345678-aaaa-bbbb-cccc-ddddeeee0000\n",
        )
        .unwrap();
        std::fs::write(
            journals.join("2024_02_07.md"),
            "- day block\n  id:: aaaabbbb-cccc-dddd-eeee-ffff00001111\n",
        )
        .unwrap();
        std::fs::write(
            pages.join("Source.md"),
            "- see ((638b7565-ac9b-4d71-ac77-758bc6ceec42)) here\n\
             - renamed ((12345678-aaaa-bbbb-cccc-ddddeeee0000))\n\
             - day ((aaaabbbb-cccc-dddd-eeee-ffff00001111))\n\
             - broken ((00000000-0000-0000-0000-000000000000))\n",
        )
        .unwrap();

        let bundle = read_graph(&root, &Options { flatten: true }).unwrap();
        std::fs::remove_dir_all(&root).ok();

        let page = |t: &str| {
            bundle
                .pages
                .iter()
                .find(|p| p.title == t)
                .unwrap_or_else(|| panic!("page {t} missing"))
        };
        // Referenced blocks grew anchors on their first content line.
        assert!(page("Target").content.contains("first line ^638b7565"));
        assert!(page("Actual Name").content.contains("kept ^12345678"));
        assert!(bundle.days[0].content.contains("day block ^aaaabbbb"));
        // Refs became block links — to the FINAL titles.
        let src = &page("Source").content;
        assert!(src.contains("see [[Target#^638b7565]] here"), "{src}");
        assert!(src.contains("renamed [[Actual Name#^12345678]]"), "{src}");
        assert!(src.contains("day [[2024-02-07#^aaaabbbb]]"), "{src}");
        // An id nothing declares stays literal.
        assert!(
            src.contains("((00000000-0000-0000-0000-000000000000))"),
            "{src}"
        );
    }

    // -- filenames --

    #[test]
    fn journal_dates_parse_and_reject() {
        assert_eq!(journal_date("2024_02_07"), Some("2024-02-07".into()));
        assert_eq!(journal_date("2024_2_7"), None);
        assert_eq!(journal_date("2024_13_07"), None);
        assert_eq!(journal_date("notes"), None);
    }

    #[test]
    fn favorites_parse_to_namespaced_titles() {
        let edn = r#"
{:something true
 ;; Favorites to list on the left sidebar
 :favorites ["Orders/Things to order" "TODO" "Cheat Sheets"]
 :other 1}
"#;
        assert_eq!(
            parse_favorites(edn),
            vec!["Orders::Things to order", "TODO", "Cheat Sheets"]
        );
        // A commented-out key is ignored; an absent one yields nothing.
        assert!(parse_favorites(";; :favorites [\"X\"]\n:other 1").is_empty());
        assert!(parse_favorites(":graph/settings {}").is_empty());
    }

    // -- whiteboards --

    #[test]
    fn text_shape_converts_with_color() {
        let shape = edn::parse(
            r##"{:type "text" :point [10 20] :fontSize 24 :text "Hi" :stroke "#ff0000"}"##,
        )
        .unwrap();
        let els = shape_to_element(&shape, 1);
        assert_eq!(els.len(), 1);
        match &els[0].kind {
            ElementKind::Text(t) => {
                assert_eq!((t.x, t.y, t.size), (10.0, 20.0, 24.0));
                assert_eq!(t.content, "Hi");
            }
            other => panic!("expected text, got {other:?}"),
        }
        assert_eq!(els[0].stroke, Some(0xff00_00ff));
    }

    #[test]
    fn labeled_box_imports_a_native_label() {
        // A tldraw box carries its text in `:label` → the shape's own label
        // (the renderer centers + auto-shrinks it), not a separate text element.
        let shape = edn::parse(
            r#"{:type "box" :point [128 96] :size [128 40] :fontSize 20 :label "Fault Ind"}"#,
        )
        .unwrap();
        let els = shape_to_element(&shape, 5);
        assert_eq!(els.len(), 1, "just the box, carrying its label");
        assert!(matches!(els[0].kind, ElementKind::Rect(_)));
        assert_eq!(els[0].id, 5);
        assert_eq!(els[0].label.as_deref(), Some("Fault Ind"));
        // An empty label leaves the box unlabeled.
        let blank = edn::parse(r#"{:type "box" :point [0 0] :size [40 40] :label ""}"#).unwrap();
        let blank_els = shape_to_element(&blank, 1);
        assert_eq!(blank_els.len(), 1);
        assert_eq!(blank_els[0].label, None);
    }

    #[test]
    fn line_uses_handle_points() {
        let shape = edn::parse(
            r#"{:type "line" :point [5 5] :handles {:start {:point [0 0]} :end {:point [0 95]}}}"#,
        )
        .unwrap();
        let els = shape_to_element(&shape, 1);
        match &els[0].kind {
            ElementKind::Line(s) => assert_eq!((s.x1, s.y1, s.x2, s.y2), (5.0, 5.0, 5.0, 100.0)),
            other => panic!("expected line, got {other:?}"),
        }
    }

    #[test]
    fn decorated_line_becomes_arrow() {
        // `:decorations {:end "arrow"}` → arrow with the head at the end (x2,y2).
        let end = edn::parse(
            r#"{:type "line" :point [5 5] :decorations {:end "arrow"}
                :handles {:start {:point [0 0]} :end {:point [0 95]}}}"#,
        )
        .unwrap();
        match &shape_to_element(&end, 1)[0].kind {
            ElementKind::Arrow(s) => assert_eq!((s.x1, s.y1, s.x2, s.y2), (5.0, 5.0, 5.0, 100.0)),
            other => panic!("expected arrow, got {other:?}"),
        }
        // A start-only arrow swaps endpoints so the head lands at the start.
        let start = edn::parse(
            r#"{:type "line" :point [5 5] :decorations {:start "arrow"}
                :handles {:start {:point [0 0]} :end {:point [0 95]}}}"#,
        )
        .unwrap();
        match &shape_to_element(&start, 1)[0].kind {
            ElementKind::Arrow(s) => assert_eq!((s.x1, s.y1, s.x2, s.y2), (5.0, 100.0, 5.0, 5.0)),
            other => panic!("expected arrow, got {other:?}"),
        }
    }

    #[test]
    fn colors_and_unsupported_shapes() {
        assert_eq!(parse_color("#fff"), Some(0xffff_ffff));
        assert_eq!(parse_color("#ff00ea"), Some(0xff00_eaff));
        assert_eq!(parse_color("var(--tl-foreground, #000)"), Some(0x0000_00ff));
        assert_eq!(parse_color("var(--x)"), None);
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("gray"), Some(0x8080_80ff));
        // Images route through `image_element`, not here → empty from this fn.
        let img = edn::parse(r#"{:type "image" :point [0 0]}"#).unwrap();
        assert!(shape_to_element(&img, 1).is_empty());
        // Empty text → skipped.
        let empty = edn::parse(r#"{:type "text" :point [0 0] :text "  "}"#).unwrap();
        assert!(shape_to_element(&empty, 1).is_empty());
    }

    #[test]
    fn image_shape_decodes_embedded_asset() {
        // "aGVsbG8=" is base64 for "hello" — enough to exercise the decode path.
        assert_eq!(
            decode_data_uri("id1", "data:image/png;base64,aGVsbG8="),
            Some(("wb-id1.png".to_string(), b"hello".to_vec()))
        );
        // jpeg normalizes to .jpg; svg / remote URLs aren't decoded.
        assert_eq!(
            decode_data_uri("x", "data:image/jpeg;base64,aGVsbG8=").map(|(n, _)| n),
            Some("wb-x.jpg".to_string())
        );
        assert!(decode_data_uri("x", "data:image/svg+xml;base64,aGVsbG8=").is_none());
        assert!(decode_data_uri("x", "https://example.com/a.png").is_none());

        // A shape resolves its assetId against the registry → one image element,
        // and its bytes are queued once (a second shape reusing it doesn't requeue).
        let assets = HashMap::from([("a1", "data:image/png;base64,aGVsbG8=")]);
        let shape =
            edn::parse(r#"{:type "image" :assetId "a1" :point [10 20] :size [30 40]}"#).unwrap();
        let mut images = Vec::new();
        let els = image_element(&shape, 7, &assets, &mut images);
        assert_eq!(els.len(), 1);
        assert_eq!(els[0].id, 7);
        match &els[0].kind {
            ElementKind::Image(im) => {
                assert_eq!(im.src, "images/wb-a1.png");
                assert_eq!((im.x, im.y, im.w, im.h), (10.0, 20.0, 30.0, 40.0));
            }
            other => panic!("expected image, got {other:?}"),
        }
        assert_eq!(images.len(), 1);
        image_element(&shape, 8, &assets, &mut images); // reuse → no new bytes
        assert_eq!(images.len(), 1);

        // A missing asset (or no assetId) yields nothing.
        let orphan =
            edn::parse(r#"{:type "image" :assetId "gone" :point [0 0] :size [1 1]}"#).unwrap();
        assert!(image_element(&orphan, 1, &assets, &mut images).is_empty());
    }

    #[test]
    fn stems_become_namespaced_titles() {
        assert_eq!(title_from_stem("Budget___2024"), "Budget::2024");
        assert_eq!(
            title_from_stem("Alan Humphrey___Alan's list"),
            "Alan Humphrey::Alan's list"
        );
        // Percent-encoding decodes, repeatedly when double-encoded.
        assert_eq!(title_from_stem("A%2FB"), "A/B");
        assert_eq!(title_from_stem("A%2520B"), "A B");
    }

    // -- outline parsing --

    #[test]
    fn outline_parses_depths_and_continuations() {
        let text = "- top\n\t- child\n\t  cont line\n\t\t- grandchild\n- top2";
        let blocks = parse_outline(text);
        assert_eq!(blocks.len(), 4);
        assert_eq!((blocks[0].depth, blocks[0].lines[0].as_str()), (0, "top"));
        assert_eq!(blocks[1].depth, 1);
        assert_eq!(blocks[1].lines, vec!["child", "cont line"]);
        assert_eq!(blocks[2].depth, 2);
        assert_eq!(blocks[3].depth, 0);
    }

    #[test]
    fn outline_preamble_becomes_first_block() {
        let text = "title:: Real Title\nalias:: a, b\n- first bullet";
        let blocks = parse_outline(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].lines, vec!["title:: Real Title", "alias:: a, b"]);
        assert_eq!(blocks[1].lines[0], "first bullet");
    }

    // -- inline conversions --

    #[test]
    fn wiki_links_convert_namespaces() {
        assert_eq!(
            convert_wiki_links("see [[Jose Professional/2024]] ok"),
            "see [[Jose Professional::2024]] ok"
        );
        // Segments are trimmed (Logseq tolerates sloppy links).
        assert_eq!(
            convert_wiki_links("[[ Will Professional/2024]]"),
            "[[Will Professional::2024]]"
        );
        // Tag-links lose the hash, URLs stay put.
        assert_eq!(convert_wiki_links("#[[multi word]]"), "[[multi word]]");
        assert_eq!(convert_wiki_links("[[https://a.b/c]]"), "[[https://a.b/c]]");
    }

    #[test]
    fn macros_convert_or_stay_visible() {
        let ids = HashMap::new();
        assert_eq!(
            convert_macros("{{video https://yt/x}} end", &ids),
            "https://yt/x end"
        );
        assert_eq!(convert_macros("{{embed [[Page]]}}", &ids), "[[Page]]");
        assert_eq!(
            convert_macros("{{query (task TODO)}}", &ids),
            "`{{query (task TODO)}}`"
        );
        let mut ids = HashMap::new();
        ids.insert(
            "abc".to_string(),
            BlockRef {
                target: String::new(),
                text: "the block text".to_string(),
            },
        );
        // No known target page: embeds and refs fall back to inlined text.
        assert_eq!(convert_macros("{{embed ((abc))}}", &ids), "the block text");
        assert_eq!(
            convert_block_refs("see ((abc)) here", &ids),
            "see the block text here"
        );
        assert_eq!(convert_block_refs("((missing))", &ids), "((missing))");
        // A known target: a block link with the shortened anchor id.
        ids.insert(
            "638b7565-ac9b-4d71-ac77-758bc6ceec42".to_string(),
            BlockRef {
                target: "Target".to_string(),
                text: "first line".to_string(),
            },
        );
        assert_eq!(
            convert_block_refs("((638b7565-ac9b-4d71-ac77-758bc6ceec42))", &ids),
            "[[Target#^638b7565]]"
        );
    }

    #[test]
    fn tasks_become_checkboxes() {
        let mut l = "TODO call Bob".to_string();
        assert!(convert_task(&mut l));
        assert_eq!(l, "[ ] call Bob");
        let mut l = "DONE [#A] ship it".to_string();
        assert!(convert_task(&mut l));
        assert_eq!(l, "[x] ship it");
        let mut l = "CANCELED bad idea".to_string();
        assert!(convert_task(&mut l));
        assert_eq!(l, "[x] ~~bad idea~~");
        let mut l = "plain text".to_string();
        assert!(!convert_task(&mut l));
        assert_eq!(l, "plain text");
    }

    // -- block conversion --

    fn conv() -> Converter {
        Converter::new(Path::new("/nonexistent"))
    }

    fn block(depth: usize, lines: &[&str]) -> Block {
        Block {
            depth,
            lines: lines.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn internal_props_drop_user_props_stay() {
        let b = block(
            0,
            &[
                "Meeting notes",
                "id:: 64b5a4b8-6242-4063-859d-e694b14f2e62",
                "collapsed:: true",
                "subject:: Budget review",
                "attendees:: [[Alan]], [[Eric/2024]]",
            ],
        );
        let cb = conv().convert_block(&b, None).unwrap();
        assert_eq!(
            cb.lines,
            vec![
                "Meeting notes",
                "subject:: Budget review",
                "attendees:: [[Alan]], [[Eric::2024]]",
            ]
        );
    }

    #[test]
    fn logbook_drawers_strip() {
        let b = block(
            0,
            &[
                "TODO thing",
                ":LOGBOOK:",
                "CLOCK: [2023-07-17 Mon 14:05:03]",
                ":END:",
                "after",
            ],
        );
        let cb = conv().convert_block(&b, None).unwrap();
        assert_eq!(cb.lines, vec!["[ ] thing", "after"]);
        assert!(cb.task);
    }

    #[test]
    fn page_props_extract_title_and_alias() {
        let b = block(
            0,
            &["title:: Budget/2024", "alias:: [[B24]], budget 24", "body"],
        );
        let mut props = PageProps::default();
        let cb = conv().convert_block(&b, Some(&mut props)).unwrap();
        assert_eq!(props.title.as_deref(), Some("Budget::2024"));
        assert_eq!(props.aliases, vec!["B24", "budget 24"]);
        assert_eq!(cb.lines, vec!["body"]);
    }

    #[test]
    fn numbered_marker_consumed() {
        let b = block(0, &["step one", "logseq.order-list-type:: number"]);
        let cb = conv().convert_block(&b, None).unwrap();
        assert!(cb.numbered);
        assert_eq!(cb.lines, vec!["step one"]);
    }

    #[test]
    fn code_fences_keep_content_verbatim() {
        let b = block(
            0,
            &[
                "```cfg",
                "vlan access 180",
                "key:: not-a-prop [[a/b]] ((x))",
                "```",
            ],
        );
        let cb = conv().convert_block(&b, None).unwrap();
        assert_eq!(cb.lines[2], "key:: not-a-prop [[a/b]] ((x))");
    }

    #[test]
    fn glued_fences_normalize_onto_own_lines() {
        // Logseq writes `- ```interface 2/1/44` and closes with `…3600``` `.
        let b = block(
            0,
            &[
                "```interface 2/1/44",
                "    no shutdown",
                "    reauth-period 3600```",
                "after the fence [[a/b]]",
            ],
        );
        let cb = conv().convert_block(&b, None).unwrap();
        assert_eq!(
            cb.lines,
            vec![
                "```",
                "interface 2/1/44",
                "    no shutdown",
                "    reauth-period 3600",
                "```",
                "after the fence [[a::b]]",
            ]
        );
        // A single-token info string stays an info string.
        let b = block(0, &["```rust", "fn main() {}", "```"]);
        let cb = conv().convert_block(&b, None).unwrap();
        assert_eq!(cb.lines[0], "```rust");
    }

    #[test]
    fn empty_blocks_disappear() {
        assert!(conv().convert_block(&block(0, &[""]), None).is_none());
        assert!(conv().convert_block(&block(1, &["-"]), None).is_some()); // "-" is content
        assert!(
            conv()
                .convert_block(&block(0, &["id:: x", "collapsed:: true"]), None)
                .is_none()
        );
    }

    // -- rendering --

    fn cb(depth: usize, lines: &[&str], numbered: bool, task: bool) -> ConvBlock {
        ConvBlock {
            depth,
            lines: lines.iter().map(|s| s.to_string()).collect(),
            numbered,
            task,
        }
    }

    #[test]
    fn flatten_makes_paragraphs_and_nested_lists() {
        let blocks = vec![
            cb(0, &["# Capital"], false, false),
            cb(1, &["switches - $40,000"], false, false),
            cb(2, &["spare"], false, false),
            cb(0, &["A paragraph", "second line"], false, false),
        ];
        let md = render(&blocks, true);
        assert_eq!(
            md,
            "# Capital\n\n- switches - $40,000\n  - spare\n\nA paragraph\nsecond line"
        );
    }

    #[test]
    fn flatten_keeps_tasks_and_numbers_as_items() {
        let blocks = vec![
            cb(0, &["[ ] call Bob"], false, true),
            cb(0, &["step one"], true, false),
            cb(0, &["step two"], true, false),
            cb(1, &["detail"], false, false),
            cb(0, &["step three"], true, false),
        ];
        let md = render(&blocks, true);
        assert_eq!(
            md,
            "- [ ] call Bob\n1. step one\n2. step two\n  - detail\n3. step three"
        );
    }

    #[test]
    fn keep_bullets_mode_keeps_everything_listed() {
        let blocks = vec![
            cb(0, &["top"], false, false),
            cb(1, &["child", "continuation"], false, false),
        ];
        let md = render(&blocks, false);
        assert_eq!(md, "- top\n  - child\n    continuation");
    }

    #[test]
    fn glued_images_split_so_each_renders_as_a_block() {
        // Logseq's "several images on one bullet's continuation lines" → one
        // block per image, so each renders as a real (block) image.
        let block = cb(
            1,
            &[
                "![a](images/a.jpg)",
                "![b](images/b.jpg)",
                "![c](images/c.jpg)",
            ],
            false,
            false,
        );
        let split = split_standalone_images(block);
        assert_eq!(split.len(), 3);
        assert!(split.iter().all(|b| b.lines.len() == 1 && b.depth == 1));
        let md = render(&split, true);
        assert_eq!(
            md,
            "- ![a](images/a.jpg)\n- ![b](images/b.jpg)\n- ![c](images/c.jpg)"
        );

        // A leading text line stays its own block; its list semantics carry over.
        let mixed = cb(0, &["intro text", "![a](images/a.jpg)"], false, true);
        let split = split_standalone_images(mixed);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].lines, vec!["intro text"]);
        assert!(split[0].task && !split[1].task);

        // A block with no images is untouched.
        let plain = cb(0, &["just text", "more text"], false, false);
        assert_eq!(split_standalone_images(plain).len(), 1);
    }

    // -- highlights --

    #[test]
    fn hls_pages_convert_to_zorite_highlights() {
        let text = "file:: [x.pdf](../assets/x.pdf)\nfile-path:: ../assets/x.pdf\n- quoted text\n  hl-page:: 3\n  id:: aaa\n- area\n  hl-page:: 9\n  hl-color:: green\n  [:span]";
        let blocks = parse_outline(text);
        let mut c = conv();
        let (title, content) = convert_highlights(&mut c, &blocks).unwrap();
        assert_eq!(title, "x.pdf (highlights)");
        assert_eq!(
            content,
            "- p3: quoted text [[pdf/x.pdf#p3|↗]]\n- p9: area {green} [[pdf/x.pdf#p9|↗]]"
        );
    }

    // -- end to end --

    #[test]
    fn read_graph_end_to_end() {
        let root = std::env::temp_dir().join("zorite-test-logseq-graph");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/pic 1.png"), b"png").unwrap();
        std::fs::write(
            root.join("pages/Budget___2024.md"),
            "- # Capital\n\t- TODO get quotes for [[Vendors/Arista]]\n- ![shot](../assets/pic 1.png)\n",
        )
        .unwrap();
        std::fs::write(
            root.join("pages/Other.md"),
            "alias:: O2\n- see [[Budget/2024]]\n",
        )
        .unwrap();
        std::fs::write(root.join("journals/2024_02_07.md"), "- met [[Alan]]\n").unwrap();
        std::fs::write(root.join("pages/Empty.md"), "-\n- id:: abc\n").unwrap();

        let bundle = read_graph(&root, &Options { flatten: true }).unwrap();

        // Journals first (scan order), then pages; empty stubs don't land.
        assert_eq!(bundle.days.len(), 1);
        assert_eq!(bundle.days[0].date, "2024-02-07");
        assert_eq!(bundle.days[0].content, "met [[Alan]]");
        let titles: Vec<&str> = bundle.pages.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, vec!["Budget::2024", "Other"]);

        let budget = &bundle.pages[0];
        assert_eq!(
            budget.content,
            "# Capital\n\n- [ ] get quotes for [[Vendors::Arista]]\n\n![shot](images/pic_1.png)"
        );
        assert!(!budget.is_highlights);
        assert_eq!(bundle.pages[1].content, "see [[Budget::2024]]");
        assert_eq!(bundle.pages[1].aliases, vec!["O2"]);

        // The referenced asset is queued with its sanitized managed name.
        assert_eq!(bundle.assets.len(), 1);
        assert_eq!(bundle.assets[0].managed, "images/pic_1.png");
        assert!(bundle.assets[0].src.ends_with("pic 1.png"));
        assert!(bundle.warnings.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }
}
