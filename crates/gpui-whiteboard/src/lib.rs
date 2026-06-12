//! An infinite, pannable/zoomable whiteboard canvas for GPUI.
//!
//! Host-agnostic — depends only on `gpui` + `serde`. Modeled on the
//! `gpui-pdf` stateful-entity pattern: the host builds a [`WhiteboardView`]
//! from a [`Scene`], stores the entity, and renders it in a tab. This crate
//! owns the scene model and its (de)serialization; the host owns persistence,
//! theme, and navigation.
//!
//! **Phase 5** (this version): on-canvas **text** — a Text tool (click to place,
//! type to edit, double-click to re-edit), rendered with real glyph layout that
//! scales with zoom and persists. Plus resize polish: corner handles track the
//! cursor (grab offset + diagonal-projection proportional resize), and text is
//! measured so its box fits the glyphs. Earlier phases: pan/zoom + grid, freehand
//! pen, rect/ellipse/line/arrow, and select / move / resize / delete / undo.
//! Rotation, marquee + multi-select, and embedded page-cards come next — see
//! `docs/whiteboard-architecture.md` in the host repo.
//!
//! Perf note: elements are re-tessellated each paint (as GPUI's own
//! `painting`/`brush` examples do). A built-`Path` cache + viewport culling is
//! the planned optimization once boards get large (Phase 6) — deferred so we
//! don't build it before there's something to measure.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, FocusHandle, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder,
    PinchEvent, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement, Styled, Window, canvas, div, fill, point, px, size,
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
/// Selection-box padding around the element, screen px (handles sit on it).
const SEL_PAD_PX: f32 = 5.0;
/// Half-size of a corner resize handle, screen px.
const HANDLE_HALF: f32 = 4.5;
/// Grab radius for a corner handle, screen px.
const HANDLE_GRAB: f32 = 10.0;
/// Default text size at creation, screen px (stored world size is this / zoom).
const TEXT_SIZE: f32 = 18.0;
/// Rough per-character advance and line height, as fractions of the font size,
/// for an approximate text bounding box (hit-testing / selection).
const TEXT_CHAR_W: f32 = 0.55;
const TEXT_LINE_H: f32 = 1.3;

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
    Text(TextGeom),
}

/// A text label: a top-left anchor, its content, and a world-space font size.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextGeom {
    pub x: f32,
    pub y: f32,
    pub content: String,
    pub size: f32,
    /// Cached world-space width, measured each render so the selection box and
    /// hit-test fit the real glyphs. Not persisted; `0.0` means unmeasured.
    #[serde(skip)]
    pub measured_w: f32,
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
    Text,
}

impl Tool {
    const ALL: [Tool; 7] = [
        Tool::Select,
        Tool::Pen,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Arrow,
        Tool::Text,
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
            Tool::Text => "T",
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
    /// Active-tool highlight (a subtle fill behind the current tool button).
    pub accent: Hsla,
    /// Selection outline — wants to be clearly visible, so a strong color.
    pub selection: Hsla,
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

/// An in-progress corner-resize of a selected box/stroke.
struct Resizing {
    id: u64,
    /// The fixed (opposite) corner, world space.
    anchor: [f32; 2],
    /// The dragged corner's original position, world space.
    from: [f32; 2],
    /// World offset from the cursor to the dragged corner at grab time, kept so
    /// the corner tracks the cursor 1:1 (no jump on grab).
    grab: [f32; 2],
    /// The element's geometry at the start of the resize.
    orig: ElementKind,
}

/// An in-progress drag of one endpoint of a selected line/arrow.
#[derive(Clone, Copy)]
struct EndpointDrag {
    id: u64,
    /// Which endpoint: 0 = (x1,y1), 1 = (x2,y2).
    which: usize,
}

/// What a press on a selection handle begins.
enum HandleGrab {
    Corner(Resizing),
    Endpoint(EndpointDrag),
}

/// The whiteboard view entity. The host holds it in an `Entity<WhiteboardView>`
/// (keyed by board id) and renders it into a tab.
pub struct WhiteboardView {
    scene: Scene,
    style: WhiteboardStyleFn,
    on_change: Option<ChangeFn>,
    tool: Tool,
    /// Keyboard focus — grabbed while editing a text element.
    focus: FocusHandle,
    /// The text element currently being edited (Text tool / double-click).
    editing: Option<u64>,
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
    /// In-progress corner-resize of the selected box/stroke.
    resizing: Option<Resizing>,
    /// In-progress endpoint-drag of the selected line/arrow.
    endpoint: Option<EndpointDrag>,
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
    pub fn new(scene: Scene, style: WhiteboardStyleFn, cx: &mut Context<Self>) -> Self {
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
            focus: cx.focus_handle(),
            editing: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
            pending: None,
            selected: None,
            drag_from: None,
            moved: false,
            resizing: None,
            endpoint: None,
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

    /// If `pos` (window coords) is on a manipulation handle of the current
    /// selection, what to begin. Lines/arrows manipulate by their two
    /// endpoints; everything else by its bounding-box corners (a line's bbox is
    /// degenerate, which would make corner-resize wildly imprecise).
    fn handle_hit(&self, pos: Point<Pixels>) -> Option<HandleGrab> {
        let id = self.selected?;
        let kind = &self.scene.elements.iter().find(|e| e.id == id)?.kind;
        let cam = self.scene.camera;
        let origin = self.bounds.get().origin;
        let cursor = self.event_to_world(pos);
        let near = |wx: f32, wy: f32, ox: f32, oy: f32| {
            let s = to_screen(wx, wy, cam, origin);
            let (dx, dy) = (
                f32::from(pos.x) - (f32::from(s.x) + ox),
                f32::from(pos.y) - (f32::from(s.y) + oy),
            );
            dx * dx + dy * dy <= HANDLE_GRAB * HANDLE_GRAB
        };

        if let ElementKind::Line(s) | ElementKind::Arrow(s) = kind {
            for (which, (wx, wy)) in [(s.x1, s.y1), (s.x2, s.y2)].into_iter().enumerate() {
                if near(wx, wy, 0.0, 0.0) {
                    return Some(HandleGrab::Endpoint(EndpointDrag { id, which }));
                }
            }
            return None;
        }

        // Corner handles sit on the padded box, so offset the hit point to match.
        let bb = bbox(kind);
        let wc = [(bb.0, bb.1), (bb.2, bb.1), (bb.0, bb.3), (bb.2, bb.3)];
        let off = [
            (-SEL_PAD_PX, -SEL_PAD_PX),
            (SEL_PAD_PX, -SEL_PAD_PX),
            (-SEL_PAD_PX, SEL_PAD_PX),
            (SEL_PAD_PX, SEL_PAD_PX),
        ];
        for i in 0..4 {
            if near(wc[i].0, wc[i].1, off[i].0, off[i].1) {
                let opp = wc[3 - i];
                return Some(HandleGrab::Corner(Resizing {
                    id,
                    anchor: [opp.0, opp.1],
                    from: [wc[i].0, wc[i].1],
                    grab: [wc[i].0 - cursor[0], wc[i].1 - cursor[1]],
                    orig: kind.clone(),
                }));
            }
        }
        None
    }

    /// The topmost text element under a world point (within `pad`), if any.
    fn text_at(&self, p: [f32; 2], pad: f32) -> Option<u64> {
        self.scene
            .elements
            .iter()
            .rev()
            .find(|e| matches!(e.kind, ElementKind::Text(_)) && hit_test(&e.kind, p[0], p[1], pad))
            .map(|e| e.id)
    }

    fn on_left_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.panning {
            return;
        }
        let p = self.event_to_world(ev.position);
        let zoom = self.scene.camera.zoom.max(MIN_ZOOM);

        // Any click first commits an in-progress text edit.
        if self.editing.is_some() {
            self.commit_text(window, cx);
        }

        if ev.click_count >= 2 {
            self.pending = None;
            // Double-click a text element (Select tool) re-opens it for editing.
            if self.tool == Tool::Select
                && let Some(id) = self.text_at(p, SELECT_PAD / zoom)
            {
                self.selected = Some(id);
                self.editing = Some(id);
                self.focus.focus(window, cx);
                cx.notify();
                return;
            }
            self.reset_view(cx);
            return;
        }

        if self.tool == Tool::Text {
            // Edit a text under the cursor, else create a new one here.
            if let Some(id) = self.text_at(p, SELECT_PAD / zoom) {
                self.selected = Some(id);
                self.editing = Some(id);
            } else {
                self.push_undo();
                let id = self.next_id;
                self.next_id += 1;
                self.scene.elements.push(Element {
                    id,
                    kind: ElementKind::Text(TextGeom {
                        x: p[0],
                        y: p[1],
                        content: String::new(),
                        size: TEXT_SIZE / zoom,
                        measured_w: 0.0,
                    }),
                });
                self.selected = Some(id);
                self.editing = Some(id);
                self.dirty = true;
            }
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }

        if self.tool == Tool::Select {
            // A handle on the current selection takes priority.
            if let Some(grab) = self.handle_hit(ev.position) {
                self.push_undo();
                match grab {
                    HandleGrab::Corner(rs) => self.resizing = Some(rs),
                    HandleGrab::Endpoint(ep) => self.endpoint = Some(ep),
                }
                cx.notify();
                return;
            }
            // Otherwise hit-test topmost-first; select + arm a move, or deselect.
            let pad = SELECT_PAD / zoom;
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

        let width = NIB / zoom;
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
            Tool::Select | Tool::Text => return,
        };
        self.pending = Some(Pending { anchor: p, kind });
        cx.notify();
    }

    fn on_left_up(&mut self, _ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.resizing.take().is_some() || self.endpoint.take().is_some() {
            self.dirty = true;
            cx.notify();
            self.flush(window, cx);
            return;
        }
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
        if self.pending.is_some()
            || self.drag_from.is_some()
            || self.resizing.is_some()
            || self.endpoint.is_some()
        {
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

    fn on_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Resizing the selection (corner-handle drag).
        if self.resizing.is_some() {
            let cur = self.event_to_world(ev.position);
            let (id, anchor, from, grab, mut kind) = {
                let r = self.resizing.as_ref().unwrap();
                (r.id, r.anchor, r.from, r.grab, r.orig.clone())
            };
            // Where the dragged corner should sit: cursor + the grab offset, so
            // it tracks the cursor without jumping when the drag starts.
            let target = [cur[0] + grab[0], cur[1] + grab[1]];
            // Text always scales proportionally (its size is a single font size);
            // Shift does so for shapes. Both use the diagonal projection so the
            // corner tracks the cursor at the right rate. Free resize is per-axis.
            let proportional = ev.modifiers.shift || matches!(kind, ElementKind::Text(_));
            let (sx, sy) = if proportional {
                let s = diagonal_scale(anchor, from, target);
                (s, s)
            } else {
                let sx = if (from[0] - anchor[0]).abs() > 1e-3 {
                    (target[0] - anchor[0]) / (from[0] - anchor[0])
                } else {
                    1.0
                };
                let sy = if (from[1] - anchor[1]).abs() > 1e-3 {
                    (target[1] - anchor[1]) / (from[1] - anchor[1])
                } else {
                    1.0
                };
                (sx, sy)
            };
            resize_about(&mut kind, anchor[0], anchor[1], sx, sy);
            let zoom = self.scene.camera.zoom.max(MIN_ZOOM);
            if let Some(e) = self.scene.elements.iter_mut().find(|e| e.id == id) {
                e.kind = kind;
                // Re-measure text now so its box tracks the cursor this frame.
                if let ElementKind::Text(t) = &mut e.kind {
                    t.measured_w = measure_text_width(&t.content, px(t.size * zoom), window) / zoom;
                }
            }
            cx.notify();
            return;
        }
        // Dragging a line/arrow endpoint (Shift snaps the angle to 45°).
        if let Some(ep) = self.endpoint {
            let cur = self.event_to_world(ev.position);
            let shift = ev.modifiers.shift;
            if let Some(e) = self.scene.elements.iter_mut().find(|e| e.id == ep.id)
                && let ElementKind::Line(s) | ElementKind::Arrow(s) = &mut e.kind
            {
                let (ox, oy) = if ep.which == 0 {
                    (s.x2, s.y2)
                } else {
                    (s.x1, s.y1)
                };
                let (nx, ny) = if shift {
                    snap_45(ox, oy, cur[0], cur[1])
                } else {
                    (cur[0], cur[1])
                };
                if ep.which == 0 {
                    s.x1 = nx;
                    s.y1 = ny;
                } else {
                    s.x2 = nx;
                    s.y2 = ny;
                }
            }
            cx.notify();
            return;
        }
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
                // Text isn't created by dragging, so it's never pending here.
                ElementKind::Text(_) => {}
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

    /// Finish editing the current text element, dropping it if it's empty.
    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.editing.take() else {
            return;
        };
        let empty =
            self.scene.elements.iter().find(|e| e.id == id).is_none_or(
                |e| matches!(&e.kind, ElementKind::Text(t) if t.content.trim().is_empty()),
            );
        if empty {
            self.scene.elements.retain(|e| e.id != id);
        }
        self.dirty = true;
        cx.notify();
        self.flush(window, cx);
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.editing else {
            cx.propagate();
            return;
        };
        let ks = &ev.keystroke;
        let cmd = ks.modifiers.platform || ks.modifiers.control;
        let res = if let Some(e) = self.scene.elements.iter_mut().find(|e| e.id == id)
            && let ElementKind::Text(t) = &mut e.kind
        {
            text_key(&mut t.content, &ks.key, ks.key_char.as_deref(), cmd)
        } else {
            KeyResult::Commit
        };
        match res {
            KeyResult::Edited => {
                self.dirty = true;
                cx.notify();
            }
            KeyResult::Commit => self.commit_text(window, cx),
            KeyResult::Pass => cx.propagate(),
        }
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
        // Text is created on click (not via a drag), so it's never pending.
        ElementKind::Text(_) => false,
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
        ElementKind::Text(t) => {
            let (w, h) = text_extent(t);
            (t.x, t.y, t.x + w, t.y + h)
        }
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
        ElementKind::Text(t) => {
            t.x += dx;
            t.y += dy;
        }
    }
}

/// The proportional-resize scale: project the (offset) cursor onto the diagonal
/// from `anchor` through the dragged corner `from`, so the corner stays on that
/// diagonal and tracks the cursor's projection. Keeps the aspect ratio *and*
/// scales at the cursor's rate (not the faster max-of-axes rate).
fn diagonal_scale(anchor: [f32; 2], from: [f32; 2], target: [f32; 2]) -> f32 {
    let d = [from[0] - anchor[0], from[1] - anchor[1]];
    let c = [target[0] - anchor[0], target[1] - anchor[1]];
    let dd = d[0] * d[0] + d[1] * d[1];
    if dd < 1e-6 {
        return 1.0;
    }
    (c[0] * d[0] + c[1] * d[1]) / dd
}

/// The rendered world-independent width (px) of the widest line of `content` at
/// `font_size`, via the real glyph layout.
fn measure_text_width(content: &str, font_size: Pixels, window: &mut Window) -> f32 {
    content
        .split('\n')
        .map(|line| {
            let run = window.text_style().to_run(line.len());
            f32::from(
                window
                    .text_system()
                    .layout_line(line, font_size, &[run], None)
                    .width,
            )
        })
        .fold(0.0_f32, f32::max)
}

/// Snap target `(tx, ty)` so its angle from `(ox, oy)` is a multiple of 45°,
/// preserving the distance (the line-drawing constraint for endpoint drags).
fn snap_45(ox: f32, oy: f32, tx: f32, ty: f32) -> (f32, f32) {
    let (dx, dy) = (tx - ox, ty - oy);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-3 {
        return (tx, ty);
    }
    let step = std::f32::consts::FRAC_PI_4;
    let ang = (dy.atan2(dx) / step).round() * step;
    (ox + len * ang.cos(), oy + len * ang.sin())
}

/// The outcome of a key press while editing text.
enum KeyResult {
    Edited,
    Commit,
    Pass,
}

/// Apply one key press to a text buffer. Basic editing only: printable input
/// (incl. space), Enter (newline), and Backspace; Escape commits; modified
/// chords pass through. No IME composition or mid-string cursor yet.
fn text_key(content: &mut String, key: &str, key_char: Option<&str>, cmd: bool) -> KeyResult {
    if cmd {
        return KeyResult::Pass;
    }
    match key {
        "escape" => KeyResult::Commit,
        "enter" => {
            content.push('\n');
            KeyResult::Edited
        }
        "backspace" => {
            content.pop();
            KeyResult::Edited
        }
        _ => match key_char {
            Some(c) if c.chars().next().is_some_and(|ch| !ch.is_control()) => {
                content.push_str(c);
                KeyResult::Edited
            }
            _ => KeyResult::Pass,
        },
    }
}

/// Approximate world-space (width, height) of a text element — enough for
/// hit-testing and the selection box (real shaping happens at paint time).
fn text_extent(t: &TextGeom) -> (f32, f32) {
    let rows = t.content.split('\n').count().max(1) as f32;
    let h = rows * t.size * TEXT_LINE_H;
    // Prefer the measured width; fall back to a character-count estimate until
    // the first render measures it.
    let w = if t.measured_w > 0.0 {
        t.measured_w
    } else {
        let cols = t
            .content
            .split('\n')
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0)
            .max(1) as f32;
        cols * t.size * TEXT_CHAR_W
    };
    (w, h)
}

/// Scale an element's geometry about `(ax, ay)` by `(sx, sy)` (world space).
/// Stroke width is left unchanged.
fn resize_about(kind: &mut ElementKind, ax: f32, ay: f32, sx: f32, sy: f32) {
    let fx = |x: f32| ax + (x - ax) * sx;
    let fy = |y: f32| ay + (y - ay) * sy;
    match kind {
        ElementKind::Draw(s) => {
            for p in &mut s.points {
                p[0] = fx(p[0]);
                p[1] = fy(p[1]);
            }
        }
        ElementKind::Rect(b) | ElementKind::Ellipse(b) => {
            let (x0, x1) = (fx(b.x), fx(b.x + b.w));
            let (y0, y1) = (fy(b.y), fy(b.y + b.h));
            b.x = x0.min(x1);
            b.w = (x1 - x0).abs();
            b.y = y0.min(y1);
            b.h = (y1 - y0).abs();
        }
        ElementKind::Line(s) | ElementKind::Arrow(s) => {
            s.x1 = fx(s.x1);
            s.x2 = fx(s.x2);
            s.y1 = fy(s.y1);
            s.y2 = fy(s.y2);
        }
        ElementKind::Text(t) => {
            // Callers pass a uniform factor for text (sx == sy), so position
            // scales uniformly and the font size by that magnitude.
            t.x = fx(t.x);
            t.y = fy(t.y);
            t.size = (t.size * sx.abs()).max(0.5);
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
        // Text is drawn as an overlay element in render(), not in the canvas.
        ElementKind::Text(_) => {}
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

/// A solid accent square handle centered at a screen point.
fn draw_handle(hx: f32, hy: f32, color: Hsla, window: &mut Window) {
    let h = HANDLE_HALF;
    window.paint_quad(fill(
        Bounds {
            origin: point(px(hx - h), px(hy - h)),
            size: size(px(h * 2.0), px(h * 2.0)),
        },
        color,
    ));
}

fn paint_selection(
    kind: &ElementKind,
    cam: Camera,
    origin: Point<Pixels>,
    color: Hsla,
    window: &mut Window,
) {
    // Lines/arrows: a handle at each endpoint (no box — its bbox is degenerate).
    if let ElementKind::Line(s) | ElementKind::Arrow(s) = kind {
        for (wx, wy) in [(s.x1, s.y1), (s.x2, s.y2)] {
            let p = to_screen(wx, wy, cam, origin);
            draw_handle(f32::from(p.x), f32::from(p.y), color, window);
        }
        return;
    }
    // Everything else: a padded box + four corner handles.
    let bb = bbox(kind);
    let tl = to_screen(bb.0, bb.1, cam, origin);
    let br = to_screen(bb.2, bb.3, cam, origin);
    let m = SEL_PAD_PX;
    let (x0, y0) = (f32::from(tl.x) - m, f32::from(tl.y) - m);
    let (x1, y1) = (f32::from(br.x) + m, f32::from(br.y) + m);
    let mut pb = PathBuilder::stroke(px(1.5));
    pb.move_to(point(px(x0), px(y0)));
    pb.line_to(point(px(x1), px(y0)));
    pb.line_to(point(px(x1), px(y1)));
    pb.line_to(point(px(x0), px(y1)));
    pb.close();
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
    for (hx, hy) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        draw_handle(hx, hy, color, window);
    }
}

impl Render for WhiteboardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = (self.style)();
        let WhiteboardStyle {
            bg,
            grid,
            text,
            ink,
            panel,
            accent,
            selection,
        } = style;
        let cam = self.scene.camera;
        let zoom = cam.zoom.max(MIN_ZOOM);
        let bounds_cell = self.bounds.clone();

        // Measure each text element's rendered width so the selection box and
        // hit-test fit the real glyphs (not a character-count estimate). Must run
        // before the `sel`/overlay snapshots below so they see fresh widths.
        for e in &mut self.scene.elements {
            if let ElementKind::Text(t) = &mut e.kind {
                t.measured_w = measure_text_width(&t.content, px(t.size * zoom), window) / zoom;
            }
        }
        // Snapshot element kinds (world coords) for the paint closure. Re-cloned
        // each frame for now; see the path-cache perf note at the top.
        let kinds: Vec<ElementKind> = self.scene.elements.iter().map(|e| e.kind.clone()).collect();
        let pending = self.pending.as_ref().map(|p| p.kind.clone());
        // No resize box while a text element is being edited — just its caret.
        let sel = self
            .selected
            .filter(|&id| Some(id) != self.editing)
            .and_then(|id| self.scene.elements.iter().find(|e| e.id == id))
            .map(|e| e.kind.clone());

        // Text renders as positioned overlay children (real glyph layout, scales
        // with zoom) above the canvas; the one being edited shows a caret. They
        // sit at root-relative offsets, so no canvas origin is needed.
        let editing = self.editing;
        let text_divs: Vec<_> = self
            .scene
            .elements
            .iter()
            .filter_map(|e| match &e.kind {
                ElementKind::Text(t) => {
                    let mut display = t.content.clone();
                    if editing == Some(e.id) {
                        display.push('│');
                    }
                    let lines = display
                        .split('\n')
                        .map(|l| div().child(SharedString::from(l.to_string())))
                        .collect::<Vec<_>>();
                    Some(
                        div()
                            .absolute()
                            .left(px((t.x - cam.x) * zoom))
                            .top(px((t.y - cam.y) * zoom))
                            .flex()
                            .flex_col()
                            .text_size(px(t.size * zoom))
                            .text_color(ink)
                            .children(lines),
                    )
                }
                _ => None,
            })
            .collect();

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
            .track_focus(&self.focus)
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
                        if let Some(k) = &sel {
                            paint_selection(k, cam, bounds.origin, selection, window);
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
            .on_key_down(cx.listener(Self::on_key))
            .children(text_divs)
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
    fn diagonal_scale_projects_the_cursor_onto_the_diagonal() {
        // On the diagonal: cursor twice as far from the anchor → 2×.
        let s = diagonal_scale([0.0, 0.0], [10.0, 10.0], [20.0, 20.0]);
        assert!((s - 2.0).abs() < 1e-4, "{s}");
        // Off-diagonal projects onto it: (20,0) onto the (10,10) line → 1×.
        let s = diagonal_scale([0.0, 0.0], [10.0, 10.0], [20.0, 0.0]);
        assert!((s - 1.0).abs() < 1e-4, "{s}");
    }

    #[test]
    fn snap_45_locks_angle_and_keeps_length() {
        // Near 45° snaps onto the exact diagonal (x == y).
        let (x, y) = snap_45(0.0, 0.0, 10.0, 9.0);
        assert!((x - y).abs() < 1e-3, "{x} vs {y}");
        // Near-horizontal snaps flat, preserving the distance.
        let (x, y) = snap_45(0.0, 0.0, 10.0, 1.0);
        assert!(y.abs() < 1e-3);
        assert!((x - 101.0f32.sqrt()).abs() < 1e-2);
    }

    #[test]
    fn resize_scales_geometry_about_the_anchor() {
        // Drag the bottom-right corner of a 20×20 rect to double it, anchored
        // at the top-left — origin stays put, size doubles.
        let mut k = ElementKind::Rect(BoxGeom {
            x: 10.0,
            y: 10.0,
            w: 20.0,
            h: 20.0,
            width: 1.0,
        });
        resize_about(&mut k, 10.0, 10.0, 2.0, 2.0);
        match k {
            ElementKind::Rect(b) => {
                assert_eq!((b.x, b.y), (10.0, 10.0));
                assert_eq!((b.w, b.h), (40.0, 40.0));
            }
            other => panic!("expected rect, got {other:?}"),
        }
    }

    #[test]
    fn text_key_handles_basic_editing() {
        let mut s = String::new();
        assert!(matches!(
            text_key(&mut s, "h", Some("h"), false),
            KeyResult::Edited
        ));
        text_key(&mut s, "i", Some("i"), false);
        text_key(&mut s, "space", Some(" "), false);
        assert_eq!(s, "hi ");
        text_key(&mut s, "enter", None, false);
        text_key(&mut s, "backspace", None, false);
        assert_eq!(s, "hi ");
        // ⌘/Ctrl chords pass through; Escape commits — neither types.
        assert!(matches!(
            text_key(&mut s, "a", Some("a"), true),
            KeyResult::Pass
        ));
        assert!(matches!(
            text_key(&mut s, "escape", None, false),
            KeyResult::Commit
        ));
        assert_eq!(s, "hi ");
    }

    #[test]
    fn text_bbox_anchors_at_origin_and_grows() {
        let t = TextGeom {
            x: 5.0,
            y: 6.0,
            content: "ab\ncde".into(),
            size: 10.0,
            measured_w: 0.0,
        };
        let bb = bbox(&ElementKind::Text(t));
        assert_eq!((bb.0, bb.1), (5.0, 6.0));
        assert!(bb.2 > bb.0 && bb.3 > bb.1);
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
