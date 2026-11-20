//! **Islands**: each topic becomes an island with a soft outline and its
//! name, packed into a sheet; links between topics are dashed. Unlinked pages
//! leave the graph for a tray of dots underneath, so none of them get lost.

use gpui::{Hsla, Pixels, Point, SharedString, point, px};
use rust_i18n::t;

use super::style::{
    Align, Frame, GraphStyle, Layout, Painter, VAlign, force_layout, hull, is_dark, palette,
};
use super::{Graph, Kind};
use crate::theme;

pub struct Islands;

/// Room around an island's nodes for its outline and name.
const PAD: f32 = 70.0;
/// Space between tray dots.
const TRAY_GAP: f32 = 16.0;

impl GraphStyle for Islands {
    fn id(&self) -> &'static str {
        "islands"
    }

    fn name(&self) -> SharedString {
        t!("graph.style_islands").into()
    }

    fn layout(&self, g: &Graph, _focus: Option<usize>) -> Layout {
        let mut pos = vec![[0.0f32; 2]; g.nodes.len()];
        // Each island on its own, then shelf-packed largest first into rows
        // about as wide as the whole sheet is tall.
        let mut boxes: Vec<[f32; 2]> = Vec::new();
        for grp in &g.groups {
            let local: std::collections::HashMap<usize, usize> = grp
                .members
                .iter()
                .enumerate()
                .map(|(l, &i)| (i, l))
                .collect();
            let edges: Vec<(usize, usize)> = g
                .edges
                .iter()
                .filter_map(|(a, b)| Some((*local.get(a)?, *local.get(b)?)))
                .collect();
            let lp = force_layout(grp.members.len(), &edges);
            let minx = lp.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
            let miny = lp.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
            let maxx = lp.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
            let maxy = lp.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
            for (p, &i) in lp.iter().zip(&grp.members) {
                pos[i] = [p[0] - minx + PAD, p[1] - miny + PAD];
            }
            boxes.push([maxx - minx + PAD * 2.0, maxy - miny + PAD * 2.0]);
        }
        let area: f32 = boxes.iter().map(|b| b[0] * b[1]).sum();
        let row_w = area
            .sqrt()
            .max(boxes.iter().map(|b| b[0]).fold(0.0, f32::max))
            * 1.25;
        let (mut x, mut y, mut row_h, mut sheet_w) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for (grp, b) in g.groups.iter().zip(&boxes) {
            if x > 0.0 && x + b[0] > row_w {
                x = 0.0;
                y += row_h;
                row_h = 0.0;
            }
            for &i in &grp.members {
                pos[i][0] += x;
                pos[i][1] += y;
            }
            x += b[0];
            sheet_w = sheet_w.max(x);
            row_h = row_h.max(b[1]);
        }
        let sheet_h = y + row_h;
        // The tray: unlinked pages in a grid under the sheet.
        let sheet_w = sheet_w.max(600.0);
        let cols = ((sheet_w - PAD) / TRAY_GAP).max(1.0) as usize;
        let tray_y = sheet_h + PAD;
        let loose = (0..g.nodes.len()).filter(|&i| g.nodes[i].group.is_none());
        for (k, i) in loose.enumerate() {
            pos[i] = [
                PAD / 2.0 + (k % cols) as f32 * TRAY_GAP,
                tray_y + (k / cols) as f32 * TRAY_GAP,
            ];
        }
        Layout::all(pos)
    }

    fn paint(&self, g: &Graph, f: &Frame, p: &mut Painter) {
        let v = p.view;
        let dark = is_dark();
        let hovered_group = f.hover.and_then(|h| g.nodes[h].group);
        // Islands: the hull of a circle around every member, so the outline
        // is rounded and hugs the group.
        for (gi, grp) in g.groups.iter().enumerate() {
            let pad = PAD * 0.45;
            let mut pts: Vec<[f32; 2]> = Vec::with_capacity(grp.members.len() * 12);
            for &i in &grp.members {
                for k in 0..12 {
                    let a = k as f32 * std::f32::consts::TAU / 12.0;
                    pts.push([f.pos[i][0] + pad * a.cos(), f.pos[i][1] + pad * a.sin()]);
                }
            }
            let outline: Vec<Point<Pixels>> = hull(&pts).into_iter().map(|q| v.at(q)).collect();
            let faded = hovered_group.is_some_and(|h| h != gi) || f.matches.is_some();
            let tint = palette(gi).opacity(match (dark, faded) {
                (true, false) => 0.16,
                (false, false) => 0.13,
                (_, true) => 0.06,
            });
            p.fill_polygon(&outline, tint);
            let top = outline
                .iter()
                .map(|q| q.y)
                .fold(px(f32::MAX), |a, b| a.min(b));
            let (l, r) = outline
                .iter()
                .fold((px(f32::MAX), px(f32::MIN)), |(l, r), q| {
                    (l.min(q.x), r.max(q.x))
                });
            let label = p.fit(&grp.label.to_uppercase().into(), 11.0, true, r - l);
            p.text(
                &label,
                point((l + r) / 2.0, top - px(4.0)),
                11.0,
                label_color(palette(gi), dark),
                Align::Center,
                VAlign::Bottom,
                true,
            );
        }
        // Edges: solid inside an island, dashed between islands.
        let quiet = theme::text_tertiary().opacity(if f.hover.is_some() || f.matches.is_some() {
            0.12
        } else {
            0.45
        });
        for (cross, dash) in [(false, None), (true, Some([4.0, 3.0]))] {
            p.stroke(1.0, quiet, dash, |pb| {
                let mut any = false;
                for &(a, b) in &g.edges {
                    if f.edge_lit(a, b) || (g.nodes[a].group != g.nodes[b].group) != cross {
                        continue;
                    }
                    pb.move_to(v.at(f.pos[a]));
                    pb.line_to(v.at(f.pos[b]));
                    any = true;
                }
                any
            });
        }
        p.stroke(1.5, theme::accent(), None, |pb| {
            let mut any = false;
            for &(a, b) in &g.edges {
                if f.edge_lit(a, b) {
                    pb.move_to(v.at(f.pos[a]));
                    pb.line_to(v.at(f.pos[b]));
                    any = true;
                }
            }
            any
        });
        // Nodes: hubs and whiteboards filled, the rest hollow in their color.
        let paper = Hsla {
            a: 1.0,
            ..theme::bg_content()
        };
        for (i, n) in g.nodes.iter().enumerate() {
            let Some(gi) = n.group else { continue };
            let c = v.at(f.pos[i]);
            let r = v.len(n.radius * 0.7, 3.0);
            let color = if f.lit(i) {
                theme::accent()
            } else {
                palette(gi)
            };
            let a = if f.dimmed(i) { 0.25 } else { 1.0 };
            if n.degree >= 6 || n.kind == Kind::Board || f.lit(i) {
                p.node(c, r, color.opacity(a), n.kind == Kind::Board);
            } else {
                p.ring(c, r, paper.opacity(a), color.opacity(a), px(1.6));
            }
        }
        for i in f.label_order(g) {
            let n = &g.nodes[i];
            let show = if f.hover.is_some() || f.matches.is_some() {
                n.group.is_some() && !f.dimmed(i)
            } else {
                n.group.is_some() && (n.degree >= 3 || p.zoom() >= 1.2)
            };
            if show {
                let c = v.at(f.pos[i]);
                let at = point(c.x, c.y - v.len(n.radius * 0.7, 3.0) - px(4.0));
                let color = if f.lit(i) {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                };
                p.label(&n.title, at, color, f.lit(i), f.lit(i));
            }
        }
        // The tray.
        let loose: Vec<usize> = (0..g.nodes.len())
            .filter(|&i| g.nodes[i].group.is_none())
            .collect();
        if let Some(&first) = loose.first() {
            let corner = v.at(f.pos[first]);
            let label: SharedString = t!("graph.unlinked", count = loose.len())
                .to_uppercase()
                .into();
            p.text(
                &label,
                point(corner.x - px(4.0), corner.y - px(14.0)),
                11.0,
                theme::text_tertiary(),
                Align::Left,
                VAlign::Bottom,
                true,
            );
        }
        for &i in &loose {
            let (r, color) = if f.lit(i) {
                (px(3.5), theme::accent())
            } else {
                let a = if f.dimmed(i) { 0.2 } else { 0.6 };
                (v.len(3.0, 1.5), theme::text_tertiary().opacity(a))
            };
            p.dot(v.at(f.pos[i]), r, color);
        }
    }
}

/// A topic color dark enough to read as text on the current theme.
fn label_color(c: Hsla, dark: bool) -> Hsla {
    if dark {
        Hsla { l: 0.78, ..c }
    } else {
        Hsla { l: 0.32, ..c }
    }
}
