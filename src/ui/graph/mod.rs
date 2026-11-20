//! The **graph view** (All pages → top-right "Graph"; its own tab): every
//! named page and whiteboard as a node, every `page_links` edge as a line.
//! Drag the background or scroll to pan, pinch or ⌘/Ctrl+scroll to zoom,
//! drag a node to reposition it, click one to open it, hover to highlight its
//! neighborhood. A floating panel holds the style picker, search, the legend,
//! node statistics, and filters (journal days default off, Logseq-style).
//!
//! This module is the shared core: the data, camera, input, panel, and the
//! animated move between layouts. Where nodes go and how they're drawn comes
//! from a [`style::GraphStyle`]; see `style.rs` for how to add one.

mod classic;
mod focus;
mod islands;
mod nebula;
mod style;
mod timeline;

use std::cell::Cell;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Bounds, ClickEvent, Context, Entity, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PinchEvent, Pixels, Point,
    ScrollDelta, ScrollEvent, SharedString, Size, StatefulInteractiveElement, Styled, canvas, div,
    px,
};
use gpui_component::input::{Input, InputState};
use gpui_component::menu::ContextMenuExt;
use gpui_component::switch::Switch;
use gpui_component::{Icon, IconName, Sizable};

use crate::app::AppView;
use crate::models::Page;
use crate::theme;
use rust_i18n::t;
use style::{Frame, GraphStyle, Layout, Painter};

/// Trackpad scroll lines → px, matching the whiteboard's feel.
const LINE_PX: f32 = 40.0;
/// A press that moves less than this is a click, not a drag.
const CLICK_SLOP: f32 = 4.0;
/// The panel's width plus its margins: the camera fit keeps clear of it.
const PANEL_W: f32 = 244.0;
/// How long the move between two layouts takes.
const MORPH: Duration = Duration::from_millis(420);

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
pub enum Kind {
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

/// One node, as every style sees it.
pub struct Node {
    /// The page's database id.
    pub id: i64,
    pub title: SharedString,
    pub kind: Kind,
    /// Links in the visible graph.
    pub degree: usize,
    /// Base radius in world units, from the degree.
    pub radius: f32,
    /// Index into [`Graph::groups`]; `None` for nodes without visible links.
    pub group: Option<usize>,
    /// The day the page first appeared (days since 1970-01-01): its creation
    /// day, or the earliest journal day linking to it if that's sooner.
    pub day: Option<i32>,
}

/// A topic: pages that link more to each other than to the rest, found from
/// the links alone (no tags or folders needed). Named after its busiest page.
pub struct Group {
    pub label: SharedString,
    pub members: Vec<usize>,
}

/// The graph every style lays out and paints. Built once per filter change.
pub struct Graph {
    pub nodes: Vec<Node>,
    /// Undirected, deduped `page_links` edges as node indices.
    pub edges: Vec<(usize, usize)>,
    /// Neighbors of each node.
    pub adj: Vec<Vec<usize>>,
    /// Largest first.
    pub groups: Vec<Group>,
}

impl Graph {
    /// The most-linked node: the default center for focus-style layouts.
    pub fn hub(&self) -> Option<usize> {
        (0..self.nodes.len()).max_by_key(|&i| (self.nodes[i].degree, Reverse(i)))
    }
}

/// A left-button press in flight: panning the background or pressing a node
/// (which moves it in styles that allow dragging, and pans in the others).
#[derive(Clone, Copy)]
enum Drag {
    Pan {
        last: Point<Pixels>,
        moved: bool,
    },
    Node {
        i: usize,
        press: Point<Pixels>,
        last: Point<Pixels>,
        moved: bool,
    },
}

/// A move between two layouts in progress.
struct Morph {
    from: Vec<[f32; 2]>,
    cam_from: ([f32; 2], f32),
    cam_to: ([f32; 2], f32),
    start: Instant,
}

/// The graph tab's model: the data, the current style's layout, plus the
/// camera and interaction state. Rebuilt by [`AppView::open_graph`] and
/// filter changes.
pub struct GraphState {
    graph: Rc<Graph>,
    style: &'static dyn GraphStyle,
    filters: GraphFilters,
    /// Orphan (degree-0) node counts: shown in the graph / dropped by the filter.
    visible_orphans: usize,
    hidden_orphans: usize,
    /// The centered node, for styles that use one.
    focus: Option<usize>,
    /// The style's target layout.
    layout: Layout,
    /// Where each node is drawn right now (differs from `layout` mid-morph).
    pos: Vec<[f32; 2]>,
    morph: Option<Morph>,
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
    /// The panel shows its controls (false: just its title bar).
    panel_open: bool,
    /// The node under the last right-click, for the page menu (none: the
    /// background, which has no menu).
    menu_node: Option<usize>,
}

impl GraphState {
    pub fn build(
        pages: &[Page],
        boards: &[Page],
        journals: &[Page],
        links: &[(i64, i64)],
        filters: GraphFilters,
        style: Option<&str>,
    ) -> Self {
        // A page's day is when it first appeared: its creation day, or the
        // earliest journal day that links to it when that's sooner — pages
        // brought in by an import all share the import's creation day, but
        // their journal mentions keep the real history. `journals` is every
        // journal day, whether or not they're shown as nodes.
        let journal_day: HashMap<i64, i32> = journals
            .iter()
            .filter_map(|j| Some((j.id, parse_day(j.journal_date.as_deref()?)?)))
            .collect();
        let mut mentioned: HashMap<i64, i32> = HashMap::new();
        for &(s, t) in links {
            for (j, other) in [(s, t), (t, s)] {
                if let Some(&d) = journal_day.get(&j)
                    && !journal_day.contains_key(&other)
                {
                    let m = mentioned.entry(other).or_insert(d);
                    *m = (*m).min(d);
                }
            }
        }
        let shown_journals: &[Page] = if filters.journals { journals } else { &[] };
        let mut cand: Vec<(i64, &str, Kind, Option<i32>)> = Vec::new();
        for (list, kind) in [
            (pages, Kind::Page),
            (boards, Kind::Board),
            (shown_journals, Kind::Journal),
        ] {
            cand.extend(list.iter().map(|p| {
                let created = p
                    .created_at
                    .as_deref()
                    .or(p.journal_date.as_deref())
                    .and_then(parse_day);
                let day = match (created, mentioned.get(&p.id)) {
                    (Some(c), Some(&m)) => Some(c.min(m)),
                    (c, m) => c.or(m.copied()),
                };
                (p.id, p.title.as_str(), kind, day)
            }));
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
        for (id, title, kind, day) in cand {
            index.insert(id, nodes.len());
            nodes.push(Node {
                id,
                title: SharedString::from(title.to_string()),
                kind,
                degree: 0,
                radius: 0.0,
                group: None,
                day,
            });
        }
        let mut edges: Vec<(usize, usize)> = id_edges
            .into_iter()
            .map(|(a, b)| (index[&a], index[&b]))
            .collect();
        edges.sort_unstable();
        let mut adj = vec![Vec::new(); nodes.len()];
        for &(a, b) in &edges {
            adj[a].push(b);
            adj[b].push(a);
        }
        for (n, a) in nodes.iter_mut().zip(&adj) {
            n.degree = a.len();
            n.radius = (5.0 + 2.5 * (n.degree as f32).sqrt()).min(18.0);
        }
        let labels = communities(&adj);
        let mut by_label: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, l) in labels.iter().enumerate() {
            if let Some(l) = l {
                by_label.entry(*l).or_default().push(i);
            }
        }
        let mut members: Vec<Vec<usize>> = by_label.into_values().collect();
        members.sort_by_key(|m| (Reverse(m.len()), m[0]));
        let groups: Vec<Group> = members
            .into_iter()
            .enumerate()
            .map(|(gi, members)| {
                for &m in &members {
                    nodes[m].group = Some(gi);
                }
                let top = *members
                    .iter()
                    .max_by_key(|&&m| (nodes[m].degree, Reverse(m)))
                    .unwrap();
                Group {
                    label: nodes[top].title.clone(),
                    members,
                }
            })
            .collect();
        let graph = Rc::new(Graph {
            nodes,
            edges,
            adj,
            groups,
        });
        let style = style::find(style.unwrap_or_default());
        let focus = graph.hub();
        let layout = style.layout(&graph, focus);
        Self {
            pos: layout.pos.clone(),
            layout,
            graph,
            style,
            filters,
            visible_orphans,
            hidden_orphans,
            focus,
            morph: None,
            pan: [0.0, 0.0],
            zoom: 1.0,
            fit_pending: true,
            hover: None,
            drag: None,
            bounds: Rc::default(),
            panel_open: true,
            menu_node: None,
        }
    }

    pub fn filters(&self) -> GraphFilters {
        self.filters
    }

    /// Carry the previous state's canvas bounds and style across a rebuild,
    /// so the fit-to-view has a real size on its first frame.
    pub fn adopt_camera_bounds(&mut self, prev: &Self) {
        self.bounds = prev.bounds.clone();
    }

    /// Show the panel's controls, or retract it to its title bar.
    pub fn set_panel_open(&mut self, open: bool) {
        self.panel_open = open;
    }

    /// Switch to the style with this id and move the nodes to its layout.
    pub fn set_style(&mut self, id: &str) {
        self.style = style::find(id);
        self.relayout();
    }

    /// Re-run the style's layout and morph to it, refitting the camera.
    fn relayout(&mut self) {
        self.layout = self.style.layout(&self.graph, self.focus);
        self.hover = None;
        let size = self.bounds.get().size;
        if size.width <= px(0.0) {
            self.pos = self.layout.pos.clone();
            self.fit_pending = true;
            return;
        }
        let cam_to = fit_camera(
            size,
            &self.graph,
            &self.layout.pos,
            &self.layout.shown,
            self.panel_open,
        );
        self.morph = Some(Morph {
            from: std::mem::take(&mut self.pos),
            cam_from: (self.pan, self.zoom),
            cam_to,
            start: Instant::now(),
        });
        self.pos = self.layout.pos.clone();
    }

    /// Store the pending fit-to-view once the canvas size is known. The first
    /// frame paints with the same fit computed locally, so input arriving
    /// before a second frame must bake it too, or hit tests would use the
    /// unfitted camera and miss what's on screen.
    fn bake_fit(&mut self) {
        let size = self.bounds.get().size;
        if self.fit_pending && size.width > px(0.0) {
            (self.pan, self.zoom) = fit_camera(
                size,
                &self.graph,
                &self.pos,
                &self.layout.shown,
                self.panel_open,
            );
            self.fit_pending = false;
        }
    }

    /// Advance a running morph to `now`; returns whether one is still running.
    /// With the OS's reduce-motion setting on, it jumps straight to the end.
    fn step_morph(&mut self, now: Instant, reduce_motion: bool) -> bool {
        let Some(m) = &self.morph else { return false };
        let t = if reduce_motion {
            1.0
        } else {
            (now.duration_since(m.start).as_secs_f32() / MORPH.as_secs_f32()).min(1.0)
        };
        let e = 1.0 - (1.0 - t).powi(3); // ease out
        let lerp = |a: f32, b: f32| a + (b - a) * e;
        for (i, p) in self.pos.iter_mut().enumerate() {
            let (a, b) = (m.from[i], self.layout.pos[i]);
            *p = [lerp(a[0], b[0]), lerp(a[1], b[1])];
        }
        let ((fp, fz), (tp, tz)) = (m.cam_from, m.cam_to);
        self.pan = [lerp(fp[0], tp[0]), lerp(fp[1], tp[1])];
        self.zoom = lerp(fz, tz);
        if t >= 1.0 {
            self.morph = None;
        }
        self.morph.is_some()
    }

    /// Canvas-local vector from the canvas center to a window-coords point.
    fn center_offset(&self, p: Point<Pixels>) -> [f32; 2] {
        let b = self.bounds.get();
        let c = b.center();
        [f32::from(p.x - c.x), f32::from(p.y - c.y)]
    }

    /// The shown node under a window-coords point, if any (topmost wins).
    fn hit(&self, p: Point<Pixels>) -> Option<usize> {
        let [cx, cy] = self.center_offset(p);
        (0..self.pos.len()).rev().find(|&i| {
            if !self.layout.shown[i] {
                return false;
            }
            let dx = cx - (self.pan[0] + self.pos[i][0] * self.zoom);
            let dy = cy - (self.pan[1] + self.pos[i][1] * self.zoom);
            let r = (self.graph.nodes[i].radius * self.zoom).max(4.0) + 3.0;
            dx * dx + dy * dy <= r * r
        })
    }

    /// Scale about a window-coords point, keeping the world under it fixed.
    fn zoom_about(&mut self, p: Point<Pixels>, factor: f32) {
        self.morph = None;
        let [cx, cy] = self.center_offset(p);
        let zoom = (self.zoom * factor).clamp(0.05, 4.0);
        let wx = (cx - self.pan[0]) / self.zoom;
        let wy = (cy - self.pan[1]) / self.zoom;
        self.pan = [cx - wx * zoom, cy - wy * zoom];
        self.zoom = zoom;
    }
}

/// The camera (pan, zoom) that fits every shown node inside `size` with margin.
fn fit_camera(
    size: Size<Pixels>,
    graph: &Graph,
    pos: &[[f32; 2]],
    shown: &[bool],
    panel_open: bool,
) -> ([f32; 2], f32) {
    let (mut minx, mut miny) = (f32::MAX, f32::MAX);
    let (mut maxx, mut maxy) = (f32::MIN, f32::MIN);
    let mut any = false;
    for (i, p) in pos.iter().enumerate() {
        if !shown[i] {
            continue;
        }
        let r = graph.nodes[i].radius;
        any = true;
        minx = minx.min(p[0] - r);
        miny = miny.min(p[1] - r);
        maxx = maxx.max(p[0] + r);
        maxy = maxy.max(p[1] + r);
    }
    if !any {
        return ([0.0, 0.0], 1.0);
    }
    const PAD: f32 = 80.0;
    // Fit right of the open panel when there's room, so it covers nothing.
    let full_w = f32::from(size.width).max(1.0);
    let panel = if panel_open && full_w > PANEL_W * 2.5 {
        PANEL_W
    } else {
        0.0
    };
    let (bw, bh) = (maxx - minx + PAD * 2.0, maxy - miny + PAD * 2.0);
    let (w, h) = (full_w - panel, f32::from(size.height).max(1.0));
    let zoom = (w / bw).min(h / bh).clamp(0.05, 1.25);
    let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
    ([-cx * zoom + panel / 2.0, -cy * zoom], zoom)
}

/// Topics by modularity (the Louvain method): each linked node joins the
/// neighboring group that gains the most "more links inside than chance
/// would give"; then each group becomes one node and it repeats, so small
/// groups merge into topics. Busy nodes go first and only strict gains move
/// a node, so the result is deterministic. `None` for nodes with no links.
fn communities(adj: &[Vec<usize>]) -> Vec<Option<usize>> {
    let n = adj.len();
    // The current level: weighted edges between its nodes (each once), and
    // the link weight inside each node (a group from the level below).
    let mut edges: Vec<(usize, usize, f32)> = adj
        .iter()
        .enumerate()
        .flat_map(|(v, ws)| {
            ws.iter()
                .filter(move |&&w| v < w)
                .map(move |&w| (v, w, 1.0))
        })
        .collect();
    let mut inner = vec![0.0f32; n];
    let mut of: Vec<usize> = (0..n).collect(); // original node -> level node
    loop {
        let size = inner.len();
        let mut nbrs: Vec<Vec<(usize, f32)>> = vec![Vec::new(); size];
        for &(a, b, w) in &edges {
            nbrs[a].push((b, w));
            nbrs[b].push((a, w));
        }
        let deg: Vec<f32> = (0..size)
            .map(|v| nbrs[v].iter().map(|e| e.1).sum::<f32>() + 2.0 * inner[v])
            .collect();
        let m2: f32 = deg.iter().sum();
        if m2 == 0.0 {
            break;
        }
        let mut comm: Vec<usize> = (0..size).collect();
        let mut tot = deg.clone();
        let mut order: Vec<usize> = (0..size).filter(|&v| !nbrs[v].is_empty()).collect();
        order.sort_by(|&a, &b| deg[b].total_cmp(&deg[a]).then(a.cmp(&b)));
        let mut any_move = false;
        for _ in 0..30 {
            let mut moved = false;
            for &v in &order {
                let mut links: Vec<(usize, f32)> = Vec::new();
                for &(w, wt) in &nbrs[v] {
                    match links.iter_mut().find(|(c, _)| *c == comm[w]) {
                        Some((_, k)) => *k += wt,
                        None => links.push((comm[w], wt)),
                    }
                }
                links.sort_by_key(|(c, _)| *c);
                let own = comm[v];
                tot[own] -= deg[v];
                let gain = |c: usize, k: f32| k - tot[c] * deg[v] / m2;
                let k_own = links.iter().find(|(c, _)| *c == own).map_or(0.0, |l| l.1);
                let (mut best, mut best_gain) = (own, gain(own, k_own));
                for &(c, k) in &links {
                    if gain(c, k) > best_gain + 1e-6 {
                        (best, best_gain) = (c, gain(c, k));
                    }
                }
                tot[best] += deg[v];
                if best != own {
                    comm[v] = best;
                    moved = true;
                }
            }
            any_move |= moved;
            if !moved {
                break;
            }
        }
        if !any_move {
            break;
        }
        // Collapse each group into one node of the next level.
        let mut renum: HashMap<usize, usize> = HashMap::new();
        for c in &mut comm {
            let next = renum.len();
            *c = *renum.entry(*c).or_insert(next);
        }
        let mut next_inner = vec![0.0f32; renum.len()];
        let mut between: HashMap<(usize, usize), f32> = HashMap::new();
        for (v, w) in inner.iter().enumerate() {
            next_inner[comm[v]] += w;
        }
        for &(a, b, w) in &edges {
            let (ca, cb) = (comm[a], comm[b]);
            if ca == cb {
                next_inner[ca] += w;
            } else {
                *between.entry((ca.min(cb), ca.max(cb))).or_default() += w;
            }
        }
        edges = between.into_iter().map(|((a, b), w)| (a, b, w)).collect();
        edges.sort_by_key(|&(a, b, _)| (a, b));
        inner = next_inner;
        for o in &mut of {
            *o = comm[*o];
        }
    }
    (0..n)
        .map(|v| (!adj[v].is_empty()).then_some(of[v]))
        .collect()
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i32, m: i32, d: i32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The day of a `YYYY-MM-DD…` timestamp (DB `created_at` or a journal date).
fn parse_day(s: &str) -> Option<i32> {
    let mut it = s.get(..10)?.split('-').map(|p| p.parse::<i32>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
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

    // Frame two onward: store the fit frame one painted (see `bake_fit`).
    state.bake_fit();
    let morphing = state.step_morph(Instant::now(), cx.reduce_motion());
    let state = &*state;

    // Snapshot for the paint closure (the state itself stays on AppView).
    let graph = state.graph.clone();
    let style = state.style;
    let pos = state.pos.clone();
    let shown = state.layout.shown.clone();
    let (pan, zoom, hover, focus) = (state.pan, state.zoom, state.hover, state.focus);
    let (fit_pending, panel_open) = (state.fit_pending, state.panel_open);
    let matches: Option<HashSet<usize>> = query.map(|q| {
        (0..graph.nodes.len())
            .filter(|&i| shown[i] && graph.nodes[i].title.to_lowercase().contains(&q))
            .collect()
    });
    let neighbors: HashSet<usize> = hover
        .map(|h| graph.adj[h].iter().copied().collect())
        .unwrap_or_default();
    let match_count = matches.as_ref().map(|m| m.len());
    let bounds_cell = state.bounds.clone();
    let empty = graph.nodes.is_empty();
    let hint = style.hint().unwrap_or_else(|| t!("graph.hint").into());

    let mut el = div()
        .id("graph")
        .size_full()
        .relative()
        .overflow_hidden()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this: &mut AppView, ev: &MouseDownEvent, _w, _cx| {
                if let Some(g) = this.graph.as_mut() {
                    g.bake_fit();
                    g.drag = Some(match g.hit(ev.position) {
                        Some(i) => Drag::Node {
                            i,
                            press: ev.position,
                            last: ev.position,
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
        // Right-click a node for THE page menu (same as page rows elsewhere);
        // this picks the page, the `context_menu` below shows the items.
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this: &mut AppView, ev: &MouseDownEvent, _w, _cx| {
                let Some(g) = this.graph.as_mut() else { return };
                g.bake_fit();
                g.menu_node = g.hit(ev.position);
                if let Some(i) = g.menu_node {
                    let n = &g.graph.nodes[i];
                    let (id, title) = (n.id, n.title.clone());
                    this.set_context_page(id, title);
                }
            }),
        )
        .on_mouse_move(
            cx.listener(|this: &mut AppView, ev: &MouseMoveEvent, _w, cx| {
                let Some(g) = this.graph.as_mut() else { return };
                g.bake_fit();
                match g.drag {
                    Some(Drag::Pan { last, moved }) => {
                        let (dx, dy) = (
                            f32::from(ev.position.x - last.x),
                            f32::from(ev.position.y - last.y),
                        );
                        g.morph = None;
                        g.pan = [g.pan[0] + dx, g.pan[1] + dy];
                        g.drag = Some(Drag::Pan {
                            last: ev.position,
                            moved: moved || dx.abs() + dy.abs() > CLICK_SLOP,
                        });
                        cx.notify();
                    }
                    Some(Drag::Node {
                        i,
                        press,
                        last,
                        moved,
                    }) => {
                        let (dx, dy) = (
                            f32::from(ev.position.x - press.x),
                            f32::from(ev.position.y - press.y),
                        );
                        let moved = moved || dx.abs() + dy.abs() > CLICK_SLOP;
                        if moved {
                            g.morph = None;
                            if g.style.draggable() {
                                let [ox, oy] = g.center_offset(ev.position);
                                let p = [(ox - g.pan[0]) / g.zoom, (oy - g.pan[1]) / g.zoom];
                                g.pos[i] = p;
                                g.layout.pos[i] = p;
                            } else {
                                g.pan[0] += f32::from(ev.position.x - last.x);
                                g.pan[1] += f32::from(ev.position.y - last.y);
                            }
                        }
                        g.drag = Some(Drag::Node {
                            i,
                            press,
                            last: ev.position,
                            moved,
                        });
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
                    if g.style.uses_focus() && g.focus != Some(i) {
                        // Focus styles center a clicked page; clicking the
                        // centered one opens it.
                        g.focus = Some(i);
                        g.relayout();
                        cx.notify();
                        return;
                    }
                    let title = g.graph.nodes[i].title.to_string();
                    // Routes like a wiki-link: boards open their canvas.
                    this.open_page_title(&title, window, cx);
                }
            }),
        )
        .on_scroll(cx.listener(|this: &mut AppView, ev: &ScrollEvent, _w, cx| {
            let Some(g) = this.graph.as_mut() else { return };
            g.bake_fit();
            let (dx, dy) = match ev.delta {
                ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
                ScrollDelta::Lines(p) => (p.x * LINE_PX, p.y * LINE_PX),
            };
            if ev.modifiers.platform || ev.modifiers.control {
                g.zoom_about(ev.position, (1.0 + dy * 0.0025).clamp(0.5, 2.0));
            } else {
                g.morph = None;
                g.pan = [g.pan[0] + dx, g.pan[1] + dy];
            }
            cx.notify();
        }))
        .on_pinch(cx.listener(|this: &mut AppView, ev: &PinchEvent, _w, cx| {
            if let Some(g) = this.graph.as_mut() {
                g.bake_fit();
                g.zoom_about(ev.position, 1.0 + ev.delta);
                cx.notify();
            }
        }));
    if hover.is_some() {
        el = el.cursor_pointer();
    }
    let app = cx.entity();
    let el = el.context_menu(move |menu, _w, cx| {
        let app = app.read(cx);
        let Some(id) = app
            .graph
            .as_ref()
            .and_then(|g| g.menu_node.map(|i| g.graph.nodes[i].id))
        else {
            return menu; // an empty menu isn't shown
        };
        crate::ui::page_menu(menu, app.is_favorite(id), cx)
    });

    el.child(
        canvas(
            move |bounds, _, _| bounds_cell.set(bounds),
            move |bounds, _, window, cx| {
                let (pan, zoom) = if fit_pending {
                    fit_camera(bounds.size, &graph, &pos, &shown, panel_open)
                } else {
                    (pan, zoom)
                };
                let frame = Frame {
                    pos: &pos,
                    hover,
                    neighbors: &neighbors,
                    matches: matches.as_ref(),
                    focus,
                };
                style.paint(
                    &graph,
                    &frame,
                    &mut Painter::new(window, cx, bounds, pan, zoom),
                );
                if morphing {
                    window.request_animation_frame();
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
                t!("graph.empty").into()
            } else {
                hint
            }),
    )
    .into_any_element()
}

/// The floating control panel: the style picker, search and the legend +
/// statistics up top, node filters and a reset action below.
fn panel(
    state: &GraphState,
    search: Option<Entity<InputState>>,
    match_count: Option<usize>,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let f = state.filters;
    let open = state.panel_open;
    let nodes = &state.graph.nodes;
    let count = |k: Kind| nodes.iter().filter(|n| n.kind == k).count();
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

    let current = state.style.id();
    let styles =
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(4.0))
            .children(style::STYLES.iter().map(|s| {
                let id = s.id();
                let active = id == current;
                div()
                    .id(SharedString::from(format!("graph-style-{id}")))
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .map(|el| {
                        if active {
                            el.bg(theme::accent_tint()).text_color(theme::accent())
                        } else {
                            el.text_color(theme::text_secondary())
                                .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
                        }
                    })
                    .on_click(
                        cx.listener(move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                            this.set_graph_style(id, cx);
                        }),
                    )
                    .child(s.name())
            }));

    let mut legend = div().flex().flex_col().gap(px(5.0)).child(legend_row(
        dot(Kind::Page.color()),
        t!("graph.filter_pages"),
        pages.to_string(),
    ));
    if boards > 0 {
        legend = legend.child(legend_row(
            dot(Kind::Board.color()),
            t!("graph.filter_whiteboards"),
            boards.to_string(),
        ));
    }
    if journals > 0 {
        legend = legend.child(legend_row(
            dot(Kind::Journal.color()),
            t!("graph.filter_journals"),
            journals.to_string(),
        ));
    }
    legend = legend
        .child(legend_row(
            dash,
            t!("graph.filter_links"),
            state.graph.edges.len().to_string(),
        ))
        .child(legend_row(ring, t!("graph.filter_orphans"), orphans));

    // Everything under the title bar, hidden while the panel is retracted.
    let body = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(styles)
        .children(search.map(|input| Input::new(&input).small().text_size(px(12.0))))
        .children(match_count.map(|n| {
            div()
                .text_size(px(11.0))
                .text_color(theme::text_tertiary())
                .child(match n {
                    0 => t!("graph.no_matches").into_owned(),
                    1 => format!("1 {}", t!("graph.match")),
                    n => format!("{n} {}", t!("graph.matches")),
                })
        }))
        .child(legend)
        .child(div().h(px(1.0)).bg(theme::divider()))
        .child(toggle_row(
            t!("graph.filter_journals"),
            toggle("graph-journals", f.journals, |f, v| f.journals = v),
        ))
        .child(toggle_row(
            t!("graph.legend_orphans"),
            toggle("graph-orphans", f.orphans, |f, v| f.orphans = v),
        ))
        .child(toggle_row(
            t!("graph.filter_whiteboards"),
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
                .child(t!("graph.reset")),
        );

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
            // The title bar: click to retract the panel to just this row.
            div()
                .id("graph-panel-head")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                        this.set_graph_panel_open(!open, cx);
                    }),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(theme::text_primary())
                        .child(t!("graph.nodes")),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme::text_tertiary())
                        .child(format!("{} {}", nodes.len(), t!("graph.shown"))),
                )
                .child(
                    Icon::new(if open {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .small()
                    .text_color(theme::text_tertiary()),
                ),
        )
        .when(open, |el| el.child(body))
}

/// One legend line: a color mark, the kind, and its count.
fn legend_row(
    mark: gpui::AnyElement,
    label: impl Into<SharedString>,
    count: String,
) -> impl IntoElement {
    let label = label.into();
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
fn toggle_row(label: impl Into<SharedString>, switch: Switch) -> impl IntoElement {
    let label = label.into();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_linked_triangles_are_two_topics() {
        // 0-1-2 and 3-4-5 are triangles; 2-3 bridges them; 6 has no links.
        let mut adj = vec![Vec::new(); 7];
        for (a, b) in [(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5), (2, 3)] {
            adj[a].push(b);
            adj[b].push(a);
        }
        let g = communities(&adj);
        assert_eq!(g[0], g[1]);
        assert_eq!(g[1], g[2]);
        assert_eq!(g[3], g[4]);
        assert_eq!(g[4], g[5]);
        assert_ne!(g[0], g[3]);
        assert_eq!(g[6], None);
    }

    fn page(id: i64, title: &str, created: Option<&str>, journal: Option<&str>) -> Page {
        Page {
            id,
            title: title.into(),
            is_journal: journal.is_some(),
            journal_date: journal.map(Into::into),
            content: String::new(),
            created_at: created.map(Into::into),
            updated_at: None,
        }
    }

    #[test]
    fn imported_pages_date_from_their_first_journal_mention() {
        // Both pages were imported on the same day; a 2024 journal day links A.
        let pages = [
            page(1, "A", Some("2026-06-16 15:00:00"), None),
            page(2, "B", Some("2026-06-16 15:00:00"), None),
        ];
        let journals = [page(3, "Mar 1st, 2024", None, Some("2024-03-01"))];
        let links = [(3, 1), (1, 2)];
        // Journal days stay hidden as nodes but still date the pages.
        let g = GraphState::build(
            &pages,
            &[],
            &journals,
            &links,
            GraphFilters::default(),
            None,
        );
        let day = |t: &str| g.graph.nodes.iter().find(|n| n.title == t).unwrap().day;
        assert_eq!(day("A"), parse_day("2024-03-01"));
        assert_eq!(day("B"), parse_day("2026-06-16"));
        assert_eq!(g.graph.nodes.len(), 2);
    }

    #[test]
    fn parses_days_since_epoch() {
        assert_eq!(parse_day("1970-01-02"), Some(1));
        assert_eq!(parse_day("2000-03-01 08:00:00"), Some(11_017));
        assert_eq!(parse_day("garbage"), None);
    }
}
