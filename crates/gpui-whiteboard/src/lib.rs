//! An infinite, pannable/zoomable whiteboard canvas for GPUI.
//!
//! Host-agnostic — depends only on `gpui` + `serde`. Modeled on the
//! `gpui-pdf` stateful-entity pattern: the host builds a [`WhiteboardView`]
//! from a [`Scene`], stores the entity, and renders it in a tab. This crate
//! owns the scene model and its (de)serialization; the host owns persistence,
//! theme, and navigation.
//!
//! **Phase 2** (this version): freehand pen. A left-drag draws a [`Stroke`] in
//! world space (rendered with `PathBuilder::stroke`); pan moves to middle-drag
//! or scroll, zoom stays on pinch / ⌘-scroll, double-click resets. Strokes (and
//! the camera) persist via the host's [`WhiteboardView::set_on_change`] hook.
//! Shapes, arrows, selection, and embedded page-cards arrive in later phases —
//! see `docs/whiteboard-architecture.md` in the host repo.
//!
//! Perf note: committed strokes are re-tessellated each paint (as GPUI's own
//! `painting`/`brush` examples do). A built-`Path` cache + viewport culling is
//! the planned optimization once boards get large (Phase 6) — deferred so we
//! don't build it before there's something to measure.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, PinchEvent, Pixels, Point, Render,
    ScrollDelta, ScrollWheelEvent, SharedString, Styled, Window, canvas, div, fill, point, px,
    size,
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
/// Pen nib in screen px. A stroke's stored width is world-space (`NIB / zoom` at
/// draw time) so it feels like a constant nib yet scales with the content.
const NIB: f32 = 2.5;
/// Minimum on-screen gap between recorded stroke points (input thinning).
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

/// The kinds of thing a board can hold. (Phase 2: freehand strokes only;
/// shapes/arrows/text/embeds are added in later phases.)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Draw(Stroke),
}

/// A freehand pen stroke: world-space points and a world-space width.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
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
    /// Pen ink (stroke color). Per-stroke color comes with the color picker.
    pub ink: Hsla,
}

/// A `() -> WhiteboardStyle` the host supplies; called each paint so the board
/// tracks theme changes without the host pushing updates.
pub type WhiteboardStyleFn = Rc<dyn Fn() -> WhiteboardStyle>;

/// Called when the board changes (a stroke committed, the camera moved), with
/// the serialized scene JSON, so the host can persist it.
pub type ChangeFn = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// The whiteboard view entity. The host holds it in an `Entity<WhiteboardView>`
/// (keyed by board id) and renders it into a tab.
pub struct WhiteboardView {
    scene: Scene,
    style: WhiteboardStyleFn,
    on_change: Option<ChangeFn>,
    /// Canvas bounds in window coords, captured each paint so input handlers can
    /// map window-relative event positions into the board.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The in-progress freehand stroke (world points), while the left button is
    /// down and dragging.
    drawing: Option<Stroke>,
    /// True while a middle-drag pan is in progress.
    panning: bool,
    /// Last pointer position (window coords) during a pan.
    last: Point<Pixels>,
    /// Next element id.
    next_id: u64,
    /// Unsaved changes since the last `on_change` flush (flushed on mouse-up).
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
            bounds: Rc::new(Cell::new(Bounds::default())),
            drawing: None,
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
            self.drawing = None;
            self.reset_view(cx);
            return;
        }
        if self.panning {
            return;
        }
        let p = self.event_to_world(ev.position);
        let width = NIB / self.scene.camera.zoom.max(MIN_ZOOM);
        self.drawing = Some(Stroke {
            points: vec![p],
            width,
        });
        cx.notify();
    }

    fn on_left_up(&mut self, _ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(stroke) = self.drawing.take() {
            // A click (or double-click) leaves no mark — only an actual drag.
            if stroke.points.len() >= 2 {
                let id = self.next_id;
                self.next_id += 1;
                self.scene.elements.push(Element {
                    id,
                    kind: ElementKind::Draw(stroke),
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
        if self.drawing.is_some() {
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
        if self.drawing.is_some() {
            let p = self.event_to_world(ev.position);
            let z = self.scene.camera.zoom.max(MIN_ZOOM);
            let stroke = self.drawing.as_mut().unwrap();
            // Thin the input: skip points within MIN_POINT_PX (screen) of the last.
            if let Some(last) = stroke.points.last() {
                let (dx, dy) = ((p[0] - last[0]) * z, (p[1] - last[1]) * z);
                if dx * dx + dy * dy < MIN_POINT_PX * MIN_POINT_PX {
                    return;
                }
            }
            stroke.points.push(p);
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
        // Persisted on the next mouse-up (avoids a write per scroll tick).
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

/// Tessellate + paint one world-space stroke at the current camera.
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
    let (ox, oy) = (f32::from(origin.x), f32::from(origin.y));
    let to_screen = |p: &[f32; 2]| point(px(ox + (p[0] - cam.x) * z), px(oy + (p[1] - cam.y) * z));
    let mut pb = PathBuilder::stroke(px((world_w * z).max(0.5)));
    pb.move_to(to_screen(&points[0]));
    for p in &points[1..] {
        pb.line_to(to_screen(p));
    }
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
}

impl Render for WhiteboardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = (self.style)();
        let (bg, grid, text, ink) = (style.bg, style.grid, style.text, style.ink);
        let cam = self.scene.camera;
        let bounds_cell = self.bounds.clone();
        // Snapshot the strokes (world coords) for the paint closure. Re-cloned
        // each frame for now; see the path-cache perf note at the top.
        let strokes: Vec<(Vec<[f32; 2]>, f32)> = self
            .scene
            .elements
            .iter()
            .map(|e| match &e.kind {
                ElementKind::Draw(s) => (s.points.clone(), s.width),
            })
            .collect();
        let inprog = self.drawing.as_ref().map(|s| (s.points.clone(), s.width));

        div()
            .size_full()
            .relative()
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    move |bounds, _, window, _| {
                        paint_board(bounds, cam, bg, grid, window);
                        for (pts, w) in &strokes {
                            paint_stroke(pts, *w, cam, bounds.origin, ink, window);
                        }
                        if let Some((pts, w)) = &inprog {
                            paint_stroke(pts, *w, cam, bounds.origin, ink, window);
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            // Left draws; middle / scroll pan; pinch / ⌘-scroll zoom. Plain
            // (non-interactive) children below don't block these.
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_left_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_left_up))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_middle_down))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_middle_up))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
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
    fn strokes_round_trip_through_json() {
        let scene = Scene {
            camera: Camera::default(),
            elements: vec![Element {
                id: 7,
                kind: ElementKind::Draw(Stroke {
                    points: vec![[0.0, 0.0], [10.0, 5.0], [20.0, -3.0]],
                    width: 3.0,
                }),
            }],
        };
        let restored = Scene::from_json(&scene.to_json());
        assert_eq!(restored.elements.len(), 1);
        assert_eq!(restored.elements[0].id, 7);
        let ElementKind::Draw(s) = &restored.elements[0].kind;
        assert_eq!(s.points, vec![[0.0, 0.0], [10.0, 5.0], [20.0, -3.0]]);
        assert_eq!(s.width, 3.0);
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
}
