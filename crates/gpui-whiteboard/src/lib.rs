//! An infinite, pannable/zoomable whiteboard canvas for GPUI.
//!
//! Host-agnostic — depends only on `gpui` + `serde`. Modeled on the
//! `gpui-pdf` stateful-entity pattern: the host builds a [`WhiteboardView`]
//! from a [`Scene`], stores the entity, and renders it in a tab. This crate
//! owns the scene model and its (de)serialization; the host owns persistence,
//! theme, and navigation.
//!
//! **Phase 3** (this version): a tool palette — freehand **pen** plus
//! **rectangle, ellipse, line, and arrow** (drag to create, rendered with
//! `PathBuilder`). Pan is middle-drag / scroll; zoom is pinch / ⌘-scroll;
//! double-click resets. Everything persists via [`WhiteboardView::set_on_change`].
//! Text, selection/manipulation, and embedded page-cards come in later phases —
//! see `docs/whiteboard-architecture.md` in the host repo.
//!
//! Perf note: elements are re-tessellated each paint (as GPUI's own
//! `painting`/`brush` examples do). A built-`Path` cache + viewport culling is
//! the planned optimization once boards get large (Phase 6) — deferred so we
//! don't build it before there's something to measure.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, PinchEvent, Pixels, Point, Render,
    ScrollDelta, ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled, Window,
    canvas, div, fill, point, px, size,
};
use serde::{Deserialize, Serialize};

/// Zoom is clamped to this range (also guards the world↔screen math against a
/// zero/negative factor from hand-edited JSON).
const MIN_ZOOM: f32 = 0.1;
const MAX_ZOOM: f32 = 8.0;
/// World-space distance between grid dots.
const GRID: f32 = 24.0;
/// Smallest on-screen dot spacing before the grid is coarsened (×4).
const MIN_DOT_SPACING: f32 = 16.0;
/// Dot size in screen px (constant — dots don't grow with zoom).
const DOT: f32 = 2.0;
/// Screen px per scroll "line" for inexact (`Lines`) scroll deltas.
const LINE_PX: f32 = 16.0;
/// Pen nib in screen px. A stored width is world-space (`NIB / zoom` at draw
/// time) so strokes/shapes feel like a constant nib yet scale with the content.
const NIB: f32 = 2.5;
/// Minimum on-screen gap between recorded freehand points (input thinning).
const MIN_POINT_PX: f32 = 2.0;

/// The board document: everything persisted for a whiteboard. Owned and
/// (de)serialized here; the host stores [`Scene::to_json`] opaquely (for zorite,
/// in the `content` column of a `kind = 'whiteboard'` page).
///
/// Every field is `#[serde(default)]` so older boards keep loading as the model
/// grows.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scene {
    /// The viewport (pan + zoom). Persisted so a board reopens where you left it.
    #[serde(default)]
    pub camera: Camera,
    /// The board's content, painted in z-order.
    #[serde(default)]
    pub elements: Vec<Element>,
}

impl Scene {
    /// Parse a board from its stored JSON, falling back to an empty board on
    /// empty or malformed input — a corrupt row never blocks opening the tab.
    pub fn from_json(s: &str) -> Self {
        let mut scene: Self = if s.trim().is_empty() {
            Self::default()
        } else {
            serde_json::from_str(s).unwrap_or_else(|e| {
                log::warn!("whiteboard: ignoring bad scene JSON ({e}); starting empty");
                Self::default()
            })
        };
        if !scene.camera.zoom.is_finite() || scene.camera.zoom <= 0.0 {
            scene.camera.zoom = 1.0;
        }
        scene
    }

    /// Serialize for persistence.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// One board element: a stable id plus its geometry/kind.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Element {
    pub id: u64,
    pub kind: ElementKind,
}

/// The kinds of thing a board can hold. Text + embeds arrive in later phases.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Draw(Stroke),
    Rect(BoxGeom),
    Ellipse(BoxGeom),
    Line(SegGeom),
    Arrow(SegGeom),
}

/// A freehand pen stroke: world-space points and a world-space width.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    pub width: f32,
}

/// An axis-aligned box (rectangle / ellipse bounding box), world-space.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BoxGeom {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub width: f32,
}

/// A directed segment (line / arrow), world-space.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SegGeom {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub width: f32,
}

/// The viewport: a world-space pan offset and a zoom factor. The offset is the
/// world point that maps to the canvas's top-left corner, so a screen point
/// `s` (relative to the canvas) is the world point `offset + s / zoom`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Camera {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default = "one")]
    pub zoom: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// The world point under a canvas-relative screen point. The inverse of the
    /// paint transform; used by tests now, and by hit-testing once it lands.
    #[cfg(test)]
    fn screen_to_world(&self, sx: f32, sy: f32) -> (f32, f32) {
        let z = self.zoom.max(MIN_ZOOM);
        (self.x + sx / z, self.y + sy / z)
    }

    /// Pan by a screen-space delta (px): the content follows the gesture.
    fn pan_by(&mut self, dx: f32, dy: f32) {
        let z = self.zoom.max(MIN_ZOOM);
        self.x -= dx / z;
        self.y -= dy / z;
    }

    /// Multiply the zoom by `factor`, keeping the world point under the
    /// canvas-relative screen point `(rx, ry)` fixed (zoom-about-cursor).
    fn zoom_about(&mut self, rx: f32, ry: f32, factor: f32) {
        let z = self.zoom;
        let z2 = (z * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if (z2 - z).abs() < f32::EPSILON {
            return;
        }
        self.x += rx * (1.0 / z - 1.0 / z2);
        self.y += ry * (1.0 / z - 1.0 / z2);
        self.zoom = z2;
    }
}

/// The active drawing tool. UI state — not part of the persisted scene.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Rect,
    Ellipse,
    Line,
    Arrow,
}

impl Tool {
    const ALL: [Tool; 5] = [
        Tool::Pen,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Arrow,
    ];

    /// A glyph for the toolbar button (dependency-free; the host has no icon set
    /// in this crate).
    fn glyph(self) -> &'static str {
        match self {
            Tool::Pen => "✎",
            Tool::Rect => "▭",
            Tool::Ellipse => "◯",
            Tool::Line => "╱",
            Tool::Arrow => "↗",
        }
    }
}

/// Theme colors, read at paint time (via [`WhiteboardStyleFn`]) so the board
/// follows live theme changes per window.
#[derive(Clone, Copy, Debug)]
pub struct WhiteboardStyle {
    /// The canvas background.
    pub bg: Hsla,
    /// The background grid dots.
    pub grid: Hsla,
    /// HUD / muted on-canvas text and toolbar glyphs.
    pub text: Hsla,
    /// Ink (stroke/shape color). Per-element color comes with the color picker.
    pub ink: Hsla,
    /// Toolbar panel background.
    pub panel: Hsla,
    /// Active-tool highlight.
    pub accent: Hsla,
}

/// A `() -> WhiteboardStyle` the host supplies; called each paint so the board
/// tracks theme changes without the host pushing updates.
pub type WhiteboardStyleFn = Rc<dyn Fn() -> WhiteboardStyle>;

/// Called when the board changes (an element committed, the camera moved), with
/// the serialized scene JSON, so the host can persist it.
pub type ChangeFn = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// An element being created by the current left-drag.
struct Pending {
    anchor: [f32; 2],
    kind: ElementKind,
}

/// The whiteboard view entity. The host holds it in an `Entity<WhiteboardView>`
/// (keyed by board id) and renders it into a tab.
pub struct WhiteboardView {
    scene: Scene,
    style: WhiteboardStyleFn,
    on_change: Option<ChangeFn>,
    tool: Tool,
    /// Canvas bounds in window coords, captured each paint so input handlers can
    /// map window-relative event positions into the board.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The element being created by the in-progress left-drag.
    pending: Option<Pending>,
    /// True while a middle-drag pan is in progress.
    panning: bool,
    /// Last pointer position (window coords) during a pan.
    last: Point<Pixels>,
    /// Next element id.
    next_id: u64,
    /// Unsaved changes since the last flush (flushed on mouse-up).
    dirty: bool,
}

impl WhiteboardView {
    /// Build a view over `scene`. Call inside `cx.new(|cx| WhiteboardView::new(..))`.
    pub fn new(scene: Scene, style: WhiteboardStyleFn, _cx: &mut Context<Self>) -> Self {
        let next_id = scene
            .elements
            .iter()
            .map(|e| e.id)
            .max()
            .map_or(0, |m| m + 1);
        Self {
            scene,
            style,
            on_change: None,
            tool: Tool::Pen,
            bounds: Rc::new(Cell::new(Bounds::default())),
            pending: None,
            panning: false,
            last: Point::default(),
            next_id,
            dirty: false,
        }
    }

    /// Install the persistence hook (called with the serialized scene on change).
    pub fn set_on_change(&mut self, f: ChangeFn) {
        self.on_change = Some(f);
    }

    /// The current board document (for the host to persist).
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The active tool (e.g. for host-driven chrome).
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Switch the active drawing tool.
    pub fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if self.tool != tool {
            self.tool = tool;
            cx.notify();
        }
    }

    /// Reset the viewport to the origin at 100% (also bound to double-click).
    pub fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.scene.camera = Camera::default();
        self.dirty = true;
        cx.notify();
    }

    /// Zoom in/out a step, centered on the canvas.
    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.zoom_centered(1.2, cx);
    }
    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.zoom_centered(1.0 / 1.2, cx);
    }

    fn zoom_centered(&mut self, factor: f32, cx: &mut Context<Self>) {
        let b = self.bounds.get();
        let rx = f32::from(b.size.width) / 2.0;
        let ry = f32::from(b.size.height) / 2.0;
        self.scene.camera.zoom_about(rx, ry, factor);
        self.dirty = true;
        cx.notify();
    }

    /// Flush pending changes through the host's persistence hook.
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Some(f) = self.on_change.clone() {
            f(self.scene.to_json(), window, cx);
        }
    }

    /// World point under a window-coords event position.
    fn event_to_world(&self, p: Point<Pixels>) -> [f32; 2] {
        let o = self.bounds.get().origin;
        let z = self.scene.camera.zoom.max(MIN_ZOOM);
        [
            self.scene.camera.x + f32::from(p.x - o.x) / z,
            self.scene.camera.y + f32::from(p.y - o.y) / z,
        ]
    }

    fn on_left_down(&mut self, ev: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if ev.click_count >= 2 {
            self.pending = None;
            self.reset_view(cx);
            return;
        }
        if self.panning {
            return;
        }
        let p = self.event_to_world(ev.position);
        let width = NIB / self.scene.camera.zoom.max(MIN_ZOOM);
        let kind = match self.tool {
            Tool::Pen => ElementKind::Draw(Stroke {
                points: vec![p],
                width,
            }),
            Tool::Rect => ElementKind::Rect(BoxGeom {
                x: p[0],
                y: p[1],
                w: 0.0,
                h: 0.0,
                width,
            }),
            Tool::Ellipse => ElementKind::Ellipse(BoxGeom {
                x: p[0],
                y: p[1],
                w: 0.0,
                h: 0.0,
                width,
            }),
            Tool::Line => ElementKind::Line(SegGeom {
                x1: p[0],
                y1: p[1],
                x2: p[0],
                y2: p[1],
                width,
            }),
            Tool::Arrow => ElementKind::Arrow(SegGeom {
                x1: p[0],
                y1: p[1],
                x2: p[0],
                y2: p[1],
                width,
            }),
        };
        self.pending = Some(Pending { anchor: p, kind });
        cx.notify();
    }

    fn on_left_up(&mut self, _ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending.take() {
            if committable(&pending.kind) {
                let id = self.next_id;
                self.next_id += 1;
                self.scene.elements.push(Element {
                    id,
                    kind: pending.kind,
                });
                self.dirty = true;
            }
            cx.notify();
        }
        self.flush(window, cx);
    }

    fn on_middle_down(
        &mut self,
        ev: &MouseDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if self.pending.is_some() {
            return;
        }
        self.panning = true;
        self.last = ev.position;
    }

    fn on_middle_up(&mut self, _ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.panning {
            self.panning = false;
            cx.notify();
        }
        self.flush(window, cx);
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            let cur = self.event_to_world(ev.position);
            let z = self.scene.camera.zoom.max(MIN_ZOOM);
            let anchor = self.pending.as_ref().unwrap().anchor;
            let pending = self.pending.as_mut().unwrap();
            match &mut pending.kind {
                ElementKind::Draw(s) => {
                    // Thin the input: skip points within MIN_POINT_PX of the last.
                    if let Some(last) = s.points.last() {
                        let (dx, dy) = ((cur[0] - last[0]) * z, (cur[1] - last[1]) * z);
                        if dx * dx + dy * dy < MIN_POINT_PX * MIN_POINT_PX {
                            return;
                        }
                    }
                    s.points.push(cur);
                }
                ElementKind::Rect(b) | ElementKind::Ellipse(b) => {
                    b.x = anchor[0].min(cur[0]);
                    b.y = anchor[1].min(cur[1]);
                    b.w = (cur[0] - anchor[0]).abs();
                    b.h = (cur[1] - anchor[1]).abs();
                }
                ElementKind::Line(s) | ElementKind::Arrow(s) => {
                    s.x2 = cur[0];
                    s.y2 = cur[1];
                }
            }
            cx.notify();
        } else if self.panning {
            let dx = f32::from(ev.position.x - self.last.x);
            let dy = f32::from(ev.position.y - self.last.y);
            self.last = ev.position;
            self.scene.camera.pan_by(dx, dy);
            self.dirty = true;
            cx.notify();
        }
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (dx, dy) = match ev.delta {
            ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
            ScrollDelta::Lines(p) => (p.x * LINE_PX, p.y * LINE_PX),
        };
        if ev.modifiers.platform || ev.modifiers.control {
            let (rx, ry) = self.relative(ev.position);
            let factor = (1.0 + dy * 0.0025).clamp(0.5, 2.0);
            self.scene.camera.zoom_about(rx, ry, factor);
        } else {
            self.scene.camera.pan_by(dx, dy);
        }
        self.dirty = true;
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (rx, ry) = self.relative(ev.position);
        self.scene.camera.zoom_about(rx, ry, 1.0 + ev.delta);
        self.dirty = true;
        cx.notify();
    }

    /// Canvas-relative position of a window-coords event point.
    fn relative(&self, p: Point<Pixels>) -> (f32, f32) {
        let o = self.bounds.get().origin;
        (f32::from(p.x - o.x), f32::from(p.y - o.y))
    }
}

/// Whether an in-progress element is big enough to keep (a click that doesn't
/// drag leaves nothing).
fn committable(kind: &ElementKind) -> bool {
    match kind {
        ElementKind::Draw(s) => s.points.len() >= 2,
        ElementKind::Rect(b) | ElementKind::Ellipse(b) => b.w > 1.0 || b.h > 1.0,
        ElementKind::Line(s) | ElementKind::Arrow(s) => {
            let (dx, dy) = (s.x2 - s.x1, s.y2 - s.y1);
            dx * dx + dy * dy > 4.0
        }
    }
}

/// World → absolute screen point at the current camera.
fn to_screen(wx: f32, wy: f32, cam: Camera, origin: Point<Pixels>) -> Point<Pixels> {
    let z = cam.zoom.max(MIN_ZOOM);
    point(
        px(f32::from(origin.x) + (wx - cam.x) * z),
        px(f32::from(origin.y) + (wy - cam.y) * z),
    )
}

/// Paint the board background + the world-space dot grid into `bounds`.
fn paint_board(bounds: Bounds<Pixels>, cam: Camera, bg: Hsla, grid: Hsla, window: &mut Window) {
    window.paint_quad(fill(bounds, bg));

    let z = cam.zoom.max(MIN_ZOOM);
    let mut step = GRID;
    while step * z < MIN_DOT_SPACING {
        step *= 4.0;
    }

    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y);
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let (left, top) = (cam.x, cam.y);
    let mut wx = (left / step).ceil() * step;
    while (wx - left) * z <= w {
        let sx = ox + (wx - left) * z;
        let mut wy = (top / step).ceil() * step;
        while (wy - top) * z <= h {
            let sy = oy + (wy - top) * z;
            window.paint_quad(fill(
                Bounds {
                    origin: point(px(sx - DOT / 2.0), px(sy - DOT / 2.0)),
                    size: size(px(DOT), px(DOT)),
                },
                grid,
            ));
            wy += step;
        }
        wx += step;
    }
}

/// Paint one element at the current camera.
fn paint_element(
    kind: &ElementKind,
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    window: &mut Window,
) {
    match kind {
        ElementKind::Draw(s) => paint_stroke(&s.points, s.width, cam, origin, ink, window),
        ElementKind::Rect(b) => paint_rect(b, cam, origin, ink, window),
        ElementKind::Ellipse(b) => paint_ellipse(b, cam, origin, ink, window),
        ElementKind::Line(s) => paint_segment(s, false, cam, origin, ink, window),
        ElementKind::Arrow(s) => paint_segment(s, true, cam, origin, ink, window),
    }
}

fn paint_stroke(
    points: &[[f32; 2]],
    world_w: f32,
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    window: &mut Window,
) {
    if points.len() < 2 {
        return;
    }
    let z = cam.zoom.max(MIN_ZOOM);
    let mut pb = PathBuilder::stroke(px((world_w * z).max(0.5)));
    pb.move_to(to_screen(points[0][0], points[0][1], cam, origin));
    for p in &points[1..] {
        pb.line_to(to_screen(p[0], p[1], cam, origin));
    }
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
}

fn paint_rect(b: &BoxGeom, cam: Camera, origin: Point<Pixels>, ink: Hsla, window: &mut Window) {
    let z = cam.zoom.max(MIN_ZOOM);
    let mut pb = PathBuilder::stroke(px((b.width * z).max(0.5)));
    pb.move_to(to_screen(b.x, b.y, cam, origin));
    pb.line_to(to_screen(b.x + b.w, b.y, cam, origin));
    pb.line_to(to_screen(b.x + b.w, b.y + b.h, cam, origin));
    pb.line_to(to_screen(b.x, b.y + b.h, cam, origin));
    pb.close();
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
}

fn paint_ellipse(b: &BoxGeom, cam: Camera, origin: Point<Pixels>, ink: Hsla, window: &mut Window) {
    let z = cam.zoom.max(MIN_ZOOM);
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let (rx, ry) = (b.w / 2.0, b.h / 2.0);
    // Kappa: control-point offset for a circle-from-4-cubics approximation.
    const K: f32 = 0.552_284_8;
    let (kx, ky) = (rx * K, ry * K);
    let s = |wx: f32, wy: f32| to_screen(wx, wy, cam, origin);
    let mut pb = PathBuilder::stroke(px((b.width * z).max(0.5)));
    pb.move_to(s(cx + rx, cy)); // right
    pb.cubic_bezier_to(s(cx, cy + ry), s(cx + rx, cy + ky), s(cx + kx, cy + ry)); // → bottom
    pb.cubic_bezier_to(s(cx - rx, cy), s(cx - kx, cy + ry), s(cx - rx, cy + ky)); // → left
    pb.cubic_bezier_to(s(cx, cy - ry), s(cx - rx, cy - ky), s(cx - kx, cy - ry)); // → top
    pb.cubic_bezier_to(s(cx + rx, cy), s(cx + kx, cy - ry), s(cx + rx, cy - ky)); // → right
    pb.close();
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
}

fn paint_segment(
    seg: &SegGeom,
    arrow: bool,
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    window: &mut Window,
) {
    let z = cam.zoom.max(MIN_ZOOM);
    let p1 = to_screen(seg.x1, seg.y1, cam, origin);
    let p2 = to_screen(seg.x2, seg.y2, cam, origin);
    let mut pb = PathBuilder::stroke(px((seg.width * z).max(0.5)));
    pb.move_to(p1);
    pb.line_to(p2);
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
    if !arrow {
        return;
    }
    // Filled arrowhead at p2 (screen space).
    let (dx, dy) = (f32::from(p2.x - p1.x), f32::from(p2.y - p1.y));
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (seg.width * z * 6.0).max(8.0);
    let (bx, by) = (f32::from(p2.x), f32::from(p2.y));
    // Barb = p2 + head * rotate((-u), ±angle).
    let barb = |a: f32| {
        let (c, s) = (a.cos(), a.sin());
        let rx = (-ux) * c - (-uy) * s;
        let ry = (-ux) * s + (-uy) * c;
        point(px(bx + head * rx), px(by + head * ry))
    };
    let mut hb = PathBuilder::fill();
    hb.move_to(p2);
    hb.line_to(barb(0.45));
    hb.line_to(barb(-0.45));
    hb.close();
    if let Ok(path) = hb.build() {
        window.paint_path(path, ink);
    }
}

impl Render for WhiteboardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = (self.style)();
        let WhiteboardStyle {
            bg,
            grid,
            text,
            ink,
            panel,
            accent,
        } = style;
        let cam = self.scene.camera;
        let bounds_cell = self.bounds.clone();
        // Snapshot element kinds (world coords) for the paint closure. Re-cloned
        // each frame for now; see the path-cache perf note at the top.
        let kinds: Vec<ElementKind> = self.scene.elements.iter().map(|e| e.kind.clone()).collect();
        let pending = self.pending.as_ref().map(|p| p.kind.clone());

        // Tool palette (top-center). The pill `occlude()`s so clicking a tool
        // doesn't also start a draw on the board beneath it.
        let active = self.tool;
        let mut buttons = Vec::with_capacity(Tool::ALL.len());
        for (i, &t) in Tool::ALL.iter().enumerate() {
            let mut b = div()
                .id(("wb-tool", i))
                .size(px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .text_size(px(15.0))
                .text_color(ink)
                .child(t.glyph())
                .on_click(cx.listener(move |this, _ev, _window, cx| this.set_tool(t, cx)));
            if t == active {
                b = b.bg(accent);
            }
            buttons.push(b);
        }
        let toolbar = div()
            .absolute()
            .top(px(10.0))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .gap(px(2.0))
                    .p(px(3.0))
                    .rounded(px(9.0))
                    .bg(panel)
                    .occlude()
                    .children(buttons),
            );

        div()
            .size_full()
            .relative()
            // Clip painting (grid + elements) to the board so panned content
            // never bleeds over the sidebar or tab bar.
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    move |bounds, _, window, _| {
                        paint_board(bounds, cam, bg, grid, window);
                        for k in &kinds {
                            paint_element(k, cam, bounds.origin, ink, window);
                        }
                        if let Some(k) = &pending {
                            paint_element(k, cam, bounds.origin, ink, window);
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            // Left = current tool; middle / scroll pan; pinch / ⌘-scroll zoom.
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_left_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_left_up))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_middle_down))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_middle_up))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
            .child(toolbar)
            .child(
                div()
                    .absolute()
                    .left(px(10.0))
                    .bottom(px(8.0))
                    .text_size(px(11.0))
                    .text_color(text)
                    .child(SharedString::from(format!("{:.0}%", cam.zoom * 100.0))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_garbage_loads_a_blank_board() {
        for s in ["", "   ", "not json", "{}", r#"{"camera":{"zoom":0}}"#] {
            let scene = Scene::from_json(s);
            assert_eq!(scene.camera.zoom, 1.0, "input {s:?}");
            assert!(scene.elements.is_empty(), "input {s:?}");
        }
    }

    #[test]
    fn camera_round_trips_through_json() {
        let scene = Scene {
            camera: Camera {
                x: 12.5,
                y: -4.0,
                zoom: 2.0,
            },
            ..Default::default()
        };
        let restored = Scene::from_json(&scene.to_json());
        assert_eq!(restored.camera.x, 12.5);
        assert_eq!(restored.camera.y, -4.0);
        assert_eq!(restored.camera.zoom, 2.0);
    }

    #[test]
    fn every_element_kind_round_trips_through_json() {
        let scene = Scene {
            camera: Camera::default(),
            elements: vec![
                Element {
                    id: 1,
                    kind: ElementKind::Draw(Stroke {
                        points: vec![[0.0, 0.0], [10.0, 5.0]],
                        width: 3.0,
                    }),
                },
                Element {
                    id: 2,
                    kind: ElementKind::Rect(BoxGeom {
                        x: 1.0,
                        y: 2.0,
                        w: 30.0,
                        h: 40.0,
                        width: 2.0,
                    }),
                },
                Element {
                    id: 3,
                    kind: ElementKind::Ellipse(BoxGeom {
                        x: 5.0,
                        y: 6.0,
                        w: 7.0,
                        h: 8.0,
                        width: 1.5,
                    }),
                },
                Element {
                    id: 4,
                    kind: ElementKind::Line(SegGeom {
                        x1: 0.0,
                        y1: 0.0,
                        x2: 9.0,
                        y2: 9.0,
                        width: 2.0,
                    }),
                },
                Element {
                    id: 5,
                    kind: ElementKind::Arrow(SegGeom {
                        x1: 1.0,
                        y1: 1.0,
                        x2: 2.0,
                        y2: 8.0,
                        width: 2.5,
                    }),
                },
            ],
        };
        let restored = Scene::from_json(&scene.to_json());
        assert_eq!(restored.elements.len(), 5);
        assert_eq!(restored.elements[1].id, 2);
        match &restored.elements[2].kind {
            ElementKind::Ellipse(b) => {
                assert_eq!(b.w, 7.0);
                assert_eq!(b.h, 8.0);
            }
            other => panic!("expected ellipse, got {other:?}"),
        }
        match &restored.elements[4].kind {
            ElementKind::Arrow(s) => assert_eq!(s.y2, 8.0),
            other => panic!("expected arrow, got {other:?}"),
        }
    }

    #[test]
    fn missing_camera_fields_fall_back_to_defaults() {
        let scene = Scene::from_json(r#"{"camera":{"zoom":3.0}}"#);
        assert_eq!(scene.camera.x, 0.0);
        assert_eq!(scene.camera.y, 0.0);
        assert_eq!(scene.camera.zoom, 3.0);
    }

    #[test]
    fn pan_moves_the_world_opposite_the_gesture() {
        let mut c = Camera::default();
        c.pan_by(50.0, -20.0);
        assert_eq!(c.x, -50.0);
        assert_eq!(c.y, 20.0);
    }

    #[test]
    fn pan_is_scaled_by_zoom() {
        let mut c = Camera {
            x: 0.0,
            y: 0.0,
            zoom: 2.0,
        };
        c.pan_by(50.0, 0.0);
        assert_eq!(c.x, -25.0);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor_fixed() {
        let mut c = Camera {
            x: 10.0,
            y: 5.0,
            zoom: 1.0,
        };
        let before = c.screen_to_world(300.0, 200.0);
        c.zoom_about(300.0, 200.0, 2.5);
        let after = c.screen_to_world(300.0, 200.0);
        assert!((before.0 - after.0).abs() < 1e-3, "{before:?} vs {after:?}");
        assert!((before.1 - after.1).abs() < 1e-3, "{before:?} vs {after:?}");
        assert_eq!(c.zoom, 2.5);
    }

    #[test]
    fn zoom_clamps_to_range() {
        let mut c = Camera::default();
        c.zoom_about(0.0, 0.0, 1000.0);
        assert_eq!(c.zoom, MAX_ZOOM);
        c.zoom_about(0.0, 0.0, 0.0001);
        assert_eq!(c.zoom, MIN_ZOOM);
    }

    #[test]
    fn tiny_drags_are_not_committed() {
        // A click (no real drag) leaves nothing on the board.
        assert!(!committable(&ElementKind::Draw(Stroke {
            points: vec![[0.0, 0.0]],
            width: 1.0,
        })));
        assert!(!committable(&ElementKind::Rect(BoxGeom {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            width: 1.0,
        })));
        assert!(committable(&ElementKind::Rect(BoxGeom {
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 5.0,
            width: 1.0,
        })));
    }
}
