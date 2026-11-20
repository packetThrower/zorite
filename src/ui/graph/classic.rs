//! **Classic**: Zorite's original graph. A force layout per connected
//! component, the biggest in the middle, smaller linked groups strewn around
//! it and unlinked pages on rings outside, Logseq-style. Gray dots, the
//! accent for whiteboards and highlights.

use gpui::{SharedString, px};
use rust_i18n::t;

use super::Graph;
use super::style::{Align, Frame, GraphStyle, Layout, Painter, VAlign, force_layout, hash_unit};
use crate::theme;

pub struct Classic;

impl GraphStyle for Classic {
    fn id(&self) -> &'static str {
        "classic"
    }

    fn name(&self) -> SharedString {
        t!("graph.style_classic").into()
    }

    fn layout(&self, g: &Graph, _focus: Option<usize>) -> Layout {
        let radii: Vec<f32> = g.nodes.iter().map(|n| n.radius).collect();
        Layout::all(arrange(&radii, &g.edges))
    }

    fn paint(&self, g: &Graph, f: &Frame, p: &mut Painter) {
        let (accent, accent_tint) = (theme::accent(), theme::accent_tint());
        let quiet = if f.matches.is_some() {
            theme::divider().opacity(0.3)
        } else {
            theme::divider()
        };
        // One path for the quiet edges, one for the hovered node's, so the
        // highlight paints on top.
        let v = p.view;
        for (lit, color, width) in [(false, quiet, 1.0), (true, accent, 1.5)] {
            p.stroke(width, color, None, |pb| {
                let mut any = false;
                for &(a, b) in &g.edges {
                    if f.edge_lit(a, b) == lit {
                        pb.move_to(v.at(f.pos[a]));
                        pb.line_to(v.at(f.pos[b]));
                        any = true;
                    }
                }
                any
            });
        }
        for (i, n) in g.nodes.iter().enumerate() {
            let color = if f.hover == Some(i) {
                accent
            } else if f.neighbors.contains(&i) {
                accent_tint
            } else if let Some(m) = f.matches {
                if m.contains(&i) {
                    accent
                } else {
                    n.kind.color().opacity(0.25)
                }
            } else {
                n.kind.color()
            };
            let c = p.at(f.pos[i]);
            let r = p.len(n.radius, 3.0);
            p.dot(c, r, color);
        }
        // Labels: all of them when zoomed in enough to read the map;
        // search matches and the hovered node always.
        for (i, n) in g.nodes.iter().enumerate() {
            if p.zoom() < 0.7 && !f.lit(i) {
                continue;
            }
            let c = p.at(f.pos[i]);
            let at = gpui::point(c.x, c.y + p.len(n.radius, 3.0) + px(3.0));
            let color = if f.lit(i) {
                accent
            } else {
                theme::text_secondary()
            };
            p.text(&n.title, at, 11.0, color, Align::Center, VAlign::Top, false);
        }
    }
}

/// Positions for nodes of these radii: a force pass per connected component
/// (see [`force_layout`]), then the components arranged around the largest —
/// isolated clusters and singletons form tidy rings instead of being crushed
/// against the big component's rim. Deterministic, one-shot.
pub fn arrange(radii: &[f32], edges: &[(usize, usize)]) -> Vec<[f32; 2]> {
    let n = radii.len();
    let mut pos = vec![[0.0f32; 2]; n];
    if n == 0 {
        return pos;
    }
    // Connected components (BFS).
    let mut adj = vec![Vec::new(); n];
    for &(a, b) in edges {
        adj[a].push(b);
        adj[b].push(a);
    }
    let mut comp_of = vec![usize::MAX; n];
    let mut comps: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if comp_of[start] != usize::MAX {
            continue;
        }
        let ci = comps.len();
        comp_of[start] = ci;
        let mut members = vec![start];
        let mut head = 0;
        while head < members.len() {
            let v = members[head];
            head += 1;
            for &w in &adj[v] {
                if comp_of[w] == usize::MAX {
                    comp_of[w] = ci;
                    members.push(w);
                }
            }
        }
        comps.push(members);
    }
    // Each component's edges, re-indexed to its member list.
    let mut local_of = vec![0usize; n];
    for comp in &comps {
        for (li, &g) in comp.iter().enumerate() {
            local_of[g] = li;
        }
    }
    let mut comp_edges: Vec<Vec<(usize, usize)>> = vec![Vec::new(); comps.len()];
    for &(a, b) in edges {
        comp_edges[comp_of[a]].push((local_of[a], local_of[b]));
    }
    // Lay out each component alone; leave its box origin at (0, 0).
    let mut boxes: Vec<[f32; 2]> = vec![[0.0, 0.0]; comps.len()];
    for (ci, (comp, ce)) in comps.iter().zip(&comp_edges).enumerate() {
        if let [g] = comp[..] {
            let r = radii[g];
            pos[g] = [r, r];
            boxes[ci] = [r * 2.0, r * 2.0];
            continue;
        }
        let local = force_layout(comp.len(), ce);
        let (mut minx, mut miny) = (f32::MAX, f32::MAX);
        let (mut maxx, mut maxy) = (f32::MIN, f32::MIN);
        for p in &local {
            minx = minx.min(p[0]);
            miny = miny.min(p[1]);
            maxx = maxx.max(p[0]);
            maxy = maxy.max(p[1]);
        }
        for (p, &g) in local.iter().zip(comp) {
            pos[g] = [p[0] - minx, p[1] - miny];
        }
        boxes[ci] = [maxx - minx, maxy - miny];
    }
    // The largest component anchors the center. The other LINKED groups
    // disperse on rings just around it — inside the field of dots — and the
    // singletons alone form the surrounding rings, Logseq-style.
    let mut order: Vec<usize> = (0..comps.len()).collect();
    order.sort_by(|&a, &b| half_diag(boxes[b]).total_cmp(&half_diag(boxes[a])));
    let center_ci = order[0];
    {
        let [w, h] = boxes[center_ci];
        for &g in &comps[center_ci] {
            pos[g][0] -= w / 2.0;
            pos[g][1] -= h / 2.0;
        }
    }
    let (groups, dots): (Vec<usize>, Vec<usize>) =
        order[1..].iter().partition(|&&ci| comps[ci].len() > 1);
    let base = half_diag(boxes[center_ci]) + RING_GAP;
    let outer = ring_disperse(&mut pos, &comps, &boxes, &groups, base, true);
    ring_disperse(&mut pos, &comps, &boxes, &dots, outer + RING_GAP, false);
    // Center the arrangement so the camera math starts near the middle.
    let (mut minx, mut miny) = (f32::MAX, f32::MAX);
    let (mut maxx, mut maxy) = (f32::MIN, f32::MIN);
    for p in &pos {
        minx = minx.min(p[0]);
        miny = miny.min(p[1]);
        maxx = maxx.max(p[0]);
        maxy = maxy.max(p[1]);
    }
    let (mx, my) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
    for p in pos.iter_mut() {
        p[0] -= mx;
        p[1] -= my;
    }
    pos
}

const GAP: f32 = 60.0; // arc length reserved between ring neighbors
const RING_GAP: f32 = 70.0; // radial gap between rings

/// Half the diagonal of a layout box — the radius that fully contains it.
fn half_diag(b: [f32; 2]) -> f32 {
    (b[0] * b[0] + b[1] * b[1]).sqrt() / 2.0
}

/// Disperse the selected components onto rings around the origin, starting
/// at `base` and stepping outward as rings fill. Two passes: greedily assign
/// members to rings by the arc each needs, then spread every ring's members
/// evenly around the full circle. Returns the outer edge consumed.
fn ring_disperse(
    pos: &mut [[f32; 2]],
    comps: &[Vec<usize>],
    boxes: &[[f32; 2]],
    sel: &[usize],
    mut base: f32,
    jitter: bool,
) -> f32 {
    let mut rings: Vec<(f32, Vec<usize>)> = Vec::new(); // (radius, components)
    let mut used = 0.0f32;
    let mut ring_max = 0.0f32;
    for &ci in sel {
        let d = half_diag(boxes[ci]).max(10.0);
        let need = 2.0 * d + GAP;
        let full = rings
            .last()
            .is_none_or(|(r, _)| used + need > std::f32::consts::TAU * r);
        if full {
            if let Some((r, _)) = rings.last() {
                // Descending sizes: a ring's first (largest) member sets its
                // radial thickness.
                base = r + ring_max + RING_GAP;
            }
            rings.push((base + d, Vec::new()));
            ring_max = d;
            used = 0.0;
        }
        rings.last_mut().unwrap().1.push(ci);
        used += need;
    }
    for (ri, (r, members)) in rings.iter().enumerate() {
        let step = std::f32::consts::TAU / members.len() as f32;
        let offset = ri as f32 * 0.35; // stagger successive rings
        for (j, &ci) in members.iter().enumerate() {
            // Optional deterministic scatter, so linked groups look strewn
            // around the center rather than machined onto a ring.
            let (ja, jr) = if jitter {
                (
                    hash_unit(ci * 2) * step * 0.35,
                    hash_unit(ci * 2 + 1) * RING_GAP,
                )
            } else {
                (0.0, 0.0)
            };
            let ang = offset + j as f32 * step + ja;
            let rj = r + jr;
            let (cx, cy) = (rj * ang.cos(), rj * ang.sin());
            let [w, h] = boxes[ci];
            for &g in &comps[ci] {
                pos[g][0] += cx - w / 2.0;
                pos[g][1] += cy - h / 2.0;
            }
        }
    }
    rings.last().map_or(base, |(r, _)| r + ring_max)
}
