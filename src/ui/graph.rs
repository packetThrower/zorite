//! The **graph view** (All pages → top-right "Graph"; its own tab): every
//! named page and whiteboard as a node, every `page_links` edge as a line,
//! laid out by a small force simulation on open. Drag or scroll to pan,
//! pinch or ⌘/Ctrl+scroll to zoom, click a node to open it, hover to
//! highlight its neighborhood. Journal days are excluded, same as the All
//! pages browser.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    Bounds, Context, Corners, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, PinchEvent, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, SharedString, Styled, TextRun, canvas, div, fill, point, px,
    size,
};

use crate::app::AppView;
use crate::models::Page;
use crate::theme;

/// Trackpad scroll lines → px, matching the whiteboard's feel.
const LINE_PX: f32 = 40.0;
/// A press that moves less than this is a click, not a pan.
const CLICK_SLOP: f32 = 4.0;

struct Node {
    title: String,
    is_board: bool,
    /// World position (the layout's coordinate space; zoom/pan map to screen).
    pos: [f32; 2],
    radius: f32,
}

/// The graph tab's model: nodes laid out once on open, plus the camera and
/// interaction state. Rebuilt by [`AppView::open_graph`].
pub struct GraphState {
    nodes: Vec<Node>,
    /// Undirected, deduped `page_links` edges as node indices.
    edges: Vec<(usize, usize)>,
    /// Camera: screen-px offset from the canvas center, and scale.
    pan: [f32; 2],
    zoom: f32,
    hover: Option<usize>,
    /// Left-button press: (last position, has it moved past the click slop).
    drag: Option<(Point<Pixels>, bool)>,
    /// Canvas bounds, captured at prepaint for event → world math.
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl GraphState {
    pub fn build(pages: &[Page], boards: &[Page], links: &[(i64, i64)]) -> Self {
        let mut nodes: Vec<Node> = Vec::new();
        let mut index: HashMap<i64, usize> = HashMap::new();
        for (pages, is_board) in [(pages, false), (boards, true)] {
            for p in pages.iter() {
                index.insert(p.id, nodes.len());
                nodes.push(Node {
                    title: p.title.clone(),
                    is_board,
                    pos: [0.0, 0.0],
                    radius: 0.0,
                });
            }
        }
        // Journal-day endpoints aren't nodes, so their edges drop out here.
        let mut seen = HashSet::new();
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for &(s, t) in links {
            if let (Some(&a), Some(&b)) = (index.get(&s), index.get(&t))
                && a != b
                && seen.insert((a.min(b), a.max(b)))
            {
                edges.push((a.min(b), a.max(b)));
            }
        }
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
            pan: [0.0, 0.0],
            zoom: 1.0,
            hover: None,
            drag: None,
            bounds: Rc::default(),
        }
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

/// Fruchterman–Reingold force layout: all pairs repel, linked nodes attract,
/// displacement capped by a cooling temperature. Deterministic (golden-angle
/// spiral start), one-shot on open — no animation to keep in sync.
fn layout(nodes: &mut [Node], edges: &[(usize, usize)]) {
    let n = nodes.len();
    if n == 0 {
        return;
    }
    for (i, node) in nodes.iter_mut().enumerate() {
        let a = i as f32 * 2.399_963; // golden angle
        let r = 28.0 * (i as f32).sqrt();
        node.pos = [r * a.cos(), r * a.sin()];
    }
    let side = 260.0 + 46.0 * (n as f32).sqrt();
    let k = side / (n as f32).sqrt();
    // ponytail: O(n²·iters) all-pairs; a Barnes-Hut grid if graphs pass ~2k nodes.
    let iters = if n > 800 { 80 } else { 200 };
    let mut disp = vec![[0.0f32; 2]; n];
    for it in 0..iters {
        let t = side / 8.0 * (1.0 - it as f32 / iters as f32);
        disp.iter_mut().for_each(|d| *d = [0.0, 0.0]);
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = nodes[i].pos[0] - nodes[j].pos[0];
                let dy = nodes[i].pos[1] - nodes[j].pos[1];
                let d2 = (dx * dx + dy * dy).max(0.01);
                let f = k * k / d2; // repulsion / distance, folded into the vector
                disp[i][0] += dx * f;
                disp[i][1] += dy * f;
                disp[j][0] -= dx * f;
                disp[j][1] -= dy * f;
            }
        }
        for &(a, b) in edges {
            let dx = nodes[a].pos[0] - nodes[b].pos[0];
            let dy = nodes[a].pos[1] - nodes[b].pos[1];
            let d = (dx * dx + dy * dy).sqrt().max(0.1);
            let f = d / k; // attraction d²/k, divided by d for the unit vector
            disp[a][0] -= dx * f;
            disp[a][1] -= dy * f;
            disp[b][0] += dx * f;
            disp[b][1] += dy * f;
        }
        for (node, d) in nodes.iter_mut().zip(&disp) {
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(0.01);
            let cap = len.min(t);
            node.pos[0] += d[0] / len * cap;
            node.pos[1] += d[1] / len * cap;
        }
    }
    // Center on the centroid so pan starts at the middle of the graph.
    let (mut sx, mut sy) = (0.0, 0.0);
    for node in nodes.iter() {
        sx += node.pos[0];
        sy += node.pos[1];
    }
    let (mx, my) = (sx / n as f32, sy / n as f32);
    for node in nodes.iter_mut() {
        node.pos[0] -= mx;
        node.pos[1] -= my;
    }
}

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> gpui::AnyElement {
    let Some(state) = app.graph.as_ref() else {
        return div().size_full().into_any_element();
    };

    // Snapshot for the paint closure (the state itself stays on AppView).
    let nodes: Vec<([f32; 2], f32, bool, SharedString)> = state
        .nodes
        .iter()
        .map(|n| {
            (
                n.pos,
                n.radius,
                n.is_board,
                SharedString::from(n.title.clone()),
            )
        })
        .collect();
    let edges = state.edges.clone();
    let (pan, zoom, hover) = (state.pan, state.zoom, state.hover);
    let neighbors: HashSet<usize> = hover
        .map(|h| {
            edges
                .iter()
                .filter_map(|&(a, b)| (a == h).then_some(b).or((b == h).then_some(a)))
                .collect()
        })
        .unwrap_or_default();
    let bounds_cell = state.bounds.clone();
    let (accent, accent_tint) = (theme::accent(), theme::accent_tint());
    let (node_color, board_color) = (theme::text_tertiary(), theme::accent());
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
                    g.drag = Some((ev.position, false));
                }
            }),
        )
        .on_mouse_move(
            cx.listener(|this: &mut AppView, ev: &MouseMoveEvent, _w, cx| {
                let Some(g) = this.graph.as_mut() else { return };
                if let Some((last, moved)) = g.drag {
                    let (dx, dy) = (
                        f32::from(ev.position.x - last.x),
                        f32::from(ev.position.y - last.y),
                    );
                    g.pan = [g.pan[0] + dx, g.pan[1] + dy];
                    g.drag = Some((ev.position, moved || dx.abs() + dy.abs() > CLICK_SLOP));
                    cx.notify();
                } else {
                    let hover = g.hit(ev.position);
                    if hover != g.hover {
                        g.hover = hover;
                        cx.notify();
                    }
                }
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this: &mut AppView, ev: &MouseUpEvent, window, cx| {
                let Some(g) = this.graph.as_mut() else { return };
                let was_click = matches!(g.drag, Some((_, false)));
                g.drag = None;
                if was_click && let Some(i) = g.hit(ev.position) {
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
                let c = bounds.center();
                let to_screen = |p: [f32; 2]| {
                    point(
                        c.x + px(pan[0] + p[0] * zoom),
                        c.y + px(pan[1] + p[1] * zoom),
                    )
                };
                // Edges: one path for the quiet ones, one for the hovered
                // node's, so the highlight paints on top.
                for (pass, color, width) in [(false, edge_color, 1.0), (true, accent, 1.5)] {
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
                for (i, (pos, radius, is_board, _)) in nodes.iter().enumerate() {
                    let p = to_screen(*pos);
                    let r = px((radius * zoom).max(3.0));
                    let color = if hover == Some(i) {
                        accent
                    } else if neighbors.contains(&i) {
                        accent_tint
                    } else if *is_board {
                        board_color
                    } else {
                        node_color
                    };
                    let mut q = fill(
                        Bounds::new(point(p.x - r, p.y - r), size(r * 2.0, r * 2.0)),
                        color,
                    );
                    q.corner_radii = Corners::all(r);
                    window.paint_quad(q);
                }
                // Labels: all of them when zoomed in enough to read the map,
                // otherwise just the hovered node's.
                for (i, (pos, radius, _, title)) in nodes.iter().enumerate() {
                    if zoom < 0.7 && hover != Some(i) {
                        continue;
                    }
                    let font_size = px(11.0);
                    let run = TextRun {
                        len: title.len(),
                        font: window.text_style().font(),
                        color: if hover == Some(i) {
                            accent
                        } else {
                            label_color
                        },
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
                "Drag or scroll to pan · pinch or ⌘-scroll to zoom · click a node to open"
            }),
    )
    .into_any_element()
}
