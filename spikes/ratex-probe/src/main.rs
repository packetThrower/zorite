//! RaTeX layout probe — the make-or-break for a structural (Casio-style) math
//! editor: does `LayoutBox` expose the math as distinct, dimensioned sub-boxes
//! (the editable slots), and does the engine compute absolute positions?
//!
//! For each formula we parse → layout, print the box tree (kind + em dimensions
//! + the shift fields that position scripts/limits; ► marks an editable slot),
//! then dump the flat `DisplayList` (absolute glyph coordinates). If the slots
//! are distinct dimensioned boxes AND the display list carries absolute
//! positions, the correlation a structural editor needs is recoverable.

use ratex_layout::layout_box::{BoxContent, LayoutBox};
use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_parser::parse;
use ratex_types::display_item::DisplayItem;

fn main() {
    let samples = [
        r"\int_0^1 4x \, dx",
        r"\sum_{i=1}^{n} i",
        r"\frac{a+b}{c}",
        r"\sqrt{x^2 + 1}",
    ];
    for latex in samples {
        println!("\n═══════════════════  {latex}  ═══════════════════");
        let nodes = match parse(latex) {
            Ok(n) => n,
            Err(e) => {
                println!("parse error: {e:?}");
                continue;
            }
        };
        let root = layout(&nodes, &LayoutOptions::default());

        println!("\n[ box tree ]  (em units; ► = an editable slot)");
        show(&root, 0);

        let dl = to_display_list(&root);
        println!(
            "\n[ display list ]  {} items · bbox {:.3} × {:.3} (+{:.3} depth) · absolute coords",
            dl.items.len(),
            dl.width,
            dl.height,
            dl.depth
        );
        for item in &dl.items {
            if let DisplayItem::GlyphPath { x, y, char_code, .. } = item {
                println!(
                    "    glyph '{}'  @ ({:.3}, {:.3})",
                    char::from_u32(*char_code).unwrap_or('\u{fffd}'),
                    x,
                    y
                );
            }
        }
    }
}

fn dims(b: &LayoutBox) -> String {
    format!("w={:.3} h={:.3} dp={:.3}", b.width, b.height, b.depth)
}

fn ind(n: usize) -> String {
    "    ".repeat(n)
}

fn show(b: &LayoutBox, depth: usize) {
    match &b.content {
        BoxContent::HBox(kids) => {
            println!("{}HBox [{}]  ({} children)", ind(depth), dims(b), kids.len());
            for k in kids {
                show(k, depth + 1);
            }
        }
        BoxContent::Glyph { char_code, .. } => println!(
            "{}Glyph '{}'  [{}]",
            ind(depth),
            char::from_u32(*char_code).unwrap_or('\u{fffd}'),
            dims(b)
        ),
        BoxContent::Kern => println!("{}Kern  [{}]", ind(depth), dims(b)),
        BoxContent::Rule { .. } => println!("{}Rule  [{}]", ind(depth), dims(b)),
        BoxContent::Fraction {
            numer,
            denom,
            numer_shift,
            denom_shift,
            ..
        } => {
            println!(
                "{}Fraction  [{}]  (numer_shift={:.3} denom_shift={:.3})",
                ind(depth),
                dims(b),
                numer_shift,
                denom_shift
            );
            println!("{}► numerator", ind(depth + 1));
            show(numer, depth + 2);
            println!("{}► denominator", ind(depth + 1));
            show(denom, depth + 2);
        }
        BoxContent::SupSub {
            base,
            sup,
            sub,
            sup_shift,
            sub_shift,
            ..
        } => {
            println!(
                "{}SupSub  [{}]  (sup_shift={:.3} sub_shift={:.3})",
                ind(depth),
                dims(b),
                sup_shift,
                sub_shift
            );
            println!("{}base", ind(depth + 1));
            show(base, depth + 2);
            if let Some(s) = sup {
                println!("{}► superscript", ind(depth + 1));
                show(s, depth + 2);
            }
            if let Some(s) = sub {
                println!("{}► subscript", ind(depth + 1));
                show(s, depth + 2);
            }
        }
        BoxContent::OpLimits {
            base, sup, sub, ..
        } => {
            println!("{}OpLimits  [{}]", ind(depth), dims(b));
            println!("{}operator", ind(depth + 1));
            show(base, depth + 2);
            if let Some(s) = sup {
                println!("{}► upper limit", ind(depth + 1));
                show(s, depth + 2);
            }
            if let Some(s) = sub {
                println!("{}► lower limit", ind(depth + 1));
                show(s, depth + 2);
            }
        }
        BoxContent::Radical { body, index, .. } => {
            println!("{}Radical  [{}]", ind(depth), dims(b));
            println!("{}► radicand", ind(depth + 1));
            show(body, depth + 2);
            if let Some(i) = index {
                println!("{}► index", ind(depth + 1));
                show(i, depth + 2);
            }
        }
        other => println!(
            "{}{:?}  [{}]",
            ind(depth),
            std::mem::discriminant(other),
            dims(b)
        ),
    }
}
