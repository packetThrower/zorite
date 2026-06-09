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
}
