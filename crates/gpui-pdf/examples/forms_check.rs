//! Verification helper for the `forms` feature (the `extract_text` pattern):
//!
//! ```sh
//! cargo run -p gpui-pdf --example forms_check --features forms -- <file.pdf>
//! ```
//!
//! Reports every form widget's shape — field type, value, and what its
//! `/AP /N` looks like — plus whether `normalize_form_appearances` changed
//! anything. Diagnoses "why doesn't this form display?".

use lopdf::{Dictionary, Document, Object};

fn shape(doc: &Document, d: &Dictionary) -> &'static str {
    let Ok(ap) = d.get(b"AP") else {
        return "NO /AP";
    };
    let Some(ap) = deref(doc, ap).and_then(|o| o.as_dict().ok()) else {
        return "/AP unreadable";
    };
    let Ok(n) = ap.get(b"N") else {
        return "/AP without /N";
    };
    match deref(doc, n) {
        Some(Object::Stream(_)) => "/N stream (renders)",
        Some(Object::Dictionary(_)) => "/N state dict",
        _ => "/N unreadable",
    }
}

fn deref<'a>(doc: &'a Document, mut o: &'a Object) -> Option<&'a Object> {
    for _ in 0..4 {
        match o {
            Object::Reference(id) => o = doc.get_object(*id).ok()?,
            _ => return Some(o),
        }
    }
    None
}

fn attr<'a>(doc: &'a Document, mut d: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    for _ in 0..32 {
        if let Ok(v) = d.get(key) {
            return deref(doc, v);
        }
        d = deref(doc, d.get(b"Parent").ok()?)?.as_dict().ok()?;
    }
    None
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: forms_check <file.pdf>");
    let bytes = std::fs::read(&path).expect("read file");
    let doc = match Document::load_mem(&bytes) {
        Ok(d) => d,
        Err(e) => {
            println!("lopdf can't parse: {e}");
            return;
        }
    };
    println!("encrypted: {}", doc.is_encrypted());

    // XFA forms render from an embedded XML template, not /AP streams —
    // nothing (including Firefox/Preview) draws them from the AcroForm side.
    if let Ok(root) = doc.trailer.get(b"Root").and_then(|r| {
        deref(&doc, r)
            .ok_or(lopdf::Error::ObjectNotFound((0, 0)))
            .and_then(|o| o.as_dict())
    }) && let Some(acro) = attr(&doc, root, b"AcroForm").and_then(|o| o.as_dict().ok())
    {
        println!("AcroForm: yes; XFA: {}", acro.has(b"XFA"));
        println!(
            "NeedAppearances: {}",
            acro.get(b"NeedAppearances")
                .and_then(|o| o.as_bool())
                .unwrap_or(false)
        );
    } else {
        println!("AcroForm: none");
    }

    let mut n = 0;
    for obj in doc.objects.values() {
        let Ok(d) = obj.as_dict() else { continue };
        if d.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Widget".as_slice()) {
            continue;
        }
        n += 1;
        let ft = attr(&doc, d, b"FT")
            .and_then(|o| o.as_name().ok())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_else(|| "?".into());
        let has_v = attr(&doc, d, b"V").is_some();
        let name = attr(&doc, d, b"T")
            .and_then(|o| o.as_str().ok())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        println!(
            "widget {n}: FT={ft} V={has_v} AP={} T={name:?}",
            shape(&doc, d)
        );
        if n >= 40 {
            println!("… (more widgets elided)");
            break;
        }
    }
    println!("total widgets shown: {n}");

    match gpui_pdf::normalize_form_appearances(&bytes) {
        Some(out) => {
            println!(
                "normalize: CHANGED ({} -> {} bytes)",
                bytes.len(),
                out.len()
            );
            // Optional second arg: write the normalized bytes for inspection.
            if let Some(out_path) = std::env::args().nth(2) {
                std::fs::write(&out_path, &out).expect("write normalized");
                println!("wrote {out_path}");
            }
        }
        None => println!("normalize: no changes"),
    }
}
