//! An infinite, pannable/zoomable whiteboard canvas for GPUI.
//!
//! Host-agnostic — depends only on `gpui` + `serde`. Modeled on the
//! `gpui-pdf` stateful-entity pattern: the host builds a [`WhiteboardView`]
//! from a [`Scene`], stores the entity, and renders it in a tab. This crate
//! owns the scene model and its (de)serialization; the host owns persistence,
//! theme, and navigation.
//!
//! **Phase 4** (this version): selection & manipulation. A **Select** tool
//! (click to select, drag to move) joins the pen and shape tools, with a
//! selection outline and toolbar **delete / undo / redo**. Earlier phases:
//! pan/zoom + grid, freehand pen, and rect/ellipse/line/arrow. Resize/rotate,
//! marquee + multi-select, text, and embedded page-cards come next — see
//! `docs/whiteboard-architecture.md` in the host repo.
//!
//! Perf note: elements are re-tessellated each paint (as GPUI's own
//! `painting`/`brush` examples do). A built-`Path` cache + viewport culling is
//! the planned optimization once boards get large (Phase 6) — deferred so we
//! don't build it before there's something to measure.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, BorderStyle, Bounds, Context, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, PinchEvent, Pixels,
    Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled,
    Window, canvas, div, fill, outline, point, px, size,
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
/// Hit-test tolerance around an element's bounds, in screen px.
const SELECT_PAD: f32 = 6.0;
/// Most undo steps kept (bounds memory; each step is a scene snapshot).
const UNDO_CAP: usize = 50;

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
    /// The world point under a canvas-relative screen point.
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

/// The active tool. UI state — not part of the persisted scene.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Pen,
    Rect,
    Ellipse,
    Line,
    Arrow,
}

impl Tool {
    const ALL: [Tool; 6] = [
        Tool::Select,
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
            Tool::Select => "↖",
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
    /// HUD / muted on-canvas text.
    pub text: Hsla,
    /// Ink (stroke/shape color). Per-element color comes with the color picker.
    pub ink: Hsla,
    /// Toolbar panel background.
    pub panel: Hsla,
    /// Active-tool highlight + selection outline.
    pub accent: Hsla,
}

/// A `() -> WhiteboardStyle` the host supplies; called each paint so the board
/// tracks theme changes without the host pushing updates.
pub type WhiteboardStyleFn = Rc<dyn Fn() -> WhiteboardStyle>;

/// Called when the board changes (an element committed/moved/deleted, the camera
/// moved), with the serialized scene JSON, so the host can persist it.
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
    /// The currently selected element (Select tool).
    selected: Option<u64>,
    /// The last world point of an in-progress move-drag, if any.
    drag_from: Option<[f32; 2]>,
    /// Whether the current move-drag has actually moved (undo is pushed once).
    moved: bool,
    /// Undo / redo stacks of scene snapshots.
    history: Vec<Scene>,
    redo: Vec<Scene>,
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
            selected: None,
            drag_from: None,
            moved: false,
            history: Vec::new(),
            redo: Vec::new(),
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

    /// Switch the active drawing tool. Leaving Select clears the selection.
    pub fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if self.tool != tool {
            self.tool = tool;
            if tool != Tool::Select {
                self.selected = None;
            }
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

    /// Snapshot the scene for undo (before a mutation), capping history.
    fn push_undo(&mut self) {
        self.history.push(self.scene.clone());
        if self.history.len() > UNDO_CAP {
            self.history.remove(0);
        }
        self.redo.clear();
    }

    /// Revert the last change.
    pub fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(prev) = self.history.pop() {
            self.redo.push(std::mem::replace(&mut self.scene, prev));
            self.selected = None;
            self.dirty = true;
            cx.notify();
            self.flush(window, cx);
        }
    }

    /// Re-apply the last undone change.
    pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.redo.pop() {
            self.history.push(std::mem::replace(&mut self.scene, next));
            self.selected = None;
            self.dirty = true;
            cx.notify();
            self.flush(window, cx);
        }
    }

    /// Delete the selected element.
    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.take() {
            self.push_undo();
            self.scene.elements.retain(|e| e.id != id);
            self.dirty = true;
            cx.notify();
            self.flush(window, cx);
        }
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
        let (rx, ry) = self.relative(p);
        let (wx, wy) = self.scene.camera.screen_to_world(rx, ry);
        [wx, wy]
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

        if self.tool == Tool::Select {
            // Hit-test topmost-first; select + arm a move, or deselect.
            let pad = SELECT_PAD / self.scene.camera.zoom.max(MIN_ZOOM);
            self.selected = self
                .scene
                .elements
                .iter()
                .rev()
                .find(|e| hit_test(&e.kind, p[0], p[1], pad))
                .map(|e| e.id);
            self.drag_from = self.selected.map(|_| p);
            self.moved = false;
            cx.notify();
            return;
        }

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
            Tool::Select => return,
        };
        self.pending = Some(Pending { anchor: p, kind });
        cx.notify();
    }

    fn on_left_up(&mut self, _ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.drag_from.take().is_some() {
            if self.moved {
                self.dirty = true;
            }
            self.moved = false;
            cx.notify();
            self.flush(window, cx);
            return;
        }
        if let Some(pending) = self.pending.take() {
            if committable(&pending.kind) {
                self.push_undo();
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
        if self.pending.is_some() || self.drag_from.is_some() {
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
        // Moving the selection.
        if let Some(from) = self.drag_from {
            let cur = self.event_to_world(ev.position);
            let (dx, dy) = (cur[0] - from[0], cur[1] - from[1]);
            if dx != 0.0 || dy != 0.0 {
                if !self.moved {
                    self.push_undo();
                    self.moved = true;
                }
                if let Some(id) = self.selected
                    && let Some(e) = self.scene.elements.iter_mut().find(|e| e.id == id)
                {
                    translate(&mut e.kind, dx, dy);
                }
                self.drag_from = Some(cur);
                cx.notify();
            }
            return;
        }
        // Creating an element.
        if self.pending.is_some() {
            let cur = self.event_to_world(ev.position);
            let z = self.scene.camera.zoom.max(MIN_ZOOM);
            let anchor = self.pending.as_ref().unwrap().anchor;
            let pending = self.pending.as_mut().unwrap();
            match &mut pending.kind {
                ElementKind::Draw(s) => {
                    if let Some(last) = s.points.last() {
                        let (ddx, ddy) = ((cur[0] - last[0]) * z, (cur[1] - last[1]) * z);
                        if ddx * ddx + ddy * ddy < MIN_POINT_PX * MIN_POINT_PX {
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
            return;
        }
        // Panning.
        if self.panning {
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

/// An element's world-space bounding box `(min_x, min_y, max_x, max_y)`.
fn bbox(kind: &ElementKind) -> (f32, f32, f32, f32) {
    match kind {
        ElementKind::Draw(s) => {
            let mut pts = s.points.iter();
            let f = pts.next().copied().unwrap_or([0.0, 0.0]);
            let (mut x0, mut y0, mut x1, mut y1) = (f[0], f[1], f[0], f[1]);
            for p in pts {
                x0 = x0.min(p[0]);
                y0 = y0.min(p[1]);
                x1 = x1.max(p[0]);
                y1 = y1.max(p[1]);
            }
            (x0, y0, x1, y1)
        }
        ElementKind::Rect(b) | ElementKind::Ellipse(b) => (b.x, b.y, b.x + b.w, b.y + b.h),
        ElementKind::Line(s) | ElementKind::Arrow(s) => (
            s.x1.min(s.x2),
            s.y1.min(s.y2),
            s.x1.max(s.x2),
            s.y1.max(s.y2),
        ),
    }
}

/// Whether `(wx, wy)` falls within an element's bounds, padded by `pad` (world).
fn hit_test(kind: &ElementKind, wx: f32, wy: f32, pad: f32) -> bool {
    let (x0, y0, x1, y1) = bbox(kind);
    wx >= x0 - pad && wx <= x1 + pad && wy >= y0 - pad && wy <= y1 + pad
}

/// Translate an element by a world-space delta.
fn translate(kind: &mut ElementKind, dx: f32, dy: f32) {
    match kind {
        ElementKind::Draw(s) => {
            for p in &mut s.points {
                p[0] += dx;
                p[1] += dy;
            }
        }
        ElementKind::Rect(b) | ElementKind::Ellipse(b) => {
            b.x += dx;
            b.y += dy;
        }
        ElementKind::Line(s) | ElementKind::Arrow(s) => {
            s.x1 += dx;
            s.x2 += dx;
            s.y1 += dy;
            s.y2 += dy;
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
    const K: f32 = 0.552_284_8;
    let (kx, ky) = (rx * K, ry * K);
    let s = |wx: f32, wy: f32| to_screen(wx, wy, cam, origin);
    let mut pb = PathBuilder::stroke(px((b.width * z).max(0.5)));
    pb.move_to(s(cx + rx, cy));
    pb.cubic_bezier_to(s(cx, cy + ry), s(cx + rx, cy + ky), s(cx + kx, cy + ry));
    pb.cubic_bezier_to(s(cx - rx, cy), s(cx - kx, cy + ry), s(cx - rx, cy + ky));
    pb.cubic_bezier_to(s(cx, cy - ry), s(cx - rx, cy - ky), s(cx - kx, cy - ry));
    pb.cubic_bezier_to(s(cx + rx, cy), s(cx + kx, cy - ry), s(cx + rx, cy - ky));
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
    let (dx, dy) = (f32::from(p2.x - p1.x), f32::from(p2.y - p1.y));
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (seg.width * z * 6.0).max(8.0);
    let (bx, by) = (f32::from(p2.x), f32::from(p2.y));
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

/// Paint the selection outline (a padded box around the selected element).
fn paint_selection(
    bb: (f32, f32, f32, f32),
    cam: Camera,
    origin: Point<Pixels>,
    accent: Hsla,
    window: &mut Window,
) {
    let tl = to_screen(bb.0, bb.1, cam, origin);
    let br = to_screen(bb.2, bb.3, cam, origin);
    let m = 4.0;
    let (ox, oy) = (f32::from(tl.x) - m, f32::from(tl.y) - m);
    let (w, h) = (
        f32::from(br.x) - f32::from(tl.x) + 2.0 * m,
        f32::from(br.y) - f32::from(tl.y) + 2.0 * m,
    );
    window.paint_quad(outline(
        Bounds {
            origin: point(px(ox), px(oy)),
            size: size(px(w), px(h)),
        },
        accent,
        BorderStyle::Dashed,
    ));
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
        let sel = self
            .selected
            .and_then(|id| self.scene.elements.iter().find(|e| e.id == id))
            .map(|e| bbox(&e.kind));

        // Tool palette + actions (top-center). The pill `occlude()`s so clicking
        // a button doesn't also act on the board beneath it.
        let active = self.tool;
        let mut tools = Vec::with_capacity(Tool::ALL.len());
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
            tools.push(b);
        }
        let act = |id: usize, glyph: &'static str| {
            div()
                .id(("wb-act", id))
                .size(px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .text_size(px(15.0))
                .text_color(ink)
                .child(glyph)
        };
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
                    .items_center()
                    .gap(px(2.0))
                    .p(px(3.0))
                    .rounded(px(9.0))
                    .bg(panel)
                    .occlude()
                    .children(tools)
                    .child(div().w(px(7.0)))
                    .child(
                        act(0, "↶")
                            .on_click(cx.listener(|this, _ev, window, cx| this.undo(window, cx))),
                    )
                    .child(
                        act(1, "↷")
                            .on_click(cx.listener(|this, _ev, window, cx| this.redo(window, cx))),
                    )
                    .child(act(2, "⌫").on_click(
                        cx.listener(|this, _ev, window, cx| this.delete_selected(window, cx)),
                    )),
            );

        div()
            .size_full()
            .relative()
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
                        if let Some(bb) = sel {
                            paint_selection(bb, cam, bounds.origin, accent, window);
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
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
        assert_eq!(restored.elements.len(), 3);
        match &restored.elements[2].kind {
            ElementKind::Arrow(s) => assert_eq!(s.y2, 8.0),
            other => panic!("expected arrow, got {other:?}"),
        }
    }

    #[test]
    fn pan_and_zoom_math() {
        let mut c = Camera::default();
        c.pan_by(50.0, -20.0);
        assert_eq!((c.x, c.y), (-50.0, 20.0));

        let mut c = Camera {
            x: 10.0,
            y: 5.0,
            zoom: 1.0,
        };
        let before = c.screen_to_world(300.0, 200.0);
        c.zoom_about(300.0, 200.0, 2.5);
        let after = c.screen_to_world(300.0, 200.0);
        assert!((before.0 - after.0).abs() < 1e-3);
        assert!((before.1 - after.1).abs() < 1e-3);
        assert_eq!(c.zoom, 2.5);
    }

    #[test]
    fn bbox_translate_and_hit_test() {
        let mut k = ElementKind::Line(SegGeom {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 4.0,
            width: 1.0,
        });
        assert_eq!(bbox(&k), (0.0, 0.0, 10.0, 4.0));
        translate(&mut k, 5.0, -2.0);
        assert_eq!(bbox(&k), (5.0, -2.0, 15.0, 2.0));
        // Within the padded bounds hits; far away misses.
        assert!(hit_test(&k, 5.0, -2.0, 1.0));
        assert!(hit_test(&k, 4.5, -2.5, 1.0)); // inside pad
        assert!(!hit_test(&k, 100.0, 100.0, 1.0));
    }

    #[test]
    fn tiny_drags_are_not_committed() {
        assert!(!committable(&ElementKind::Draw(Stroke {
            points: vec![[0.0, 0.0]],
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
