//! The **graph view** (All pages → top-right "Graph"; its own tab): every
//! named page and whiteboard as a node, every `page_links` edge as a line,
//! laid out by a small force simulation on open. Drag or scroll to pan,
//! pinch or ⌘/Ctrl+scroll to zoom, click a node to open it, hover to
//! highlight its neighborhood. A floating panel holds the legend, node
//! statistics, and filters (journal days default off, Logseq-style).

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    Bounds, ClickEvent, Context, Corners, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, PinchEvent, Pixels,
    Point, ScrollDelta, ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled,
    TextRun, canvas, div, fill, point, px, size,
};
use gpui_component::switch::Switch;

use crate::app::AppView;
use crate::models::Page;
use crate::theme;

/// Trackpad scroll lines → px, matching the whiteboard's feel.
const LINE_PX: f32 = 40.0;
/// A press that moves less than this is a click, not a pan.
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
    hover: Option<usize>,
    /// Left-button press: (last position, has it moved past the click slop).
    drag: Option<(Point<Pixels>, bool)>,
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
        let linked: HashSet<i64> = id_edges.iter().flat_map(|&(a, b)| [a, b]).collect();
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
            hover: None,
            drag: None,
            bounds: Rc::default(),
        }
    }

    pub fn filters(&self) -> GraphFilters {
        self.filters
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
    let nodes: Vec<([f32; 2], f32, Kind, SharedString)> = state
        .nodes
        .iter()
        .map(|n| (n.pos, n.radius, n.kind, SharedString::from(n.title.clone())))
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
                for (i, (pos, radius, kind, _)) in nodes.iter().enumerate() {
                    let p = to_screen(*pos);
                    let r = px((radius * zoom).max(3.0));
                    let color = if hover == Some(i) {
                        accent
                    } else if neighbors.contains(&i) {
                        accent_tint
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
    .child(panel(state, cx))
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

/// The floating control panel: legend + statistics up top, node filters and
/// a reset action below.
fn panel(state: &GraphState, cx: &mut Context<AppView>) -> impl IntoElement {
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
                    // Fresh layout + camera, same filters.
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
