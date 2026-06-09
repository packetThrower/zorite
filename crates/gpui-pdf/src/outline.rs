//! PDF outline (bookmarks / "table of contents") extraction.
//!
//! Walks the document's `/Outlines` tree through hayro-syntax's low-level object
//! API and flattens it to `(title, nesting depth, target page index)`. Pure Rust,
//! no extra deps. Destinations given as an explicit `[pageRef /XYZ …]` array (and
//! `/A` GoTo actions wrapping one) resolve to a page index; named destinations are
//! left unresolved for now (the title still shows). Malformed trees are bounded by
//! a visited-set + item/-depth caps so a cyclic or huge outline can't hang us.

use std::collections::{HashMap, HashSet};

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, MaybeRef, ObjRef, String as PdfString};

/// One entry in a PDF's outline, flattened depth-first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineItem {
    /// The bookmark label.
    pub title: String,
    /// Nesting depth, 0 = top level.
    pub level: usize,
    /// 0-based target page index, or `None` if the destination couldn't be resolved.
    pub page: Option<usize>,
}

/// Hard caps so a malformed/hostile outline can't hang or OOM us.
const MAX_ITEMS: usize = 10_000;
const MAX_DEPTH: usize = 32;

/// Extract the document outline (bookmarks), flattened depth-first. Returns an
/// empty vec when the PDF has no `/Outlines`.
pub fn outline(doc: &Pdf) -> Vec<OutlineItem> {
    let xref = doc.xref();
    let Some(catalog) = xref.get::<Dict>(xref.root_id()) else {
        return Vec::new();
    };
    let Some(outlines) = catalog.get::<Dict>("Outlines") else {
        return Vec::new();
    };
    let Some(first) = outlines.get_ref("First") else {
        return Vec::new();
    };

    let page_index = build_page_index(doc);
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    walk_items(doc, first, 0, &page_index, &mut visited, &mut out);
    out
}

/// Follow `/Next` siblings, recursing into `/First` children one level deeper.
fn walk_items(
    doc: &Pdf,
    start: ObjRef,
    level: usize,
    page_index: &HashMap<ObjRef, usize>,
    visited: &mut HashSet<ObjRef>,
    out: &mut Vec<OutlineItem>,
) {
    if level > MAX_DEPTH {
        return;
    }
    let xref = doc.xref();
    let mut cur = Some(start);
    while let Some(r) = cur {
        if out.len() >= MAX_ITEMS || !visited.insert(r) {
            return;
        }
        let Some(item) = xref.get::<Dict>(r.into()) else {
            return;
        };
        if let Some(title) = item.get::<PdfString>("Title") {
            out.push(OutlineItem {
                title: decode_pdf_string(title.as_bytes()),
                level,
                page: resolve_dest_page(&item, page_index),
            });
        }
        if let Some(child) = item.get_ref("First") {
            walk_items(doc, child, level + 1, page_index, visited, out);
        }
        cur = item.get_ref("Next");
    }
}

/// Resolve an outline item's destination to a page index. Handles `/Dest` and
/// `/A` (GoTo action) `/D` when they're an explicit `[pageRef …]` array.
fn resolve_dest_page(item: &Dict, page_index: &HashMap<ObjRef, usize>) -> Option<usize> {
    let dest = item
        .get::<Array>("Dest")
        .or_else(|| item.get::<Dict>("A").and_then(|a| a.get::<Array>("D")))?;
    // The first array element is an indirect reference to the page object.
    match dest.raw_iter().next()? {
        MaybeRef::Ref(page_ref) => page_index.get(&page_ref).copied(),
        MaybeRef::NotRef(_) => None,
    }
}

/// Map every page's object reference to its 0-based index by walking the page
/// tree, so destinations (which reference a page by object ref) can be resolved.
fn build_page_index(doc: &Pdf) -> HashMap<ObjRef, usize> {
    let xref = doc.xref();
    let mut map = HashMap::new();
    let Some(catalog) = xref.get::<Dict>(xref.root_id()) else {
        return map;
    };
    let Some(root) = catalog.get_ref("Pages") else {
        return map;
    };
    let mut idx = 0;
    let mut visited = HashSet::new();
    walk_pages(doc, root, 0, &mut idx, &mut visited, &mut map);
    map
}

fn walk_pages(
    doc: &Pdf,
    r: ObjRef,
    depth: usize,
    idx: &mut usize,
    visited: &mut HashSet<ObjRef>,
    map: &mut HashMap<ObjRef, usize>,
) {
    if depth > MAX_DEPTH || !visited.insert(r) {
        return;
    }
    let xref = doc.xref();
    let Some(dict) = xref.get::<Dict>(r.into()) else {
        return;
    };
    // An internal node has `/Kids`; a leaf is a `/Page`.
    if let Some(kids) = dict.get::<Array>("Kids") {
        for kid in kids.raw_iter() {
            if let MaybeRef::Ref(kr) = kid {
                walk_pages(doc, kr, depth + 1, idx, visited, map);
            }
        }
    } else {
        map.insert(r, *idx);
        *idx += 1;
    }
}

/// Decode a PDF text string: UTF-16BE when it carries the BOM, otherwise treated
/// as Latin-1 (a close-enough stand-in for PDFDocEncoding for titles).
fn decode_pdf_string(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units).trim().to_string()
    } else {
        bytes
            .iter()
            .map(|&b| b as char)
            .collect::<String>()
            .trim()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16be_bom_decodes() {
        // "Hi" in UTF-16BE with BOM.
        let b = [0xFE, 0xFF, 0x00, b'H', 0x00, b'i'];
        assert_eq!(decode_pdf_string(&b), "Hi");
    }

    #[test]
    fn latin1_decodes_and_trims() {
        assert_eq!(decode_pdf_string(b"  Intro  "), "Intro");
        assert_eq!(decode_pdf_string(&[b'C', 0xE9]), "Cé"); // 0xE9 = é in Latin-1
    }
}
