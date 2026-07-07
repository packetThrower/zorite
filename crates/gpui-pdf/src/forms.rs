//! AcroForm display correctness (the `forms` feature): normalize a document's
//! form-field appearances *before* hayro parses it, so form PDFs display the
//! values they carry.
//!
//! hayro composites annotation appearance streams (`/AP /N`) but has two gaps
//! this pass fills at the byte level (hayro reads; `lopdf` rewrites):
//!
//! 1. **State-dict appearances** — a checkbox/radio stores `/AP /N` as a
//!    dictionary of states (`/Yes`, `/Off`, …) selected by the widget's `/AS`;
//!    hayro only draws a *stream* `/N` and silently skips these. The pass
//!    resolves `/N` to the `/AS`-selected stream.
//! 2. **Missing appearances** — a text field with a value but no `/AP` (the
//!    `NeedAppearances` case: the producer left rendering to the viewer)
//!    draws nothing. The pass synthesizes a simple appearance stream showing
//!    `/V` in Helvetica, clipped to the widget rect.
//!
//! Deliberate ceilings (ponytail: display correctness, not a form engine):
//! synthesized text is single-line, left-aligned, WinAnsi-lossy (non-Latin-1
//! chars become `?`), and ignores `/DA` font choices, `/Q` quadding, comb
//! fields, and rich-text values. Encrypted documents are left untouched
//! (lopdf would need the password; hayro decrypts on its own).

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

/// Rewrite `bytes` so every form widget has a directly-renderable appearance
/// stream. `Some(fixed)` only when something actually changed; `None` means
/// nothing to do (no forms, already normalized, encrypted, or unparseable) —
/// the caller keeps the original bytes either way.
pub fn normalize_form_appearances(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut doc = Document::load_mem(bytes).ok()?;
    if doc.is_encrypted() {
        return None;
    }

    // Widget annotations, collected up front (mutating `doc` invalidates
    // iteration). Merged field+widget dicts — the common case — are found
    // here too, since the widget half carries `/Subtype /Widget`.
    let widgets: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(id, obj)| {
            let d = obj.as_dict().ok()?;
            (d.get(b"Subtype").ok()?.as_name().ok()? == b"Widget").then_some(*id)
        })
        .collect();

    let mut changed = false;
    let mut helv: Option<ObjectId> = None;
    for id in widgets {
        changed |= resolve_state_dict(&mut doc, id);
        changed |= synthesize_text_appearance(&mut doc, id, &mut helv);
    }
    if !changed {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() + 4096);
    doc.save_to(&mut out).ok()?;
    Some(out)
}

/// Gap 1: `/AP /N` is a dict of states — replace it with the `/AS`-selected
/// entry so the renderer sees a plain stream. True if the widget changed.
fn resolve_state_dict(doc: &mut Document, widget: ObjectId) -> bool {
    // Read phase: find the selected state's object without holding borrows.
    let Some(selected) = (|| {
        let w = doc.get_object(widget).ok()?.as_dict().ok()?;
        let state = w.get(b"AS").ok()?.as_name().ok()?.to_vec();
        let ap = deref(doc, w.get(b"AP").ok()?)?.as_dict().ok()?;
        let n = deref(doc, ap.get(b"N").ok()?)?;
        let states = n.as_dict().ok()?; // a direct-stream `/N` is already fine
        Some(states.get(&state).ok()?.clone())
    })() else {
        return false;
    };
    // An inline stream in the state dict (unusual) becomes its own object so
    // `/N` can reference it.
    let target = match selected {
        Object::Reference(id) => id,
        stream @ Object::Stream(_) => doc.add_object(stream),
        _ => return false,
    };
    let Ok(w) = doc.get_object_mut(widget).and_then(|o| o.as_dict_mut()) else {
        return false;
    };
    // `/AP` may be shared via a reference; rewriting the widget's own `/AP`
    // entry (a fresh direct dict) keeps the change local to this widget.
    let mut ap = Dictionary::new();
    ap.set("N", Object::Reference(target));
    w.set("AP", Object::Dictionary(ap));
    true
}

/// Gap 2: a text field with a value but no `/AP` — synthesize one. True if an
/// appearance was added.
fn synthesize_text_appearance(
    doc: &mut Document,
    widget: ObjectId,
    helv: &mut Option<ObjectId>,
) -> bool {
    let Some((rect, text)) = (|| {
        let w = doc.get_object(widget).ok()?.as_dict().ok()?;
        if w.has(b"AP") {
            return None;
        }
        // `/FT` and `/V` may live on the parent field (split field/widget).
        if field_attr(doc, w, b"FT")?.as_name().ok()? != b"Tx" {
            return None;
        }
        let v = field_attr(doc, w, b"V")?;
        let text = decode_pdf_string(v.as_str().ok()?);
        if text.trim().is_empty() {
            return None;
        }
        Some((rect_of(doc, w)?, text))
    })() else {
        return false;
    };

    let font = *helv.get_or_insert_with(|| {
        let mut f = Dictionary::new();
        f.set("Type", Object::Name(b"Font".to_vec()));
        f.set("Subtype", Object::Name(b"Type1".to_vec()));
        f.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
        f.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
        doc.add_object(Object::Dictionary(f))
    });

    let (w, h) = rect;
    // Fit the line to the box like viewers do for auto-sized fields: cap at
    // 12pt, floor at 6, baseline vertically centered. The XObject's BBox
    // clips overflow.
    let size = (h - 4.0).clamp(6.0, 12.0);
    let y = (h - size) / 2.0 + size * 0.18;
    let mut content = format!("BT /Helv {size:.1} Tf 0 g 2 {y:.1} Td (").into_bytes();
    content.extend(escape_pdf_text(&text));
    content.extend_from_slice(b") Tj ET");

    let mut fonts = Dictionary::new();
    fonts.set("Helv", Object::Reference(font));
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));
    let mut xd = Dictionary::new();
    xd.set("Type", Object::Name(b"XObject".to_vec()));
    xd.set("Subtype", Object::Name(b"Form".to_vec()));
    xd.set(
        "BBox",
        Object::Array(vec![0.into(), 0.into(), Object::Real(w), Object::Real(h)]),
    );
    xd.set("Resources", Object::Dictionary(res));
    let xobj = doc.add_object(Object::Stream(Stream::new(xd, content)));

    let Ok(wd) = doc.get_object_mut(widget).and_then(|o| o.as_dict_mut()) else {
        return false;
    };
    let mut ap = Dictionary::new();
    ap.set("N", Object::Reference(xobj));
    wd.set("AP", Object::Dictionary(ap));
    true
}

/// Follow a `Reference` to its object (one level is all PDF allows for
/// indirect values; a chain is malformed — bail after a few hops).
fn deref<'a>(doc: &'a Document, mut obj: &'a Object) -> Option<&'a Object> {
    for _ in 0..4 {
        match obj {
            Object::Reference(id) => obj = doc.get_object(*id).ok()?,
            _ => return Some(obj),
        }
    }
    None
}

/// A field attribute (`/FT`, `/V`, …) on the widget or inherited up its
/// `/Parent` chain, dereferenced. Bounded — a cyclic parent chain is malformed.
fn field_attr<'a>(doc: &'a Document, mut dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    for _ in 0..32 {
        if let Ok(v) = dict.get(key) {
            return deref(doc, v);
        }
        dict = deref(doc, dict.get(b"Parent").ok()?)?.as_dict().ok()?;
    }
    None
}

/// The widget's `/Rect` as a positive `(width, height)`.
fn rect_of(doc: &Document, w: &Dictionary) -> Option<(f32, f32)> {
    let arr = deref(doc, w.get(b"Rect").ok()?)?.as_array().ok()?;
    let n = |o: &Object| -> Option<f32> {
        match o {
            Object::Integer(i) => Some(*i as f32),
            Object::Real(r) => Some(*r),
            _ => None,
        }
    };
    let (x0, y0, x1, y1) = (
        n(arr.first()?)?,
        n(arr.get(1)?)?,
        n(arr.get(2)?)?,
        n(arr.get(3)?)?,
    );
    Some(((x1 - x0).abs(), (y1 - y0).abs()))
}

/// A PDF text string to UTF-8: UTF-16BE with BOM, else PDFDocEncoding treated
/// as Latin-1 (close enough for the synthesized display).
fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

/// Encode for a `( … )` literal in the content stream: WinAnsi-ish Latin-1
/// (lossy `?` beyond it), with `\`, `(`, `)` escaped.
fn escape_pdf_text(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let b = if (ch as u32) < 256 { ch as u8 } else { b'?' };
        if matches!(b, b'\\' | b'(' | b')') {
            out.push(b'\\');
        }
        // Newlines in a single-line synthesis read best as spaces.
        out.push(if b == b'\n' || b == b'\r' { b' ' } else { b });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    /// A 420×420 one-page PDF with the three problem widgets: a checked
    /// checkbox whose `/AP /N` is a state dict (hayro gap 1), a merged text
    /// field with `/V` but no `/AP` (gap 2), and a split parent-field/kid-
    /// widget pair (gap 2 + `/Parent` inheritance).
    fn build_test_pdf() -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let on = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 40.into(), 40.into()] },
            b"0 0.6 0 RG 3 w 1 1 38 38 re S 0 0.6 0 rg 8 8 24 24 re f".to_vec(),
        ));
        let off = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 40.into(), 40.into()] },
            b"1 w 0.5 0.5 39 39 re S".to_vec(),
        ));
        let checkbox = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn",
            "T" => Object::string_literal("check1"),
            "V" => "Yes", "AS" => "Yes",
            "Rect" => vec![20.into(), 140.into(), 60.into(), 180.into()],
            "F" => 4,
            "AP" => dictionary! { "N" => dictionary! { "Yes" => on, "Off" => off } },
        });
        let merged = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
            "T" => Object::string_literal("name1"),
            "V" => Object::string_literal("Merged value"),
            "Rect" => vec![20.into(), 60.into(), 220.into(), 100.into()],
            "F" => 4,
        });
        // Split pair: the widget kid carries no /FT or /V of its own.
        let kid = doc.new_object_id();
        let parent = doc.add_object(dictionary! {
            "FT" => "Tx", "T" => Object::string_literal("name2"),
            "V" => Object::string_literal("Inherited value"),
            "Kids" => vec![Object::Reference(kid)],
        });
        doc.objects.insert(
            kid,
            Object::Dictionary(dictionary! {
                "Type" => "Annot", "Subtype" => "Widget",
                "Rect" => vec![20.into(), 200.into(), 220.into(), 240.into()],
                "F" => 4, "Parent" => parent,
            }),
        );
        let page = doc.new_object_id();
        let pages = doc.add_object(dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1,
        });
        doc.objects.insert(
            page,
            Object::Dictionary(dictionary! {
                "Type" => "Page", "Parent" => pages,
                "MediaBox" => vec![0.into(), 0.into(), 420.into(), 420.into()],
                "Annots" => vec![
                    Object::Reference(checkbox),
                    Object::Reference(merged),
                    Object::Reference(kid),
                ],
            }),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages,
            "AcroForm" => dictionary! {
                "Fields" => vec![
                    Object::Reference(checkbox),
                    Object::Reference(merged),
                    Object::Reference(parent),
                ],
                "NeedAppearances" => true,
            },
        });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    /// The `/AP /N` of `title`'s widget, after reloading `bytes`.
    fn widget_n(bytes: &[u8], title: &str) -> Object {
        let doc = Document::load_mem(bytes).unwrap();
        for obj in doc.objects.values() {
            if let Ok(d) = obj.as_dict()
                && d.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Widget".as_slice())
                && field_attr(&doc, d, b"T")
                    .and_then(|t| t.as_str().ok())
                    .is_some_and(|t| t == title.as_bytes())
            {
                let ap = deref(&doc, d.get(b"AP").expect("has AP")).unwrap();
                return deref(&doc, ap.as_dict().unwrap().get(b"N").unwrap())
                    .unwrap()
                    .clone();
            }
        }
        panic!("widget {title} not found");
    }

    #[test]
    fn state_dicts_resolve_and_missing_appearances_synthesize() {
        let fixed = normalize_form_appearances(&build_test_pdf()).expect("changes made");

        // Gap 1: the checkbox's /N is now the /AS-selected stream, not a dict.
        let n = widget_n(&fixed, "check1");
        let stream = n.as_stream().expect("direct stream");
        assert!(String::from_utf8_lossy(&stream.content).contains("re S"));

        // Gap 2: both text fields (merged + split/inherited) gained a
        // synthesized appearance showing their value.
        for (title, value) in [("name1", "Merged value"), ("name2", "Inherited value")] {
            let n = widget_n(&fixed, title);
            let stream = n.as_stream().expect("synthesized stream");
            let content = String::from_utf8_lossy(&stream.content);
            assert!(content.contains(value), "{title}: {content}");
            assert!(content.contains("/Helv"));
        }

        // Idempotent: a second pass finds nothing left to fix.
        assert!(normalize_form_appearances(&fixed).is_none());
    }

    #[test]
    fn untouched_documents_return_none() {
        // No widgets at all → None (a minimal empty document).
        let mut doc = Document::with_version("1.7");
        let pages = doc.add_object(
            dictionary! { "Type" => "Pages", "Kids" => Vec::<Object>::new(), "Count" => 0 },
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        assert!(normalize_form_appearances(&bytes).is_none());
        // Garbage in → None, never a panic.
        assert!(normalize_form_appearances(b"not a pdf").is_none());
    }

    /// End-to-end through hayro: the checkbox region is blank before the fix
    /// and carries ink after; same for the synthesized text field.
    #[test]
    fn hayro_renders_normalized_forms() {
        let before = build_test_pdf();
        let after = normalize_form_appearances(&before).unwrap();

        // Page is 420pt; rendered at scale 1 the pixmap is 420×420 with y
        // flipped (PDF origin bottom-left; pixmap top-left).
        let ink_in = |bytes: &[u8], rect: (usize, usize, usize, usize)| -> usize {
            let pdf =
                hayro::hayro_syntax::Pdf::new(std::sync::Arc::new(bytes.to_vec())).expect("parse");
            let pm = hayro::render_pdf(
                &pdf,
                1.0,
                hayro::hayro_interpret::InterpreterSettings::default(),
                Some(0..=0),
            )
            .expect("render")
            .remove(0);
            let (w, data) = (pm.width() as usize, pm.data_as_u8_slice().to_vec());
            let (x0, y0, x1, y1) = rect;
            let mut ink = 0;
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &data[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    // Premultiplied RGBA: any non-transparent, non-white pixel.
                    if p[3] > 0 && (p[0] < 250 || p[1] < 250 || p[2] < 250) {
                        ink += 1;
                    }
                }
            }
            ink
        };

        // Checkbox at (20,140)-(60,180) → pixmap rows 240..280.
        assert_eq!(ink_in(&before, (20, 240, 60, 280)), 0, "checkbox before");
        assert!(ink_in(&after, (20, 240, 60, 280)) > 50, "checkbox after");
        // Merged text field at (20,60)-(220,100) → rows 320..360.
        assert_eq!(ink_in(&before, (20, 320, 220, 360)), 0, "text before");
        assert!(ink_in(&after, (20, 320, 220, 360)) > 50, "text after");
    }
}
