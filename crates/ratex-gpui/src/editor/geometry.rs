//! Geometry — map the model + RaTeX layout to caret/slot rects. GUI-free, em units.
//!
//! M1 scope: the caret within the **top-level** row, positioned exactly from RaTeX's
//! `LayoutBox` (kern-aware, so the caret sits in the right gap even around operator
//! spacing). Nested-slot rects (fraction numerator, script body, …) extend the same
//! walk in the next increment.

use crate::editor::model::Row;
use ratex_layout::layout_box::{BoxContent, LayoutBox};
use ratex_layout::{LayoutOptions, layout};
use ratex_parser::parse;

/// A rectangle in **em** units (1 em = the layout font size), origin at the laid-out
/// formula's top-left, y growing downward. The view scales by the render font size
/// and offsets by the render origin.
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

/// Caret rect for the cursor at `index` (`0..=atom_count`) in the **top-level** row.
/// `x` is the gap position; the caret spans the row's full height. `w` is 0 — the
/// view renders it as a thin line.
pub fn caret_in_top_row(row: &Row, index: usize) -> Rect {
    let lbox = layout_row(row);
    let x = match &lbox.content {
        BoxContent::HBox(children) => caret_x_in_hbox(children, index),
        // Single-box row (one atom / glyph): caret before (0) or after (width).
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

/// Walk an HBox left-to-right; return the x of the left edge of the `index`-th
/// non-kern child (the "caret before atom `index`" position), or the total width if
/// the caret is past the last atom. Kerns (inter-atom spacing) advance x but don't
/// count as atoms, so the mapping to model atoms stays 1:1.
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
    x // past the last atom → total width
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model::Row;

    fn width(row: &Row) -> f64 {
        layout_row(row).width
    }

    #[test]
    fn caret_start_is_zero() {
        let r = Row::syms("abc");
        assert!(caret_in_top_row(&r, 0).x.abs() < 1e-9);
    }

    #[test]
    fn caret_end_is_full_width() {
        let r = Row::syms("abc");
        let end = caret_in_top_row(&r, 3).x;
        assert!((end - width(&r)).abs() < 1e-6, "end caret {end} != width");
    }

    #[test]
    fn carets_are_monotonic_across_operators() {
        // Operators insert spacing kerns; positions must still be non-decreasing.
        let r = Row::syms("a+b=c");
        let xs: Vec<f64> = (0..=r.atoms.len())
            .map(|i| caret_in_top_row(&r, i).x)
            .collect();
        for w in xs.windows(2) {
            assert!(w[1] >= w[0] - 1e-9, "caret x not monotonic: {xs:?}");
        }
        assert!(xs[0].abs() < 1e-9, "first caret not at 0");
        assert!(
            (xs[xs.len() - 1] - width(&r)).abs() < 1e-6,
            "last caret not at full width"
        );
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
}
