//! Dump a page's extracted text layer, to sanity-check `extract_page_text`.
//!
//! `cargo run -p gpui-pdf --example extract_text --features markup -- <file.pdf> [page]`

use std::sync::Arc;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: extract_text <file.pdf> [page]");
    let page: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let bytes = Arc::new(std::fs::read(&path).expect("read pdf"));
    let doc = gpui_pdf::parse(bytes).expect("parse pdf");

    match gpui_pdf::extract_page_text(&doc, page) {
        Some(pt) if !pt.is_empty() => {
            println!("--- page {page} text ---");
            println!("{}", pt.text());
        }
        Some(_) => eprintln!("page {page}: no extractable text (scanned image?)"),
        None => eprintln!("no page {page}"),
    }
}
