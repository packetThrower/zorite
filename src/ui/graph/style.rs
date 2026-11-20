//! Graph styles: where the nodes go and how the graph is drawn.
//!
//! The core (`mod.rs`) owns everything a style doesn't: the data, camera,
//! pan/zoom, hover, search, dragging, the panel, and the animated move from
//! one layout to the next. A style answers two questions:
//!
//! - [`GraphStyle::layout`]: a world position for every node. It runs once per
//!   change (filters, a style switch, a new focus), so slow work belongs here.
//! - [`GraphStyle::paint`]: draw edges, nodes and labels for one frame, through
//!   a [`Painter`] that maps world positions to the screen.
//!
//! ## Adding a style
//!
//! 1. Create `src/ui/graph/<name>.rs` with a unit struct implementing
//!    [`GraphStyle`]. `classic.rs` is the plainest one to copy.
//! 2. Add `mod <name>;` to `mod.rs` and your struct to [`STYLES`].
//! 3. Add the picker label as `graph.style_<id>` to every file in `locales/`.
//!
//! The id is saved in the user's settings, so never rename it once shipped.
//! Take colors from `crate::theme` and [`palette`] so the style follows the
//! user's theme, and paint no background: glass themes show the desktop
//! through the graph. Each node carries its topic ([`super::Node::group`],
//! found from the links) and creation day, for styles that group or sort.

use std::collections::HashSet;

use gpui::{
    App, BorderStyle, Bounds, BoxShadow, Corners, Edges, FontWeight, Hsla, PathBuilder, Pixels,
    Point, SharedString, TextRun, Window, fill, point, px, quad, size,
};

use super::Graph;
use super::{classic::Classic, focus::Focus, islands::Islands, nebula::Nebula, timeline::Timeline};
use crate::theme;

/// Every style, in picker order. The first is the default.
pub static STYLES: &[&dyn GraphStyle] = &[&Classic, &Nebula, &Islands, &Focus, &Timeline];

/// The style with this id, or the default when it's unknown (a style that
/// was removed, or nothing saved yet).
pub fn find(id: &str) -> &'static dyn GraphStyle {
    STYLES
        .iter()
        .copied()
        .find(|s| s.id() == id)
        .unwrap_or(STYLES[0])
}

pub trait GraphStyle: Sync {
    /// Stable id, saved in settings.
    fn id(&self) -> &'static str;
    /// The picker label.
    fn name(&self) -> SharedString;
    /// Where every node goes. `focus` is the centered node, for styles that
    /// use one (see [`Self::uses_focus`]).
    fn layout(&self, g: &Graph, focus: Option<usize>) -> Layout;
    /// Draw one frame.
    fn paint(&self, g: &Graph, f: &Frame, p: &mut Painter);
    /// Whether dragging a node moves it. When false, dragging pans.
    fn draggable(&self) -> bool {
        true
    }
    /// Whether clicking a node centers it (and re-runs the layout) instead
    /// of opening it. Clicking the centered node still opens it.
    fn uses_focus(&self) -> bool {
        false
    }
    /// A hint for the bottom-left corner, when the default one doesn't fit.
    fn hint(&self) -> Option<SharedString> {
        None
    }
}

/// A style's layout: a world position per node, and which nodes take part.
/// Hidden nodes aren't drawn, hit or counted in the camera fit.
pub struct Layout {
    pub pos: Vec<[f32; 2]>,
    pub shown: Vec<bool>,
}

impl Layout {
    /// Every node shown.
    pub fn all(pos: Vec<[f32; 2]>) -> Self {
        let shown = vec![true; pos.len()];
        Self { pos, shown }
    }
}

/// What a style paints from, for one frame.
pub struct Frame<'a> {
    /// Where each node is right now (mid-animation, between layouts).
    pub pos: &'a [[f32; 2]],
    pub hover: Option<usize>,
    /// The hovered node's neighbors.
    pub neighbors: &'a HashSet<usize>,
    /// Search matches, while the search box has text.
    pub matches: Option<&'a HashSet<usize>>,
    pub focus: Option<usize>,
}

impl Frame<'_> {
    /// Nodes in label order: lit first, then the most linked, so when labels
    /// collide the hovered page and the hubs keep theirs.
    pub fn label_order(&self, g: &Graph) -> Vec<usize> {
        let mut order: Vec<usize> = (0..g.nodes.len()).collect();
        order.sort_by_key(|&i| (!self.lit(i), std::cmp::Reverse(g.nodes[i].degree)));
        order
    }

    /// Hovered or a search match: draw it emphasized.
    pub fn lit(&self, i: usize) -> bool {
        self.hover == Some(i) || self.matches.is_some_and(|m| m.contains(&i))
    }

    /// Faded because something else is hovered or searched for.
    pub fn dimmed(&self, i: usize) -> bool {
        match (self.hover, self.matches) {
            (Some(h), _) => h != i && !self.neighbors.contains(&i),
            (None, Some(m)) => !m.contains(&i),
            (None, None) => false,
        }
    }

    /// Whether an edge touches the hovered node.
    pub fn edge_lit(&self, a: usize, b: usize) -> bool {
        self.hover.is_some_and(|h| h == a || h == b)
    }
}

/// Topic color `i`, tuned for the current theme's lightness.
pub fn palette(i: usize) -> Hsla {
    const HUES: [f32; 8] = [215.0, 168.0, 32.0, 270.0, 132.0, 345.0, 48.0, 192.0];
    Hsla {
        h: HUES[i % HUES.len()] / 360.0,
        s: 0.72,
        l: if is_dark() { 0.66 } else { 0.46 },
        a: 1.0,
    }
}

/// Whether the current theme is dark (light text).
pub fn is_dark() -> bool {
    theme::text_primary().l > 0.5
}

/// A deterministic hash → [-1, 1), for layout jitter (`rand` stays out of
/// the tree, and a seeded look survives rebuilds).
pub fn hash_unit(seed: usize) -> f32 {
    let mut x = seed as u64 ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x & 0xFFFF) as f32 / 32768.0 - 1.0
}

/// A Fruchterman–Reingold force layout of `n` nodes: all pairs repel, linked
/// nodes attract, settled positions around an arbitrary origin.
pub fn force_layout(n: usize, edges: &[(usize, usize)]) -> Vec<[f32; 2]> {
    let mut pos: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let a = i as f32 * 2.399_963; // golden angle
            let r = 28.0 * (i as f32).sqrt();
            [r * a.cos(), r * a.sin()]
        })
        .collect();
    if n < 2 {
        return pos;
    }
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

/// The convex hull of `pts` (Andrew's monotone chain), counter-clockwise.
pub fn hull(pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut p = pts.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(p.len() * 2);
    for pass in 0..2 {
        let start = out.len();
        let iter: Box<dyn Iterator<Item = &[f32; 2]>> = if pass == 0 {
            Box::new(p.iter())
        } else {
            Box::new(p.iter().rev())
        };
        for &q in iter {
            while out.len() >= start + 2 && cross(out[out.len() - 2], out[out.len() - 1], q) <= 0.0
            {
                out.pop();
            }
            out.push(q);
        }
        out.pop();
    }
    out
}

/// Horizontal text alignment for [`Painter::text`].
#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Vertical text alignment for [`Painter::text`].
#[derive(Clone, Copy)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

/// The world → screen mapping for one frame. `Copy`, so a path-building
/// closure can use it while the [`Painter`] is borrowed.
#[derive(Clone, Copy)]
pub struct View {
    center: Point<Pixels>,
    pan: [f32; 2],
    zoom: f32,
}

impl View {
    /// The screen point of a world position.
    pub fn at(&self, p: [f32; 2]) -> Point<Pixels> {
        point(
            self.center.x + px(self.pan[0] + p[0] * self.zoom),
            self.center.y + px(self.pan[1] + p[1] * self.zoom),
        )
    }

    /// A world length on screen, never smaller than `min` px.
    pub fn len(&self, world: f32, min: f32) -> Pixels {
        px((world * self.zoom).max(min))
    }
}

/// Drawing for styles: the [`View`] plus the few primitives the built-in
/// styles need. `window` and `cx` stay public for anything else.
pub struct Painter<'a> {
    pub window: &'a mut Window,
    pub cx: &'a mut App,
    pub bounds: Bounds<Pixels>,
    pub view: View,
    /// Text drawn so far this frame, so [`Painter::label`] can keep clear.
    placed: Vec<Bounds<Pixels>>,
}

impl<'a> Painter<'a> {
    pub fn new(
        window: &'a mut Window,
        cx: &'a mut App,
        bounds: Bounds<Pixels>,
        pan: [f32; 2],
        zoom: f32,
    ) -> Self {
        let view = View {
            center: bounds.center(),
            pan,
            zoom,
        };
        Self {
            window,
            cx,
            bounds,
            view,
            placed: Vec::new(),
        }
    }

    pub fn zoom(&self) -> f32 {
        self.view.zoom
    }

    /// The screen point of a world position.
    pub fn at(&self, p: [f32; 2]) -> Point<Pixels> {
        self.view.at(p)
    }

    /// A world length on screen, never smaller than `min` px.
    pub fn len(&self, world: f32, min: f32) -> Pixels {
        self.view.len(world, min)
    }

    pub fn dot(&mut self, c: Point<Pixels>, r: Pixels, color: Hsla) {
        let mut q = fill(
            Bounds::new(point(c.x - r, c.y - r), size(r * 2.0, r * 2.0)),
            color,
        );
        q.corner_radii = Corners::all(r);
        self.window.paint_quad(q);
    }

    /// A rounded square of about the same weight as a dot of radius `r`:
    /// how whiteboards are drawn.
    pub fn square(&mut self, c: Point<Pixels>, r: Pixels, color: Hsla) {
        let s = r * 1.8;
        let mut q = fill(
            Bounds::new(point(c.x - s / 2.0, c.y - s / 2.0), size(s, s)),
            color,
        );
        q.corner_radii = Corners::all(s * 0.28);
        self.window.paint_quad(q);
    }

    /// A dot for a page, a rounded square for a whiteboard.
    pub fn node(&mut self, c: Point<Pixels>, r: Pixels, color: Hsla, board: bool) {
        if board {
            self.square(c, r, color)
        } else {
            self.dot(c, r, color)
        }
    }

    /// A filled circle with a border.
    pub fn ring(&mut self, c: Point<Pixels>, r: Pixels, fill: Hsla, border: Hsla, width: Pixels) {
        self.window.paint_quad(quad(
            Bounds::new(point(c.x - r, c.y - r), size(r * 2.0, r * 2.0)),
            Corners::all(r),
            fill,
            Edges::all(width),
            border,
            BorderStyle::default(),
        ));
    }

    /// A soft glow around a circle of radius `r`.
    pub fn glow(&mut self, c: Point<Pixels>, r: Pixels, color: Hsla) {
        self.window.paint_drop_shadows(
            Bounds::new(point(c.x - r, c.y - r), size(r * 2.0, r * 2.0)),
            Corners::all(r),
            &[BoxShadow {
                color,
                offset: point(px(0.0), px(0.0)),
                blur_radius: r * 2.4,
                spread_radius: r * 0.4,
                inset: false,
            }],
        );
    }

    /// Stroke whatever `draw` adds to a path; `draw` returns whether it added
    /// anything. `dash` is `[on, off]` in px.
    pub fn stroke(
        &mut self,
        width: f32,
        color: Hsla,
        dash: Option<[f32; 2]>,
        draw: impl FnOnce(&mut PathBuilder) -> bool,
    ) {
        let mut pb = PathBuilder::stroke(px(width));
        if let Some([on, off]) = dash {
            pb = pb.dash_array(&[px(on), px(off)]);
        }
        if draw(&mut pb)
            && let Ok(path) = pb.build()
        {
            self.window.paint_path(path, color);
        }
    }

    pub fn fill_polygon(&mut self, pts: &[Point<Pixels>], color: Hsla) {
        if pts.len() < 3 {
            return;
        }
        let mut pb = PathBuilder::fill();
        pb.add_polygon(pts, true);
        if let Ok(path) = pb.build() {
            self.window.paint_path(path, color);
        }
    }

    /// The width `text` takes at `size` px.
    pub fn measure(&mut self, text: &SharedString, size: f32, bold: bool) -> Pixels {
        self.shape(text, size, theme::text_primary(), bold).width()
    }

    fn shape(
        &mut self,
        text: &SharedString,
        size: f32,
        color: Hsla,
        bold: bool,
    ) -> gpui::ShapedLine {
        let mut font = self.window.text_style().font();
        if bold {
            font.weight = FontWeight::SEMIBOLD;
        }
        let run = TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        self.window
            .text_system()
            .shape_line(text.clone(), px(size), &[run], None)
    }

    /// One line of text anchored at `at`.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        text: &SharedString,
        at: Point<Pixels>,
        size: f32,
        color: Hsla,
        align: Align,
        valign: VAlign,
        bold: bool,
    ) {
        self.place(text, at, size, color, align, valign, bold, false);
    }

    /// A node's name, centered just above `top`: above, because the pointer
    /// hangs below its hotspot and would cover it. Unless `always` (the
    /// hovered or searched-for page), it's skipped when it would overlap text
    /// already drawn, so draw the important ones first (see
    /// [`Frame::label_order`]).
    pub fn label(
        &mut self,
        text: &SharedString,
        top: Point<Pixels>,
        color: Hsla,
        bold: bool,
        always: bool,
    ) {
        self.place(
            text,
            top,
            11.0,
            color,
            Align::Center,
            VAlign::Bottom,
            bold,
            !always,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        text: &SharedString,
        at: Point<Pixels>,
        size: f32,
        color: Hsla,
        align: Align,
        valign: VAlign,
        bold: bool,
        keep_clear: bool,
    ) {
        let shaped = self.shape(text, size, color, bold);
        let lh = px(size * 1.3);
        let x = match align {
            Align::Left => at.x,
            Align::Center => at.x - shaped.width() / 2.0,
            Align::Right => at.x - shaped.width(),
        };
        let y = match valign {
            VAlign::Top => at.y,
            VAlign::Middle => at.y - lh / 2.0,
            VAlign::Bottom => at.y - lh,
        };
        let b = Bounds::new(point(x, y), gpui::size(shaped.width(), lh));
        if keep_clear && self.placed.iter().any(|q| q.intersects(&b.dilate(px(3.0)))) {
            return;
        }
        self.placed.push(b);
        let _ = shaped.paint(
            point(x, y),
            lh,
            gpui::TextAlign::Left,
            None,
            self.window,
            self.cx,
        );
    }

    /// `text` cut short with an ellipsis to fit `max` px.
    pub fn fit(&mut self, text: &SharedString, size: f32, bold: bool, max: Pixels) -> SharedString {
        if self.measure(text, size, bold) <= max {
            return text.clone();
        }
        let mut s = text.to_string();
        while s.pop().is_some() {
            let cut: SharedString = format!("{}…", s.trim_end()).into();
            if s.is_empty() || self.measure(&cut, size, bold) <= max {
                return cut;
            }
        }
        text.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_drops_inner_points() {
        let h = hull(&[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [2.0, 2.0]]);
        assert_eq!(h.len(), 4);
        assert!(!h.contains(&[2.0, 2.0]));
    }

    #[test]
    fn unknown_style_falls_back_to_the_default() {
        assert_eq!(find("no-such-style").id(), STYLES[0].id());
        let mut ids: Vec<_> = STYLES.iter().map(|s| s.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), STYLES.len(), "style ids must be unique");
    }
}
