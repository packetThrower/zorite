//! Dev tool: dump a PDF's outline. `cargo run -p gpui-pdf --example dump_outline -- file.pdf`
use std::sync::Arc;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_outline <file.pdf>");
    let bytes = std::fs::read(&path).expect("read pdf");
    let doc = gpui_pdf::parse(Arc::new(bytes)).expect("parse pdf");
    let pages = gpui_pdf::page_dims(&doc).len();
    let items = gpui_pdf::outline(&doc);
    println!("== {pages} pages, {} outline items ==", items.len());
    let (mut resolved, mut unresolved) = (0, 0);
    for it in &items {
        let page = match it.page {
            Some(p) => {
                resolved += 1;
                format!("p{}", p + 1)
            }
            None => {
                unresolved += 1;
                "?".to_string()
            }
        };
        println!("{}{}  [{page}]", "    ".repeat(it.level), it.title);
    }
    println!("== {resolved} resolved to a page, {unresolved} unresolved ==");

    let links = gpui_pdf::page_links(&doc);
    let total: usize = links.iter().map(Vec::len).sum();
    let (mut internal, mut external) = (0, 0);
    for l in links.iter().flatten() {
        match l.target {
            gpui_pdf::LinkTarget::Page(_) => internal += 1,
            gpui_pdf::LinkTarget::Uri(_) => external += 1,
        }
    }
    let pages_with_links = links.iter().filter(|p| !p.is_empty()).count();
    println!(
        "== {total} link annotations ({internal} internal, {external} external) on \
         {pages_with_links} pages =="
    );
}
