//! The **graph view** (All pages → top-right "Graph"; its own tab): every
//! named page and whiteboard as a node, every `page_links` edge as a line,
//! laid out by a small force simulation on open (then zoomed to fit). Drag
//! the background or scroll to pan, pinch or ⌘/Ctrl+scroll to zoom, drag a
//! node to reposition it, click one to open it, hover to highlight its
//! neighborhood. A floating panel holds search, the legend, node statistics,
//! and filters (journal days default off, Logseq-style).

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    Bounds, ClickEvent, Context, Corners, Entity, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder,
    PinchEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, SharedString, Size,
    StatefulInteractiveElement, Styled, TextRun, canvas, div, fill, point, px, size,
};
use gpui_component::Sizable;
use gpui_component::input::{Input, InputState};
use gpui_component::switch::Switch;

use crate::app::AppView;
use crate::models::Page;
use crate::theme;

/// Trackpad scroll lines → px, matching the whiteboard's feel.
const LINE_PX: f32 = 40.0;
/// A press that moves less than this is a click, not a drag.
const CLICK_SLOP: f32 = 4.0;

/// Which node sets are in the graph. Journal days default off — thousands of
/// day nodes swamp the layout, same reasoning as the All pages browser.
#[derive(Clone, Copy)]
pub struct GraphFilters {
    pub journals: bool,
    pub orphans: bool,
    pub whiteboards: bool,
}

impl Default for GraphFilters {
    fn default() -> Self {
        Self {
            journals: false,
            orphans: true,
            whiteboards: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Page,
    Board,
    Journal,
}

impl Kind {
    fn color(self) -> Hsla {
        match self {
            Self::Page => theme::text_tertiary(),
            Self::Board => theme::accent(),
            Self::Journal => theme::divider(),
        }
    }
}

struct Node {
    title: String,
    kind: Kind,
    /// World position (the layout's coordinate space; zoom/pan map to screen).
    pos: [f32; 2],
    radius: f32,
}

/// A left-button press in flight: panning the background or moving a node.
#[derive(Clone, Copy)]
enum Drag {
    Pan {
        last: Point<Pixels>,
        moved: bool,
    },
    Node {
        i: usize,
        press: Point<Pixels>,
        moved: bool,
    },
}

/// The graph tab's model: nodes laid out once on open, plus the camera and
/// interaction state. Rebuilt by [`AppView::open_graph`] and filter changes.
pub struct GraphState {
    nodes: Vec<Node>,
    /// Undirected, deduped `page_links` edges as node indices.
    edges: Vec<(usize, usize)>,
    filters: GraphFilters,
    /// Orphan (degree-0) node counts: shown in the graph / dropped by the filter.
    visible_orphans: usize,
    hidden_orphans: usize,
    /// Camera: screen-px offset from the canvas center, and scale.
    pan: [f32; 2],
    zoom: f32,
    /// Fit-to-view once the canvas size is known (the first painted frame
    /// computes the same fit locally so nothing flashes unfitted).
    fit_pending: bool,
    hover: Option<usize>,
    drag: Option<Drag>,
    /// Canvas bounds, captured at prepaint for event → world math.
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl GraphState {
    pub fn build(
        pages: &[Page],
        boards: &[Page],
        journals: &[Page],
        links: &[(i64, i64)],
        filters: GraphFilters,
    ) -> Self {
        let mut cand: Vec<(i64, &str, Kind)> = Vec::new();
        for (list, kind) in [
            (pages, Kind::Page),
            (boards, Kind::Board),
            (journals, Kind::Journal),
        ] {
            cand.extend(list.iter().map(|p| (p.id, p.title.as_str(), kind)));
        }
        let ids: HashSet<i64> = cand.iter().map(|(id, ..)| *id).collect();
        // Edges whose endpoints aren't candidate nodes (journal days while
        // they're filtered out) drop here.
        let mut id_edges: HashSet<(i64, i64)> = HashSet::new();
        for &(s, t) in links {
            if s != t && ids.contains(&s) && ids.contains(&t) {
                id_edges.insert((s.min(t), s.max(t)));
            }
        }
        // Orphan-ness is judged against the FULL link table, not the visible
        // subgraph — a page linked only from (hidden) journal days is not an
        // orphan, it just has no visible edges right now.
        let linked: HashSet<i64> = links
            .iter()
            .filter(|(s, t)| s != t)
            .flat_map(|&(s, t)| [s, t])
            .collect();
        let orphan_count = cand.iter().filter(|(id, ..)| !linked.contains(id)).count();
        let (visible_orphans, hidden_orphans) = if filters.orphans {
            (orphan_count, 0)
        } else {
            cand.retain(|(id, ..)| linked.contains(id));
            (0, orphan_count)
        };

        let mut nodes: Vec<Node> = Vec::new();
        let mut index: HashMap<i64, usize> = HashMap::new();
        for (id, title, kind) in cand {
            index.insert(id, nodes.len());
            nodes.push(Node {
                title: title.to_string(),
                kind,
                pos: [0.0, 0.0],
                radius: 0.0,
            });
        }
        let edges: Vec<(usize, usize)> = id_edges
            .into_iter()
            .map(|(a, b)| (index[&a], index[&b]))
            .collect();
        let mut degree = vec![0usize; nodes.len()];
        for &(a, b) in &edges {
            degree[a] += 1;
            degree[b] += 1;
        }
        for (n, d) in nodes.iter_mut().zip(&degree) {
            n.radius = (5.0 + 2.5 * (*d as f32).sqrt()).min(18.0);
        }
        layout(&mut nodes, &edges);
        Self {
            nodes,
            edges,
            filters,
            visible_orphans,
            hidden_orphans,
            pan: [0.0, 0.0],
            zoom: 1.0,
            fit_pending: true,
            hover: None,
            drag: None,
            bounds: Rc::default(),
        }
    }

    pub fn filters(&self) -> GraphFilters {
        self.filters
    }

    /// Carry the previous state's canvas bounds across a rebuild, so the
    /// fit-to-view has a real size on its first frame.
    pub fn adopt_camera_bounds(&mut self, prev: &Self) {
        self.bounds = prev.bounds.clone();
    }

    /// Canvas-local vector from the canvas center to a window-coords point.
    fn center_offset(&self, p: Point<Pixels>) -> [f32; 2] {
        let b = self.bounds.get();
        let c = b.center();
        [f32::from(p.x - c.x), f32::from(p.y - c.y)]
    }

    /// The node under a window-coords point, if any (topmost wins).
    fn hit(&self, p: Point<Pixels>) -> Option<usize> {
        let [cx, cy] = self.center_offset(p);
        self.nodes.iter().enumerate().rev().find_map(|(i, n)| {
            let dx = cx - (self.pan[0] + n.pos[0] * self.zoom);
            let dy = cy - (self.pan[1] + n.pos[1] * self.zoom);
            let r = (n.radius * self.zoom).max(3.0) + 2.0;
            (dx * dx + dy * dy <= r * r).then_some(i)
        })
    }

    /// Scale about a window-coords point, keeping the world under it fixed.
    fn zoom_about(&mut self, p: Point<Pixels>, factor: f32) {
        let [cx, cy] = self.center_offset(p);
        let zoom = (self.zoom * factor).clamp(0.15, 4.0);
        let wx = (cx - self.pan[0]) / self.zoom;
        let wy = (cy - self.pan[1]) / self.zoom;
        self.pan = [cx - wx * zoom, cy - wy * zoom];
        self.zoom = zoom;
    }
}

/// The camera (pan, zoom) that fits every node inside `size` with margin.
fn fit_camera(size: Size<Pixels>, nodes: impl Iterator<Item = ([f32; 2], f32)>) -> ([f32; 2], f32) {
    let (mut minx, mut miny) = (f32::MAX, f32::MAX);
    let (mut maxx, mut maxy) = (f32::MIN, f32::MIN);
    let mut any = false;
    for (p, r) in nodes {
        any = true;
        minx = minx.min(p[0] - r);
        miny = miny.min(p[1] - r);
        maxx = maxx.max(p[0] + r);
        maxy = maxy.max(p[1] + r);
    }
    if !any {
        return ([0.0, 0.0], 1.0);
    }
    const PAD: f32 = 60.0;
    let (bw, bh) = (maxx - minx + PAD * 2.0, maxy - miny + PAD * 2.0);
    let (w, h) = (
        f32::from(size.width).max(1.0),
        f32::from(size.height).max(1.0),
    );
    let zoom = (w / bw).min(h / bh).clamp(0.05, 1.25);
    let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
    ([-cx * zoom, -cy * zoom], zoom)
}

/// Layout: a Fruchterman–Reingold force pass per connected component (all
/// pairs repel, linked nodes attract, displacement capped by a cooling
/// temperature), then the components shelf-packed into a roughly square
/// sheet — isolated clusters and singletons form a tidy grid instead of
/// being crushed against the big component's rim. Deterministic
/// (golden-angle spiral start), one-shot on open — no animation to keep in
/// sync.
fn layout(nodes: &mut [Node], edges: &[(usize, usize)]) {
    let n = nodes.len();
    if n == 0 {
        return;
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
            let r = nodes[g].radius;
            nodes[g].pos = [r, r];
            boxes[ci] = [r * 2.0, r * 2.0];
            continue;
        }
        let mut pos = fr(comp.len(), ce);
        let (mut minx, mut miny) = (f32::MAX, f32::MAX);
        let (mut maxx, mut maxy) = (f32::MIN, f32::MIN);
        for p in &pos {
            minx = minx.min(p[0]);
            miny = miny.min(p[1]);
            maxx = maxx.max(p[0]);
            maxy = maxy.max(p[1]);
        }
        for (p, &g) in pos.iter_mut().zip(comp) {
            nodes[g].pos = [p[0] - minx, p[1] - miny];
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
            nodes[g].pos[0] -= w / 2.0;
            nodes[g].pos[1] -= h / 2.0;
        }
    }
    let (groups, dots): (Vec<usize>, Vec<usize>) =
        order[1..].iter().partition(|&&ci| comps[ci].len() > 1);
    let base = half_diag(boxes[center_ci]) + RING_GAP;
    let outer = ring_disperse(nodes, &comps, &boxes, &groups, base, true);
    ring_disperse(nodes, &comps, &boxes, &dots, outer + RING_GAP, false);
    // Center the arrangement so the camera math starts near the middle.
    let (mut minx, mut miny) = (f32::MAX, f32::MAX);
    let (mut maxx, mut maxy) = (f32::MIN, f32::MIN);
    for node in nodes.iter() {
        minx = minx.min(node.pos[0]);
        miny = miny.min(node.pos[1]);
        maxx = maxx.max(node.pos[0]);
        maxy = maxy.max(node.pos[1]);
    }
    let (mx, my) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
    for node in nodes.iter_mut() {
        node.pos[0] -= mx;
        node.pos[1] -= my;
    }
}

const GAP: f32 = 60.0; // arc length reserved between ring neighbors
const RING_GAP: f32 = 70.0; // radial gap between rings

/// A deterministic hash → [-1, 1), for layout jitter (`rand` stays out of
/// the tree, and a seeded look survives rebuilds).
fn hash_unit(seed: usize) -> f32 {
    let mut x = seed as u64 ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x & 0xFFFF) as f32 / 32768.0 - 1.0
}

/// Half the diagonal of a layout box — the radius that fully contains it.
fn half_diag(b: [f32; 2]) -> f32 {
    (b[0] * b[0] + b[1] * b[1]).sqrt() / 2.0
}

/// Disperse the selected components onto rings around the origin, starting
/// at `base` and stepping outward as rings fill. Two passes: greedily assign
/// members to rings by the arc each needs, then spread every ring's members
/// evenly around the full circle. Returns the outer edge consumed.
fn ring_disperse(
    nodes: &mut [Node],
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
                nodes[g].pos[0] += cx - w / 2.0;
                nodes[g].pos[1] += cy - h / 2.0;
            }
        }
    }
    rings.last().map_or(base, |(r, _)| r + ring_max)
}

/// The FR core for one connected component: returns settled positions
/// (arbitrary origin — the caller normalizes).
fn fr(n: usize, edges: &[(usize, usize)]) -> Vec<[f32; 2]> {
    let mut pos: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let a = i as f32 * 2.399_963; // golden angle
            let r = 28.0 * (i as f32).sqrt();
            [r * a.cos(), r * a.sin()]
        })
        .collect();
    let side = 260.0 + 46.0 * (n as f32).sqrt();
    let k = side / (n as f32).sqrt();
    // ponytail: O(n²·iters) all-pairs; a Barnes-Hut grid if one component passes ~2k nodes.
    let iters = if n > 800 { 80 } else { 200 };
    let half = side * 0.8;
    let mut disp = vec![[0.0f32; 2]; n];
    for it in 0..iters {
        let t = side / 8.0 * (1.0 - it as f32 / iters as f32);
        disp.iter_mut().for_each(|d| *d = [0.0, 0.0]);
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pos[i][0] - pos[j][0];
                let dy = pos[i][1] - pos[j][1];
                let d2 = (dx * dx + dy * dy).max(0.01);
                let f = k * k / d2; // repulsion / distance, folded into the vector
                disp[i][0] += dx * f;
                disp[i][1] += dy * f;
                disp[j][0] -= dx * f;
                disp[j][1] -= dy * f;
            }
        }
        for &(a, b) in edges {
            let dx = pos[a][0] - pos[b][0];
            let dy = pos[a][1] - pos[b][1];
            let d = (dx * dx + dy * dy).sqrt().max(0.1);
            let f = d / k; // attraction d²/k, divided by d for the unit vector
            disp[a][0] -= dx * f;
            disp[a][1] -= dy * f;
            disp[b][0] += dx * f;
            disp[b][1] += dy * f;
        }
        // A CIRCULAR frame clamp: it binds when hub-and-spoke fans inflate
        // past the frame, and a square one visibly flattens the cluster's
        // edges — a circle reads organic.
        for (p, d) in pos.iter_mut().zip(&disp) {
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(0.01);
            let cap = len.min(t);
            p[0] += d[0] / len * cap;
            p[1] += d[1] / len * cap;
            let rr = (p[0] * p[0] + p[1] * p[1]).sqrt();
            if rr > half {
                p[0] *= half / rr;
                p[1] *= half / rr;
            }
        }
    }
    pos
}

pub fn render(app: &mut AppView, cx: &mut Context<AppView>) -> gpui::AnyElement {
    let query = app
        .graph_search
        .as_ref()
        .map(|s| s.input.read(cx).value().trim().to_lowercase())
        .filter(|q| !q.is_empty());
    let search_input = app.graph_search.as_ref().map(|s| s.input.clone());
    let Some(state) = app.graph.as_mut() else {
        return div().size_full().into_any_element();
    };

    // Bake the pending fit once the canvas size is known (frame two; frame
    // one's paint computes the identical fit locally below).
    let b = state.bounds.get();
    if state.fit_pending && b.size.width > px(0.0) {
        let (pan, zoom) = fit_camera(b.size, state.nodes.iter().map(|n| (n.pos, n.radius)));
        state.pan = pan;
        state.zoom = zoom;
        state.fit_pending = false;
    }
    let state = &*state;

    // Snapshot for the paint closure (the state itself stays on AppView).
    let nodes: Vec<([f32; 2], f32, Kind, SharedString)> = state
        .nodes
        .iter()
        .map(|n| (n.pos, n.radius, n.kind, SharedString::from(n.title.clone())))
        .collect();
    let edges = state.edges.clone();
    let (pan, zoom, hover) = (state.pan, state.zoom, state.hover);
    let fit_pending = state.fit_pending;
    let matches: Option<HashSet<usize>> = query.map(|q| {
        state
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.title.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    });
    let neighbors: HashSet<usize> = hover
        .map(|h| {
            edges
                .iter()
                .filter_map(|&(a, b)| (a == h).then_some(b).or((b == h).then_some(a)))
                .collect()
        })
        .unwrap_or_default();
    let match_count = matches.as_ref().map(|m| m.len());
    let bounds_cell = state.bounds.clone();
    let (accent, accent_tint) = (theme::accent(), theme::accent_tint());
    let (edge_color, label_color) = (theme::divider(), theme::text_secondary());
    let empty = state.nodes.is_empty();

    let mut el = div()
        .id("graph")
        .size_full()
        .relative()
        .overflow_hidden()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this: &mut AppView, ev: &MouseDownEvent, _w, _cx| {
                if let Some(g) = this.graph.as_mut() {
                    g.drag = Some(match g.hit(ev.position) {
                        Some(i) => Drag::Node {
                            i,
                            press: ev.position,
                            moved: false,
                        },
                        None => Drag::Pan {
                            last: ev.position,
                            moved: false,
                        },
                    });
                }
            }),
        )
        .on_mouse_move(
            cx.listener(|this: &mut AppView, ev: &MouseMoveEvent, _w, cx| {
                let Some(g) = this.graph.as_mut() else { return };
                match g.drag {
                    Some(Drag::Pan { last, moved }) => {
                        let (dx, dy) = (
                            f32::from(ev.position.x - last.x),
                            f32::from(ev.position.y - last.y),
                        );
                        g.pan = [g.pan[0] + dx, g.pan[1] + dy];
                        g.drag = Some(Drag::Pan {
                            last: ev.position,
                            moved: moved || dx.abs() + dy.abs() > CLICK_SLOP,
                        });
                        cx.notify();
                    }
                    Some(Drag::Node { i, press, moved }) => {
                        let (dx, dy) = (
                            f32::from(ev.position.x - press.x),
                            f32::from(ev.position.y - press.y),
                        );
                        let moved = moved || dx.abs() + dy.abs() > CLICK_SLOP;
                        if moved {
                            let [ox, oy] = g.center_offset(ev.position);
                            g.nodes[i].pos = [(ox - g.pan[0]) / g.zoom, (oy - g.pan[1]) / g.zoom];
                        }
                        g.drag = Some(Drag::Node { i, press, moved });
                        cx.notify();
                    }
                    None => {
                        let hover = g.hit(ev.position);
                        if hover != g.hover {
                            g.hover = hover;
                            cx.notify();
                        }
                    }
                }
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this: &mut AppView, _ev: &MouseUpEvent, window, cx| {
                let Some(g) = this.graph.as_mut() else { return };
                if let Some(Drag::Node {
                    i, moved: false, ..
                }) = g.drag.take()
                {
                    let title = g.nodes[i].title.clone();
                    // Routes like a wiki-link: boards open their canvas.
                    this.open_page_title(&title, window, cx);
                }
            }),
        )
        .on_scroll_wheel(
            cx.listener(|this: &mut AppView, ev: &ScrollWheelEvent, _w, cx| {
                let Some(g) = this.graph.as_mut() else { return };
                let (dx, dy) = match ev.delta {
                    ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                    ScrollDelta::Lines(p) => (p.x * LINE_PX, p.y * LINE_PX),
                };
                if ev.modifiers.platform || ev.modifiers.control {
                    g.zoom_about(ev.position, (1.0 + dy * 0.0025).clamp(0.5, 2.0));
                } else {
                    g.pan = [g.pan[0] + dx, g.pan[1] + dy];
                }
                cx.notify();
            }),
        )
        .on_pinch(cx.listener(|this: &mut AppView, ev: &PinchEvent, _w, cx| {
            if let Some(g) = this.graph.as_mut() {
                g.zoom_about(ev.position, 1.0 + ev.delta);
                cx.notify();
            }
        }));
    if hover.is_some() {
        el = el.cursor_pointer();
    }

    el.child(
        canvas(
            move |bounds, _, _| bounds_cell.set(bounds),
            move |bounds, _, window, cx| {
                let (pan, zoom) = if fit_pending {
                    fit_camera(bounds.size, nodes.iter().map(|(p, r, ..)| (*p, *r)))
                } else {
                    (pan, zoom)
                };
                let c = bounds.center();
                let to_screen = |p: [f32; 2]| {
                    point(
                        c.x + px(pan[0] + p[0] * zoom),
                        c.y + px(pan[1] + p[1] * zoom),
                    )
                };
                // Edges: one path for the quiet ones (dimmed while searching),
                // one for the hovered node's, so the highlight paints on top.
                let quiet = if matches.is_some() {
                    edge_color.opacity(0.3)
                } else {
                    edge_color
                };
                for (pass, color, width) in [(false, quiet, 1.0), (true, accent, 1.5)] {
                    let mut pb = PathBuilder::stroke(px(width));
                    let mut any = false;
                    for &(a, b) in &edges {
                        if (hover == Some(a) || hover == Some(b)) != pass {
                            continue;
                        }
                        pb.move_to(to_screen(nodes[a].0));
                        pb.line_to(to_screen(nodes[b].0));
                        any = true;
                    }
                    if any && let Ok(path) = pb.build() {
                        window.paint_path(path, color);
                    }
                }
                let is_match = |i: usize| matches.as_ref().is_some_and(|m| m.contains(&i));
                for (i, (pos, radius, kind, _)) in nodes.iter().enumerate() {
                    let p = to_screen(*pos);
                    let r = px((radius * zoom).max(3.0));
                    let color = if hover == Some(i) {
                        accent
                    } else if neighbors.contains(&i) {
                        accent_tint
                    } else if let Some(m) = &matches {
                        if m.contains(&i) {
                            accent
                        } else {
                            kind.color().opacity(0.25)
                        }
                    } else {
                        kind.color()
                    };
                    let mut q = fill(
                        Bounds::new(point(p.x - r, p.y - r), size(r * 2.0, r * 2.0)),
                        color,
                    );
                    q.corner_radii = Corners::all(r);
                    window.paint_quad(q);
                }
                // Labels: all of them when zoomed in enough to read the map;
                // search matches and the hovered node always.
                for (i, (pos, radius, _, title)) in nodes.iter().enumerate() {
                    if zoom < 0.7 && hover != Some(i) && !is_match(i) {
                        continue;
                    }
                    let font_size = px(11.0);
                    let emphasized = hover == Some(i) || is_match(i);
                    let run = TextRun {
                        len: title.len(),
                        font: window.text_style().font(),
                        color: if emphasized { accent } else { label_color },
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped =
                        window
                            .text_system()
                            .shape_line(title.clone(), font_size, &[run], None);
                    let p = to_screen(*pos);
                    let _ = shaped.paint(
                        point(
                            p.x - shaped.width() / 2.0,
                            p.y + px((radius * zoom).max(3.0) + 3.0),
                        ),
                        font_size * 1.3,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            },
        )
        .absolute()
        .size_full(),
    )
    .child(panel(state, search_input, match_count, cx))
    .child(
        // A quiet hint; doubles as the empty-graph message.
        div()
            .absolute()
            .bottom(px(10.0))
            .left(px(16.0))
            .text_size(px(11.0))
            .text_color(theme::text_tertiary())
            .child(if empty {
                "No linked pages yet — [[wiki-links]] and #tags build the graph."
            } else {
                "Drag to pan or move a node · pinch or ⌘-scroll to zoom · click a node to open"
            }),
    )
    .into_any_element()
}

/// The floating control panel: search and the legend + statistics up top,
/// node filters and a reset action below.
fn panel(
    state: &GraphState,
    search: Option<Entity<InputState>>,
    match_count: Option<usize>,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let f = state.filters;
    let count = |k: Kind| state.nodes.iter().filter(|n| n.kind == k).count();
    let (pages, boards, journals) = (count(Kind::Page), count(Kind::Board), count(Kind::Journal));
    let orphans = if f.orphans {
        state.visible_orphans.to_string()
    } else {
        format!("{} hidden", state.hidden_orphans)
    };

    let dot = |color: Hsla| {
        div()
            .w(px(8.0))
            .h(px(8.0))
            .rounded_full()
            .bg(color)
            .into_any_element()
    };
    let dash = div()
        .w(px(10.0))
        .h(px(2.0))
        .bg(theme::divider())
        .into_any_element();
    let ring = div()
        .w(px(8.0))
        .h(px(8.0))
        .rounded_full()
        .border_1()
        .border_color(theme::text_tertiary())
        .into_any_element();

    let toggle = |id: &'static str, on: bool, set: fn(&mut GraphFilters, bool)| {
        let ent = cx.entity();
        Switch::new(id)
            .small()
            .checked(on)
            .on_click(move |on: &bool, _w, cx| {
                let mut nf = f;
                set(&mut nf, *on);
                ent.update(cx, |a, cx| a.set_graph_filters(nf, cx));
            })
    };

    let mut legend = div().flex().flex_col().gap(px(5.0)).child(legend_row(
        dot(Kind::Page.color()),
        "Pages",
        pages.to_string(),
    ));
    if boards > 0 {
        legend = legend.child(legend_row(
            dot(Kind::Board.color()),
            "Whiteboards",
            boards.to_string(),
        ));
    }
    if journals > 0 {
        legend = legend.child(legend_row(
            dot(Kind::Journal.color()),
            "Journals",
            journals.to_string(),
        ));
    }
    legend = legend
        .child(legend_row(dash, "Links", state.edges.len().to_string()))
        .child(legend_row(ring, "Orphans", orphans));

    div()
        .id("graph-panel")
        .occlude()
        .absolute()
        .top(px(12.0))
        .left(px(12.0))
        .w(px(220.0))
        .rounded(px(10.0))
        .bg(theme::elevated())
        .border_1()
        .border_color(theme::border_subtle())
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(theme::text_primary())
                        .child("Nodes"),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme::text_tertiary())
                        .child(format!("{} shown", state.nodes.len())),
                ),
        )
        .children(search.map(|input| Input::new(&input).small().text_size(px(12.0))))
        .children(match_count.map(|n| {
            div()
                .text_size(px(11.0))
                .text_color(theme::text_tertiary())
                .child(match n {
                    0 => "No matches".to_string(),
                    1 => "1 match".to_string(),
                    n => format!("{n} matches"),
                })
        }))
        .child(legend)
        .child(div().h(px(1.0)).bg(theme::divider()))
        .child(toggle_row(
            "Journals",
            toggle("graph-journals", f.journals, |f, v| f.journals = v),
        ))
        .child(toggle_row(
            "Orphan pages",
            toggle("graph-orphans", f.orphans, |f, v| f.orphans = v),
        ))
        .child(toggle_row(
            "Whiteboards",
            toggle("graph-boards", f.whiteboards, |f, v| f.whiteboards = v),
        ))
        .child(
            div()
                .id("graph-reset")
                .text_size(px(12.0))
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|s| s.text_color(theme::text_primary()))
                .on_click(cx.listener(|this: &mut AppView, _: &ClickEvent, _w, cx| {
                    // Fresh layout + camera fit, same filters.
                    this.rebuild_graph();
                    cx.notify();
                }))
                .child("Reset graph"),
        )
}

/// One legend line: a color mark, the kind, and its count.
fn legend_row(mark: gpui::AnyElement, label: &'static str, count: String) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .text_size(px(12.0))
        .text_color(theme::text_secondary())
        .child(
            div()
                .w(px(10.0))
                .flex()
                .flex_row()
                .justify_center()
                .child(mark),
        )
        .child(div().flex_1().child(label))
        .child(div().text_color(theme::text_tertiary()).child(count))
}

/// One filter line: the label left, its switch right.
fn toggle_row(label: &'static str, switch: Switch) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .text_size(px(12.0))
        .text_color(theme::text_secondary())
        .child(label)
        .child(switch)
}
