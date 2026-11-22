//! **Focus**: one page in the middle, the pages it links with on an inner
//! ring, their links on an outer ring, everything else out of the way.
//! Clicking a page brings it to the center; clicking the center opens it.

use std::f32::consts::{FRAC_PI_2, TAU};

use gpui::{Bounds, BoxShadow, Corners, SharedString, point, px, quad, size};
use rust_i18n::t;

use super::style::{Align, Frame, GraphStyle, Layout, Painter, VAlign, palette};
use super::{Graph, Kind};
use crate::theme;

pub struct Focus;

/// The rings around a center: first ring, then each second-ring node with
/// its first-ring parent, both in drawing order.
fn rings(g: &Graph, f: usize) -> (Vec<usize>, Vec<(usize, usize)>) {
    let mut l1 = g.adj[f].clone();
    l1.sort_by(|&a, &b| {
        (g.nodes[a].group, &g.nodes[a].title).cmp(&(g.nodes[b].group, &g.nodes[b].title))
    });
    let mut seen = vec![false; g.nodes.len()];
    seen[f] = true;
    l1.iter().for_each(|&i| seen[i] = true);
    let mut l2 = Vec::new();
    for &p in &l1 {
        for &q in &g.adj[p] {
            if !seen[q] {
                seen[q] = true;
                l2.push((q, p));
            }
        }
    }
    (l1, l2)
}

impl GraphStyle for Focus {
    fn id(&self) -> &'static str {
        "focus"
    }

    fn name(&self) -> SharedString {
        t!("graph.style_focus").into()
    }

    fn layout(&self, g: &Graph, focus: Option<usize>) -> Layout {
        let n = g.nodes.len();
        let mut pos = vec![[0.0f32; 2]; n];
        let mut shown = vec![false; n];
        let Some(f) = focus.or(g.hub()) else {
            return Layout { pos, shown };
        };
        shown[f] = true;
        let (l1, l2) = rings(g, f);
        // Rings big enough that neighbors get ~36 / ~26 units of arc each.
        let r1 = (l1.len() as f32 * 36.0 / TAU).max(170.0);
        let r2 = (l2.len() as f32 * 26.0 / TAU).max(r1 + 150.0);
        let place = |k: usize, count: usize, r: f32| {
            let a = -FRAC_PI_2 + (k as f32 + 0.5) / count as f32 * TAU;
            [r * a.cos(), r * a.sin()]
        };
        for (k, &i) in l1.iter().enumerate() {
            pos[i] = place(k, l1.len(), r1);
            shown[i] = true;
        }
        // The outer ring spread evenly, in parent order, so a busy parent's
        // pages don't pile up; every other one steps out to fit more labels.
        for (k, &(i, _)) in l2.iter().enumerate() {
            pos[i] = place(k, l2.len(), r2 + (k % 2) as f32 * 22.0);
            shown[i] = true;
        }
        Layout { pos, shown }
    }

    fn paint(&self, g: &Graph, fr: &Frame, p: &mut Painter) {
        let v = p.view;
        let Some(f) = fr.focus.or(g.hub()) else {
            return;
        };
        let (l1, l2) = rings(g, f);
        let c = v.at(fr.pos[f]);
        let dim = |i: usize| {
            if fr.dimmed(i) && fr.hover != Some(f) {
                0.25
            } else {
                1.0
            }
        };
        // Guide rings through the first node of each ring.
        for &i in [l1.first(), l2.first().map(|(i, _)| i)].iter().flatten() {
            let q = v.at(fr.pos[*i]);
            let r = f32::from(q.x - c.x).hypot(f32::from(q.y - c.y));
            p.stroke(1.0, theme::divider().opacity(0.5), None, |pb| {
                let pts: Vec<_> = (0..=96)
                    .map(|k| {
                        let a = k as f32 / 96.0 * TAU;
                        point(c.x + px(r * a.cos()), c.y + px(r * a.sin()))
                    })
                    .collect();
                pb.add_polygon(&pts, true);
                true
            });
        }
        // Spokes from the center, then thin lines out to the second ring.
        for &i in &l1 {
            let q = v.at(fr.pos[i]);
            let color = if fr.edge_lit(f, i) {
                theme::accent()
            } else {
                group_color(g, i).opacity(0.55 * dim(i))
            };
            p.stroke(1.4, color, None, |pb| {
                let (dx, dy) = (q.x - c.x, q.y - c.y);
                pb.move_to(c);
                pb.curve_to(
                    q,
                    point((c.x + q.x) / 2.0 - dy * 0.12, (c.y + q.y) / 2.0 + dx * 0.12),
                );
                true
            });
        }
        p.stroke(1.0, theme::text_tertiary().opacity(0.35), None, |pb| {
            for &(i, parent) in &l2 {
                pb.move_to(v.at(fr.pos[parent]));
                pb.line_to(v.at(fr.pos[i]));
            }
            !l2.is_empty()
        });
        // Outer ring, then inner ring.
        for &(i, _) in &l2 {
            let q = v.at(fr.pos[i]);
            let color = group_color(g, i).opacity(dim(i));
            p.node(q, px(3.0), color, g.nodes[i].kind == Kind::Board);
        }
        for &i in &l1 {
            let q = v.at(fr.pos[i]);
            let r = px(if fr.hover == Some(i) { 6.5 } else { 5.0 });
            let color = group_color(g, i).opacity(dim(i));
            p.node(q, r, color, g.nodes[i].kind == Kind::Board);
        }
        // Labels pointing away from the center: the hovered page's first, then
        // the inner ring's, then the outer ring's where there's room.
        let mut order: Vec<(usize, bool)> = l1.iter().map(|&i| (i, true)).collect();
        order.extend(l2.iter().map(|&(i, _)| (i, false)));
        order.sort_by_key(|&(i, _)| !fr.lit(i));
        for (i, inner) in order {
            let q = v.at(fr.pos[i]);
            let lit = fr.lit(i);
            let (gap, size, color, bold) = match (inner, lit) {
                (true, true) => (10.0, 12.0, theme::text_primary(), true),
                (false, true) => (7.0, 10.5, theme::text_primary(), true),
                (true, false) => (
                    10.0,
                    12.0,
                    theme::text_primary().opacity(0.85 * dim(i)),
                    true,
                ),
                (false, false) => (7.0, 10.5, theme::text_secondary().opacity(dim(i)), false),
            };
            radial_label(p, &g.nodes[i].title, c, q, gap, size, color, bold, lit);
        }
        // The center: a pill with a soft accent glow.
        let title = &g.nodes[f].title;
        let w = p.measure(title, 15.0, true) + px(32.0);
        let h = px(36.0);
        let b = Bounds::new(point(c.x - w / 2.0, c.y - h / 2.0), size(w, h));
        p.window.paint_drop_shadows(
            b,
            Corners::all(h / 2.0),
            &[BoxShadow {
                color: theme::accent().opacity(0.45),
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(24.0),
                spread_radius: px(0.0),
                inset: false,
            }],
        );
        p.window.paint_quad(quad(
            b,
            Corners::all(h / 2.0),
            gpui::Hsla {
                a: 1.0,
                ..theme::elevated()
            },
            gpui::Edges::all(px(1.2)),
            theme::accent(),
            gpui::BorderStyle::default(),
        ));
        p.text(
            title,
            c,
            15.0,
            theme::text_primary(),
            Align::Center,
            VAlign::Middle,
            true,
        );
        // Ring counts, bottom right.
        let legend: SharedString =
            t!("graph.focus_legend", linked = l1.len(), two = l2.len()).into();
        let corner = p.bounds.bottom_right();
        p.text(
            &legend,
            point(corner.x - px(16.0), corner.y - px(10.0)),
            11.0,
            theme::text_tertiary(),
            Align::Right,
            VAlign::Bottom,
            false,
        );
    }

    fn draggable(&self) -> bool {
        false
    }

    fn uses_focus(&self) -> bool {
        true
    }

    fn hint(&self) -> Option<SharedString> {
        Some(t!("graph.hint_focus", key = crate::settings::keys::MOD).into())
    }
}

fn group_color(g: &Graph, i: usize) -> gpui::Hsla {
    g.nodes[i].group.map_or(theme::text_tertiary(), palette)
}

/// A label just outside `at`, on the far side from `center`.
#[allow(clippy::too_many_arguments)]
fn radial_label(
    p: &mut Painter,
    text: &SharedString,
    center: gpui::Point<gpui::Pixels>,
    at: gpui::Point<gpui::Pixels>,
    gap: f32,
    size: f32,
    color: gpui::Hsla,
    bold: bool,
    always: bool,
) {
    let (dx, dy) = (f32::from(at.x - center.x), f32::from(at.y - center.y));
    let len = dx.hypot(dy).max(0.01);
    let (cx, cy) = (dx / len, dy / len);
    let align = if cx > 0.25 {
        Align::Left
    } else if cx < -0.25 {
        Align::Right
    } else {
        Align::Center
    };
    let valign = if cy > 0.45 {
        VAlign::Top
    } else if cy < -0.45 {
        VAlign::Bottom
    } else {
        VAlign::Middle
    };
    let q = point(at.x + px(cx * gap), at.y + px(cy * gap));
    p.label_at(text, q, size, color, align, valign, bold, always);
}
