//! The Obsidian reader: turns a vault folder into an [`ImportBundle`] for
//! [`super::write_bundle`]. Pure filesystem + string work, no DB.
//!
//! Obsidian notes are plain CommonMark plus a few extensions, so much passes
//! through untouched. What this reader handles:
//!
//! - **Folders → namespaces** (default) — `Projects/Tasks.md` becomes the
//!   page `Projects::Tasks`, and links resolve through a name→title map so a
//!   bare `[[Tasks]]` and a full `[[Projects/Tasks]]` both land on it. The
//!   `namespaces: false` option flattens instead (title = note name).
//! - **Links** — `[[Note]]` / `[[Note|alias]]` pass through (resolved to
//!   their namespaced title); `[[Note#Heading]]` / `[[Note#^block]]` keep the
//!   anchor (Zorite jumps to headings and `^id` blocks), and trailing
//!   ` ^block-id` markers stay in the text as anchor targets. A nested
//!   Obsidian heading path (`#H1#H2`) keeps its last segment.
//! - **Embeds** — `![[image.png]]` → `![](images/…)` (asset copied);
//!   `![[Other Note]]` / `![[Note#Heading]]` / `![[Note#^id]]` transclusions
//!   pass through as Zorite embeds (promoted onto their own line — Zorite
//!   renders only standalone `![[…]]` lines).
//! - **Callouts** — `> [!note]` … (any of Obsidian's ~13 types) → Zorite's
//!   five GitHub-style alerts (`> [!NOTE]` …) with a fallback.
//! - **Highlights / comments** — `==text==` → `<mark>text</mark>`;
//!   `%%comment%%` is dropped.
//! - **Frontmatter** — YAML `aliases:` feed the alias table, `tags:` are
//!   hoisted to `#tags` in the body, other keys become `key:: value` lines.
//! - **Daily notes** — a `YYYY-MM-DD` filename (in the configured daily
//!   folder, if any) becomes a journal day.
//! - **Assets** — images copied to `images/`, PDFs referenced as `pdf/…`
//!   chips; attachments are resolved by filename anywhere in the vault.
//! - **Canvases** — each `.canvas` board becomes a Zorite whiteboard: text
//!   cards → labeled boxes, note cards → clickable page cards, image cards →
//!   placed images, links/other files → named boxes, groups → outlines, and
//!   edges → arrows/lines with their labels as midpoint text (see
//!   [`convert_canvas`] for the exact downgrades).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_whiteboard::{
    BoxGeom, Element, ElementKind, EmbedGeom, ImageGeom, Scene, SegGeom, SegmentStyle, TextGeom,
};

use super::{AssetCopy, ImportBundle, ImportDay, ImportPage};

/// The namespace separator in Zorite page titles.
const SEP: &str = "::";

pub struct Options {
    /// Preserve folders as `::` namespaces (default). `false` flattens: the
    /// page title is just the note name.
    pub namespaces: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { namespaces: true }
    }
}

/// Image extensions Zorite can render (so an embed of one becomes an image,
/// not a transclusion downgrade).
const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tiff", "tif", "ico", "heic", "heif", "avif",
];

// --- Scanning ---

struct Note {
    /// Vault-relative path without the `.md` extension, `/`-separated.
    rel: String,
    /// The page title (namespaced or flat) — unused for daily notes.
    title: String,
    /// ISO `YYYY-MM-DD` when this is a daily note.
    day: Option<String>,
    path: PathBuf,
}

/// Read a vault into an [`ImportBundle`] (write it with
/// [`super::write_bundle`]).
pub fn read_vault(root: &Path, opts: &Options) -> Result<ImportBundle, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    let daily = daily_config(root);
    let mut md_files: Vec<PathBuf> = Vec::new();
    // Every non-md file, indexed by lowercase basename, for attachment lookup.
    let mut assets: HashMap<String, PathBuf> = HashMap::new();
    let mut canvas_files: Vec<PathBuf> = Vec::new();
    walk(root, &mut md_files, &mut canvas_files, &mut assets);
    if md_files.is_empty() {
        return Err(format!(
            "{} doesn't contain any Markdown notes",
            root.display()
        ));
    }

    // Build the notes list + the link-resolution maps.
    let mut notes: Vec<Note> = Vec::new();
    for path in md_files {
        let rel = rel_no_ext(root, &path);
        let stem = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        let day = daily_date(&rel, &stem, &daily);
        let title = if opts.namespaces {
            rel.replace('/', SEP)
        } else {
            stem.clone()
        };
        notes.push(Note {
            rel,
            title,
            day,
            path,
        });
    }
    // basename (lowercase) → titles that share it; and full path → title.
    let mut by_base: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_path: HashMap<String, String> = HashMap::new();
    for n in &notes {
        if n.day.is_some() {
            continue; // days aren't link targets
        }
        let base = n.rel.rsplit('/').next().unwrap_or(&n.rel).to_lowercase();
        by_base.entry(base).or_default().push(n.title.clone());
        by_path.insert(n.rel.to_lowercase(), n.title.clone());
    }

    let mut bundle = ImportBundle::default();
    let mut conv = Converter {
        root: root.to_path_buf(),
        opts,
        by_base,
        by_path,
        assets,
        copies: Vec::new(),
        warnings: Vec::new(),
    };
    for n in &notes {
        let raw = match std::fs::read_to_string(&n.path) {
            Ok(t) => t,
            Err(e) => {
                conv.warnings.push(format!("{}: {e}", n.path.display()));
                continue;
            }
        };
        let (aliases, tags, props, body) = split_frontmatter(&raw);
        let content = conv.convert(&body, tags, &props);
        match &n.day {
            Some(date) => bundle.days.push(ImportDay {
                date: date.clone(),
                content,
            }),
            None => bundle.pages.push(ImportPage {
                title: n.title.clone(),
                content,
                aliases,
                is_highlights: false,
            }),
        }
    }
    // Canvas boards → Zorite whiteboards (best-effort; see `convert_canvas`).
    for path in &canvas_files {
        let title = {
            let rel = rel_no_ext(root, path);
            if opts.namespaces {
                rel.replace('/', SEP)
            } else {
                rel.rsplit('/').next().unwrap_or(&rel).to_string()
            }
        };
        let Ok(json) = std::fs::read_to_string(path) else {
            conv.warnings
                .push(format!("{}: unreadable", path.display()));
            continue;
        };
        match convert_canvas(&json, &title, &mut conv) {
            Some(scene_json) => bundle
                .whiteboards
                .push(super::ImportWhiteboard { title, scene_json }),
            None => conv.warnings.push(format!(
                "canvas {}: malformed JSON, skipped",
                path.display()
            )),
        }
    }
    bundle.assets = conv
        .copies
        .into_iter()
        .map(|(src, managed)| AssetCopy { src, managed })
        .collect();
    bundle.warnings.extend(conv.warnings);
    Ok(bundle)
}

/// Recursively collect `.md` paths, `.canvas` boards, and index every other
/// file by basename. Skips Obsidian's config/trash dirs and dotfolders.
fn walk(
    dir: &Path,
    md: &mut Vec<PathBuf>,
    canvases: &mut Vec<PathBuf>,
    assets: &mut HashMap<String, PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // .obsidian, .trash, .git, …
        }
        if path.is_dir() {
            walk(&path, md, canvases, assets);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            md.push(path);
        } else if path.extension().and_then(|e| e.to_str()) == Some("canvas") {
            canvases.push(path);
        } else {
            assets.entry(name.to_lowercase()).or_insert(path);
        }
    }
}

/// Vault-relative path without the `.md` extension, forward-slashed.
fn rel_no_ext(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.with_extension("");
    s.to_string_lossy().replace('\\', "/")
}

// --- Daily notes ---

/// `(folder, )` from `.obsidian/daily-notes.json` — the format is ignored;
/// ISO `YYYY-MM-DD` basenames are what we recognize (Obsidian's default).
struct DailyConfig {
    folder: String,
}

fn daily_config(root: &Path) -> DailyConfig {
    let text = std::fs::read_to_string(root.join(".obsidian/daily-notes.json")).unwrap_or_default();
    // Cheap field extraction (avoid a JSON dep for one string).
    let folder = json_str_field(&text, "folder").unwrap_or_default();
    DailyConfig {
        folder: folder.trim_matches('/').to_string(),
    }
}

/// The ISO date if `rel`/`stem` is a daily note: an `YYYY-MM-DD` basename,
/// inside the configured daily folder when one is set.
fn daily_date(rel: &str, stem: &str, cfg: &DailyConfig) -> Option<String> {
    if !cfg.folder.is_empty() {
        let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        if dir != cfg.folder {
            return None;
        }
    }
    is_iso_date(stem).then(|| stem.to_string())
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..].iter().all(u8::is_ascii_digit)
}

/// The string value of a top-level `"key": "value"` in flat JSON.
fn json_str_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let after = &json[json.find(&needle)? + needle.len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    let rest = after.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// --- Frontmatter ---

/// `(aliases, tags, other props as (key,value), body)` — the split of a
/// note's leading YAML frontmatter from its body.
type Frontmatter = (Vec<String>, Vec<String>, Vec<(String, String)>, String);

/// Split leading YAML frontmatter from the body. Minimal YAML: scalar
/// `key: v`, flow list `key: [a, b]`, and block lists (`  - item` lines).
fn split_frontmatter(raw: &str) -> Frontmatter {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return (Vec::new(), Vec::new(), Vec::new(), raw.to_string());
    };
    let Some(end) = rest.find("\n---") else {
        return (Vec::new(), Vec::new(), Vec::new(), raw.to_string());
    };
    let yaml = &rest[..end];
    // Body starts after the closing fence's line.
    let body_start = end + "\n---".len();
    let body = rest[body_start..]
        .trim_start_matches(['\n', '\r'])
        .to_string();

    let (mut aliases, mut tags, mut props) = (Vec::new(), Vec::new(), Vec::new());
    let lines: Vec<&str> = yaml.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        // Gather a block list that follows an empty-value key.
        let mut items: Vec<String> = Vec::new();
        if value.is_empty() {
            while i < lines.len() {
                let item = lines[i].trim();
                if let Some(v) = item.strip_prefix("- ") {
                    items.push(unquote(v.trim()).to_string());
                    i += 1;
                } else {
                    break;
                }
            }
        } else if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            items.extend(inner.split(',').map(|v| unquote(v.trim()).to_string()));
        } else {
            items.push(unquote(value).to_string());
        }
        let items: Vec<String> = items.into_iter().filter(|s| !s.is_empty()).collect();
        match key {
            "alias" | "aliases" => aliases.extend(items),
            "tag" | "tags" => tags.extend(items),
            _ if !items.is_empty() => props.push((key.to_string(), items.join(", "))),
            _ => {}
        }
    }
    (aliases, tags, props, body)
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(s)
}

// --- Conversion ---

struct Converter<'a> {
    root: PathBuf,
    opts: &'a Options,
    by_base: HashMap<String, Vec<String>>,
    by_path: HashMap<String, String>,
    assets: HashMap<String, PathBuf>,
    copies: Vec<(PathBuf, String)>,
    warnings: Vec<String>,
}

impl Converter<'_> {
    fn convert(&mut self, body: &str, tags: Vec<String>, props: &[(String, String)]) -> String {
        // Prepend other frontmatter props as `key:: value` lines (like the
        // Logseq importer preserves non-internal properties).
        let mut out = String::new();
        for (k, v) in props {
            out.push_str(&format!("{k}:: {v}\n"));
        }
        if !props.is_empty() {
            out.push('\n');
        }

        let mut in_fence = false;
        for (i, line) in body.split('\n').enumerate() {
            if i > 0 {
                out.push('\n');
            }
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                out.push_str(line);
                continue;
            }
            if in_fence {
                out.push_str(line); // code is verbatim
                continue;
            }
            out.push_str(&self.convert_line(line));
        }

        // Zorite renders an image only when it LEADS a line (block object)
        // and a `![[…]]` embed only when it's ALONE on one; an Obsidian embed
        // mid-sentence would otherwise vanish. Break either onto its own line.
        out = promote_images(&out);

        // Hoist frontmatter tags into the body as `#tags` (nested ones keep
        // their `/`).
        if !tags.is_empty() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            let hashed: Vec<String> = tags
                .iter()
                .map(|t| format!("#{}", t.trim_start_matches('#')))
                .collect();
            out.push('\n');
            out.push_str(&hashed.join(" "));
        }
        out.trim_end().to_string()
    }

    fn convert_line(&mut self, line: &str) -> String {
        // Callout header: `> [!type]` (optionally `> [!type]- Title`).
        if let Some(conv) = convert_callout(line) {
            return conv;
        }
        let line = strip_comments(line);
        let line = convert_highlights(&line);
        self.convert_embeds_and_links(&line)
    }

    /// Handle `![[embed]]`, `![](path)` images, and `[[wiki-links]]` in one
    /// left-to-right scan.
    fn convert_embeds_and_links(&mut self, line: &str) -> String {
        let b = line.as_bytes();
        let mut out = String::with_capacity(line.len());
        let mut i = 0;
        while i < line.len() {
            // Embed: ![[target]]
            if b[i] == b'!'
                && line[i + 1..].starts_with("[[")
                && let Some(close) = line[i + 3..].find("]]")
            {
                let inner = &line[i + 3..i + 3 + close];
                out.push_str(&self.embed(inner));
                i += 3 + close + 2;
                continue;
            }
            // Markdown image: ![alt](path)
            if b[i] == b'!'
                && line[i + 1..].starts_with('[')
                && let Some(rb) = line[i + 2..].find(']')
                && line[i + 2 + rb + 1..].starts_with('(')
                && let Some(rp) = line[i + 2 + rb + 2..].find(')')
            {
                let alt = &line[i + 2..i + 2 + rb];
                let path = &line[i + 2 + rb + 2..i + 2 + rb + 2 + rp];
                out.push_str(&self.markdown_image(alt, path));
                i += 2 + rb + 2 + rp + 1;
                continue;
            }
            // Wiki link: [[target(#anchor)(|alias)]]
            if b[i] == b'['
                && line[i + 1..].starts_with('[')
                && let Some(close) = line[i + 2..].find("]]")
            {
                let inner = &line[i + 2..i + 2 + close];
                out.push_str(&self.wiki_link(inner));
                i += 2 + close + 2;
                continue;
            }
            out.push(b[i] as char);
            // Advance by the full UTF-8 char, not one byte.
            let ch_len = line[i..].chars().next().map_or(1, char::len_utf8);
            if ch_len > 1 {
                out.pop();
                out.push_str(&line[i..i + ch_len]);
            }
            i += ch_len;
        }
        out
    }

    /// `![[target]]` — an image/PDF asset, or a note transclusion kept as a
    /// real Zorite embed (`![[Title]]` / `![[Title#Heading]]` / `![[Title#^id]]`,
    /// an `|alias` renaming the label).
    fn embed(&mut self, inner: &str) -> String {
        let (target, alias) = split_alias(inner);
        let (name, anchor) = split_anchor(target);
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        if IMAGE_EXTS.contains(&ext.as_str()) {
            if let Some(managed) = self.copy_asset(name, "images") {
                return format!("![]({managed})");
            }
            self.warnings
                .push(format!("missing embedded image: {name}"));
            return format!("![]({name})");
        }
        if ext == "pdf" {
            if let Some(managed) = self.copy_asset(name, "pdf") {
                return format!("[[{managed}]]");
            }
            self.warnings.push(format!("missing embedded PDF: {name}"));
            return format!("[[{name}]]");
        }
        // A note transclusion → a Zorite embed (promoted onto its own line
        // later — Zorite renders only standalone `![[…]]` lines).
        let title = self.resolve(name.trim());
        let full = match anchor {
            Some(a) => format!("{title}#{}", flatten_anchor(a)),
            None => title,
        };
        match alias {
            Some(a) => format!("![[{full}|{a}]]"),
            None => format!("![[{full}]]"),
        }
    }

    fn markdown_image(&mut self, alt: &str, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            return format!("![{alt}]({path})");
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        let dir = if ext == "pdf" { "pdf" } else { "images" };
        if let Some(managed) = self.copy_asset(name, dir) {
            if dir == "pdf" {
                return format!("[[{managed}]]");
            }
            return format!("![{alt}]({managed})");
        }
        format!("![{alt}]({path})")
    }

    /// `[[target(#anchor)(|alias)]]` → a resolved Zorite wiki-link. `#Heading`
    /// and `#^block` anchors are kept — Zorite jumps to them.
    fn wiki_link(&mut self, inner: &str) -> String {
        let (target, alias) = split_alias(inner);
        let (name, anchor) = split_anchor(target);
        let title = self.resolve(name.trim());
        let full = match anchor {
            Some(a) => format!("{title}#{}", flatten_anchor(a)),
            None => title.clone(),
        };
        match alias {
            Some(a) => format!("[[{full}|{a}]]"),
            // A namespaced resolution shows the original bare name as the
            // alias — unless an anchor is present (Zorite then renders the
            // `Title → anchor` display itself, which an alias would hide).
            None if title != name.trim() && anchor.is_none() => {
                format!("[[{full}|{}]]", name.trim())
            }
            None => format!("[[{full}]]"),
        }
    }

    /// A link target name → its namespaced page title.
    fn resolve(&mut self, name: &str) -> String {
        let name = name.trim_end_matches(".md");
        if !self.opts.namespaces {
            return name.rsplit('/').next().unwrap_or(name).to_string();
        }
        // A path-qualified link (`Projects/Tasks`) → its exact title.
        if name.contains('/') {
            let key = name.to_lowercase();
            if let Some(title) = self.by_path.get(&key) {
                return title.clone();
            }
            return name.replace('/', SEP); // unknown, best-effort
        }
        // A bare name → unique basename match, else left as-is.
        match self.by_base.get(&name.to_lowercase()) {
            Some(titles) if titles.len() == 1 => titles[0].clone(),
            Some(_) => {
                self.warnings
                    .push(format!("ambiguous link [[{name}]] — left un-namespaced"));
                name.to_string()
            }
            None => name.to_string(),
        }
    }

    /// Copy an attachment (resolved by basename anywhere in the vault) into
    /// the managed store; returns its `<dir>/<name>` ref.
    fn copy_asset(&mut self, name: &str, dir: &str) -> Option<String> {
        let base = name.rsplit('/').next().unwrap_or(name);
        let src = self.assets.get(&base.to_lowercase())?.clone();
        let managed = format!("{dir}/{base}");
        if !self.copies.iter().any(|(_, m)| m == &managed) {
            self.copies.push((src, managed.clone()));
        }
        let _ = &self.root; // (kept for symmetry with logseq's Converter)
        Some(managed)
    }
}

// --- Canvas (`.canvas` → Zorite whiteboard) ---

/// One JSON-Canvas node (<https://jsoncanvas.org>). Unknown fields ignored.
#[derive(serde::Deserialize)]
struct CanvasNode {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    #[serde(default)]
    width: f32,
    #[serde(default)]
    height: f32,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

/// One JSON-Canvas edge. Unknown fields ignored.
#[derive(serde::Deserialize)]
struct CanvasEdge {
    #[serde(rename = "fromNode", default)]
    from_node: String,
    #[serde(rename = "fromSide", default)]
    from_side: Option<String>,
    #[serde(rename = "fromEnd", default)]
    from_end: Option<String>,
    #[serde(rename = "toNode", default)]
    to_node: String,
    #[serde(rename = "toSide", default)]
    to_side: Option<String>,
    #[serde(rename = "toEnd", default)]
    to_end: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

#[derive(serde::Deserialize)]
struct CanvasData {
    #[serde(default)]
    nodes: Vec<CanvasNode>,
    #[serde(default)]
    edges: Vec<CanvasEdge>,
}

/// Convert one JSON-Canvas board into a Zorite whiteboard scene, best-effort:
///
/// - text cards → rounded, labeled boxes (markdown inside shows literally);
/// - note-file cards → page cards (their `page_id` is a placeholder the writer
///   resolves once pages exist — see `write_bundle`);
/// - image-file cards → placed images (asset copied); other files → a labeled
///   box naming the file;
/// - link cards → a labeled box showing the URL;
/// - groups → unfilled outlines painted below everything;
/// - edges → arrows/lines between node-side midpoints, labels as midpoint text.
///
/// `None` = unparseable JSON (the caller warns and skips the board).
fn convert_canvas(json: &str, title: &str, conv: &mut Converter) -> Option<String> {
    let data: CanvasData = serde_json::from_str(json).ok()?;
    let mut next_id = 1u64;
    let mut id = || {
        next_id += 1;
        next_id - 1
    };
    // Node id → bounds, for edge endpoints.
    let bounds: HashMap<&str, (f32, f32, f32, f32)> = data
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), (n.x, n.y, n.width, n.height)))
        .collect();

    // Paint order: groups (backdrops) first, then edges, then cards.
    let mut groups: Vec<Element> = Vec::new();
    let mut edges: Vec<Element> = Vec::new();
    let mut cards: Vec<Element> = Vec::new();
    let mut skipped = 0usize;

    for n in &data.nodes {
        let color = n.color.as_deref().and_then(canvas_color);
        let geom = BoxGeom {
            x: n.x,
            y: n.y,
            w: n.width,
            h: n.height,
            width: 2.0,
            rotation: 0.0,
        };
        let boxed = |kind: ElementKind, label: Option<String>, id: u64| Element {
            id,
            kind,
            stroke: color,
            // A translucent tint like Obsidian's cards; `None` stays outline.
            fill: color.map(|c| (c & 0xFFFF_FF00) | 0x22),
            label,
            label_color: color,
            styles: Vec::new(),
            mindmap: None,
        };
        match n.kind.as_str() {
            "text" => {
                let text = n.text.clone().unwrap_or_default();
                cards.push(boxed(ElementKind::RoundRect(geom), Some(text), id()));
            }
            "file" => {
                let file = n.file.clone().unwrap_or_default();
                let base = file.rsplit('/').next().unwrap_or(&file).to_string();
                let ext = base.rsplit('.').next().unwrap_or("").to_lowercase();
                if IMAGE_EXTS.contains(&ext.as_str()) {
                    if let Some(managed) = conv.copy_asset(&base, "images") {
                        cards.push(Element {
                            id: id(),
                            kind: ElementKind::Image(ImageGeom {
                                src: managed,
                                x: n.x,
                                y: n.y,
                                w: n.width,
                                h: n.height,
                                rotation: 0.0,
                            }),
                            stroke: None,
                            fill: None,
                            label: None,
                            label_color: None,
                            styles: Vec::new(),
                            mindmap: None,
                        });
                    } else {
                        conv.warnings
                            .push(format!("canvas \"{title}\": missing image {base}"));
                        cards.push(boxed(ElementKind::RoundRect(geom), Some(base), id()));
                    }
                } else if ext == "md" || !base.contains('.') {
                    // A note card → a real page card; the writer fills the
                    // page id in once pages exist (placeholder 0).
                    let note = conv.resolve(file.trim_end_matches(".md"));
                    cards.push(Element {
                        id: id(),
                        kind: ElementKind::Embed(EmbedGeom {
                            page_id: 0,
                            title: note,
                            x: n.x,
                            y: n.y,
                            w: n.width,
                            h: n.height,
                        }),
                        stroke: color,
                        fill: None,
                        label: None,
                        label_color: None,
                        styles: Vec::new(),
                        mindmap: None,
                    });
                } else {
                    // PDFs, nested .canvas boards, audio, … — a named box.
                    skipped += 1;
                    cards.push(boxed(ElementKind::RoundRect(geom), Some(base), id()));
                }
            }
            "link" => {
                let url = n.url.clone().unwrap_or_default();
                cards.push(boxed(ElementKind::RoundRect(geom), Some(url), id()));
            }
            "group" => {
                groups.push(Element {
                    id: id(),
                    kind: ElementKind::Rect(geom),
                    stroke: color,
                    fill: None,
                    label: n.label.clone(),
                    label_color: color,
                    styles: Vec::new(),
                    mindmap: None,
                });
            }
            other => {
                skipped += 1;
                conv.warnings.push(format!(
                    "canvas \"{title}\": unsupported node type '{other}'"
                ));
            }
        }
    }

    for e in &data.edges {
        let (Some(from), Some(to)) = (
            bounds.get(e.from_node.as_str()),
            bounds.get(e.to_node.as_str()),
        ) else {
            skipped += 1;
            continue;
        };
        let (x1, y1) = side_point(*from, e.from_side.as_deref(), *to);
        let (x2, y2) = side_point(*to, e.to_side.as_deref(), *from);
        // Zorite arrows are single-headed (at the segment's end). Obsidian's
        // default is an arrowhead at `to`; a from-only arrow flips direction.
        let color = e.color.as_deref().and_then(canvas_color);
        let to_arrow = e.to_end.as_deref() != Some("none");
        let from_arrow = e.from_end.as_deref() == Some("arrow");
        let seg = if to_arrow || !from_arrow {
            SegGeom {
                x1,
                y1,
                x2,
                y2,
                width: 2.0,
                style: SegmentStyle::Solid,
                start_anchor: None,
                end_anchor: None,
            }
        } else {
            SegGeom {
                x1: x2,
                y1: y2,
                x2: x1,
                y2: y1,
                width: 2.0,
                style: SegmentStyle::Solid,
                start_anchor: None,
                end_anchor: None,
            }
        };
        let kind = if to_arrow || from_arrow {
            ElementKind::Arrow(seg)
        } else {
            ElementKind::Line(seg)
        };
        edges.push(Element {
            id: id(),
            kind,
            stroke: color,
            fill: None,
            label: None,
            label_color: None,
            styles: Vec::new(),
            mindmap: None,
        });
        if let Some(label) = e.label.clone().filter(|l| !l.trim().is_empty()) {
            edges.push(Element {
                id: id(),
                kind: ElementKind::Text(TextGeom {
                    x: (x1 + x2) / 2.0,
                    y: (y1 + y2) / 2.0,
                    content: label,
                    size: 14.0,
                    rotation: 0.0,
                    measured_w: 0.0,
                    measured_h: 0.0,
                }),
                stroke: color,
                fill: None,
                label: None,
                label_color: None,
                styles: Vec::new(),
                mindmap: None,
            });
        }
    }

    if skipped > 0 {
        conv.warnings.push(format!(
            "canvas \"{title}\": {skipped} item(s) downgraded or skipped"
        ));
    }
    let mut elements = groups;
    elements.extend(edges);
    elements.extend(cards);
    // Open on the content: camera at the bounding box's top-left, padded.
    let (mut cam_x, mut cam_y) = (0.0f32, 0.0f32);
    if let Some(min_x) = data.nodes.iter().map(|n| n.x).reduce(f32::min) {
        let min_y = data
            .nodes
            .iter()
            .map(|n| n.y)
            .reduce(f32::min)
            .unwrap_or(0.0);
        cam_x = min_x - 60.0;
        cam_y = min_y - 60.0;
    }
    let scene = Scene {
        camera: gpui_whiteboard::Camera {
            x: cam_x,
            y: cam_y,
            zoom: 1.0,
        },
        elements,
    };
    Some(scene.to_json())
}

/// The midpoint of a node side for an edge endpoint. A missing side picks the
/// one facing the `other` node's center (Obsidian omits sides on auto-routed
/// edges).
fn side_point(
    (x, y, w, h): (f32, f32, f32, f32),
    side: Option<&str>,
    other: (f32, f32, f32, f32),
) -> (f32, f32) {
    let side = match side {
        Some(s) => s.to_string(),
        None => {
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            let (ox, oy) = (other.0 + other.2 / 2.0, other.1 + other.3 / 2.0);
            let (dx, dy) = (ox - cx, oy - cy);
            if dx.abs() > dy.abs() {
                if dx > 0.0 { "right" } else { "left" }.to_string()
            } else if dy > 0.0 {
                "bottom".to_string()
            } else {
                "top".to_string()
            }
        }
    };
    match side.as_str() {
        "top" => (x + w / 2.0, y),
        "bottom" => (x + w / 2.0, y + h),
        "left" => (x, y + h / 2.0),
        _ => (x + w, y + h / 2.0),
    }
}

/// A canvas color — a preset digit (`"1"`–`"6"`, Obsidian's palette) or a
/// `#RRGGBB` hex — as Zorite's packed `0xRRGGBBAA`.
fn canvas_color(c: &str) -> Option<u32> {
    match c {
        "1" => Some(0xFB46_4CFF), // red
        "2" => Some(0xE997_3FFF), // orange
        "3" => Some(0xE0DE_71FF), // yellow
        "4" => Some(0x44CF_6EFF), // green
        "5" => Some(0x53DF_DDFF), // cyan
        "6" => Some(0xA882_FFFF), // purple
        hex => {
            let hex = hex.strip_prefix('#')?;
            if hex.len() != 6 {
                return None;
            }
            u32::from_str_radix(hex, 16).ok().map(|v| (v << 8) | 0xFF)
        }
    }
}

/// `> [!type] title` → `> [!ZORITE] title`. `None` if the line isn't a
/// callout header.
fn convert_callout(line: &str) -> Option<String> {
    let after = line.trim_start().strip_prefix('>')?.trim_start();
    let rest = after.strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = rest[..close].trim().to_lowercase();
    // Obsidian allows a fold marker (`]-`/`]+`) and an optional title after.
    let tail = rest[close + 1..].trim_start_matches(['-', '+']);
    let mapped = map_callout(&kind);
    let prefix = &line[..line.len() - after.len()]; // keep leading `> ` / indent
    Some(format!("{prefix}[!{mapped}]{tail}"))
}

/// Obsidian's ~13 callout types → Zorite's five alerts.
fn map_callout(kind: &str) -> &'static str {
    match kind {
        "tip" | "hint" | "success" | "check" | "done" => "TIP",
        "important" | "abstract" | "summary" | "tldr" => "IMPORTANT",
        "warning" | "attention" | "question" | "help" | "faq" => "WARNING",
        "caution" | "danger" | "error" | "failure" | "fail" | "missing" | "bug" => "CAUTION",
        // note, info, todo, example, quote, cite, and anything unknown.
        _ => "NOTE",
    }
}

/// Drop `%%comment%%` spans (Obsidian comments). Handles inline spans; a lone
/// `%%` toggling across lines is left as-is (rare).
fn strip_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find("%%") {
        out.push_str(&rest[..open]);
        match rest[open + 2..].find("%%") {
            Some(close) => rest = &rest[open + 2 + close + 2..],
            None => {
                out.push_str(&rest[open..]); // unterminated — keep it
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `==text==` → `<mark>text</mark>` (skips `====` and empty spans).
fn convert_highlights(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find("==") {
        let before = &rest[..open];
        let after = &rest[open + 2..];
        if let Some(close) = after.find("==")
            && close > 0
        {
            out.push_str(before);
            out.push_str("<mark>");
            out.push_str(&after[..close]);
            out.push_str("</mark>");
            rest = &after[close + 2..];
        } else {
            out.push_str(before);
            out.push_str("==");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Split a wiki target into `(name, Some(alias))` on the first `|`.
fn split_alias(inner: &str) -> (&str, Option<&str>) {
    match inner.split_once('|') {
        Some((t, a)) => (t.trim(), Some(a.trim())),
        None => (inner.trim(), None),
    }
}

/// Split a wiki target into `(name, Some(anchor))` on the first `#`.
fn split_anchor(target: &str) -> (&str, Option<&str>) {
    match target.split_once('#') {
        Some((n, a)) => (n.trim(), Some(a.trim())),
        None => (target.trim(), None),
    }
}

/// A nested Obsidian heading path (`H1#H2`) keeps only its last segment — the
/// deepest heading is what Zorite can jump to. Block anchors (`^id`) pass
/// through whole.
fn flatten_anchor(a: &str) -> &str {
    if a.starts_with('^') {
        a
    } else {
        a.rsplit('#').next().unwrap_or(a)
    }
}

/// Put any image or `![[…]]` embed that shares a line with other text on its
/// own line (a blank line before and after) so Zorite renders it as a block.
/// Lines that are already just an image (optionally with a `{width=N}` tail)
/// are left alone.
fn promote_images(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    for (i, line) in body.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence || !line_has_mixed_image(line) {
            out.push_str(line);
            continue;
        }
        // Split into alternating text / image blocks, each on its own line,
        // blank-separated so the image leads its paragraph.
        for (j, seg) in split_images(line).into_iter().enumerate() {
            if seg.trim().is_empty() {
                continue;
            }
            if j > 0 {
                out.push_str("\n\n");
            }
            out.push_str(seg.trim());
        }
    }
    out
}

/// Whether `line` contains an image AND other non-whitespace content.
fn line_has_mixed_image(line: &str) -> bool {
    let spans = image_spans(line);
    if spans.is_empty() {
        return false;
    }
    let covered: usize = spans.iter().map(|r| r.len()).sum();
    line.trim().len() > covered
}

/// Split a line at image boundaries: `["text ", "![](a)", " more"]`.
fn split_images(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut last = 0;
    for r in image_spans(line) {
        if r.start > last {
            out.push(&line[last..r.start]);
        }
        out.push(&line[r.clone()]);
        last = r.end;
    }
    if last < line.len() {
        out.push(&line[last..]);
    }
    out
}

/// Byte ranges of every `![alt](url)` image and `![[…]]` note embed in `line`.
fn image_spans(line: &str) -> Vec<std::ops::Range<usize>> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'!'
            && line[i + 1..].starts_with("[[")
            && let Some(close) = line[i + 3..].find("]]")
        {
            let end = i + 3 + close + 2;
            out.push(i..end);
            i = end;
            continue;
        }
        if b[i] == b'!'
            && b[i + 1] == b'['
            && let Some(rb) = line[i + 2..].find(']')
            && line[i + 2 + rb + 1..].starts_with('(')
            && let Some(rp) = line[i + 2 + rb + 2..].find(')')
        {
            let end = i + 2 + rb + 2 + rp + 1;
            out.push(i..end);
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv() -> Converter<'static> {
        // A converter with a small resolution map for link tests.
        static OPTS: Options = Options { namespaces: true };
        let mut by_base: HashMap<String, Vec<String>> = HashMap::new();
        by_base.insert("meeting notes".into(), vec!["Meeting Notes".into()]);
        by_base.insert(
            "tasks".into(),
            vec!["Projects::Tasks".into(), "Archive::Tasks".into()],
        );
        by_base.insert("roadmap".into(), vec!["Projects::Roadmap".into()]);
        let mut by_path: HashMap<String, String> = HashMap::new();
        by_path.insert("projects/tasks".into(), "Projects::Tasks".into());
        by_path.insert("archive/tasks".into(), "Archive::Tasks".into());
        Converter {
            root: PathBuf::new(),
            opts: &OPTS,
            by_base,
            by_path,
            assets: HashMap::new(),
            copies: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn canvas_converts_nodes_and_edges() {
        let mut c = conv();
        c.assets
            .insert("pic.png".into(), PathBuf::from("/vault/pic.png"));
        let json = r##"{
            "nodes": [
                {"id":"a","type":"text","x":0,"y":0,"width":200,"height":80,"text":"hello **md**","color":"4"},
                {"id":"b","type":"file","x":300,"y":0,"width":200,"height":80,"file":"Projects/Tasks.md"},
                {"id":"c","type":"file","x":0,"y":200,"width":120,"height":90,"file":"assets/pic.png"},
                {"id":"d","type":"link","x":300,"y":200,"width":200,"height":60,"url":"https://example.com","color":"#112233"},
                {"id":"g","type":"group","x":-40,"y":-40,"width":700,"height":420,"label":"Everything"},
                {"id":"z","type":"mystery","x":0,"y":0,"width":1,"height":1}
            ],
            "edges": [
                {"id":"e1","fromNode":"a","fromSide":"right","toNode":"b","toSide":"left","label":"leads to"},
                {"id":"e2","fromNode":"c","toNode":"d","toEnd":"none"}
            ]
        }"##;
        let scene_json = convert_canvas(json, "Board", &mut c).unwrap();
        let scene = Scene::from_json(&scene_json);

        // Group first (backdrop), then edges (+ label text), then cards.
        assert!(matches!(scene.elements[0].kind, ElementKind::Rect(_)));
        assert_eq!(scene.elements[0].label.as_deref(), Some("Everything"));
        let arrows = scene
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Arrow(_)))
            .count();
        let lines = scene
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Line(_)))
            .count();
        assert_eq!((arrows, lines), (1, 1)); // default toEnd=arrow; e2 explicit none
        // The edge label lands as midpoint text.
        assert!(
            scene
                .elements
                .iter()
                .any(|e| matches!(&e.kind, ElementKind::Text(t) if t.content == "leads to"))
        );
        // The e1 arrow runs right-side of a → left-side of b.
        let seg = scene
            .elements
            .iter()
            .find_map(|e| match &e.kind {
                ElementKind::Arrow(s) => Some(*s),
                _ => None,
            })
            .unwrap();
        assert_eq!((seg.x1, seg.y1, seg.x2, seg.y2), (200.0, 40.0, 300.0, 40.0));
        // Text card: rounded box, literal markdown label, preset-4 green.
        let card = scene
            .elements
            .iter()
            .find(|e| {
                matches!(e.kind, ElementKind::RoundRect(_))
                    && e.label.as_deref() == Some("hello **md**")
            })
            .unwrap();
        assert_eq!(card.stroke, Some(0x44CF_6EFF));
        // Note card: a page card with the placeholder id + resolved title.
        let embed = scene
            .elements
            .iter()
            .find_map(|e| match &e.kind {
                ElementKind::Embed(g) => Some(g.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(embed.page_id, 0);
        assert_eq!(embed.title, "Projects::Tasks");
        // Image card: copied into the managed store.
        assert!(
            scene
                .elements
                .iter()
                .any(|e| matches!(&e.kind, ElementKind::Image(g) if g.src == "images/pic.png"))
        );
        assert!(c.copies.iter().any(|(_, m)| m == "images/pic.png"));
        // Link card keeps its hex color; the mystery node warned.
        assert!(
            scene
                .elements
                .iter()
                .any(|e| e.label.as_deref() == Some("https://example.com")
                    && e.stroke == Some(0x1122_33FF))
        );
        assert!(c.warnings.iter().any(|w| w.contains("mystery")));
    }

    #[test]
    fn links_resolve_to_namespaces() {
        let mut c = conv();
        // Unique bare name → namespaced, original shown as the alias.
        assert_eq!(c.wiki_link("Meeting Notes"), "[[Meeting Notes]]");
        assert_eq!(c.wiki_link("Roadmap"), "[[Projects::Roadmap|Roadmap]]");
        // Path-qualified → exact title.
        assert_eq!(
            c.wiki_link("Projects/Tasks"),
            "[[Projects::Tasks|Projects/Tasks]]"
        );
        // Anchors kept: heading, block id, and a nested heading path (last
        // segment wins). An explicit alias still applies; a namespaced
        // resolution skips the auto-alias so Zorite's `Title → anchor` shows.
        assert_eq!(
            c.wiki_link("Meeting Notes#Heading|see"),
            "[[Meeting Notes#Heading|see]]"
        );
        assert_eq!(
            c.wiki_link("Meeting Notes#^decision1"),
            "[[Meeting Notes#^decision1]]"
        );
        assert_eq!(c.wiki_link("Meeting Notes#A#B"), "[[Meeting Notes#B]]");
        assert_eq!(c.wiki_link("Roadmap#Plan"), "[[Projects::Roadmap#Plan]]");
        // Ambiguous bare name → left as-is + warning.
        assert_eq!(c.wiki_link("Tasks"), "[[Tasks]]");
        assert!(c.warnings.iter().any(|w| w.contains("ambiguous")));
    }

    #[test]
    fn embeds_split_image_from_transclusion() {
        let mut c = conv();
        // No asset in the index → keeps the raw ref + warns.
        assert_eq!(c.embed("image.png"), "![](image.png)");
        assert!(
            c.warnings
                .iter()
                .any(|w| w.contains("missing embedded image"))
        );
        // A note embed stays a transclusion, resolved + anchor/alias kept.
        assert_eq!(c.embed("Meeting Notes"), "![[Meeting Notes]]");
        assert_eq!(
            c.embed("Roadmap#^goals|The goals"),
            "![[Projects::Roadmap#^goals|The goals]]"
        );
        assert_eq!(
            c.embed("Meeting Notes#Decisions"),
            "![[Meeting Notes#Decisions]]"
        );
    }

    #[test]
    fn callouts_map_and_keep_titles() {
        assert_eq!(convert_callout("> [!warning]").unwrap(), "> [!WARNING]");
        assert_eq!(
            convert_callout("> [!tip] Pro tip").unwrap(),
            "> [!TIP] Pro tip"
        );
        assert_eq!(convert_callout("> [!abstract]").unwrap(), "> [!IMPORTANT]");
        // Unknown type folds to NOTE; fold marker stripped.
        assert_eq!(
            convert_callout("> [!weird]- Folded").unwrap(),
            "> [!NOTE] Folded"
        );
        assert!(convert_callout("> plain quote").is_none());
    }

    #[test]
    fn highlights_and_comments() {
        assert_eq!(convert_highlights("a ==b== c"), "a <mark>b</mark> c");
        assert_eq!(convert_highlights("nope ==="), "nope ===");
        assert_eq!(strip_comments("keep %%drop this%% keep"), "keep  keep");
        assert_eq!(strip_comments("no comment here"), "no comment here");
    }

    #[test]
    fn frontmatter_extracts_aliases_tags_props() {
        let raw = "---\naliases:\n  - Standup\n  - Weekly Sync\ntags: [work, meetings/weekly]\nstatus: active\n---\n\n# Body\n";
        let (aliases, tags, props, body) = split_frontmatter(raw);
        assert_eq!(aliases, vec!["Standup", "Weekly Sync"]);
        assert_eq!(tags, vec!["work", "meetings/weekly"]);
        assert_eq!(props, vec![("status".to_string(), "active".to_string())]);
        assert_eq!(body, "# Body\n");
    }

    #[test]
    fn tags_hoisted_to_body() {
        let mut c = conv();
        let out = c.convert("# Note", vec!["work".into(), "meetings/weekly".into()], &[]);
        assert!(out.ends_with("#work #meetings/weekly"));
    }

    #[test]
    fn images_promoted_to_their_own_line() {
        // Mixed line → text / image / text as separate blocks.
        assert_eq!(
            promote_images("see ![](images/a.png) here"),
            "see\n\n![](images/a.png)\n\nhere"
        );
        // Already own-line → untouched.
        assert_eq!(promote_images("![](images/a.png)"), "![](images/a.png)");
        // A `{width=N}` caption tail stays with a lone image.
        assert_eq!(promote_images("no images"), "no images");
        // A mid-sentence `![[…]]` embed moves onto its own line too.
        assert_eq!(
            promote_images("see ![[Meeting Notes]] too"),
            "see\n\n![[Meeting Notes]]\n\ntoo"
        );
        assert_eq!(promote_images("![[Meeting Notes]]"), "![[Meeting Notes]]");
    }

    #[test]
    fn block_id_markers_kept() {
        let mut c = conv();
        // A trailing ` ^id` is an anchor target Zorite indexes — keep it.
        assert_eq!(
            c.convert_line("Decision here. ^decision1"),
            "Decision here. ^decision1"
        );
    }

    #[test]
    fn iso_date_detection() {
        assert!(is_iso_date("2026-07-01"));
        assert!(!is_iso_date("2026-7-1"));
        assert!(!is_iso_date("Meeting Notes"));
    }
}
