//! **Nebula**: linked pages pull together and glow in their topic's color,
//! sized by how linked they are; pages without links drift as faint dust in
//! a loose halo around them instead of forming rings. Only hubs carry labels
//! until you hover or zoom in.

use std::collections::HashMap;

use gpui::{SharedString, point, px};
use rust_i18n::t;

use super::Graph;
use super::classic::arrange;
use super::style::{Frame, GraphStyle, Layout, Painter, hash_unit, is_dark, palette};
use crate::theme;

pub struct Nebula;

impl GraphStyle for Nebula {
    fn id(&self) -> &'static str {
        "nebula"
    }

    fn name(&self) -> SharedString {
        t!("graph.style_nebula").into()
    }

    fn layout(&self, g: &Graph, _focus: Option<usize>) -> Layout {
        let n = g.nodes.len();
        // Linked nodes: Classic's arrangement, without the singletons.
        let linked: Vec<usize> = (0..n).filter(|&i| g.nodes[i].degree > 0).collect();
        let local: HashMap<usize, usize> =
            linked.iter().enumerate().map(|(l, &i)| (i, l)).collect();
        let edges: Vec<(usize, usize)> =
            g.edges.iter().map(|(a, b)| (local[a], local[b])).collect();
        let radii: Vec<f32> = linked.iter().map(|&i| g.nodes[i].radius).collect();
        let core = arrange(&radii, &edges);
        let mut pos = vec![[0.0f32; 2]; n];
        let mut extent = 160.0f32;
        for (l, &i) in linked.iter().enumerate() {
            pos[i] = core[l];
            extent = extent.max(core[l][0].hypot(core[l][1]));
        }
        // The rest: a loose halo just outside, scattered by a seeded hash.
        let mut k = 0;
        for (i, p) in pos.iter_mut().enumerate() {
            if g.nodes[i].degree > 0 {
                continue;
            }
            let a = k as f32 * 2.399_963 + hash_unit(i) * 0.4;
            let r = extent * (1.12 + 0.28 * (hash_unit(i + 7919) + 1.0) / 2.0) + 40.0;
            *p = [r * a.cos(), r * a.sin()];
            k += 1;
        }
        Layout::all(pos)
    }

    fn paint(&self, g: &Graph, f: &Frame, p: &mut Painter) {
        let v = p.view;
        let dark = is_dark();
        let color_of = |i: usize| g.nodes[i].group.map_or(theme::text_tertiary(), palette);
        // Dust.
        for (i, n) in g.nodes.iter().enumerate() {
            if n.degree > 0 {
                continue;
            }
            let (r, color) = if f.lit(i) {
                (px(3.5), theme::accent())
            } else {
                let a = if f.dimmed(i) { 0.15 } else { 0.45 };
                (v.len(1.6, 1.3), theme::text_tertiary().opacity(a))
            };
            p.dot(v.at(f.pos[i]), r, color);
        }
        // Edges, bowed slightly, one path per topic color.
        let quiet = if f.hover.is_some() || f.matches.is_some() {
            0.07
        } else {
            0.38
        };
        let mut by_color: HashMap<Option<usize>, Vec<(usize, usize)>> = HashMap::new();
        for &(a, b) in &g.edges {
            if !f.edge_lit(a, b) {
                by_color.entry(g.nodes[a].group).or_default().push((a, b));
            }
        }
        let curve = |pb: &mut gpui::PathBuilder, a: usize, b: usize| {
            let (pa, pb_) = (v.at(f.pos[a]), v.at(f.pos[b]));
            let (dx, dy) = (pb_.x - pa.x, pb_.y - pa.y);
            let ctrl = point(
                (pa.x + pb_.x) / 2.0 - dy * 0.14,
                (pa.y + pb_.y) / 2.0 + dx * 0.14,
            );
            pb.move_to(pa);
            pb.curve_to(pb_, ctrl);
        };
        for (group, list) in &by_color {
            let color = group.map_or(theme::divider(), palette).opacity(quiet);
            p.stroke(1.0, color, None, |pb| {
                for &(a, b) in list {
                    curve(pb, a, b);
                }
                !list.is_empty()
            });
        }
        p.stroke(1.6, theme::accent(), None, |pb| {
            let mut any = false;
            for &(a, b) in &g.edges {
                if f.edge_lit(a, b) {
                    curve(pb, a, b);
                    any = true;
                }
            }
            any
        });
        // Glow, then cores on top.
        for (i, n) in g.nodes.iter().enumerate() {
            if n.degree == 0 || f.dimmed(i) {
                continue;
            }
            let strength = if f.lit(i) {
                0.8
            } else if dark {
                0.5
            } else {
                0.32
            };
            p.glow(
                v.at(f.pos[i]),
                v.len(n.radius * 0.7, 3.0),
                color_of(i).opacity(strength),
            );
        }
        for (i, n) in g.nodes.iter().enumerate() {
            if n.degree == 0 {
                continue;
            }
            let base = color_of(i);
            let core = if dark {
                gpui::Hsla {
                    l: (base.l + 0.14).min(0.92),
                    ..base
                }
            } else {
                base
            };
            let core = if f.dimmed(i) { core.opacity(0.2) } else { core };
            p.node(
                v.at(f.pos[i]),
                v.len(n.radius * 0.62, 2.5),
                core,
                n.kind == super::Kind::Board,
            );
        }
        // Labels: hubs at rest, the neighborhood on hover, everything when
        // zoomed in.
        let top = g.nodes.iter().map(|n| n.degree).max().unwrap_or(0);
        for i in f.label_order(g) {
            let n = &g.nodes[i];
            let show = if f.hover.is_some() || f.matches.is_some() {
                !f.dimmed(i)
            } else {
                n.degree > 0 && (p.zoom() >= 1.3 || (n.degree >= 3 && n.degree * 2 >= top))
            };
            if !show {
                continue;
            }
            let c = v.at(f.pos[i]);
            let at = point(c.x, c.y - v.len(n.radius * 0.62, 2.5) - px(5.0));
            let color = if f.lit(i) {
                theme::text_primary()
            } else {
                theme::text_secondary()
            };
            p.label(&n.title, at, color, f.lit(i), f.lit(i));
        }
    }
}
