//! Geometry — map the model + RaTeX layout to caret/slot rects. GUI-free, em units.
//!
//! [`caret_rect`] walks the cursor's path through the laid-out `LayoutBox` tree, mirroring
//! RaTeX's own box positioning so the caret aligns with the rendered raster. It handles
//! the top row plus descent into fraction numerator/denominator and super/subscript slots;
//! roots, delimiters, and operator limits fall through to `None` (the view hides the bar)
//! until extended. The tricky correlation: my model keeps a script as its own atom after
//! its base, but RaTeX folds `x^2` into a single `SupSub` box — so a base atom and a
//! trailing `SupSub` atom map to one layout child (a "cell").

use crate::editor::cursor::{Cursor, Slot, Step};
use crate::editor::model::{Atom, Row};
use ratex_layout::layout_box::{BoxContent, LayoutBox};
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parse;

/// A rectangle in **em** units (1 em = the layout font size), origin at the laid-out
/// formula's top-left, y growing downward. The view scales by the render font size and
/// offsets by the render origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Lay out a row's serialized LaTeX into a RaTeX `LayoutBox`.
pub fn layout_row(row: &Row) -> LayoutBox {
    let nodes = parse(&row.to_latex()).unwrap_or_default();
    layout(&nodes, &LayoutOptions::default())
}

// ---------------------------------------------------------------------------
// Top-row caret (kept for its unit tests; `caret_rect` supersedes it generally).
// ---------------------------------------------------------------------------

/// Caret rect for the cursor at `index` (`0..=atom_count`) in the **top-level** row.
pub fn caret_in_top_row(row: &Row, index: usize) -> Rect {
    let lbox = layout_row(row);
    let x = match &lbox.content {
        BoxContent::HBox(children) => caret_x_in_hbox(children, index),
        _ if index == 0 => 0.0,
        _ => lbox.width,
    };
    Rect {
        x,
        y: 0.0,
        w: 0.0,
        h: lbox.height + lbox.depth,
    }
}

/// Left edge of the `index`-th non-kern child of an HBox.
fn caret_x_in_hbox(children: &[LayoutBox], index: usize) -> f64 {
    let mut x = 0.0;
    let mut atoms = 0usize;
    for c in children {
        if !matches!(c.content, BoxContent::Kern) {
            if atoms == index {
                return x;
            }
            atoms += 1;
        }
        x += c.width;
    }
    x
}

// ---------------------------------------------------------------------------
// General caret geometry (top row + nested slots).
// ---------------------------------------------------------------------------

/// A laid-out "cell": the model atoms it covers, the layout box, and its absolute left x.
struct Cell<'a> {
    lo: usize,
    hi: usize,
    boxx: &'a LayoutBox,
    x: f64,
}

/// Pair a row's model atoms with its HBox's non-kern children, folding a base atom and a
/// trailing `SupSub` atom into one cell. `x0`/`scale` are the row's absolute origin.
fn cells<'a>(atoms: &[Atom], children: &'a [LayoutBox], x0: f64, scale: f64) -> Vec<Cell<'a>> {
    let mut out = Vec::new();
    let mut x = x0;
    let mut ci = 0;
    let mut mi = 0;
    while mi < atoms.len() && ci < children.len() {
        while ci < children.len() && matches!(children[ci].content, BoxContent::Kern) {
            x += children[ci].width * scale;
            ci += 1;
        }
        if ci >= children.len() {
            break;
        }
        let boxx = &children[ci];
        let consumes = if matches!(atoms.get(mi + 1), Some(Atom::SupSub { .. })) {
            2
        } else {
            1
        };
        out.push(Cell {
            lo: mi,
            hi: mi + consumes,
            boxx,
            x,
        });
        x += boxx.width * scale;
        ci += 1;
        mi += consumes;
    }
    out
}

/// Absolute x of the caret *before* model atom `index`.
fn caret_x(atoms: &[Atom], children: &[LayoutBox], index: usize, x0: f64, scale: f64) -> f64 {
    let cells = cells(atoms, children, x0, scale);
    for c in &cells {
        if index <= c.lo {
            return c.x;
        }
        if index < c.hi {
            // Between a base and its script: sit at the base's right edge.
            if let BoxContent::SupSub { base, .. } = &c.boxx.content {
                return c.x + base.width * scale;
            }
            return c.x;
        }
    }
    cells.last().map_or(x0, |c| c.x + c.boxx.width * scale)
}

/// The model row for a structural atom's slot (non-panicking).
fn slot_model_row(atom: &Atom, slot: Slot) -> Option<&Row> {
    match (atom, slot) {
        (Atom::Frac { num, .. }, Slot::Num) => Some(num),
        (Atom::Frac { den, .. }, Slot::Den) => Some(den),
        (Atom::Sqrt { radicand, .. }, Slot::Radicand) => Some(radicand),
        (Atom::Sqrt { index: Some(i), .. }, Slot::Index) => Some(i),
        (Atom::Delim { body, .. }, Slot::Body) => Some(body),
        (Atom::SupSub { sub: Some(r), .. }, Slot::Sub) => Some(r),
        (Atom::SupSub { sup: Some(r), .. }, Slot::Sup) => Some(r),
        (Atom::Matrix { rows }, Slot::Cell(r, c)) => rows.get(r).and_then(|row| row.get(c)),
        _ => None,
    }
}

/// Dig through single-child HBox "spacing wrappers" (e.g. `\frac` sits inside an HBox with
/// surrounding kerns) to the structural box, accumulating leading-kern x. Multi-child
/// HBoxes (rows, delimiters) are left alone.
fn unwrap_box(boxx: &LayoutBox, x: f64, scale: f64) -> (&LayoutBox, f64) {
    if let BoxContent::HBox(children) = &boxx.content {
        let significant = children
            .iter()
            .filter(|c| !matches!(c.content, BoxContent::Kern))
            .count();
        if significant == 1 {
            let mut cx = x;
            for ch in children {
                if matches!(ch.content, BoxContent::Kern) {
                    cx += ch.width * scale;
                } else {
                    return unwrap_box(ch, cx, scale);
                }
            }
        }
    }
    (boxx, x)
}

/// Position cell `(tr, tc)` within a RaTeX `Array` box whose origin is `(ax, ay)` (baseline).
/// Mirrors `to_display_list`'s array layout: column widths set x, row heights/depths set y,
/// cells centered in their column.
fn array_cell(
    array: &LayoutBox,
    ax: f64,
    ay: f64,
    scale: f64,
    tr: usize,
    tc: usize,
) -> Option<(&LayoutBox, f64, f64, f64)> {
    let BoxContent::Array {
        cells,
        col_widths,
        col_aligns,
        row_heights,
        row_depths,
        col_gap,
        offset,
        content_x_offset,
        ..
    } = &array.content
    else {
        return None;
    };
    let cell = cells.get(tr)?.get(tc)?;
    let mut cy = ay - *offset * scale;
    for r in 0..tr {
        cy += (row_heights[r] + row_depths[r]) * scale;
    }
    cy += row_heights[tr] * scale;
    let cx = ax
        + *content_x_offset * scale
        + col_widths
            .iter()
            .take(tc)
            .map(|w| (*w + *col_gap) * scale)
            .sum::<f64>();
    let cw = col_widths[tc];
    let cell_x = match col_aligns.get(tc).copied().unwrap_or(b'c') {
        b'l' => cx,
        b'r' => cx + (cw - cell.width) * scale,
        _ => cx + (cw - cell.width) * scale / 2.0,
    };
    Some((cell, cell_x, cy, scale))
}

/// Position a structural box's slot: `(sub_box, x, baseline_y, scale)`. Mirrors RaTeX's
/// `to_display_list`. Handles fraction numerator/denominator and super/subscript; other
/// boxes (root, delimiter, operator limits, a structure carrying its own script) → `None`.
fn descend(
    boxx: &LayoutBox,
    x: f64,
    slot: Slot,
    y: f64,
    scale: f64,
) -> Option<(&LayoutBox, f64, f64, f64)> {
    match (&boxx.content, slot) {
        (
            BoxContent::Fraction {
                numer,
                numer_shift,
                numer_scale,
                ..
            },
            Slot::Num,
        ) => {
            let cs = scale * *numer_scale;
            let fx = x + (boxx.width * scale - numer.width * cs) / 2.0;
            Some((&**numer, fx, y - *numer_shift * scale, cs))
        }
        (
            BoxContent::Fraction {
                denom,
                denom_shift,
                denom_scale,
                ..
            },
            Slot::Den,
        ) => {
            let cs = scale * *denom_scale;
            let fx = x + (boxx.width * scale - denom.width * cs) / 2.0;
            Some((&**denom, fx, y + *denom_shift * scale, cs))
        }
        (
            BoxContent::SupSub {
                base,
                sup: Some(sup),
                sup_shift,
                sup_scale,
                center_scripts,
                italic_correction,
                ..
            },
            Slot::Sup,
        ) => {
            let cs = scale * *sup_scale;
            let sx = if *center_scripts {
                x + (boxx.width * scale - sup.width * cs) / 2.0
            } else {
                x + (base.width + *italic_correction) * scale
            };
            Some((&**sup, sx, y - *sup_shift * scale, cs))
        }
        (
            BoxContent::SupSub {
                base,
                sub: Some(sub),
                sub_shift,
                sub_scale,
                center_scripts,
                sub_h_kern,
                ..
            },
            Slot::Sub,
        ) => {
            let cs = scale * *sub_scale;
            let sx = if *center_scripts {
                x + (boxx.width * scale - sub.width * cs) / 2.0
            } else {
                x + (base.width + *sub_h_kern) * scale
            };
            Some((&**sub, sx, y + *sub_shift * scale, cs))
        }
        // Delimiter body: between the open and close delimiters.
        (BoxContent::LeftRight { left, inner, .. }, Slot::Body) => {
            Some((&**inner, x + left.width * scale, y, scale))
        }
        // Square-root radicand: right of the surd glyph.
        (BoxContent::Radical { body, .. }, Slot::Radicand) => {
            Some((&**body, x + (boxx.width - body.width) * scale, y, scale))
        }
        // Big-operator upper limit (∑, display ∫): centered above the operator.
        (
            BoxContent::OpLimits {
                base,
                sup: Some(sup),
                sup_kern,
                sup_scale,
                slant,
                base_shift,
                ..
            },
            Slot::Sup,
        ) => {
            let cs = scale * *sup_scale;
            let sx = x + (boxx.width * scale - sup.width * cs) / 2.0 + *slant * scale / 2.0;
            let sy = y - (base.height - *base_shift) * scale - *sup_kern * scale - sup.depth * cs;
            Some((&**sup, sx, sy, cs))
        }
        // Big-operator lower limit: centered below the operator.
        (
            BoxContent::OpLimits {
                base,
                sub: Some(sub),
                sub_kern,
                sub_scale,
                slant,
                base_shift,
                ..
            },
            Slot::Sub,
        ) => {
            let cs = scale * *sub_scale;
            let sx = x + (boxx.width * scale - sub.width * cs) / 2.0 - *slant * scale / 2.0;
            let sy = y + (base.depth + *base_shift) * scale + *sub_kern * scale + sub.height * cs;
            Some((&**sub, sx, sy, cs))
        }
        // Matrix cell — pmatrix wraps the grid in delimiters, so dig to the inner Array.
        (BoxContent::LeftRight { left, inner, .. }, Slot::Cell(r, c)) => {
            let (array, ax) = unwrap_box(inner, x + left.width * scale, scale);
            array_cell(array, ax, y, scale, r, c)
        }
        (BoxContent::Array { .. }, Slot::Cell(r, c)) => array_cell(boxx, x, y, scale, r, c),
        _ => None,
    }
}

/// Recursively descend `path`, then place the caret at `index` in the reached row.
fn locate(
    row: &Row,
    row_box: &LayoutBox,
    path: &[Step],
    index: usize,
    x: f64,
    y: f64,
    scale: f64,
) -> Option<Rect> {
    let children: &[LayoutBox] = match &row_box.content {
        BoxContent::HBox(c) => c,
        _ => std::slice::from_ref(row_box),
    };
    let Some((step, rest)) = path.split_first() else {
        return Some(Rect {
            x: caret_x(&row.atoms, children, index, x, scale),
            y: y - row_box.height * scale,
            w: 0.0,
            h: (row_box.height + row_box.depth) * scale,
        });
    };
    let cell = cells(&row.atoms, children, x, scale)
        .into_iter()
        .find(|c| step.atom >= c.lo && step.atom < c.hi)?;
    let (sbox, sx0) = unwrap_box(cell.boxx, cell.x, scale);
    match (
        descend(sbox, sx0, step.slot, y, scale),
        slot_model_row(&row.atoms[step.atom], step.slot),
    ) {
        (Some((sub_box, sx, sy, ss)), Some(sub_row)) => {
            locate(sub_row, sub_box, rest, index, sx, sy, ss)
        }
        // Fallback for a slot we can't descend into yet: a caret on the structure itself,
        // so the bar never fully vanishes.
        _ => Some(Rect {
            x: sx0,
            y: y - sbox.height * scale,
            w: 0.0,
            h: (sbox.height + sbox.depth) * scale,
        }),
    }
}

/// The absolute caret rect (em, top-left origin) for `cursor`, or `None` if it lands in a
/// structure the walk doesn't handle yet. Uses the display list's baseline so the caret
/// lines up with `render`'s raster.
pub fn caret_rect(top: &Row, cursor: &Cursor) -> Option<Rect> {
    let root = layout_row(top);
    let baseline = to_display_list(&root).height;
    locate(top, &root, &cursor.path, cursor.index, 0.0, baseline, 1.0)
}

/// The screen rect (em, top-left origin) of the matrix box the cursor is inside — the
/// structure box its final path step descends into, when that step is a matrix cell.
/// `None` if the cursor isn't in a matrix. Used to dock the matrix toolbar beside the grid.
pub fn matrix_rect(top: &Row, cursor: &Cursor) -> Option<Rect> {
    if !matches!(
        cursor.path.last(),
        Some(Step {
            slot: Slot::Cell(..),
            ..
        })
    ) {
        return None;
    }
    let root = layout_row(top);
    let baseline = to_display_list(&root).height;
    box_at_last_step(top, &root, &cursor.path, 0.0, baseline, 1.0)
}

/// Walk `path`, returning the rect of the structural box the FINAL step descends into
/// (rather than the caret inside its slot). Mirrors `locate`'s descent.
fn box_at_last_step(
    row: &Row,
    row_box: &LayoutBox,
    path: &[Step],
    x: f64,
    y: f64,
    scale: f64,
) -> Option<Rect> {
    let children: &[LayoutBox] = match &row_box.content {
        BoxContent::HBox(c) => c,
        _ => std::slice::from_ref(row_box),
    };
    let (step, rest) = path.split_first()?;
    let cell = cells(&row.atoms, children, x, scale)
        .into_iter()
        .find(|c| step.atom >= c.lo && step.atom < c.hi)?;
    let (sbox, sx0) = unwrap_box(cell.boxx, cell.x, scale);
    if rest.is_empty() {
        return Some(Rect {
            x: sx0,
            y: y - sbox.height * scale,
            w: sbox.width * scale,
            h: (sbox.height + sbox.depth) * scale,
        });
    }
    let (sub_box, sx, sy, ss) = descend(sbox, sx0, step.slot, y, scale)?;
    let sub_row = slot_model_row(&row.atoms[step.atom], step.slot)?;
    box_at_last_step(sub_row, sub_box, rest, sx, sy, ss)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::cursor::Cursor;
    use crate::editor::model::{Atom, Row};

    fn width(row: &Row) -> f64 {
        layout_row(row).width
    }

    #[test]
    fn caret_start_is_zero() {
        assert!(caret_in_top_row(&Row::syms("abc"), 0).x.abs() < 1e-9);
    }

    #[test]
    fn caret_end_is_full_width() {
        let r = Row::syms("abc");
        let end = caret_in_top_row(&r, 3).x;
        assert!((end - width(&r)).abs() < 1e-6, "end caret {end} != width");
    }

    #[test]
    fn carets_are_monotonic_across_operators() {
        let r = Row::syms("a+b=c");
        let xs: Vec<f64> = (0..=r.atoms.len())
            .map(|i| caret_in_top_row(&r, i).x)
            .collect();
        for w in xs.windows(2) {
            assert!(w[1] >= w[0] - 1e-9, "caret x not monotonic: {xs:?}");
        }
        assert!(xs[0].abs() < 1e-9);
        assert!((xs[xs.len() - 1] - width(&r)).abs() < 1e-6);
    }

    #[test]
    fn caret_between_atoms_is_interior() {
        let r = Row::syms("ab");
        let mid = caret_in_top_row(&r, 1).x;
        let full = width(&r);
        assert!(mid > 0.0 && mid < full, "mid {mid} not inside (0, {full})");
    }

    #[test]
    fn caret_has_height() {
        assert!(caret_in_top_row(&Row::syms("x"), 0).h > 0.0);
    }

    #[test]
    fn caret_rect_matches_top_row_when_flat() {
        // For a script-free row, the general walk agrees with the top-row helper on x.
        for s in ["abc", "a+b=c"] {
            let r = Row::syms(s);
            for i in 0..=r.atoms.len() {
                let cur = Cursor {
                    path: vec![],
                    index: i,
                };
                let got = caret_rect(&r, &cur).expect("top caret").x;
                let want = caret_in_top_row(&r, i).x;
                assert!((got - want).abs() < 1e-6, "{s}@{i}: {got} != {want}");
            }
        }
    }

    fn frac(n: &str, d: &str) -> Atom {
        Atom::Frac {
            num: Row::syms(n),
            den: Row::syms(d),
        }
    }

    #[test]
    fn fraction_carets_stack_numerator_above_denominator() {
        let top = Row {
            atoms: vec![frac("a", "b")],
        };
        let num = Cursor {
            path: vec![Step {
                atom: 0,
                slot: Slot::Num,
            }],
            index: 1,
        };
        let den = Cursor {
            path: vec![Step {
                atom: 0,
                slot: Slot::Den,
            }],
            index: 1,
        };
        let nr = caret_rect(&top, &num).expect("numerator caret");
        let dr = caret_rect(&top, &den).expect("denominator caret");
        assert!(
            dr.y > nr.y,
            "denominator caret ({}) not below numerator ({})",
            dr.y,
            nr.y
        );
        let w = width(&top);
        assert!(
            nr.x > 0.0 && nr.x <= w,
            "numerator caret x {} out of (0,{w}]",
            nr.x
        );
    }

    #[test]
    fn superscript_caret_is_located() {
        // x^2 with the cursor in the (empty) superscript.
        let top = Row {
            atoms: vec![
                Atom::Sym("x".into()),
                Atom::SupSub {
                    sup: Some(Row::syms("2")),
                    sub: None,
                },
            ],
        };
        let cur = Cursor {
            path: vec![Step {
                atom: 1,
                slot: Slot::Sup,
            }],
            index: 0,
        };
        let r = caret_rect(&top, &cur).expect("superscript caret");
        assert!(r.x > 0.0, "sup caret right of the base, got x={}", r.x);
        assert!(
            r.y >= -1e-9 && r.h > 0.0,
            "sup caret within bounds, got {r:?}"
        );
    }

    #[test]
    fn matrix_cell_carets_are_distinct() {
        let top = Row {
            atoms: vec![Atom::Matrix {
                rows: vec![
                    vec![Row::syms("a"), Row::syms("b")],
                    vec![Row::syms("c"), Row::syms("d")],
                ],
            }],
        };
        let at = |r, c| Cursor {
            path: vec![Step {
                atom: 0,
                slot: Slot::Cell(r, c),
            }],
            index: 0,
        };
        let r00 = caret_rect(&top, &at(0, 0)).expect("cell (0,0)");
        let r11 = caret_rect(&top, &at(1, 1)).expect("cell (1,1)");
        // Bottom-right cell is right of and below the top-left one (not the matrix fallback).
        assert!(
            r11.x > r00.x,
            "(1,1) right of (0,0): {} vs {}",
            r11.x,
            r00.x
        );
        assert!(r11.y > r00.y, "(1,1) below (0,0): {} vs {}", r11.y, r00.y);
    }

    #[test]
    fn matrix_rect_encloses_the_caret() {
        let top = Row {
            atoms: vec![Atom::Matrix {
                rows: vec![
                    vec![Row::syms("a"), Row::syms("b")],
                    vec![Row::syms("c"), Row::syms("d")],
                ],
            }],
        };
        let cur = Cursor {
            path: vec![Step {
                atom: 0,
                slot: Slot::Cell(0, 0),
            }],
            index: 0,
        };
        let m = matrix_rect(&top, &cur).expect("in a matrix");
        let c = caret_rect(&top, &cur).expect("cell caret");
        assert!(m.w > 0.0 && m.h > 0.0, "matrix has area: {m:?}");
        assert!(
            m.x <= c.x + 1e-6 && c.x <= m.x + m.w + 1e-6,
            "caret x within matrix: caret {c:?} matrix {m:?}"
        );
        assert!(
            m.y <= c.y + 1e-6 && c.y <= m.y + m.h + 1e-6,
            "caret y within matrix: caret {c:?} matrix {m:?}"
        );
        // Outside a matrix there's no toolbar anchor.
        let flat = Cursor {
            path: vec![],
            index: 0,
        };
        assert!(matrix_rect(&Row::syms("x"), &flat).is_none());
    }
}
