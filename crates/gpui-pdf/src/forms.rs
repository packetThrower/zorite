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

    write_text_appearance(doc, widget, helv, rect, &text)
}

/// Build a single-line Helvetica appearance stream showing `text` and install
/// it as the widget's `/AP /N`. Shared by the display-time synthesis and
/// [`set_form_value`]'s write-back (which must regenerate appearances so its
/// output renders in every viewer).
fn write_text_appearance(
    doc: &mut Document,
    widget: ObjectId,
    helv: &mut Option<ObjectId>,
    rect: (f32, f32),
    text: &str,
) -> bool {
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
    content.extend(escape_pdf_text(text));
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

// ───────────────────────────── Field enumeration + write-back ─────────────────────────────

/// What kind of input a form field takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// Free text (`/FT /Tx`).
    Text,
    /// An on/off checkbox (`/FT /Btn`, not radio, not pushbutton).
    Checkbox,
    /// One widget of a radio group (`/FT /Btn` with the radio flag).
    Radio,
    /// A choice — combo or list box (`/FT /Ch`).
    Choice,
    /// A signature field (`/FT /Sig`) — display-only, never writable here.
    Signature,
}

/// One form widget, described for a host UI: where it is, what it takes, and
/// what it currently holds.
#[derive(Clone, Debug)]
pub struct FormField {
    /// The fully-qualified field name (`/T` up the `/Parent` chain, joined
    /// with `.`) — the key [`set_form_value`] takes.
    pub name: String,
    pub kind: FieldKind,
    /// 0-based page index the widget sits on.
    pub page: usize,
    /// The widget's `/Rect` in PDF points, bottom-left origin, normalized to
    /// `(x0, y0, x1, y1)` with `x0 < x1`, `y0 < y1`.
    pub rect: (f32, f32, f32, f32),
    /// The current value: the text for `Text`/`Choice`; the on-state name (or
    /// `"Off"`) for `Checkbox`/`Radio`.
    pub value: String,
    /// The field is flagged read-only (`/Ff` bit 1).
    pub read_only: bool,
    /// `Choice`: the `/Opt` entries. `Checkbox`/`Radio`: this widget's
    /// on-state names (its `/AP /N` keys minus `Off`) — what to pass to
    /// [`set_form_value`] to check it.
    pub options: Vec<String>,
}

/// Every form-field widget in the document, in page order — what a host needs
/// to overlay inputs on the viewer. Pushbuttons (no value) are skipped; an
/// encrypted or unparseable file yields an empty list.
pub fn form_fields(bytes: &[u8]) -> Vec<FormField> {
    let Ok(doc) = Document::load_mem(bytes) else {
        return Vec::new();
    };
    if doc.is_encrypted() {
        return Vec::new();
    }
    let mut out = Vec::new();
    // lopdf numbers pages from 1, in document order.
    for (page_no, page_id) in doc.get_pages() {
        let Some(annots) = (|| {
            let page = doc.get_object(page_id).ok()?.as_dict().ok()?;
            deref(&doc, page.get(b"Annots").ok()?)?.as_array().ok()
        })() else {
            continue;
        };
        for a in annots {
            let Some(w) = deref(&doc, a).and_then(|o| o.as_dict().ok()) else {
                continue;
            };
            if w.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Widget".as_slice()) {
                continue;
            }
            let Some(field) = describe_widget(&doc, w, page_no as usize - 1) else {
                continue;
            };
            out.push(field);
        }
    }
    out
}

/// Set the value of the field named `name` (fully qualified, as reported by
/// [`form_fields`]) and regenerate its appearance so the result renders in
/// any viewer — not just ours. For `Text`/`Choice` pass the literal text; for
/// `Checkbox`/`Radio` pass an on-state name from [`FormField::options`] (or
/// `"Off"` to clear). Returns the rewritten bytes, or `None` when nothing
/// matched (unknown/read-only/signature field, encrypted or unparseable
/// file).
pub fn set_form_value(bytes: &[u8], name: &str, value: &str) -> Option<Vec<u8>> {
    let mut doc = Document::load_mem(bytes).ok()?;
    if doc.is_encrypted() {
        return None;
    }
    // All widgets carrying that qualified name — a radio group is one field
    // with several widgets, and each needs its /AS set.
    let widgets: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(id, obj)| {
            let d = obj.as_dict().ok()?;
            (d.get(b"Subtype").ok()?.as_name().ok()? == b"Widget"
                && qualified_name(&doc, d)? == name)
                .then_some(*id)
        })
        .collect();
    if widgets.is_empty() {
        return None;
    }

    let mut changed = false;
    let mut helv = None;
    for id in widgets {
        let (Some(w), Some(kind)) = (
            doc.get_object(id).ok().and_then(|o| o.as_dict().ok()),
            doc.get_object(id)
                .ok()
                .and_then(|o| o.as_dict().ok())
                .and_then(|d| kind_of(&doc, d)),
        ) else {
            continue;
        };
        if matches!(kind, FieldKind::Signature)
            || field_attr(&doc, w, b"Ff")
                .and_then(|o| o.as_i64().ok())
                .is_some_and(|f| f & 1 != 0)
        {
            continue;
        }
        match kind {
            FieldKind::Text | FieldKind::Choice => {
                let Some(rect) = rect_of(&doc, w) else {
                    continue;
                };
                set_value_object(&mut doc, id, Object::string_literal(value));
                changed |= write_text_appearance(&mut doc, id, &mut helv, rect, value);
            }
            FieldKind::Checkbox | FieldKind::Radio => {
                // This widget shows `value` if its own /AP carries that
                // state; every other widget in the group turns Off.
                let has_state = deref(&doc, w.get(b"AP").ok()?)
                    .and_then(|o| o.as_dict().ok())
                    .and_then(|ap| deref(&doc, ap.get(b"N").ok()?))
                    .and_then(|o| o.as_dict().ok())
                    .is_some_and(|states| states.has(value.as_bytes()));
                let state = if has_state { value } else { "Off" };
                set_value_object(&mut doc, id, Object::Name(value.as_bytes().to_vec()));
                if let Ok(wd) = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()) {
                    wd.set("AS", Object::Name(state.as_bytes().to_vec()));
                    changed = true;
                }
            }
            FieldKind::Signature => unreachable!(),
        }
    }
    if !changed {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() + 4096);
    doc.save_to(&mut out).ok()?;
    Some(out)
}

/// Write `/V` where the field keeps it: on the widget itself when merged, on
/// the parent field dict when split (so sibling widgets agree).
fn set_value_object(doc: &mut Document, widget: ObjectId, value: Object) {
    // Find the dict that OWNS /FT — that's the field; /V belongs beside it.
    let mut target = widget;
    for _ in 0..32 {
        let Some(d) = doc.get_object(target).ok().and_then(|o| o.as_dict().ok()) else {
            break;
        };
        if d.has(b"FT") {
            break;
        }
        let Some(parent) = d.get(b"Parent").ok().and_then(|p| p.as_reference().ok()) else {
            break;
        };
        target = parent;
    }
    if let Ok(d) = doc.get_object_mut(target).and_then(|o| o.as_dict_mut()) {
        d.set("V", value);
    }
}

/// Describe one widget for [`form_fields`]. `None` skips it (pushbutton, no
/// field type, unreadable rect).
fn describe_widget(doc: &Document, w: &Dictionary, page: usize) -> Option<FormField> {
    let kind = kind_of(doc, w)?;
    let name = qualified_name(doc, w)?;
    let (x0, y0, x1, y1) = rect_corners(doc, w)?;
    let flags = field_attr(doc, w, b"Ff")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0);
    let value = match kind {
        FieldKind::Text | FieldKind::Choice | FieldKind::Signature => field_attr(doc, w, b"V")
            .and_then(|o| o.as_str().ok())
            .map(decode_pdf_string)
            .unwrap_or_default(),
        FieldKind::Checkbox | FieldKind::Radio => field_attr(doc, w, b"V")
            .or_else(|| w.get(b"AS").ok())
            .and_then(|o| o.as_name().ok())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_else(|| "Off".into()),
    };
    let options = match kind {
        FieldKind::Choice => field_attr(doc, w, b"Opt")
            .and_then(|o| o.as_array().ok())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        // /Opt entries are strings or [export, display] pairs.
                        let o = deref(doc, o)?;
                        match o {
                            Object::Array(pair) => pair.first().and_then(|s| s.as_str().ok()),
                            other => other.as_str().ok(),
                        }
                        .map(decode_pdf_string)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        FieldKind::Checkbox | FieldKind::Radio => (|| {
            let ap = deref(doc, w.get(b"AP").ok()?)?.as_dict().ok()?;
            let states = deref(doc, ap.get(b"N").ok()?)?.as_dict().ok()?;
            Some(
                states
                    .iter()
                    .filter(|(k, _)| k.as_slice() != b"Off")
                    .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
                    .collect::<Vec<_>>(),
            )
        })()
        .unwrap_or_default(),
        _ => Vec::new(),
    };
    Some(FormField {
        name,
        kind,
        page,
        rect: (x0, y0, x1, y1),
        value,
        read_only: flags & 1 != 0,
        options,
    })
}

/// The widget's field kind from its (inherited) `/FT` + `/Ff` flags. `None`
/// for pushbuttons (bit 17 — they hold no value) and unknown types.
fn kind_of(doc: &Document, w: &Dictionary) -> Option<FieldKind> {
    let ft = field_attr(doc, w, b"FT")?.as_name().ok()?.to_vec();
    let flags = field_attr(doc, w, b"Ff")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0);
    match ft.as_slice() {
        b"Tx" => Some(FieldKind::Text),
        b"Ch" => Some(FieldKind::Choice),
        b"Sig" => Some(FieldKind::Signature),
        b"Btn" if flags & (1 << 16) != 0 => None, // pushbutton
        b"Btn" if flags & (1 << 15) != 0 => Some(FieldKind::Radio),
        b"Btn" => Some(FieldKind::Checkbox),
        _ => None,
    }
}

/// The fully-qualified field name: every `/T` up the `/Parent` chain, joined
/// root-first with `.` (the AcroForm convention).
fn qualified_name(doc: &Document, w: &Dictionary) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut d = w;
    for _ in 0..32 {
        if let Some(t) = d.get(b"T").ok().and_then(|o| deref(doc, o)) {
            parts.push(decode_pdf_string(t.as_str().ok()?));
        }
        match d.get(b"Parent").ok().and_then(|p| deref(doc, p)) {
            Some(p) => d = p.as_dict().ok()?,
            None => break,
        }
    }
    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    Some(parts.join("."))
}

/// The widget's `/Rect` normalized to `(x0, y0, x1, y1)`, corners ordered.
fn rect_corners(doc: &Document, w: &Dictionary) -> Option<(f32, f32, f32, f32)> {
    let arr = deref(doc, w.get(b"Rect").ok()?)?.as_array().ok()?;
    let n = |o: &Object| -> Option<f32> {
        match o {
            Object::Integer(i) => Some(*i as f32),
            Object::Real(r) => Some(*r),
            _ => None,
        }
    };
    let (a, b, c, d) = (
        n(arr.first()?)?,
        n(arr.get(1)?)?,
        n(arr.get(2)?)?,
        n(arr.get(3)?)?,
    );
    Some((a.min(c), b.min(d), a.max(c), b.max(d)))
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
    fn fields_enumerate_with_kinds_and_values() {
        let fields = form_fields(&build_test_pdf());
        assert_eq!(fields.len(), 3);
        let by_name = |n: &str| fields.iter().find(|f| f.name == n).unwrap();

        let cb = by_name("check1");
        assert_eq!(cb.kind, FieldKind::Checkbox);
        assert_eq!(cb.value, "Yes");
        assert_eq!(cb.options, vec!["Yes".to_string()]);
        assert_eq!(cb.page, 0);
        assert_eq!(cb.rect, (20.0, 140.0, 60.0, 180.0));

        let merged = by_name("name1");
        assert_eq!(merged.kind, FieldKind::Text);
        assert_eq!(merged.value, "Merged value");
        assert!(!merged.read_only);

        // The split pair reports under the parent's name with the kid's rect.
        let split = by_name("name2");
        assert_eq!(split.value, "Inherited value");
        assert_eq!(split.rect, (20.0, 200.0, 220.0, 240.0));
    }

    #[test]
    fn set_form_value_writes_and_renders_everywhere() {
        let bytes = build_test_pdf();

        // Text: new value lands in /V and in a regenerated appearance.
        let out = set_form_value(&bytes, "name1", "Rewritten").expect("text write");
        let n = widget_n(&out, "name1");
        let content = String::from_utf8_lossy(&n.as_stream().unwrap().content).into_owned();
        assert!(content.contains("Rewritten"), "{content}");
        assert_eq!(
            form_fields(&out)
                .iter()
                .find(|f| f.name == "name1")
                .unwrap()
                .value,
            "Rewritten"
        );

        // Checkbox: turning it Off flips /V + /AS; back on restores them.
        let out = set_form_value(&bytes, "check1", "Off").expect("uncheck");
        assert_eq!(
            form_fields(&out)
                .iter()
                .find(|f| f.name == "check1")
                .unwrap()
                .value,
            "Off"
        );
        let out = set_form_value(&out, "check1", "Yes").expect("recheck");
        assert_eq!(
            form_fields(&out)
                .iter()
                .find(|f| f.name == "check1")
                .unwrap()
                .value,
            "Yes"
        );

        // The split field writes /V on the PARENT (so siblings agree) and the
        // appearance on the widget.
        let out = set_form_value(&bytes, "name2", "Via parent").expect("split write");
        assert_eq!(
            form_fields(&out)
                .iter()
                .find(|f| f.name == "name2")
                .unwrap()
                .value,
            "Via parent"
        );

        // Unknown field → None.
        assert!(set_form_value(&bytes, "nope", "x").is_none());
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
