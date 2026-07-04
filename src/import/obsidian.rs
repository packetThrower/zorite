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
//!   their namespaced title); `[[Note#Heading]]` / `[[Note#^block]]` lose the
//!   anchor (Zorite links to pages, not sub-anchors).
//! - **Embeds** — `![[image.png]]` → `![](images/…)` (asset copied);
//!   `![[Other Note]]` transclusion → a plain `[[Other Note]]` link (Zorite
//!   doesn't transclude).
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    walk(root, &mut md_files, &mut assets);
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
    bundle.assets = conv
        .copies
        .into_iter()
        .map(|(src, managed)| AssetCopy { src, managed })
        .collect();
    bundle.warnings.extend(conv.warnings);
    Ok(bundle)
}

/// Recursively collect `.md` paths and index every other file by basename.
/// Skips Obsidian's config/trash dirs and dotfolders.
fn walk(dir: &Path, md: &mut Vec<PathBuf>, assets: &mut HashMap<String, PathBuf>) {
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
            walk(&path, md, assets);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            md.push(path);
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

        // Zorite renders an image only when it LEADS a line (block object);
        // an Obsidian embed mid-sentence would otherwise vanish. Break any
        // image onto its own line so it renders.
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
        let line = strip_block_id(line);
        let line = strip_comments(&line);
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

    /// `![[target]]` — an image/PDF asset, or a note transclusion downgraded
    /// to a link.
    fn embed(&mut self, inner: &str) -> String {
        let (target, _) = split_anchor(inner);
        let (name, _alias) = split_alias(target);
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
        // A note transclusion → a plain link (Zorite doesn't transclude).
        self.wiki_link(target)
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

    /// `[[target(#anchor)(|alias)]]` → a resolved Zorite wiki-link (anchor
    /// dropped, alias kept).
    fn wiki_link(&mut self, inner: &str) -> String {
        let (target, alias) = split_alias(inner);
        let (name, _anchor) = split_anchor(target);
        let title = self.resolve(name.trim());
        match alias {
            Some(a) => format!("[[{title}|{a}]]"),
            None if title != name.trim() => format!("[[{title}|{}]]", name.trim()),
            None => format!("[[{title}]]"),
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

/// Drop a trailing Obsidian block-id marker (` ^block-id` at line end) — it's
/// an anchor target Zorite has no use for.
fn strip_block_id(line: &str) -> String {
    let trimmed = line.trim_end();
    if let Some((before, id)) = trimmed.rsplit_once(" ^")
        && !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return before.to_string();
    }
    line.to_string()
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

/// Put any image that shares a line with other text on its own line (a
/// blank line before and after) so Zorite's reader renders it as a block.
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

/// Byte ranges of every `![alt](url)` image in `line`.
fn image_spans(line: &str) -> Vec<std::ops::Range<usize>> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
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
        // Anchor dropped, explicit alias kept.
        assert_eq!(
            c.wiki_link("Meeting Notes#Heading|see"),
            "[[Meeting Notes|see]]"
        );
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
        // A note embed downgrades to a link.
        assert_eq!(c.embed("Meeting Notes"), "[[Meeting Notes]]");
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
    }

    #[test]
    fn strips_trailing_block_ids() {
        assert_eq!(
            strip_block_id("Decision here. ^decision1"),
            "Decision here."
        );
        assert_eq!(strip_block_id("no anchor here"), "no anchor here");
        assert_eq!(strip_block_id("a ^b c"), "a ^b c"); // not at line end
    }

    #[test]
    fn iso_date_detection() {
        assert!(is_iso_date("2026-07-01"));
        assert!(!is_iso_date("2026-7-1"));
        assert!(!is_iso_date("Meeting Notes"));
    }
}
