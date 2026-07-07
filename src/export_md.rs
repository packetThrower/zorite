//! Export the notebook to a folder of **plain markdown + assets** — portable
//! to any app, not just Obsidian (File → Export → Notebook as Markdown…).
//!
//! The mirror of `import/`: a pure planner ([`plan_export`], unit-testable,
//! no filesystem) and a writer ([`write_export`]) that only ever writes into
//! an empty destination folder.
//!
//! Mapping (kept as portable as possible — Obsidian-flavored constructs pass
//! through only where they're the note-app lingua franca):
//! - `Foo::Bar` namespaces → `Foo/Bar.md` folders, with `[[Foo::Bar]]` /
//!   `![[Foo::Bar]]` targets rewritten to the path form (anchors + `|alias`
//!   preserved; fenced code untouched).
//! - Journal days → `journals/YYYY-MM-DD.md` — the date filename is the
//!   interop key our own importer and Obsidian's daily notes both read.
//! - Aliases → YAML frontmatter `aliases:` (links *via* an alias are left
//!   as written; the frontmatter lets alias-aware apps resolve them).
//! - `#tags`, `key:: value` properties, callouts, `^block-ids`,
//!   `[[Note#Heading]]` / `[[Note#^id]]`, `![[embeds]]`, `$…$` math, mermaid:
//!   passthrough. `<mark>` and `{width=N}` also pass through — HTML and
//!   Pandoc-style attributes are more portable than any app dialect.
//! - Referenced `images/…` and `pdf/…` files copy to same-named folders at
//!   the export root, so relative references keep working.
//! - Whiteboards are skipped (warned) — no portable format yet.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::db::ExportPage;

/// What [`plan_export`] produces: files to write (export-relative path →
/// body), asset references to copy (data-dir-relative, e.g. `images/x.png`),
/// and human-readable warnings for the summary dialog.
pub struct ExportPlan {
    pub files: Vec<(PathBuf, String)>,
    pub assets: BTreeSet<String>,
    pub warnings: Vec<String>,
    pub pages: usize,
    pub days: usize,
}

/// Counts for the completion dialog.
pub struct ExportSummary {
    pub pages: usize,
    pub days: usize,
    pub assets: usize,
    pub warnings: Vec<String>,
}

/// Lay the whole notebook out as files — pure (no I/O).
pub fn plan_export(pages: &[ExportPage]) -> ExportPlan {
    let mut plan = ExportPlan {
        files: Vec::new(),
        assets: BTreeSet::new(),
        warnings: Vec::new(),
        pages: 0,
        days: 0,
    };

    // Pass 1: assign every page a path, and build the link-rewrite map
    // (lowercased title → export link target). Journal days keep their date
    // name; whiteboards are skipped; everything else maps `::` → folders with
    // sanitized, case-insensitively uniquified segments.
    let mut used: HashSet<String> = HashSet::new(); // lowercased paths
    let mut targets: HashMap<String, String> = HashMap::new(); // title(lc) → link target
    let mut placed: Vec<(&ExportPage, PathBuf)> = Vec::new();
    let mut whiteboards = 0usize;
    for page in pages {
        if page.kind == "whiteboard" {
            whiteboards += 1;
            continue;
        }
        let rel = if let Some(date) = &page.journal_date {
            PathBuf::from("journals").join(format!("{}.md", sanitize_segment(date)))
        } else {
            let mut segs: Vec<String> = page
                .title
                .split("::")
                .map(sanitize_segment)
                .filter(|s| !s.is_empty())
                .collect();
            if segs.is_empty() {
                segs.push("untitled".into());
            }
            let mut rel: PathBuf = segs.iter().take(segs.len() - 1).collect();
            rel.push(format!("{}.md", segs.last().expect("non-empty")));
            rel
        };
        let rel = uniquify(rel, &mut used);
        // The link target: the path without `.md`, `/`-separated.
        let target = rel
            .with_extension("")
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        targets.insert(page.title.to_lowercase(), target);
        placed.push((page, rel));
    }
    if whiteboards > 0 {
        plan.warnings.push(format!(
            "{whiteboards} whiteboard{} skipped — no portable whiteboard format yet",
            if whiteboards == 1 { "" } else { "s" }
        ));
    }

    // Pass 2: emit each file — frontmatter aliases + link-rewritten body —
    // and collect its asset references.
    for (page, rel) in placed {
        let body = rewrite_links(&page.content, &targets);
        collect_assets(&page.content, &mut plan.assets);
        let text = if page.aliases.is_empty() {
            body
        } else {
            let mut fm = String::from("---\naliases:\n");
            for a in &page.aliases {
                fm.push_str(&format!("  - {}\n", yaml_quote(a)));
            }
            fm.push_str("---\n\n");
            fm + &body
        };
        if page.journal_date.is_some() {
            plan.days += 1;
        } else {
            plan.pages += 1;
        }
        plan.files.push((rel, text));
    }
    plan
}

/// Write a plan into `dest`, which must exist and be empty (never merge into
/// someone's real folder). `data_dir` is where `images/` / `pdf/` live.
pub fn write_export(
    data_dir: &Path,
    dest: &Path,
    plan: ExportPlan,
) -> Result<ExportSummary, String> {
    if !dest.is_dir() {
        return Err("The chosen destination isn't a folder.".into());
    }
    let occupied = std::fs::read_dir(dest)
        .map_err(|e| format!("read destination: {e}"))?
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy() != ".DS_Store");
    if occupied {
        return Err("The destination folder isn't empty — pick (or create) an empty one.".into());
    }

    let mut warnings = plan.warnings;
    for (rel, text) in &plan.files {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {parent:?}: {e}"))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("write {rel:?}: {e}"))?;
    }
    let mut copied = 0usize;
    for rel in &plan.assets {
        let src = data_dir.join(rel);
        if !src.is_file() {
            warnings.push(format!("missing asset: {rel}"));
            continue;
        }
        let dst = dest.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {parent:?}: {e}"))?;
        }
        std::fs::copy(&src, &dst).map_err(|e| format!("copy {rel}: {e}"))?;
        copied += 1;
    }
    Ok(ExportSummary {
        pages: plan.pages,
        days: plan.days,
        assets: copied,
        warnings,
    })
}

/// Rewrite `[[target]]` / `![[target]]` page targets to their export paths
/// (`A::B` → `A/B`, sanitized/uniquified), preserving `#Heading` / `#^id`
/// anchors and `|alias` labels. Unknown targets — pdf chips, aliases, missing
/// pages — pass through unchanged. Fenced code is untouched.
fn rewrite_links(content: &str, targets: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(content.len());
    let mut in_fence = false;
    for (i, line) in content.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        out.push_str(&rewrite_line(line, targets));
    }
    out
}

fn rewrite_line(line: &str, targets: &HashMap<String, String>) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < line.len() {
        // `[[…]]` (and the `![[…]]` embed form — the `!` passes through just
        // before the brackets match).
        if b[i] == b'['
            && line[i + 1..].starts_with('[')
            && let Some(close) = line[i + 2..].find("]]")
        {
            let inner = &line[i + 2..i + 2 + close];
            out.push_str("[[");
            out.push_str(&rewrite_target(inner, targets));
            out.push_str("]]");
            i += 2 + close + 2;
            continue;
        }
        let ch_len = line[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&line[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Rewrite one wiki inner text (`target(#anchor)(|alias)`), or return it
/// unchanged when the page part isn't a known exported title.
fn rewrite_target(inner: &str, targets: &HashMap<String, String>) -> String {
    let (target, alias) = match inner.split_once('|') {
        Some((t, a)) => (t.trim(), Some(a)),
        None => (inner.trim(), None),
    };
    // Block anchor first (`#^` is unambiguous), then heading (guards `.pdf`).
    let (page, anchor) = match gpui_markdown::syntax::split_block_anchor(target) {
        (p, Some(id)) => (p, Some(format!("#^{id}"))),
        _ => match gpui_markdown::syntax::split_heading_anchor(target) {
            (p, Some(h)) => (p, Some(format!("#{h}"))),
            (p, None) => (p, None),
        },
    };
    let Some(mapped) = targets.get(&page.trim().to_lowercase()) else {
        return inner.to_string();
    };
    let mut out = mapped.clone();
    if let Some(a) = anchor {
        out.push_str(&a);
    }
    if let Some(a) = alias {
        out.push('|');
        out.push_str(a);
    }
    out
}

/// Collect the `images/…` and `pdf/…` references in `content` (markdown
/// images — block and inline — plus `[[pdf/…]]` chips).
fn collect_assets(content: &str, out: &mut BTreeSet<String>) {
    for src in gpui_markdown::all_image_srcs(content) {
        let s = src.to_string();
        if s.starts_with("images/") || s.starts_with("pdf/") {
            out.insert(s);
        }
    }
    let mut in_fence = false;
    for line in content.split('\n') {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for (_, hit) in gpui_markdown::syntax::links(line) {
            if let gpui_markdown::syntax::LinkHit::Page(t) = hit {
                // A pdf chip's target may carry a `#pN` page anchor.
                let t = t.split('#').next().unwrap_or(&t);
                if t.starts_with("pdf/") || t.starts_with("images/") {
                    out.insert(t.to_string());
                }
            }
        }
    }
}

/// One path segment, made safe for every filesystem: path separators and
/// Windows-reserved punctuation become `-`, control chars drop, ends are
/// trimmed of dots/spaces (Windows), reserved device names get a suffix.
fn sanitize_segment(seg: &str) -> String {
    let mut out: String = seg
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']' => '-',
            c => c,
        })
        .collect();
    out = out.trim().trim_matches('.').trim().to_string();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&out.to_ascii_uppercase().as_str()) {
        out.push('-');
    }
    out
}

/// Case-insensitively uniquify `rel` against `used` by suffixing the file
/// stem with ` 2`, ` 3`, … (macOS/Windows filesystems fold case).
fn uniquify(rel: PathBuf, used: &mut HashSet<String>) -> PathBuf {
    let key = |p: &Path| p.to_string_lossy().to_lowercase();
    if used.insert(key(&rel)) {
        return rel;
    }
    let stem = rel.file_stem().unwrap_or_default().to_string_lossy();
    for n in 2.. {
        let candidate = rel.with_file_name(format!("{stem} {n}.md"));
        if used.insert(key(&candidate)) {
            return candidate;
        }
    }
    unreachable!()
}

/// Quote a YAML scalar defensively (double-quoted with `\` / `"` escaped) —
/// aliases are arbitrary user text.
fn yaml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(title: &str, content: &str) -> ExportPage {
        ExportPage {
            title: title.into(),
            content: content.into(),
            journal_date: None,
            kind: "page".into(),
            aliases: Vec::new(),
        }
    }

    #[test]
    fn namespaces_become_folders_and_links_follow() {
        let pages = vec![
            page("Projects::Tasks", "back to [[Home]]"),
            page(
                "Home",
                "see [[Projects::Tasks]], [[projects::tasks|the list]], \
                 [[Projects::Tasks#^id1]], and ![[Projects::Tasks#Plan]]\n\
                 ```\n[[Projects::Tasks]] stays raw in code\n```",
            ),
        ];
        let plan = plan_export(&pages);
        let paths: Vec<String> = plan
            .files
            .iter()
            .map(|(p, _)| p.to_string_lossy().into_owned())
            .collect();
        assert!(
            paths.contains(&"Projects/Tasks.md".to_string()),
            "{paths:?}"
        );
        let home = &plan
            .files
            .iter()
            .find(|(p, _)| p.ends_with("Home.md"))
            .unwrap()
            .1;
        assert!(home.contains("[[Projects/Tasks]]"), "{home}");
        assert!(home.contains("[[Projects/Tasks|the list]]"));
        assert!(home.contains("[[Projects/Tasks#^id1]]"));
        assert!(home.contains("![[Projects/Tasks#Plan]]"));
        assert!(home.contains("[[Projects::Tasks]] stays raw in code"));
    }

    #[test]
    fn journals_aliases_and_collisions() {
        let mut day = page("2026-07-06", "today");
        day.journal_date = Some("2026-07-06".into());
        let mut aliased = page("Chicken", "cluck");
        aliased.aliases = vec!["hen".into(), "a \"quoted\" bird".into()];
        // These two sanitize to the same filename → the second uniquifies.
        let clash_a = page("Foo?", "a");
        let clash_b = page("Foo-", "b");
        let mut wb = page("Board", "{}");
        wb.kind = "whiteboard".into();

        let plan = plan_export(&[day, aliased, clash_a, clash_b, wb]);
        assert_eq!(plan.days, 1);
        assert_eq!(plan.pages, 3);
        let paths: Vec<String> = plan
            .files
            .iter()
            .map(|(p, _)| p.to_string_lossy().into_owned())
            .collect();
        assert!(paths.contains(&"journals/2026-07-06.md".to_string()));
        assert!(paths.contains(&"Foo-.md".to_string()));
        assert!(paths.contains(&"Foo- 2.md".to_string()), "{paths:?}");
        let chicken = &plan
            .files
            .iter()
            .find(|(p, _)| p.ends_with("Chicken.md"))
            .unwrap()
            .1;
        assert!(
            chicken
                .starts_with("---\naliases:\n  - \"hen\"\n  - \"a \\\"quoted\\\" bird\"\n---\n\n")
        );
        assert!(plan.warnings.iter().any(|w| w.contains("whiteboard")));
    }

    #[test]
    fn assets_collected_from_images_and_pdf_chips() {
        let pages = vec![page(
            "Note",
            "![](images/pic.png)\ntext ![inline](images/small.jpg) more\n\
             open [[pdf/doc.pdf]] and jump [[pdf/doc.pdf#p3|↗]]\n\
             remote ![](https://x.io/a.png) is skipped",
        )];
        let plan = plan_export(&pages);
        let assets: Vec<&String> = plan.assets.iter().collect();
        assert_eq!(
            assets,
            ["images/pic.png", "images/small.jpg", "pdf/doc.pdf"]
        );
    }

    #[test]
    fn round_trip_through_the_obsidian_importer() {
        // export → read_vault: titles, day, and link targets survive.
        let pages = vec![
            page("Projects::Tasks", "- [ ] ship the exporter ^t1"),
            page(
                "Home",
                "see [[Projects::Tasks]] and ![[Projects::Tasks#^t1]]",
            ),
            {
                let mut d = page("2026-07-06", "today I exported");
                d.journal_date = Some("2026-07-06".into());
                d
            },
        ];
        let plan = plan_export(&pages);
        let dir = std::env::temp_dir().join(format!("zorite-export-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let summary = write_export(Path::new("/nonexistent-data-dir"), &dir, plan).unwrap();
        assert_eq!((summary.pages, summary.days), (2, 1));

        let opts = crate::import::obsidian::Options { namespaces: true };
        let bundle = crate::import::obsidian::read_vault(&dir, &opts).unwrap();
        let titles: Vec<&str> = bundle.pages.iter().map(|p| p.title.as_str()).collect();
        assert!(titles.contains(&"Projects::Tasks"), "{titles:?}");
        assert!(titles.contains(&"Home"));
        assert_eq!(bundle.days.len(), 1);
        assert_eq!(bundle.days[0].date, "2026-07-06");
        let home = &bundle
            .pages
            .iter()
            .find(|p| p.title == "Home")
            .unwrap()
            .content;
        // The importer maps `Projects/Tasks` back to the namespaced title.
        assert!(home.contains("[[Projects::Tasks"), "{home}");
        assert!(home.contains("![[Projects::Tasks#^t1]]"));
        let tasks = &bundle
            .pages
            .iter()
            .find(|p| p.title == "Projects::Tasks")
            .unwrap()
            .content;
        assert!(tasks.contains("^t1"), "block id survives: {tasks}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_refuses_a_non_empty_destination() {
        let dir = std::env::temp_dir().join(format!("zorite-export-ne-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("existing.txt"), "keep me").unwrap();
        let plan = plan_export(&[page("A", "x")]);
        assert!(write_export(Path::new("/tmp"), &dir, plan).is_err());
        // The pre-existing file is untouched.
        assert_eq!(
            std::fs::read_to_string(dir.join("existing.txt")).unwrap(),
            "keep me"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
