//! An infinite, pannable/zoomable whiteboard canvas for GPUI.
//!
//! Host-agnostic — depends only on `gpui`, `serde`, and `ttf-parser` (no
//! `gpui-component`, no native libraries). Two layers: a serializable scene model
//! ([`Scene`] / [`Element`]) the host persists as opaque JSON, and a
//! [`WhiteboardView`] entity that renders the board *and* its editing UI (toolbar,
//! color picker, flyouts, templates gallery, context menu) and drives all
//! interaction. The host supplies a theme ([`WhiteboardStyle`]) and optional
//! callbacks (persist on change, open a page, fetch an image bitmap, read/write the
//! clipboard, store templates); with none installed it's still a working board.
//!
//! Elements: freehand pen, rect / ellipse / diamond / triangle / rounded-rect /
//! hexagon / star, line, arrow, text, images, and page-cards — sharing one select /
//! move / resize / rotate / fill / z-order machinery, plus copy-paste, templates,
//! and undo/redo. Text renders as **vector outlines** (the `font` module, via
//! `ttf-parser`) rather than gpui overlay glyphs, so it rotates + scales with the
//! camera and a host can supply a custom face ([`Font`]). See `README.md` for the
//! full API and usage; design notes in `docs/whiteboard-architecture.md`.
//!
//! Perf note: elements are re-tessellated each paint (as GPUI's own
//! `painting`/`brush` examples do). A built-`Path` cache + viewport culling is the
//! planned optimization once boards get large — deferred so we don't build it
//! before there's something to measure.

mod font;

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

pub use font::Font;

use gpui::{
    AnyView, App, AppContext, Bounds, Context, CursorStyle, FocusHandle, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ObjectFit, ParentElement, PathBuilder, PinchEvent, Pixels, Point, Render, Rgba, ScrollDelta,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled, StyledImage,
    TransformationMatrix, Window, canvas, div, fill, hsla, linear_color_stop, linear_gradient,
    point, px, rgba, size,
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
/// Gap from the selection's top to the rotate handle, screen px.
const ROTATE_DIST: f32 = 22.0;
/// Color picker: saturation/brightness square + hue strip dimensions, px.
const SV_W: f32 = 216.0;
const SV_H: f32 = 140.0;
const HUE_H: f32 = 14.0;
/// Below this absolute rotation (radians), a box is treated as upright — it
/// shows resize corners. Rotated past it, only the rotate handle is offered
/// (rotated-frame resize is intentionally out of scope; rotate back to resize).
const ROT_EPS: f32 = 0.05;
/// While rotating, an orientation within this many radians (~6°) of horizontal
/// or vertical snaps to it, so boxes square up to the grid easily.
const ROT_SNAP: f32 = 0.105;
/// Default text size at creation, screen px (stored world size is this / zoom).
const TEXT_SIZE: f32 = 18.0;
/// Rough per-character advance and line height, as fractions of the font size,
/// for an approximate text bounding box (hit-testing / selection).
const TEXT_CHAR_W: f32 = 0.55;
const TEXT_LINE_H: f32 = 1.3;
/// Default page-card size at creation, screen px (stored world size is / zoom).
const EMBED_W: f32 = 210.0;
const EMBED_H: f32 = 76.0;
/// Longest edge of a freshly placed image, screen px (aspect preserved).
const IMAGE_PLACE_PX: f32 = 280.0;

/// The board document: everything persisted for a whiteboard. Owned and
/// (de)serialized here; the host stores [`Scene::to_json`] opaquely (for Zorite,
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
    /// Stroke / ink color, packed `0xRRGGBBAA`. `None` follows the theme ink, so
    /// uncolored elements still adapt to light/dark. Absent in older boards → 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<u32>,
    /// Fill color for closed shapes (rect/ellipse), packed `0xRRGGBBAA`. `None`
    /// is an unfilled outline. Ignored by other kinds. Absent in older boards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<u32>,
}

/// The kinds of thing a board can hold.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Draw(Stroke),
    Rect(BoxGeom),
    Ellipse(BoxGeom),
    Diamond(BoxGeom),
    Triangle(BoxGeom),
    RoundRect(BoxGeom),
    Star(BoxGeom),
    Hexagon(BoxGeom),
    Line(SegGeom),
    Arrow(SegGeom),
    Text(TextGeom),
    Embed(EmbedGeom),
    Image(ImageGeom),
}

/// A page-card: a titled box anchored at `(x, y)` that links to a host page
/// (`page_id`). The crate is page-agnostic — the host supplies the id + title
/// and handles opening it; this just stores and draws the card.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbedGeom {
    pub page_id: i64,
    pub title: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// An image: a box anchored at `(x, y)` referencing a host-managed file (`src`,
/// e.g. `images/<name>`). The crate is storage-agnostic — the host imports the
/// file and supplies the decoded bitmap (see [`ImageFn`]); this stores the
/// reference + geometry and draws it as an overlay.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageGeom {
    pub src: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Rotation about the image's center, radians. Absent in older boards → 0.
    /// The host re-rotates the bitmap to match (see [`ImageFn`]).
    #[serde(default)]
    pub rotation: f32,
}

/// A text label: a top-left anchor, its content, and a world-space font size.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextGeom {
    pub x: f32,
    pub y: f32,
    pub content: String,
    pub size: f32,
    /// Rotation about the text block's center, radians. Absent in older boards → 0.
    #[serde(default)]
    pub rotation: f32,
    /// Cached world-space extent, set each render from the font layout so the
    /// selection box and hit-test fit the real glyphs. Not persisted; a zero
    /// height means unmeasured (a fallback estimate is used until then).
    #[serde(skip)]
    pub measured_w: f32,
    #[serde(skip)]
    pub measured_h: f32,
}

/// A freehand pen stroke: world-space points and a world-space width.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    pub width: f32,
}

/// A box (rectangle / ellipse), world-space. `x,y,w,h` describe the *unrotated*
/// box; `rotation` (radians, clockwise) spins it about its center at paint time.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BoxGeom {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub width: f32,
    /// Rotation about the box center, radians. Absent in older boards → 0.
    #[serde(default)]
    pub rotation: f32,
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
    /// Drag to pan the canvas (the default — navigation before drawing).
    Pan,
    Select,
    Pen,
    Rect,
    Ellipse,
    Diamond,
    Triangle,
    RoundRect,
    Star,
    Hexagon,
    Line,
    Arrow,
    Text,
    Embed,
    Image,
}

impl Tool {
    /// A glyph for the toolbar button (dependency-free; the host has no icon set
    /// in this crate).
    fn glyph(self) -> &'static str {
        match self {
            // A dingbat hand (pre-emoji, so it always renders flat/monochrome —
            // unlike ✋, which macOS re-colors even with a VS15 text request).
            Tool::Pan => "☞",
            Tool::Select => "↖",
            Tool::Pen => "✎",
            Tool::Rect => "▭",
            Tool::Ellipse => "◯",
            Tool::Diamond => "◇",
            Tool::Triangle => "△",
            Tool::RoundRect => "▢",
            Tool::Star => "☆",
            Tool::Hexagon => "⬡",
            Tool::Line => "╱",
            Tool::Arrow => "↗",
            Tool::Text => "T",
            Tool::Embed => "▤",
            Tool::Image => "▦",
        }
    }

    /// A human label for the tooltip (the toolbar is icon-only), with the
    /// keyboard shortcut where one exists (see [`shortcut`](Tool::shortcut)).
    fn label(self) -> &'static str {
        match self {
            Tool::Pan => "Pan — drag to move (H)",
            Tool::Select => "Select (V)",
            Tool::Pen => "Pen (P)",
            Tool::Rect => "Rectangle (R)",
            Tool::Ellipse => "Ellipse (O)",
            Tool::Diamond => "Diamond (D)",
            Tool::Triangle => "Triangle (G)",
            Tool::RoundRect => "Rounded rectangle (U)",
            Tool::Star => "Star (S)",
            Tool::Hexagon => "Hexagon (X)",
            Tool::Line => "Line (L)",
            Tool::Arrow => "Arrow (A)",
            Tool::Text => "Text (T)",
            Tool::Embed => "Page card",
            Tool::Image => "Image (I) — click to place",
        }
    }

    /// The single-key shortcut that selects this tool, if any.
    fn shortcut(key: &str) -> Option<Tool> {
        Some(match key {
            "h" => Tool::Pan,
            "v" => Tool::Select,
            "p" => Tool::Pen,
            "r" => Tool::Rect,
            "o" => Tool::Ellipse,
            "d" => Tool::Diamond,
            "g" => Tool::Triangle,
            "u" => Tool::RoundRect,
            "s" => Tool::Star,
            "x" => Tool::Hexagon,
            "l" => Tool::Line,
            "a" => Tool::Arrow,
            "t" => Tool::Text,
            "i" => Tool::Image,
            _ => return None,
        })
    }

    /// The bundled SVG icon for this tool as `(cache-key, bytes)`, or `None` to
    /// fall back to [`glyph`]. Rendered flat in the theme color via gpui's SVG
    /// rasterizer (the SVG's own colors are ignored — it's tinted as an alpha
    /// mask). Lucide, ISC-licensed (see `assets/icons/LICENSE`).
    ///
    /// [`glyph`]: Tool::glyph
    fn icon(self) -> Option<(&'static str, &'static [u8])> {
        const PAN: &[u8] = include_bytes!("../assets/icons/pan.svg");
        const SELECT: &[u8] = include_bytes!("../assets/icons/select.svg");
        const PEN: &[u8] = include_bytes!("../assets/icons/pen.svg");
        const RECT: &[u8] = include_bytes!("../assets/icons/rect.svg");
        const ELLIPSE: &[u8] = include_bytes!("../assets/icons/ellipse.svg");
        const DIAMOND: &[u8] = include_bytes!("../assets/icons/diamond.svg");
        const TRIANGLE: &[u8] = include_bytes!("../assets/icons/triangle.svg");
        const ROUND_RECT: &[u8] = include_bytes!("../assets/icons/round-rect.svg");
        const STAR: &[u8] = include_bytes!("../assets/icons/star.svg");
        const HEXAGON: &[u8] = include_bytes!("../assets/icons/hexagon.svg");
        const LINE: &[u8] = include_bytes!("../assets/icons/line.svg");
        const ARROW: &[u8] = include_bytes!("../assets/icons/arrow.svg");
        const TEXT: &[u8] = include_bytes!("../assets/icons/text.svg");
        const EMBED: &[u8] = include_bytes!("../assets/icons/embed.svg");
        const IMAGE: &[u8] = include_bytes!("../assets/icons/image.svg");
        match self {
            Tool::Pan => Some(("wb-icon-pan", PAN)),
            Tool::Select => Some(("wb-icon-select", SELECT)),
            Tool::Pen => Some(("wb-icon-pen", PEN)),
            Tool::Rect => Some(("wb-icon-rect", RECT)),
            Tool::Ellipse => Some(("wb-icon-ellipse", ELLIPSE)),
            Tool::Diamond => Some(("wb-icon-diamond", DIAMOND)),
            Tool::Triangle => Some(("wb-icon-triangle", TRIANGLE)),
            Tool::RoundRect => Some(("wb-icon-round-rect", ROUND_RECT)),
            Tool::Star => Some(("wb-icon-star", STAR)),
            Tool::Hexagon => Some(("wb-icon-hexagon", HEXAGON)),
            Tool::Line => Some(("wb-icon-line", LINE)),
            Tool::Arrow => Some(("wb-icon-arrow", ARROW)),
            Tool::Text => Some(("wb-icon-text", TEXT)),
            Tool::Embed => Some(("wb-icon-embed", EMBED)),
            Tool::Image => Some(("wb-icon-image", IMAGE)),
        }
    }
}

/// A toolbar category whose tools live in a click-to-open flyout, keeping the
/// main bar trim. The category button shows the active tool of the group (or a
/// representative when none is active).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolGroup {
    /// Freehand pen and the closed shapes.
    Shapes,
    /// Line and arrow connectors.
    Lines,
    /// Page-cards (and, later, images).
    PagesImages,
}

impl ToolGroup {
    const ALL: [ToolGroup; 3] = [ToolGroup::Shapes, ToolGroup::Lines, ToolGroup::PagesImages];

    /// The tools shown in this group's flyout.
    fn tools(self) -> &'static [Tool] {
        match self {
            ToolGroup::Shapes => &[
                Tool::Rect,
                Tool::RoundRect,
                Tool::Ellipse,
                Tool::Diamond,
                Tool::Triangle,
                Tool::Hexagon,
                Tool::Star,
            ],
            ToolGroup::Lines => &[Tool::Pen, Tool::Line, Tool::Arrow],
            ToolGroup::PagesImages => &[Tool::Embed, Tool::Image],
        }
    }

    fn contains(self, t: Tool) -> bool {
        self.tools().contains(&t)
    }

    /// The icon shown on the category button when none of its tools is active.
    fn representative(self) -> Tool {
        match self {
            ToolGroup::Shapes => Tool::Rect,
            ToolGroup::Lines => Tool::Line,
            ToolGroup::PagesImages => Tool::Embed,
        }
    }

    fn label(self) -> &'static str {
        match self {
            ToolGroup::Shapes => "Shapes",
            ToolGroup::Lines => "Lines",
            ToolGroup::PagesImages => "Pages & images",
        }
    }
}

/// A flat, theme-colored toolbar icon: render the bundled SVG `bytes` (a 16×16
/// Lucide glyph) tinted to `color` via gpui's rasterizer, in a `size`-px box.
/// `key` is a stable per-icon cache id.
fn svg_icon(key: &'static str, bytes: &'static [u8], color: Hsla, sz: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let _ = window.paint_svg(
                bounds,
                SharedString::from(key),
                Some(bytes),
                TransformationMatrix::default(),
                color,
                cx,
            );
        },
    )
    .w(px(sz))
    .h(px(sz))
}

/// A hairline vertical divider separating toolbar tool groups.
fn toolbar_divider(color: Hsla) -> gpui::AnyElement {
    div()
        .w(px(1.0))
        .h(px(16.0))
        .mx(px(3.0))
        .bg(color)
        .into_any_element()
}

/// A minimal themed tooltip view. gpui has the `.tooltip()` *hook* but no
/// tooltip *view* (those live in UI crates this crate doesn't depend on), so —
/// like `gpui-pdf` — we render our own small label.
struct Tip {
    text: SharedString,
    fg: Hsla,
    bg: Hsla,
    border: Hsla,
}

impl Render for Tip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // gpui anchors the tooltip at the cursor; a small transparent top
        // padding drops the visible box just clear of the hovered button.
        div().pt(px(16.0)).child(
            div()
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(self.border)
                .bg(self.bg)
                .text_color(self.fg)
                .text_size(px(11.0))
                .child(self.text.clone()),
        )
    }
}

/// Theme colors, read at paint time (via [`WhiteboardStyleFn`]) so the board
/// follows live theme changes per window.
#[derive(Clone, Debug)]
pub struct WhiteboardStyle {
    /// The canvas background.
    pub bg: Hsla,
    /// The background grid dots.
    pub grid: Hsla,
    /// HUD / muted on-canvas text.
    pub text: Hsla,
    /// Ink (stroke/shape color). Per-element color comes with the color picker.
    pub ink: Hsla,
    /// Toolbar / flyout panel background — small pills, so it can be quite glassy.
    pub panel: Hsla,
    /// Background for the larger color-picker panel. Wants to stay readable over
    /// a busy canvas, so it should be much more opaque than `panel`.
    pub panel_strong: Hsla,
    /// Active-tool highlight (a subtle fill behind the current tool button).
    pub accent: Hsla,
    /// Selection outline — wants to be clearly visible, so a strong color.
    pub selection: Hsla,
    /// Palette shown as quick swatches in the color picker. The host supplies
    /// these (typically its theme colors) so the picker matches the app.
    pub swatches: Vec<Hsla>,
}

/// A `() -> WhiteboardStyle` the host supplies; called each paint so the board
/// tracks theme changes without the host pushing updates.
pub type WhiteboardStyleFn = Rc<dyn Fn() -> WhiteboardStyle>;

/// Called when the board changes (an element committed/moved/deleted, the camera
/// moved), with the serialized scene JSON, so the host can persist it.
pub type ChangeFn = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// Called when the page-card tool is clicked at world `(x, y)` — the host picks
/// a page and calls [`WhiteboardView::add_embed`].
pub type PlaceEmbedFn = Rc<dyn Fn(f32, f32, &mut Window, &mut App)>;

/// Called to open a page (double-clicking a card) — the host opens it in a tab.
pub type OpenPageFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;

/// Called when the user saves the current selection as a template, with the
/// selected elements serialized (normalized to origin). The host names + stores
/// it, then feeds the updated list back via [`WhiteboardView::set_templates`].
pub type SaveTemplateFn = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// Called to delete a stored template by its host id (right-click a card).
pub type DeleteTemplateFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;

/// Called on ⌘C / ⌘X with the selection serialized (same format as
/// [`SaveTemplateFn`]); the host writes it to the system clipboard. Paste is the
/// reverse: the host reads the clipboard and calls [`WhiteboardView::paste_elements`].
pub type CopyFn = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// Called by the context-menu **Paste**: the host reads the clipboard and returns
/// previously copied whiteboard elements (the JSON a [`CopyFn`] wrote — same format
/// as [`SaveTemplateFn`]), or `None` if it holds no board elements. Pass the JSON to
/// [`WhiteboardView::paste_elements`]. (Keyboard ⌘V is handled internally.)
pub type PasteFn = Rc<dyn Fn(&mut Window, &mut App) -> Option<String>>;

/// Called each render to fetch the decoded bitmap for an image element's `src`,
/// rotated by `rotation` radians (0 = upright). The host serves it from its image
/// cache, decoding/rotating on demand (returning `None` until ready, then
/// re-rendering the board); a steady angle hits the cache, so it only re-rotates
/// when the angle changes.
pub type ImageFn = Rc<dyn Fn(&str, f32, &mut Window, &mut App) -> Option<gpui::ImageSource>>;

/// Called when the image tool is clicked at world `(x, y)` — the host picks a
/// file and calls [`WhiteboardView::add_image_at`].
pub type PlaceImageFn = Rc<dyn Fn(f32, f32, &mut Window, &mut App)>;

/// Called when files are dropped onto the canvas at world `(x, y)` — the host
/// imports any images and places them via [`WhiteboardView::add_image_at`].
pub type DropFilesFn = Rc<dyn Fn(Vec<std::path::PathBuf>, f32, f32, &mut Window, &mut App)>;

/// A reusable group of elements the user can stamp onto a board. Element
/// positions are normalized so the group's bounding box starts at the origin;
/// applying re-bases them to the viewport. The host owns persistence and the
/// `id`; the crate renders the preview + instantiates on click.
#[derive(Clone, Debug)]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub elements: Vec<Element>,
}

impl Template {
    /// Build from the host's stored row. `elements_json` is a serialized
    /// `Vec<Element>` (the JSON a [`SaveTemplateFn`] handed the host); malformed
    /// JSON yields an empty (still-listable) template.
    pub fn from_json(id: i64, name: impl Into<String>, elements_json: &str) -> Self {
        Template {
            id,
            name: name.into(),
            elements: serde_json::from_str(elements_json).unwrap_or_default(),
        }
    }
}

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

/// An in-progress proportional resize of a multi-selection by a corner of its
/// (axis-aligned) group bounds. Every member scales uniformly about the opposite
/// corner, so the group grows/shrinks as one.
struct GroupResizing {
    /// The fixed opposite corner of the group bounds, world space.
    anchor: [f32; 2],
    /// The dragged corner's original position, world space.
    from: [f32; 2],
    /// Cursor → dragged-corner offset at grab (1:1 tracking, no jump).
    grab: [f32; 2],
    /// Each selected element's id + geometry at the start of the resize.
    orig: Vec<(u64, ElementKind)>,
}

/// An in-progress drag of one endpoint of a selected line/arrow.
#[derive(Clone, Copy)]
struct EndpointDrag {
    id: u64,
    /// Which endpoint: 0 = (x1,y1), 1 = (x2,y2).
    which: usize,
}

/// An in-progress rotation of the selection (one element or a group) about a
/// fixed center. Drives every selected element, so it needs no element id.
#[derive(Clone, Copy)]
struct Rotating {
    /// Pivot (world), captured at grab so it can't drift between frames.
    center: [f32; 2],
    /// Pointer angle about `center` at grab (radians).
    start_pointer: f32,
    /// Rotation already applied since grab (radians).
    applied: f32,
    /// Orientation to snap to horizontal/vertical: a single element's angle (box
    /// / text) or line direction; `Some(0)` for a group (snaps quarter-turns);
    /// `None` when there's nothing meaningful to snap (a lone freehand stroke).
    base: Option<f32>,
}

/// What a press on a selection handle begins.
enum HandleGrab {
    Corner(Resizing),
    Endpoint(EndpointDrag),
    Rotate,
    GroupCorner(GroupResizing),
}

/// Which property the picker is editing.
#[derive(Clone, Copy, PartialEq)]
enum PickerTarget {
    /// Outline / ink color (`None` = theme ink).
    Stroke,
    /// Shape fill (`None` = unfilled).
    Fill,
}

/// Open color-picker state: the HSVA the controls currently reflect, and which
/// property (stroke or fill) it edits. Recolors the selection live.
#[derive(Clone, Copy)]
struct Picker {
    target: PickerTarget,
    h: f32,
    s: f32,
    v: f32,
    a: f32,
}

/// Which picker control an in-progress drag is manipulating.
#[derive(Clone, Copy, PartialEq)]
enum PickerDrag {
    /// The saturation/brightness square.
    Sv,
    /// The hue strip.
    Hue,
    /// The alpha (opacity) strip.
    Alpha,
}

/// The whiteboard view entity. The host holds it in an `Entity<WhiteboardView>`
/// (keyed by board id) and renders it into a tab.
pub struct WhiteboardView {
    scene: Scene,
    style: WhiteboardStyleFn,
    on_change: Option<ChangeFn>,
    on_place_embed: Option<PlaceEmbedFn>,
    on_open: Option<OpenPageFn>,
    on_save_template: Option<SaveTemplateFn>,
    on_delete_template: Option<DeleteTemplateFn>,
    on_image: Option<ImageFn>,
    on_place_image: Option<PlaceImageFn>,
    on_drop_files: Option<DropFilesFn>,
    on_copy: Option<CopyFn>,
    on_paste: Option<PasteFn>,
    /// Stored templates, supplied by the host; shown as cards in the Pages &
    /// Images flyout.
    templates: Vec<Template>,
    /// Screen position of an open right-click context menu (a selection's
    /// "save as template"), or `None`.
    context_menu: Option<Point<Pixels>>,
    /// The face used to render text as vector outlines. Defaults to the bundled
    /// JetBrains Mono; the host can swap in a custom/user-uploaded font.
    font: Font,
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
    /// The currently selected elements (Select tool).
    selected: Vec<u64>,
    /// In-progress marquee box (start, current) in world coords.
    marquee: Option<([f32; 2], [f32; 2])>,
    /// The world point where an in-progress move-drag was grabbed (a *fixed*
    /// anchor — the move uses the total cursor delta from here, so grid-snapping
    /// stays cursor-synced and doesn't lose sub-grid motion).
    drag_from: Option<[f32; 2]>,
    /// The primary (first-selected) element's top-left at move-grab, the
    /// reference the move drives toward (`move_origin + total_delta`).
    move_origin: [f32; 2],
    /// Whether the current move-drag has actually moved (undo is pushed once).
    moved: bool,
    /// In-progress corner-resize of the selected box/stroke.
    resizing: Option<Resizing>,
    /// In-progress proportional resize of a multi-selection.
    group_resizing: Option<GroupResizing>,
    /// In-progress endpoint-drag of the selected line/arrow.
    endpoint: Option<EndpointDrag>,
    /// In-progress rotation of the selected element.
    rotating: Option<Rotating>,
    /// Current ink color for new elements (`None` follows the theme ink).
    active_stroke: Option<u32>,
    /// Current fill for new shapes (`None` = unfilled).
    active_fill: Option<u32>,
    /// Open color picker, if any.
    picker: Option<Picker>,
    /// The tool category whose flyout is open, if any.
    open_group: Option<ToolGroup>,
    /// Whether the templates gallery modal is open.
    templates_open: bool,
    /// In-progress drag inside the open picker.
    picker_drag: Option<PickerDrag>,
    /// Screen bounds of the picker panel and its draggable regions, captured each
    /// paint so press/drag handlers can hit-test them.
    picker_bounds: Rc<Cell<Bounds<Pixels>>>,
    sv_bounds: Rc<Cell<Bounds<Pixels>>>,
    hue_bounds: Rc<Cell<Bounds<Pixels>>>,
    alpha_bounds: Rc<Cell<Bounds<Pixels>>>,
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
            on_place_embed: None,
            on_open: None,
            on_save_template: None,
            on_delete_template: None,
            on_image: None,
            on_place_image: None,
            on_drop_files: None,
            on_copy: None,
            on_paste: None,
            templates: Vec::new(),
            context_menu: None,
            font: Font::default(),
            tool: Tool::Pan,
            focus: cx.focus_handle(),
            editing: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
            pending: None,
            selected: Vec::new(),
            marquee: None,
            drag_from: None,
            move_origin: [0.0, 0.0],
            moved: false,
            resizing: None,
            group_resizing: None,
            endpoint: None,
            rotating: None,
            active_stroke: None,
            active_fill: None,
            picker: None,
            open_group: None,
            templates_open: false,
            picker_drag: None,
            picker_bounds: Rc::new(Cell::new(Bounds::default())),
            sv_bounds: Rc::new(Cell::new(Bounds::default())),
            hue_bounds: Rc::new(Cell::new(Bounds::default())),
            alpha_bounds: Rc::new(Cell::new(Bounds::default())),
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

    /// Install the page-card placement hook (page-card tool click).
    pub fn set_on_place_embed(&mut self, f: PlaceEmbedFn) {
        self.on_place_embed = Some(f);
    }

    /// Install the open-page hook (double-click a card).
    pub fn set_on_open(&mut self, f: OpenPageFn) {
        self.on_open = Some(f);
    }

    /// Install the save-template hook (right-click selection → save).
    pub fn set_on_save_template(&mut self, f: SaveTemplateFn) {
        self.on_save_template = Some(f);
    }

    /// Install the delete-template hook (right-click a template card → delete).
    pub fn set_on_delete_template(&mut self, f: DeleteTemplateFn) {
        self.on_delete_template = Some(f);
    }

    /// Install the image-fetch hook (decoded bitmap for an element's `src`).
    pub fn set_on_image(&mut self, f: ImageFn) {
        self.on_image = Some(f);
    }

    /// Install the place-image hook (image tool click → host file picker).
    pub fn set_on_place_image(&mut self, f: PlaceImageFn) {
        self.on_place_image = Some(f);
    }

    /// Install the file-drop hook (files dropped on the canvas).
    pub fn set_on_drop_files(&mut self, f: DropFilesFn) {
        self.on_drop_files = Some(f);
    }

    /// Install the copy hook (⌘C / ⌘X → write the selection to the clipboard).
    pub fn set_on_copy(&mut self, f: CopyFn) {
        self.on_copy = Some(f);
    }

    /// Install the paste hook (context-menu Paste → read board elements from the
    /// clipboard). Without it, the Paste menu item is hidden.
    pub fn set_on_paste(&mut self, f: PasteFn) {
        self.on_paste = Some(f);
    }

    /// Replace the stored templates shown in the Pages & Images flyout. The host
    /// calls this on open and after any save/delete.
    pub fn set_templates(&mut self, templates: Vec<Template>, cx: &mut Context<Self>) {
        self.templates = templates;
        cx.notify();
    }

    /// Swap the font used to render text (e.g. a user-uploaded face). Build one
    /// with [`Font::from_bytes`].
    pub fn set_font(&mut self, font: Font, cx: &mut Context<Self>) {
        self.font = font;
        cx.notify();
    }

    /// Build a `.tooltip(..)` closure for a toolbar control — a small themed
    /// [`Tip`], reading colors through the style closure at show time.
    fn tip(
        &self,
        text: impl Into<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let style_fn = self.style.clone();
        let text = text.into();
        move |_window, cx| {
            let s = style_fn();
            let text = text.clone();
            cx.new(move |_| Tip {
                text,
                fg: s.ink,
                bg: s.panel,
                border: s.grid,
            })
            .into()
        }
    }

    /// Insert a page-card at world `(x, y)` and select it. Called by the host
    /// after the user picks a page (in response to [`PlaceEmbedFn`]). Does *not*
    /// fire `on_change` — the host calls this mid-update, so a re-entrant save
    /// would panic; the host persists explicitly via [`scene`](Self::scene).
    pub fn add_embed(
        &mut self,
        page_id: i64,
        title: impl Into<String>,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) {
        self.push_undo();
        let id = self.next_id;
        self.next_id += 1;
        let zoom = self.scene.camera.zoom.max(MIN_ZOOM);
        self.scene.elements.push(Element {
            id,
            kind: ElementKind::Embed(EmbedGeom {
                page_id,
                title: title.into(),
                x,
                y,
                w: EMBED_W / zoom,
                h: EMBED_H / zoom,
            }),
            stroke: None,
            fill: None,
        });
        self.selected = vec![id];
        self.tool = Tool::Select;
        cx.notify();
    }

    /// Add an image element referencing `src`, centered at world `(cx_world,
    /// cy_world)` and sized from its pixel dimensions (`px_w`/`px_h`) so the longest
    /// edge gets a sensible default on-screen size (aspect preserved). Like
    /// [`add_embed`], the host persists afterward (this is called mid-host-update).
    ///
    /// [`add_embed`]: Self::add_embed
    pub fn add_image_at(
        &mut self,
        src: impl Into<String>,
        px_w: f32,
        px_h: f32,
        cx_world: f32,
        cy_world: f32,
        cx: &mut Context<Self>,
    ) {
        self.push_undo();
        let id = self.next_id;
        self.next_id += 1;
        let zoom = self.scene.camera.zoom.max(MIN_ZOOM);
        let longest = px_w.max(px_h).max(1.0);
        let scale = IMAGE_PLACE_PX / longest / zoom;
        let (w, h) = (px_w * scale, px_h * scale);
        self.scene.elements.push(Element {
            id,
            kind: ElementKind::Image(ImageGeom {
                src: src.into(),
                x: cx_world - w / 2.0,
                y: cy_world - h / 2.0,
                w,
                h,
                rotation: 0.0,
            }),
            stroke: None,
            fill: None,
        });
        self.selected = vec![id];
        self.tool = Tool::Select;
        cx.notify();
    }

    /// The world point at the center of the current viewport — where paste drops
    /// an image (the host has no access to the camera otherwise).
    pub fn viewport_center(&self) -> [f32; 2] {
        let b = self.bounds.get();
        let cam = self.scene.camera;
        let z = cam.zoom.max(MIN_ZOOM);
        [
            cam.x + f32::from(b.size.width) / 2.0 / z,
            cam.y + f32::from(b.size.height) / 2.0 / z,
        ]
    }

    /// The current board document (for the host to persist).
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The lone selected id, if exactly one element is selected. Single-element
    /// manipulation (resize, endpoints, edit) only applies then.
    fn selected_single(&self) -> Option<u64> {
        match self.selected.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }

    fn is_selected(&self, id: u64) -> bool {
        self.selected.contains(&id)
    }

    /// World-space bounds enclosing the whole selection, or `None` if empty.
    fn selection_bbox(&self) -> Option<(f32, f32, f32, f32)> {
        let mut it = self
            .scene
            .elements
            .iter()
            .filter(|e| self.selected.contains(&e.id))
            .map(|e| bbox(&e.kind));
        let first = it.next()?;
        Some(it.fold(first, |a, b| {
            (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
        }))
    }

    /// Whether a *group* rotation applies: more than one element selected, at
    /// least one of which can rotate (so an all-cards group offers no grip).
    fn group_rotatable(&self) -> bool {
        self.selected.len() > 1
            && self
                .scene
                .elements
                .iter()
                .any(|e| self.selected.contains(&e.id) && rotatable(&e.kind))
    }

    /// The active tool (e.g. for host-driven chrome).
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Switch the active drawing tool. Leaving Select clears the selection.
    /// Always closes an open tool flyout (the tool was just chosen).
    pub fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.open_group = None;
        if self.tool != tool {
            self.tool = tool;
            if tool != Tool::Select {
                self.selected.clear();
            }
        }
        cx.notify();
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
            self.selected.clear();
            self.dirty = true;
            cx.notify();
            self.flush(window, cx);
        }
    }

    /// Re-apply the last undone change.
    pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.redo.pop() {
            self.history.push(std::mem::replace(&mut self.scene, next));
            self.selected.clear();
            self.dirty = true;
            cx.notify();
            self.flush(window, cx);
        }
    }

    /// Delete the selected elements.
    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            return;
        }
        self.push_undo();
        let gone = std::mem::take(&mut self.selected);
        self.scene.elements.retain(|e| !gone.contains(&e.id));
        self.editing = None;
        self.dirty = true;
        cx.notify();
        self.flush(window, cx);
    }

    /// Move the selected elements through the paint order (their position in
    /// `elements`; later = painted on top, so it can cover earlier ones). One step
    /// or all the way, per `op`. A no-op (already at that edge) leaves undo/redo
    /// untouched.
    fn reorder_selection(&mut self, op: ZOrder, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            return;
        }
        let sel = self.selected.clone();
        let on = |id: u64| sel.contains(&id);
        self.push_undo();
        let before: Vec<u64> = self.scene.elements.iter().map(|e| e.id).collect();
        let els = &mut self.scene.elements;
        match op {
            // Stable partition: the non-selected keep their order and the selected
            // keep theirs, so a multi-selection moves as a block.
            ZOrder::ToFront => els.sort_by_key(|e| on(e.id)),
            ZOrder::ToBack => els.sort_by_key(|e| !on(e.id)),
            // One step: swap each selected past its adjacent non-selected neighbor,
            // walking away from the destination edge so an element isn't moved twice
            // and selected elements don't leapfrog each other.
            ZOrder::Forward => {
                for i in (0..els.len().saturating_sub(1)).rev() {
                    if on(els[i].id) && !on(els[i + 1].id) {
                        els.swap(i, i + 1);
                    }
                }
            }
            ZOrder::Backward => {
                for i in 1..els.len() {
                    if on(els[i].id) && !on(els[i - 1].id) {
                        els.swap(i, i - 1);
                    }
                }
            }
        }
        if self.scene.elements.iter().map(|e| e.id).eq(before) {
            self.history.pop(); // nothing moved — drop the speculative snapshot
            return;
        }
        self.dirty = true;
        cx.notify();
        self.flush(window, cx);
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

    // --- templates ---------------------------------------------------------

    /// Serialize the current selection: the selected elements translated so their
    /// collective bounding box starts at the origin (so the group can be re-based
    /// anywhere when applied). `None` if nothing is selected. Used for both saving
    /// a template and copying to the clipboard — the two share this format, so a
    /// copied selection can be pasted on any board (see [`Self::paste_elements`]).
    fn selection_json(&self) -> Option<String> {
        let sel: Vec<&Element> = self
            .scene
            .elements
            .iter()
            .filter(|e| self.selected.contains(&e.id))
            .collect();
        if sel.is_empty() {
            return None;
        }
        let (minx, miny) = sel
            .iter()
            .fold((f32::INFINITY, f32::INFINITY), |(mx, my), e| {
                let (x0, y0, ..) = bbox(&e.kind);
                (mx.min(x0), my.min(y0))
            });
        let elems: Vec<Element> = sel
            .iter()
            .map(|e| {
                let mut c = (*e).clone();
                translate(&mut c.kind, -minx, -miny);
                c
            })
            .collect();
        serde_json::to_string(&elems).ok()
    }

    /// Hand the current selection to the host to be saved as a named template.
    fn save_selection_as_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        if let Some(json) = self.selection_json()
            && let Some(f) = self.on_save_template.clone()
        {
            f(json, window, cx);
        }
        cx.notify();
    }

    /// Stamp template `index` onto the board, centered in the current viewport,
    /// with fresh ids; the new elements become the selection.
    fn apply_template(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(elems) = self.templates.get(index).map(|t| t.elements.clone()) else {
            return;
        };
        self.templates_open = false;
        self.stamp_elements(&elems, window, cx);
    }

    /// Place `elems` (origin-normalized, as produced by [`Self::selection_json`])
    /// onto the board, centered in the current viewport with fresh ids; they
    /// become the new selection. Shared by template apply and clipboard paste.
    /// No-op for an empty group.
    fn stamp_elements(&mut self, elems: &[Element], window: &mut Window, cx: &mut Context<Self>) {
        if elems.is_empty() {
            return;
        }
        self.open_group = None;
        self.push_undo();
        // Center the (origin-normalized) group in the viewport.
        let b = self.bounds.get();
        let cam = self.scene.camera;
        let z = cam.zoom.max(MIN_ZOOM);
        let (tw, th) = elements_extent(elems);
        let off = [
            cam.x + (f32::from(b.size.width) / 2.0) / z - tw / 2.0,
            cam.y + (f32::from(b.size.height) / 2.0) / z - th / 2.0,
        ];
        let mut new_ids = Vec::with_capacity(elems.len());
        for e in elems {
            let mut c = e.clone();
            translate(&mut c.kind, off[0], off[1]);
            c.id = self.next_id;
            self.next_id += 1;
            new_ids.push(c.id);
            self.scene.elements.push(c);
        }
        self.selected = new_ids;
        self.tool = Tool::Select;
        self.dirty = true;
        self.flush(window, cx);
        cx.notify();
    }

    /// Copy the selection to the clipboard via the host's `on_copy` hook (the
    /// crate can't touch the system clipboard). Returns whether anything was
    /// copied. `⌘X` reuses this, then deletes.
    fn copy_selection(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(json) = self.selection_json() else {
            return false;
        };
        if let Some(f) = self.on_copy.clone() {
            f(json, window, cx);
        }
        true
    }

    /// Paste a serialized `Vec<Element>` (the JSON a [`CopyFn`] wrote) onto the
    /// board — centered in the viewport, selected, with fresh ids. Ignores invalid
    /// JSON. The host calls this from its [`PasteFn`] when the clipboard holds
    /// whiteboard elements rather than an image.
    pub fn paste_elements(&mut self, json: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(elems) = serde_json::from_str::<Vec<Element>>(json) {
            self.stamp_elements(&elems, window, cx);
        }
    }

    /// Ask the host to delete a stored template (right-click a card). The host
    /// confirms, removes it, and feeds the updated list back.
    fn delete_template(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.on_delete_template.clone() {
            f(id, window, cx);
        }
    }

    /// A template preview card for the gallery modal: a scaled mini-paint of the
    /// template's shapes over its name. Click to stamp it; right-click to delete.
    /// (Text and page-cards don't appear in the mini-paint — only drawn shapes —
    /// but they're still placed on apply.)
    fn template_card(
        &self,
        index: usize,
        ink: Hsla,
        text: Hsla,
        grid: Hsla,
        bg: Hsla,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = &self.templates[index];
        let id = t.id;
        let name: SharedString = t.name.clone().into();
        let elems = t.elements.clone();
        let (tw, th) = elements_extent(&elems);
        let preview = canvas(
            |_, _, _| {},
            move |bounds, _, window: &mut Window, _: &mut App| {
                let pad = 8.0;
                let aw = f32::from(bounds.size.width) - 2.0 * pad;
                let ah = f32::from(bounds.size.height) - 2.0 * pad;
                if tw <= 0.0 || th <= 0.0 || aw <= 0.0 || ah <= 0.0 {
                    return;
                }
                // Fit the (origin-normalized) template into the card, centered,
                // never magnifying past 1:1.
                let scale = (aw / tw).min(ah / th).min(1.0);
                let ox = (f32::from(bounds.size.width) - tw * scale) / 2.0;
                let oy = (f32::from(bounds.size.height) - th * scale) / 2.0;
                let cam = Camera {
                    x: -ox / scale,
                    y: -oy / scale,
                    zoom: scale,
                };
                for e in &elems {
                    let stroke = e.stroke.map_or(ink, u32_to_hsla);
                    let fill = e.fill.map(u32_to_hsla);
                    paint_element(&e.kind, cam, bounds.origin, stroke, fill, window);
                }
            },
        )
        .size_full();
        div()
            .id(("wb-template", index))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(5.0))
            .p(px(6.0))
            .rounded(px(8.0))
            .hover(|s| s.bg(grid))
            .child(
                div()
                    .w(px(150.0))
                    .h(px(104.0))
                    .rounded(px(6.0))
                    .bg(bg)
                    .border_1()
                    .border_color(grid)
                    .child(preview),
            )
            .child(
                div()
                    .w(px(150.0))
                    .h(px(15.0))
                    .overflow_hidden()
                    .text_size(px(11.0))
                    .text_color(text)
                    .child(name),
            )
            .on_click(
                cx.listener(move |this, _ev, window, cx| this.apply_template(index, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _ev, window, cx| this.delete_template(id, window, cx)),
            )
            .into_any_element()
    }

    // --- color picker ------------------------------------------------------

    /// The color the picker should start from for `target`: the single
    /// selection's color (if any), else the active color, else a default.
    fn seed_color(&self, target: PickerTarget) -> u32 {
        let from_sel = self
            .selected_single()
            .and_then(|id| self.scene.elements.iter().find(|e| e.id == id))
            .and_then(|e| match target {
                PickerTarget::Stroke => e.stroke,
                PickerTarget::Fill => e.fill,
            });
        let active = match target {
            PickerTarget::Stroke => self.active_stroke,
            PickerTarget::Fill => self.active_fill,
        };
        from_sel.or(active).unwrap_or(0x4080f0ff)
    }

    /// Point the picker's HSVA controls at `target`'s current color.
    fn seed_picker(&mut self, target: PickerTarget) {
        let c = self.seed_color(target);
        let (h, s, v) = u32_to_hsv(c);
        self.picker = Some(Picker {
            target,
            h,
            s,
            v,
            a: u32_alpha(c),
        });
    }

    /// Open or close the color picker. Opening seeds the controls from the
    /// stroke color (selection's, else active, else a default).
    fn toggle_picker(&mut self, cx: &mut Context<Self>) {
        self.open_group = None;
        self.templates_open = false;
        if self.picker.is_some() {
            self.picker = None;
        } else {
            self.seed_picker(PickerTarget::Stroke);
        }
        cx.notify();
    }

    /// Open the given tool category's flyout (or close it if already open).
    /// Closes the color picker so only one popover shows at a time.
    fn toggle_group(&mut self, group: ToolGroup, cx: &mut Context<Self>) {
        self.picker = None;
        self.templates_open = false;
        self.open_group = if self.open_group == Some(group) {
            None
        } else {
            Some(group)
        };
        cx.notify();
    }

    /// Open / close the templates gallery modal (closing the other popovers).
    fn toggle_templates(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        self.open_group = None;
        self.context_menu = None;
        self.templates_open = !self.templates_open;
        cx.notify();
    }

    /// Switch which property (stroke / fill) the picker edits, re-seeding its
    /// controls from that property's current color.
    fn set_picker_target(&mut self, target: PickerTarget, cx: &mut Context<Self>) {
        if self.picker.map(|p| p.target) != Some(target) {
            self.seed_picker(target);
            cx.notify();
        }
    }

    /// The picker's current target (stroke unless the picker says otherwise).
    fn picker_target(&self) -> PickerTarget {
        self.picker.map_or(PickerTarget::Stroke, |p| p.target)
    }

    /// Apply `color` to the active target on the active swatch and the selection,
    /// *without* undo/flush — used for live picker drags (undo is pushed once at
    /// drag start; the flush happens on release).
    fn set_color_live(&mut self, color: Option<u32>, cx: &mut Context<Self>) {
        let target = self.picker_target();
        match target {
            PickerTarget::Stroke => self.active_stroke = color,
            PickerTarget::Fill => self.active_fill = color,
        }
        if !self.selected.is_empty() {
            let sel = self.selected.clone();
            for e in self.scene.elements.iter_mut() {
                if !sel.contains(&e.id) {
                    continue;
                }
                match target {
                    PickerTarget::Stroke => e.stroke = color,
                    // Fill only attaches to closed shapes — never lines/strokes.
                    PickerTarget::Fill => {
                        if is_closed_shape(&e.kind) {
                            e.fill = color;
                        }
                    }
                }
            }
            self.dirty = true;
        }
        cx.notify();
    }

    /// A discrete, undoable color choice (a swatch, or the Auto / None reset).
    /// Recolors the selection and syncs the picker controls to the chosen color.
    fn pick_color(&mut self, color: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            self.push_undo();
        }
        if let (Some(c), Some(p)) = (color, self.picker.as_mut()) {
            let (h, s, v) = u32_to_hsv(c);
            // Keep the hue stable on greys (s == 0) so the strip thumb won't jump.
            if s > 0.0 {
                p.h = h;
            }
            p.s = s;
            p.v = v;
            p.a = u32_alpha(c);
        }
        self.set_color_live(color, cx);
        self.flush(window, cx);
    }

    /// Saturation/brightness under a window-coords position in the SV square.
    fn sv_from_pos(&self, pos: Point<Pixels>) -> (f32, f32) {
        let b = self.sv_bounds.get();
        let w = f32::from(b.size.width).max(1.0);
        let h = f32::from(b.size.height).max(1.0);
        let s = ((f32::from(pos.x) - f32::from(b.origin.x)) / w).clamp(0.0, 1.0);
        let v = 1.0 - ((f32::from(pos.y) - f32::from(b.origin.y)) / h).clamp(0.0, 1.0);
        (s, v)
    }

    /// A 0..1 fraction along a horizontal strip (hue or alpha) under `pos`.
    fn frac_x(&self, bounds: Bounds<Pixels>, pos: Point<Pixels>) -> f32 {
        let w = f32::from(bounds.size.width).max(1.0);
        ((f32::from(pos.x) - f32::from(bounds.origin.x)) / w).clamp(0.0, 1.0)
    }

    /// The picker's current color as a packed int (for live application).
    fn picker_u32(&self) -> Option<u32> {
        self.picker.map(|p| hsva_to_u32(p.h, p.s, p.v, p.a))
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

        // A multi-selection offers a group rotate grip (if anything's rotatable)
        // and proportional corner-resize of the group bounds.
        if self.selected.len() > 1 {
            let bb = self.selection_bbox()?;
            if self.group_rotatable() {
                let (rx, ry) = rotate_handle_for_bbox(bb, cam, origin);
                let (dx, dy) = (f32::from(pos.x) - rx, f32::from(pos.y) - ry);
                if dx * dx + dy * dy <= HANDLE_GRAB * HANDLE_GRAB {
                    return Some(HandleGrab::Rotate);
                }
            }
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
                    let orig = self
                        .scene
                        .elements
                        .iter()
                        .filter(|e| self.is_selected(e.id))
                        .map(|e| (e.id, e.kind.clone()))
                        .collect();
                    return Some(HandleGrab::GroupCorner(GroupResizing {
                        anchor: [opp.0, opp.1],
                        from: [wc[i].0, wc[i].1],
                        grab: [wc[i].0 - cursor[0], wc[i].1 - cursor[1]],
                        orig,
                    }));
                }
            }
            return None;
        }

        let id = self.selected_single()?;
        let kind = &self.scene.elements.iter().find(|e| e.id == id)?.kind;

        // The rotate handle floats above every rotatable element (not text/cards).
        if rotatable(kind) {
            let (rx, ry) = rotate_handle_screen(kind, cam, origin);
            let (dx, dy) = (f32::from(pos.x) - rx, f32::from(pos.y) - ry);
            if dx * dx + dy * dy <= HANDLE_GRAB * HANDLE_GRAB {
                return Some(HandleGrab::Rotate);
            }
        }

        if let ElementKind::Line(s) | ElementKind::Arrow(s) = kind {
            for (which, (wx, wy)) in [(s.x1, s.y1), (s.x2, s.y2)].into_iter().enumerate() {
                if near(wx, wy, 0.0, 0.0) {
                    return Some(HandleGrab::Endpoint(EndpointDrag { id, which }));
                }
            }
            return None;
        }

        // Box-like (rect/ellipse/text): corners on the (possibly rotated) box.
        // Upright resizes about the opposite corner (free aspect ratio); rotated
        // resizes proportionally about the center — a similarity transform that
        // stays correct under rotation (set up here, applied in `on_move`).
        if let Some((x, y, w, h, rot)) = box_like(kind) {
            let z = cam.zoom.max(MIN_ZOOM);
            let cu = box_padded_corners(x, y, w, h, rot, 0.0);
            let cp = box_padded_corners(x, y, w, h, rot, SEL_PAD_PX / z);
            let center = [x + w / 2.0, y + h / 2.0];
            let rotated = rot.abs() > ROT_EPS;
            for i in 0..4 {
                if near(cp[i][0], cp[i][1], 0.0, 0.0) {
                    let anchor = if rotated { center } else { cu[(i + 2) % 4] };
                    return Some(HandleGrab::Corner(Resizing {
                        id,
                        anchor,
                        from: cu[i],
                        grab: [cu[i][0] - cursor[0], cu[i][1] - cursor[1]],
                        orig: kind.clone(),
                    }));
                }
            }
            return None;
        }

        // Draw / Embed: corners on the padded AABB (offset the hit to match).
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

    /// The topmost page-card under a world point: `(element id, page id)`.
    fn embed_at(&self, p: [f32; 2], pad: f32) -> Option<(u64, i64)> {
        self.scene
            .elements
            .iter()
            .rev()
            .find_map(|e| match &e.kind {
                ElementKind::Embed(em) if hit_test(&e.kind, p[0], p[1], pad) => {
                    Some((e.id, em.page_id))
                }
                _ => None,
            })
    }

    fn on_left_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.panning {
            return;
        }

        // A press dismisses an open right-click menu (its own button is occluded,
        // so a press reaching here is outside it).
        if self.context_menu.take().is_some() {
            cx.notify();
            return;
        }
        // A press on the canvas closes an open tool flyout (the flyout itself is
        // occluded, so a press reaching here is outside it).
        if self.open_group.is_some() {
            self.open_group = None;
            cx.notify();
            return;
        }

        // The color picker takes input priority while open. Its draggable regions
        // (SV square, hue strip) start a drag here; presses on the rest of the
        // panel are consumed (the swatch / Auto buttons fire via their own
        // `on_click`); a press anywhere else closes it.
        if self.picker.is_some() {
            let pos = ev.position;
            if self.sv_bounds.get().contains(&pos) {
                if !self.selected.is_empty() {
                    self.push_undo();
                }
                self.picker_drag = Some(PickerDrag::Sv);
                let (s, v) = self.sv_from_pos(pos);
                if let Some(p) = self.picker.as_mut() {
                    (p.s, p.v) = (s, v);
                }
                if let Some(c) = self.picker_u32() {
                    self.set_color_live(Some(c), cx);
                }
                return;
            }
            if self.hue_bounds.get().contains(&pos) {
                if !self.selected.is_empty() {
                    self.push_undo();
                }
                self.picker_drag = Some(PickerDrag::Hue);
                let h = self.frac_x(self.hue_bounds.get(), pos);
                if let Some(p) = self.picker.as_mut() {
                    p.h = h;
                }
                if let Some(c) = self.picker_u32() {
                    self.set_color_live(Some(c), cx);
                }
                return;
            }
            if self.alpha_bounds.get().contains(&pos) {
                if !self.selected.is_empty() {
                    self.push_undo();
                }
                self.picker_drag = Some(PickerDrag::Alpha);
                let a = self.frac_x(self.alpha_bounds.get(), pos);
                if let Some(p) = self.picker.as_mut() {
                    p.a = a;
                }
                if let Some(c) = self.picker_u32() {
                    self.set_color_live(Some(c), cx);
                }
                return;
            }
            if self.picker_bounds.get().contains(&pos) {
                return;
            }
            self.picker = None;
            cx.notify();
            return;
        }

        // Take keyboard focus so the board's shortcuts (tool keys, ⌫, ⌘Z…) work
        // after a click on the canvas.
        self.focus.focus(window, cx);

        let p = self.event_to_world(ev.position);
        let zoom = self.scene.camera.zoom.max(MIN_ZOOM);

        // Any click first commits an in-progress text edit.
        if self.editing.is_some() {
            self.commit_text(window, cx);
        }

        if ev.click_count >= 2 {
            self.pending = None;
            if self.tool == Tool::Select {
                // Double-click a text element re-opens it for editing.
                if let Some(id) = self.text_at(p, SELECT_PAD / zoom) {
                    self.selected = vec![id];
                    self.editing = Some(id);
                    self.focus.focus(window, cx);
                    cx.notify();
                    return;
                }
                // Double-click a page-card opens its page.
                if let Some((id, page_id)) = self.embed_at(p, SELECT_PAD / zoom) {
                    self.selected = vec![id];
                    if let Some(f) = self.on_open.clone() {
                        f(page_id, window, cx);
                    }
                    cx.notify();
                    return;
                }
            }
            self.reset_view(cx);
            return;
        }

        // Pan tool: a left-drag pans the canvas (the default navigation tool;
        // double-click above still recenters). Reuses the middle-drag machinery.
        if self.tool == Tool::Pan {
            self.panning = true;
            self.last = ev.position;
            return;
        }

        if self.tool == Tool::Text {
            // Edit a text under the cursor, else create a new one here.
            if let Some(id) = self.text_at(p, SELECT_PAD / zoom) {
                self.selected = vec![id];
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
                        rotation: 0.0,
                        measured_w: 0.0,
                        measured_h: 0.0,
                    }),
                    stroke: self.active_stroke,
                    fill: None,
                });
                self.selected = vec![id];
                self.editing = Some(id);
                self.dirty = true;
            }
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }

        if self.tool == Tool::Embed {
            // The host picks a page, then calls back into `add_embed`.
            if let Some(f) = self.on_place_embed.clone() {
                f(p[0], p[1], window, cx);
            }
            return;
        }

        if self.tool == Tool::Image {
            // The host picks an image file, then calls back into `add_image_at`.
            if let Some(f) = self.on_place_image.clone() {
                f(p[0], p[1], window, cx);
            }
            return;
        }

        if self.tool == Tool::Select {
            // A handle on the current selection takes priority.
            if let Some(grab) = self.handle_hit(ev.position) {
                self.push_undo();
                match grab {
                    HandleGrab::Corner(rs) => self.resizing = Some(rs),
                    HandleGrab::GroupCorner(gr) => self.group_resizing = Some(gr),
                    HandleGrab::Endpoint(ep) => self.endpoint = Some(ep),
                    HandleGrab::Rotate => {
                        // Pivot = the whole selection's bounds center (a single
                        // element's own center, or the group's). Snap on the lone
                        // element's orientation, or — for a group — the first
                        // oriented member's, so it squares to horizontal/vertical
                        // (falling back to quarter-turns if nothing's oriented).
                        if let Some(bb) = self.selection_bbox() {
                            let center = [(bb.0 + bb.2) / 2.0, (bb.1 + bb.3) / 2.0];
                            let base = match self.selected_single() {
                                Some(id) => self
                                    .scene
                                    .elements
                                    .iter()
                                    .find(|e| e.id == id)
                                    .and_then(|e| reference_angle(&e.kind)),
                                None => self
                                    .scene
                                    .elements
                                    .iter()
                                    .filter(|e| self.is_selected(e.id))
                                    .find_map(|e| reference_angle(&e.kind))
                                    .or(Some(0.0)),
                            };
                            let start_pointer = (p[1] - center[1]).atan2(p[0] - center[0]);
                            self.rotating = Some(Rotating {
                                center,
                                start_pointer,
                                applied: 0.0,
                                base,
                            });
                        }
                    }
                }
                cx.notify();
                return;
            }
            // Otherwise hit-test topmost-first.
            let pad = SELECT_PAD / zoom;
            let hit = self
                .scene
                .elements
                .iter()
                .rev()
                .find(|e| hit_test(&e.kind, p[0], p[1], pad))
                .map(|e| e.id);
            match hit {
                Some(id) if ev.modifiers.shift => {
                    // Shift-click toggles membership (no move).
                    if let Some(pos) = self.selected.iter().position(|&s| s == id) {
                        self.selected.remove(pos);
                    } else {
                        self.selected.push(id);
                    }
                    self.drag_from = None;
                }
                Some(id) => {
                    // Click an unselected element selects only it; clicking one
                    // already in the selection keeps the group (so a drag moves
                    // them all). Either way, arm a move.
                    if !self.is_selected(id) {
                        self.selected = vec![id];
                    }
                    self.drag_from = Some(p);
                    // Capture the primary element's top-left so the move can drive
                    // an absolute target (and snap it) without drifting.
                    self.move_origin = self
                        .selected
                        .first()
                        .and_then(|&pid| self.scene.elements.iter().find(|e| e.id == pid))
                        .map(|e| {
                            let (x, y, ..) = bbox(&e.kind);
                            [x, y]
                        })
                        .unwrap_or(p);
                    self.moved = false;
                }
                None => {
                    // Empty space: clear (unless extending) and start a marquee.
                    if !ev.modifiers.shift {
                        self.selected.clear();
                    }
                    self.marquee = Some((p, p));
                    self.drag_from = None;
                }
            }
            cx.notify();
            return;
        }

        let width = NIB / zoom;
        // While the snap modifier (Option) is held, start the shape on a grid
        // line; the move handler snaps the opposite corner / endpoint too.
        let anchor = if ev.modifiers.alt {
            [snap_grid(p[0]), snap_grid(p[1])]
        } else {
            p
        };
        // A zero-size box anchored at the press; the move handler grows it.
        let box0 = BoxGeom {
            x: anchor[0],
            y: anchor[1],
            w: 0.0,
            h: 0.0,
            width,
            rotation: 0.0,
        };
        let kind = match self.tool {
            // Freehand keeps the raw point — strokes aren't grid-aligned.
            Tool::Pen => ElementKind::Draw(Stroke {
                points: vec![p],
                width,
            }),
            Tool::Rect => ElementKind::Rect(box0),
            Tool::Ellipse => ElementKind::Ellipse(box0),
            Tool::Diamond => ElementKind::Diamond(box0),
            Tool::Triangle => ElementKind::Triangle(box0),
            Tool::RoundRect => ElementKind::RoundRect(box0),
            Tool::Star => ElementKind::Star(box0),
            Tool::Hexagon => ElementKind::Hexagon(box0),
            Tool::Line => ElementKind::Line(SegGeom {
                x1: anchor[0],
                y1: anchor[1],
                x2: anchor[0],
                y2: anchor[1],
                width,
            }),
            Tool::Arrow => ElementKind::Arrow(SegGeom {
                x1: anchor[0],
                y1: anchor[1],
                x2: anchor[0],
                y2: anchor[1],
                width,
            }),
            // These tools don't create a drag-element here (handled earlier).
            Tool::Pan | Tool::Select | Tool::Text | Tool::Embed | Tool::Image => return,
        };
        self.pending = Some(Pending { anchor, kind });
        cx.notify();
    }

    fn on_left_up(&mut self, _ev: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        // End a Pan-tool drag (left-button pan).
        if self.panning {
            self.panning = false;
            cx.notify();
            self.flush(window, cx);
            return;
        }
        // End a picker drag: the live changes are already applied; just persist.
        if self.picker_drag.take().is_some() {
            self.flush(window, cx);
            return;
        }
        if self.resizing.take().is_some()
            || self.group_resizing.take().is_some()
            || self.endpoint.take().is_some()
            || self.rotating.take().is_some()
        {
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
        // Finish a marquee: add every element whose bounds intersect the box.
        if let Some((a, b)) = self.marquee.take() {
            let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
            let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
            for e in &self.scene.elements {
                let bb = bbox(&e.kind);
                let hits = bb.0 <= x1 && bb.2 >= x0 && bb.1 <= y1 && bb.3 >= y0;
                if hits && !self.selected.contains(&e.id) {
                    self.selected.push(e.id);
                }
            }
            cx.notify();
            return;
        }
        if let Some(pending) = self.pending.take() {
            if committable(&pending.kind) {
                self.push_undo();
                let id = self.next_id;
                self.next_id += 1;
                // Fill applies only to closed shapes.
                let fill = if is_closed_shape(&pending.kind) {
                    self.active_fill
                } else {
                    None
                };
                self.scene.elements.push(Element {
                    id,
                    kind: pending.kind,
                    stroke: self.active_stroke,
                    fill,
                });
                self.dirty = true;
            }
            cx.notify();
        }
        self.flush(window, cx);
    }

    /// Right-click: with a selection (and a host save hook), open a small menu to
    /// save it as a template; otherwise just dismiss any open menu.
    fn on_right_down(&mut self, ev: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // Show the menu when there's a selection (copy / cut / z-order / save) or
        // paste is wired (so you can paste onto empty canvas). Positioned at the click.
        if self.selected.is_empty() && self.on_paste.is_none() {
            self.context_menu = None;
        } else {
            let b = self.bounds.get();
            self.context_menu = Some(point(
                ev.position.x - b.origin.x,
                ev.position.y - b.origin.y,
            ));
        }
        cx.notify();
    }

    /// Paste board elements from the clipboard (via the host's `on_paste` hook),
    /// centered + selected. Returns whether anything was pasted, so ⌘V can fall
    /// through to image paste when the clipboard holds no board elements.
    fn try_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(f) = self.on_paste.clone()
            && let Some(json) = f(window, cx)
        {
            self.paste_elements(&json, window, cx);
            true
        } else {
            false
        }
    }

    /// Context-menu Paste.
    fn paste_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.try_paste(window, cx);
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
            || self.group_resizing.is_some()
            || self.endpoint.is_some()
            || self.rotating.is_some()
            || self.picker_drag.is_some()
            || self.marquee.is_some()
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

    fn on_move(&mut self, ev: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // Dragging inside the color picker (SV square, hue strip, alpha strip).
        if let Some(drag) = self.picker_drag {
            let pos = ev.position;
            match drag {
                PickerDrag::Sv => {
                    let (s, v) = self.sv_from_pos(pos);
                    if let Some(p) = self.picker.as_mut() {
                        (p.s, p.v) = (s, v);
                    }
                }
                PickerDrag::Hue => {
                    let h = self.frac_x(self.hue_bounds.get(), pos);
                    if let Some(p) = self.picker.as_mut() {
                        p.h = h;
                    }
                }
                PickerDrag::Alpha => {
                    let a = self.frac_x(self.alpha_bounds.get(), pos);
                    if let Some(p) = self.picker.as_mut() {
                        p.a = a;
                    }
                }
            }
            if let Some(c) = self.picker_u32() {
                self.set_color_live(Some(c), cx);
            }
            return;
        }
        // Rotating the selection (rotate-handle drag). Shift snaps to 15° steps.
        if let Some(mut rot) = self.rotating.take() {
            let cur = self.event_to_world(ev.position);
            let ang = (cur[1] - rot.center[1]).atan2(cur[0] - rot.center[0]);
            let mut total = ang - rot.start_pointer;
            match rot.base {
                // Box/text/line: work in absolute orientation so Shift gives
                // clean 15° angles and, unmodified, it snaps to horizontal /
                // vertical when within ROT_SNAP (the easy-squaring the user wants).
                Some(base) => total = snap_angle(base + total, ev.modifiers.shift) - base,
                // Freehand: no absolute orientation; Shift still steps relatively.
                None => {
                    if ev.modifiers.shift {
                        let step = std::f32::consts::PI / 12.0;
                        total = (total / step).round() * step;
                    }
                }
            }
            // Apply only the change since last frame, normalized to [-π, π] so the
            // atan2 wrap-around at ±π doesn't spin the element a full turn.
            let tau = std::f32::consts::TAU;
            let mut delta = total - rot.applied;
            delta -= (delta / tau).round() * tau;
            // Every selected element turns about the shared pivot (a single
            // selection is just the one, pivoting on its own center).
            let sel = self.selected.clone();
            for e in self.scene.elements.iter_mut() {
                if sel.contains(&e.id) {
                    rotate_element(&mut e.kind, rot.center[0], rot.center[1], delta);
                }
            }
            rot.applied += delta;
            self.rotating = Some(rot);
            cx.notify();
            return;
        }
        // Resizing a multi-selection by a group-bounds corner: scale every
        // member uniformly (proportional) about the opposite corner, each from
        // its geometry at grab so the scaling never compounds.
        if let Some(gr) = self.group_resizing.take() {
            let cur = self.event_to_world(ev.position);
            let mut target = [cur[0] + gr.grab[0], cur[1] + gr.grab[1]];
            if ev.modifiers.alt {
                target = [snap_grid(target[0]), snap_grid(target[1])];
            }
            let s = diagonal_scale(gr.anchor, gr.from, target);
            let font = self.font.clone();
            for (id, orig) in &gr.orig {
                let mut kind = orig.clone();
                resize_about(&mut kind, gr.anchor[0], gr.anchor[1], s, s);
                if let ElementKind::Text(t) = &mut kind {
                    let (w, h) = font.measure(&t.content, t.size);
                    (t.measured_w, t.measured_h) = (w, h);
                }
                if let Some(e) = self.scene.elements.iter_mut().find(|e| e.id == *id) {
                    e.kind = kind;
                }
            }
            self.group_resizing = Some(gr);
            cx.notify();
            return;
        }
        // Resizing the selection (corner-handle drag).
        if let Some(r) = self.resizing.as_ref() {
            let (id, anchor, from, grab, mut kind) =
                (r.id, r.anchor, r.from, r.grab, r.orig.clone());
            let cur = self.event_to_world(ev.position);
            // Where the dragged corner should sit: cursor + the grab offset, so
            // it tracks the cursor without jumping when the drag starts. The snap
            // modifier (Option) lands that corner on the grid.
            let mut target = [cur[0] + grab[0], cur[1] + grab[1]];
            if ev.modifiers.alt {
                target = [snap_grid(target[0]), snap_grid(target[1])];
            }
            // Text and images always scale proportionally (text is a single font
            // size; an image would distort otherwise); Shift does so for shapes;
            // and a *rotated* box-like element must (its anchor is the center, so
            // a uniform scale keeps it correct under rotation). Both use the
            // diagonal projection so the corner tracks the cursor at the right
            // rate. Free resize is per-axis.
            let rotated = box_like(&kind).is_some_and(|(.., r)| r.abs() > ROT_EPS);
            let proportional = ev.modifiers.shift
                || rotated
                || matches!(kind, ElementKind::Text(_) | ElementKind::Image(_));
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
            let font = self.font.clone();
            if let Some(e) = self.scene.elements.iter_mut().find(|e| e.id == id) {
                e.kind = kind;
                // Re-measure text now so its box tracks the cursor this frame.
                if let ElementKind::Text(t) = &mut e.kind {
                    let (w, h) = font.measure(&t.content, t.size);
                    t.measured_w = w;
                    t.measured_h = h;
                }
            }
            cx.notify();
            return;
        }
        // Dragging a line/arrow endpoint (Shift snaps the angle to 45°, Option
        // snaps the endpoint to the grid).
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
                } else if ev.modifiers.alt {
                    (snap_grid(cur[0]), snap_grid(cur[1]))
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
        // Moving the selection (all selected elements together). The target is
        // the primary's grab position plus the *total* cursor delta from the
        // fixed grab anchor; the snap modifier (Option) rounds that target to the
        // grid. Computing the absolute target each frame (vs. snapping the
        // per-frame delta) keeps the shape under the cursor and never loses
        // sub-grid motion — so it moves on every axis, not just one.
        if let Some(from) = self.drag_from {
            let cur = self.event_to_world(ev.position);
            let target = move_target(self.move_origin, from, cur, ev.modifiers.alt);
            // Where the primary sits now → the delta to apply this frame. Every
            // element kind's bbox-min translates 1:1, so this tracks exactly.
            let cur_min = self
                .selected
                .first()
                .and_then(|&pid| self.scene.elements.iter().find(|e| e.id == pid))
                .map(|e| {
                    let (x, y, ..) = bbox(&e.kind);
                    [x, y]
                })
                .unwrap_or(self.move_origin);
            let (dx, dy) = (target[0] - cur_min[0], target[1] - cur_min[1]);
            if dx != 0.0 || dy != 0.0 {
                if !self.moved {
                    self.push_undo();
                    self.moved = true;
                }
                let sel = self.selected.clone();
                for e in self.scene.elements.iter_mut() {
                    if sel.contains(&e.id) {
                        translate(&mut e.kind, dx, dy);
                    }
                }
                cx.notify();
            }
            return;
        }
        // Dragging a marquee box (started on empty space).
        if let Some((start, _)) = self.marquee {
            let cur = self.event_to_world(ev.position);
            self.marquee = Some((start, cur));
            cx.notify();
            return;
        }
        // Creating an element.
        if self.pending.is_some() {
            let cur = self.event_to_world(ev.position);
            let z = self.scene.camera.zoom.max(MIN_ZOOM);
            let Some(pending) = self.pending.as_mut() else {
                return;
            };
            let anchor = pending.anchor;
            // Snap the growing corner / endpoint to the grid while Option is held
            // (freehand strokes keep the raw point).
            let c = if ev.modifiers.alt {
                [snap_grid(cur[0]), snap_grid(cur[1])]
            } else {
                cur
            };
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
                ElementKind::Rect(b)
                | ElementKind::Ellipse(b)
                | ElementKind::Diamond(b)
                | ElementKind::Triangle(b)
                | ElementKind::RoundRect(b)
                | ElementKind::Star(b)
                | ElementKind::Hexagon(b) => {
                    b.x = anchor[0].min(c[0]);
                    b.y = anchor[1].min(c[1]);
                    b.w = (c[0] - anchor[0]).abs();
                    b.h = (c[1] - anchor[1]).abs();
                }
                ElementKind::Line(s) | ElementKind::Arrow(s) => {
                    s.x2 = c[0];
                    s.y2 = c[1];
                }
                // Text/cards/images aren't created by dragging, never pending here.
                ElementKind::Text(_) | ElementKind::Embed(_) | ElementKind::Image(_) => {}
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

    /// Handle a board keyboard shortcut (the board has focus and isn't editing
    /// text). Returns whether the key was consumed. Single letters pick a tool;
    /// ⌫/Del clears the selection's elements; ⌘Z / ⌘⇧Z undo / redo; ⌘C / ⌘X / ⌘V
    /// copy / cut / paste; ⌘] / ⌘[ (± ⇧) reorder z-order; Esc deselects. ⌘V with no
    /// copied elements and other modified chords (⌘W, …) pass through to the host.
    fn handle_shortcut(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let ks = &ev.keystroke;
        let cmd = ks.modifiers.platform || ks.modifiers.control;
        if cmd && ks.key == "z" {
            if ks.modifiers.shift {
                self.redo(window, cx);
            } else {
                self.undo(window, cx);
            }
            return true;
        }
        // Z-order: ⌘] / ⌘[ nudge one step, ⌘⇧] / ⌘⇧[ go all the way. Some keymaps
        // report the shifted bracket as `}` / `{`, so treat that as "all the way"
        // too. Only consumed when something is selected.
        let close = ks.key == "]" || ks.key == "}";
        let open = ks.key == "[" || ks.key == "{";
        if cmd && (close || open) {
            if self.selected.is_empty() {
                return false;
            }
            let all_the_way = ks.modifiers.shift || ks.key == "}" || ks.key == "{";
            let op = match (close, all_the_way) {
                (true, true) => ZOrder::ToFront,
                (true, false) => ZOrder::Forward,
                (false, true) => ZOrder::ToBack,
                (false, false) => ZOrder::Backward,
            };
            self.reorder_selection(op, window, cx);
            return true;
        }
        // Copy / cut the selection to the clipboard (the host's `on_copy` writes
        // it). ⌘V paste is left to propagate so the host can read the clipboard and
        // prefer elements over an image. ⌘C/⌘X are consumed even with nothing
        // selected, so they never fall through to a text copy on the board.
        if cmd && ks.key == "c" {
            self.copy_selection(window, cx);
            return true;
        }
        if cmd && ks.key == "x" {
            if self.copy_selection(window, cx) {
                self.delete_selected(window, cx);
            }
            return true;
        }
        if cmd && ks.key == "v" {
            // Paste copied elements; if the clipboard holds none, fall through so
            // the host can paste a clipboard image instead.
            return self.try_paste(window, cx);
        }
        if cmd || ks.modifiers.alt {
            return false;
        }
        if let Some(tool) = Tool::shortcut(&ks.key) {
            self.set_tool(tool, cx);
            return true;
        }
        match ks.key.as_str() {
            "backspace" | "delete" => self.delete_selected(window, cx),
            "escape" if !self.selected.is_empty() => {
                self.selected.clear();
                cx.notify();
            }
            _ => return false,
        }
        true
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Escape closes an open color picker or the templates modal (when the
        // board holds focus).
        if ev.keystroke.key == "escape" && (self.picker.is_some() || self.templates_open) {
            self.picker = None;
            self.templates_open = false;
            cx.notify();
            return;
        }
        // Not editing text → keys are board shortcuts (tools, delete, undo/redo).
        let Some(id) = self.editing else {
            if !self.handle_shortcut(ev, window, cx) {
                cx.propagate();
            }
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

/// How [`WhiteboardView::reorder_selection`] moves the selection through the
/// paint order (`elements` order; later = on top).
#[derive(Clone, Copy)]
enum ZOrder {
    ToFront,
    Forward,
    Backward,
    ToBack,
}

/// Whether an in-progress element is big enough to keep (a click that doesn't
/// drag leaves nothing).
fn committable(kind: &ElementKind) -> bool {
    match kind {
        ElementKind::Draw(s) => s.points.len() >= 2,
        ElementKind::Rect(b)
        | ElementKind::Ellipse(b)
        | ElementKind::Diamond(b)
        | ElementKind::Triangle(b)
        | ElementKind::RoundRect(b)
        | ElementKind::Star(b)
        | ElementKind::Hexagon(b) => b.w > 1.0 || b.h > 1.0,
        ElementKind::Line(s) | ElementKind::Arrow(s) => {
            let (dx, dy) = (s.x2 - s.x1, s.y2 - s.y1);
            dx * dx + dy * dy > 4.0
        }
        // Text / cards / images are placed on click (not via a drag), never pending.
        ElementKind::Text(_) | ElementKind::Embed(_) | ElementKind::Image(_) => false,
    }
}

/// A closed shape whose interior can take a fill — every box-like polygon
/// (rect / rounded-rect / ellipse / diamond / triangle / hexagon / star), but
/// not open kinds (pen / line / arrow / text / card).
fn is_closed_shape(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Rect(_)
            | ElementKind::Ellipse(_)
            | ElementKind::Diamond(_)
            | ElementKind::Triangle(_)
            | ElementKind::RoundRect(_)
            | ElementKind::Star(_)
            | ElementKind::Hexagon(_)
    )
}

// --- color ----------------------------------------------------------------
//
// Element colors are stored as packed `0xRRGGBBAA` so the scene JSON stays
// dependency-free. The picker works in HSV (the usual hue / saturation /
// brightness controls); these convert between HSV, packed ints, and gpui's
// `Hsla` for painting.

/// Pack 0..1 RGBA components into `0xRRGGBBAA`.
fn pack_rgba(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let q = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u32;
    (q(r) << 24) | (q(g) << 16) | (q(b) << 8) | q(a)
}

/// A packed color as a gpui `Hsla`, for painting.
fn u32_to_hsla(c: u32) -> Hsla {
    rgba(c).into()
}

/// A gpui `Hsla` packed into `0xRRGGBBAA` (used to store theme swatches).
fn hsla_to_u32(c: Hsla) -> u32 {
    let r = Rgba::from(c);
    pack_rgba(r.r, r.g, r.b, r.a)
}

/// HSV (each 0..1) → RGB (each 0..1).
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// RGB (each 0..1) → HSV (each 0..1).
fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, v)
}

/// HSV (each 0..1) packed into an opaque `0xRRGGBBff`.
fn hsv_to_u32(h: f32, s: f32, v: f32) -> u32 {
    hsva_to_u32(h, s, v, 1.0)
}

/// HSVA (each 0..1) packed into `0xRRGGBBAA`.
fn hsva_to_u32(h: f32, s: f32, v: f32, a: f32) -> u32 {
    let (r, g, b) = hsv_to_rgb(h, s, v);
    pack_rgba(r, g, b, a)
}

/// A packed color's HSV (alpha dropped).
fn u32_to_hsv(c: u32) -> (f32, f32, f32) {
    let p = rgba(c);
    rgb_to_hsv(p.r, p.g, p.b)
}

/// A packed color's alpha as 0..1.
fn u32_alpha(c: u32) -> f32 {
    (c & 0xff) as f32 / 255.0
}

/// Rotate `(x, y)` by `a` radians about `(cx, cy)`.
fn rotate_pt(x: f32, y: f32, cx: f32, cy: f32, a: f32) -> (f32, f32) {
    if a == 0.0 {
        return (x, y);
    }
    let (s, c) = a.sin_cos();
    let (dx, dy) = (x - cx, y - cy);
    (cx + dx * c - dy * s, cy + dx * s + dy * c)
}

/// The unrotated box `(x, y, w, h)` plus rotation of a "box-like" element —
/// rect, ellipse, or text (whose size is its measured extent). `None` for the
/// other kinds. Lets the rotation/selection/resize code treat all three alike.
fn box_like(kind: &ElementKind) -> Option<(f32, f32, f32, f32, f32)> {
    match kind {
        ElementKind::Rect(b)
        | ElementKind::Ellipse(b)
        | ElementKind::Diamond(b)
        | ElementKind::Triangle(b)
        | ElementKind::RoundRect(b)
        | ElementKind::Star(b)
        | ElementKind::Hexagon(b) => Some((b.x, b.y, b.w, b.h, b.rotation)),
        ElementKind::Text(t) => {
            let (w, h) = text_extent(t);
            Some((t.x, t.y, w, h, t.rotation))
        }
        // Images rotate only in quarter turns, so the selection box (and bbox)
        // snap to 90° too — keeping the box aligned with the rendered bitmap.
        ElementKind::Image(im) => Some((im.x, im.y, im.w, im.h, snap_quarter(im.rotation))),
        _ => None,
    }
}

/// The four world-space corners of a box (TL, TR, BR, BL order), grown outward
/// by `pad` on every side and spun by `rotation` about its center.
fn box_padded_corners(x: f32, y: f32, w: f32, h: f32, rotation: f32, pad: f32) -> [[f32; 2]; 4] {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (x0, y0) = (x - pad, y - pad);
    let (x1, y1) = (x + w + pad, y + h + pad);
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|[px_, py_]| {
        let (rx, ry) = rotate_pt(px_, py_, cx, cy, rotation);
        [rx, ry]
    })
}

/// Axis-aligned bounds of a set of points (empty → a zero box at the origin).
fn aabb(pts: &[[f32; 2]]) -> (f32, f32, f32, f32) {
    if pts.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let (mut x0, mut y0) = (f32::MAX, f32::MAX);
    let (mut x1, mut y1) = (f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    (x0, y0, x1, y1)
}

/// Whether an element can be rotated. Page-cards are HTML overlays GPUI can't
/// transform, so they're excluded (the rotate handle never shows for them).
fn rotatable(kind: &ElementKind) -> bool {
    !matches!(kind, ElementKind::Embed(_))
}

/// Snap an absolute orientation (radians) while rotating: with `shift`, to the
/// nearest 15°; otherwise to horizontal/vertical when within [`ROT_SNAP`], else
/// left free so any angle is still reachable away from the cardinals.
fn snap_angle(abs: f32, shift: bool) -> f32 {
    if shift {
        let step = std::f32::consts::PI / 12.0;
        return (abs / step).round() * step;
    }
    let quarter = std::f32::consts::FRAC_PI_2;
    let card = (abs / quarter).round() * quarter;
    if (abs - card).abs() < ROT_SNAP {
        card
    } else {
        abs
    }
}

/// An element's absolute orientation for cardinal-snapping while rotating: a
/// box/text angle, or a line/arrow's direction. `None` for freehand strokes
/// (which have no meaningful single orientation).
fn reference_angle(kind: &ElementKind) -> Option<f32> {
    match kind {
        ElementKind::Rect(b)
        | ElementKind::Ellipse(b)
        | ElementKind::Diamond(b)
        | ElementKind::Triangle(b)
        | ElementKind::RoundRect(b)
        | ElementKind::Star(b)
        | ElementKind::Hexagon(b) => Some(b.rotation),
        ElementKind::Text(t) => Some(t.rotation),
        ElementKind::Image(im) => Some(snap_quarter(im.rotation)),
        ElementKind::Line(s) | ElementKind::Arrow(s) => Some((s.y2 - s.y1).atan2(s.x2 - s.x1)),
        ElementKind::Draw(_) | ElementKind::Embed(_) => None,
    }
}

/// Rotate an element by `delta` radians about a fixed pivot `(cx, cy)`. A
/// box/text/card's *center* orbits the pivot and (for the rotatable ones) its
/// own angle accumulates; lines/strokes bake the rotation into their points. For
/// a single-element rotation the pivot is the element's own center, so the orbit
/// is a no-op and it just spins in place; for a group it's the shared center, so
/// the whole selection turns as one.
fn rotate_element(kind: &mut ElementKind, cx: f32, cy: f32, delta: f32) {
    // Orbit a box's top-left so its center lands where the pivot rotation sends
    // it; returns the new top-left.
    let orbit = |x: f32, y: f32, w: f32, h: f32| {
        let (nx, ny) = rotate_pt(x + w / 2.0, y + h / 2.0, cx, cy, delta);
        (nx - w / 2.0, ny - h / 2.0)
    };
    match kind {
        ElementKind::Rect(b)
        | ElementKind::Ellipse(b)
        | ElementKind::Diamond(b)
        | ElementKind::Triangle(b)
        | ElementKind::RoundRect(b)
        | ElementKind::Star(b)
        | ElementKind::Hexagon(b) => {
            (b.x, b.y) = orbit(b.x, b.y, b.w, b.h);
            b.rotation += delta;
        }
        ElementKind::Text(t) => {
            let (w, h) = text_extent(t);
            (t.x, t.y) = orbit(t.x, t.y, w, h);
            t.rotation += delta;
        }
        ElementKind::Line(s) | ElementKind::Arrow(s) => {
            let (x1, y1) = rotate_pt(s.x1, s.y1, cx, cy, delta);
            let (x2, y2) = rotate_pt(s.x2, s.y2, cx, cy, delta);
            (s.x1, s.y1, s.x2, s.y2) = (x1, y1, x2, y2);
        }
        ElementKind::Draw(st) => {
            for p in &mut st.points {
                let (x, y) = rotate_pt(p[0], p[1], cx, cy, delta);
                (p[0], p[1]) = (x, y);
            }
        }
        // A card can't tilt (it's an HTML overlay), but in a group it orbits the
        // pivot so the selection moves together.
        ElementKind::Embed(em) => (em.x, em.y) = orbit(em.x, em.y, em.w, em.h),
        // An image's center orbits and its angle accumulates (the host re-rotates
        // the bitmap to match on release).
        ElementKind::Image(im) => {
            (im.x, im.y) = orbit(im.x, im.y, im.w, im.h);
            im.rotation += delta;
        }
    }
}

/// Screen position of a rotate handle: above a bounds' top-center. `handle_hit`
/// and `paint_selection` agree via this one source (for single elements and for
/// a group, whose bounds is the union of the selection).
fn rotate_handle_for_bbox(
    bb: (f32, f32, f32, f32),
    cam: Camera,
    origin: Point<Pixels>,
) -> (f32, f32) {
    let top = to_screen((bb.0 + bb.2) / 2.0, bb.1, cam, origin);
    (
        f32::from(top.x),
        f32::from(top.y) - SEL_PAD_PX - ROTATE_DIST,
    )
}

/// Screen position of a single element's rotate handle.
fn rotate_handle_screen(kind: &ElementKind, cam: Camera, origin: Point<Pixels>) -> (f32, f32) {
    rotate_handle_for_bbox(bbox(kind), cam, origin)
}

/// An element's world-space bounding box `(min_x, min_y, max_x, max_y)`.
fn bbox(kind: &ElementKind) -> (f32, f32, f32, f32) {
    // Box-like kinds (rect/ellipse/text): AABB of the (possibly rotated) box.
    if let Some((x, y, w, h, rot)) = box_like(kind) {
        return aabb(&box_padded_corners(x, y, w, h, rot, 0.0));
    }
    match kind {
        ElementKind::Draw(s) => aabb(&s.points),
        ElementKind::Line(s) | ElementKind::Arrow(s) => (
            s.x1.min(s.x2),
            s.y1.min(s.y2),
            s.x1.max(s.x2),
            s.y1.max(s.y2),
        ),
        ElementKind::Embed(em) => (em.x, em.y, em.x + em.w, em.y + em.h),
        // Handled above (all box-like kinds go through `box_like`).
        ElementKind::Rect(_)
        | ElementKind::Ellipse(_)
        | ElementKind::Diamond(_)
        | ElementKind::Triangle(_)
        | ElementKind::RoundRect(_)
        | ElementKind::Star(_)
        | ElementKind::Hexagon(_)
        | ElementKind::Text(_)
        | ElementKind::Image(_) => unreachable!(),
    }
}

/// The collective bounding-box size `(w, h)` of a group of elements (0×0 if
/// empty). Used to center a template when it's stamped onto a board.
fn elements_extent(elems: &[Element]) -> (f32, f32) {
    let (mut minx, mut miny, mut maxx, mut maxy) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for e in elems {
        let (x0, y0, x1, y1) = bbox(&e.kind);
        minx = minx.min(x0);
        miny = miny.min(y0);
        maxx = maxx.max(x1);
        maxy = maxy.max(y1);
    }
    if minx.is_finite() {
        (maxx - minx, maxy - miny)
    } else {
        (0.0, 0.0)
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
        ElementKind::Rect(b)
        | ElementKind::Ellipse(b)
        | ElementKind::Diamond(b)
        | ElementKind::Triangle(b)
        | ElementKind::RoundRect(b)
        | ElementKind::Star(b)
        | ElementKind::Hexagon(b) => {
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
        ElementKind::Embed(em) => {
            em.x += dx;
            em.y += dy;
        }
        ElementKind::Image(im) => {
            im.x += dx;
            im.y += dy;
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

/// Round a world coordinate to the nearest [`GRID`] line. Used while the snap
/// modifier (Option) is held during create / move / resize so geometry lands on
/// the visible dot grid — handy for aligning template layouts.
fn snap_grid(v: f32) -> f32 {
    (v / GRID).round() * GRID
}

/// Round an angle (radians) to the nearest quarter turn. Images rotate only in
/// 90° steps — gpui can't transform a raster sprite, so the host re-rotates the
/// pixels, and quarter turns keep that exact (no resampling) and cheap.
fn snap_quarter(rad: f32) -> f32 {
    let q = std::f32::consts::FRAC_PI_2;
    (rad / q).round() * q
}

/// Where a move-drag's primary element should sit: its grab-time top-left
/// (`origin`) plus the *total* cursor delta since the grab `anchor`, optionally
/// snapped to the grid. Driving an absolute target from the total delta (rather
/// than snapping each frame's increment) keeps the shape under the cursor and
/// lets sub-grid motion accumulate across frames instead of sticking.
fn move_target(origin: [f32; 2], anchor: [f32; 2], cursor: [f32; 2], snap: bool) -> [f32; 2] {
    let t = [
        origin[0] + (cursor[0] - anchor[0]),
        origin[1] + (cursor[1] - anchor[1]),
    ];
    if snap {
        [snap_grid(t[0]), snap_grid(t[1])]
    } else {
        t
    }
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
    // Once a render has laid the text out, use the real extent. Before that
    // (e.g. a freshly loaded board, pre-first-paint), fall back to a rough
    // character-count estimate so hit-test/bounds aren't degenerate.
    if t.measured_h > 0.0 {
        return (t.measured_w, t.measured_h);
    }
    let rows = t.content.split('\n').count().max(1) as f32;
    let cols = t
        .content
        .split('\n')
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    (cols * t.size * TEXT_CHAR_W, rows * t.size * TEXT_LINE_H)
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
        ElementKind::Rect(b)
        | ElementKind::Ellipse(b)
        | ElementKind::Diamond(b)
        | ElementKind::Triangle(b)
        | ElementKind::RoundRect(b)
        | ElementKind::Star(b)
        | ElementKind::Hexagon(b) => {
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
        ElementKind::Embed(em) => {
            let (x0, x1) = (fx(em.x), fx(em.x + em.w));
            let (y0, y1) = (fy(em.y), fy(em.y + em.h));
            em.x = x0.min(x1);
            em.w = (x1 - x0).abs();
            em.y = y0.min(y1);
            em.h = (y1 - y0).abs();
        }
        ElementKind::Image(im) => {
            let (x0, x1) = (fx(im.x), fx(im.x + im.w));
            let (y0, y1) = (fy(im.y), fy(im.y + im.h));
            im.x = x0.min(x1);
            im.w = (x1 - x0).abs();
            im.y = y0.min(y1);
            im.h = (y1 - y0).abs();
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

/// One element prepared for the paint closure: its geometry + resolved colors,
/// plus pre-laid-out text outlines for Text elements (the layout needs the font,
/// which the paint closure can't reach, so `render` builds it up front).
struct ElemPaint {
    kind: ElementKind,
    stroke: Hsla,
    fill: Option<Hsla>,
    text: Option<TextOutline>,
}

/// One slice of the board's z-order paint stack. Canvas-drawn elements collect
/// into a `Band` (one canvas); an image or page-card is an `Overlay` div between
/// bands. `render` builds these in `elements` order so paint order = z-order,
/// which lets a shape sit above or below an image. See [`band_canvas`].
enum Layer {
    Band(Vec<ElemPaint>),
    Overlay(gpui::AnyElement),
}

/// A transparent, full-size canvas painting one run of canvas-drawn elements
/// (shapes / lines / pen / text) in order. Stacked between [`Layer::Overlay`]
/// divs so paint order follows the element list.
fn band_canvas(elems: Vec<ElemPaint>, cam: Camera) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            for ep in &elems {
                match &ep.text {
                    Some(t) => paint_text(t, cam, bounds.origin, ep.stroke, window),
                    None => paint_element(&ep.kind, cam, bounds.origin, ep.stroke, ep.fill, window),
                }
            }
        },
    )
    .absolute()
    .size_full()
}

/// A text element's glyph outlines (text-local space) plus placement, captured
/// for the paint closure to transform (camera + rotation) and fill.
struct TextOutline {
    segs: Vec<font::Seg>,
    x: f32,
    y: f32,
    rotation: f32,
    w: f32,
    h: f32,
    line_height: f32,
    /// Caret's text-local top, when this text is being edited.
    caret: Option<[f32; 2]>,
}

/// Paint a text element's vector outlines (and, when editing, its caret). Local
/// glyph points are placed at `(x, y)`, rotated about the block's center, then
/// projected to the screen — so text rotates and scales like the shapes.
fn paint_text(
    t: &TextOutline,
    cam: Camera,
    origin: Point<Pixels>,
    color: Hsla,
    window: &mut Window,
) {
    let (cx, cy) = (t.x + t.w / 2.0, t.y + t.h / 2.0);
    let tf = |p: [f32; 2]| {
        let (rx, ry) = rotate_pt(t.x + p[0], t.y + p[1], cx, cy, t.rotation);
        to_screen(rx, ry, cam, origin)
    };
    // Convert the two-thirds-toward-the-control-point so a quadratic Bézier
    // becomes the equivalent cubic the path builder accepts.
    let two_thirds = |a: Point<Pixels>, b: Point<Pixels>| {
        point(
            px(f32::from(a.x) + (f32::from(b.x) - f32::from(a.x)) * 2.0 / 3.0),
            px(f32::from(a.y) + (f32::from(b.y) - f32::from(a.y)) * 2.0 / 3.0),
        )
    };
    if !t.segs.is_empty() {
        let mut pb = PathBuilder::fill();
        let mut cur = point(px(0.0), px(0.0));
        for seg in &t.segs {
            match *seg {
                font::Seg::Move(p) => {
                    cur = tf(p);
                    pb.move_to(cur);
                }
                font::Seg::Line(p) => {
                    cur = tf(p);
                    pb.line_to(cur);
                }
                font::Seg::Quad(c, e) => {
                    let (sc, se) = (tf(c), tf(e));
                    pb.cubic_bezier_to(se, two_thirds(cur, sc), two_thirds(se, sc));
                    cur = se;
                }
                font::Seg::Cubic(c1, c2, e) => {
                    let se = tf(e);
                    pb.cubic_bezier_to(se, tf(c1), tf(c2));
                    cur = se;
                }
                font::Seg::Close => pb.close(),
            }
        }
        if let Ok(path) = pb.build() {
            window.paint_path(path, color);
        }
    }
    if let Some(cp) = t.caret {
        let mut pb = PathBuilder::stroke(px(1.5));
        pb.move_to(tf(cp));
        pb.line_to(tf([cp[0], cp[1] + t.line_height]));
        if let Ok(path) = pb.build() {
            window.paint_path(path, color);
        }
    }
}

/// Paint one element at the current camera.
fn paint_element(
    kind: &ElementKind,
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    fill: Option<Hsla>,
    window: &mut Window,
) {
    match kind {
        ElementKind::Draw(s) => paint_stroke(&s.points, s.width, cam, origin, ink, window),
        ElementKind::Rect(b) => paint_rect(b, cam, origin, ink, fill, window),
        ElementKind::Ellipse(b) => paint_ellipse(b, cam, origin, ink, fill, window),
        ElementKind::Diamond(b) => {
            paint_box_polygon(b, &DIAMOND_UNIT, cam, origin, ink, fill, window)
        }
        ElementKind::Triangle(b) => {
            paint_box_polygon(b, &TRIANGLE_UNIT, cam, origin, ink, fill, window)
        }
        ElementKind::RoundRect(b) => paint_round_rect(b, cam, origin, ink, fill, window),
        ElementKind::Star(b) => paint_box_polygon(b, &star_unit(), cam, origin, ink, fill, window),
        ElementKind::Hexagon(b) => {
            paint_box_polygon(b, &hexagon_unit(), cam, origin, ink, fill, window)
        }
        ElementKind::Line(s) => paint_segment(s, false, cam, origin, ink, window),
        ElementKind::Arrow(s) => paint_segment(s, true, cam, origin, ink, window),
        // Text / cards / images are drawn as overlay elements in render(), not here.
        ElementKind::Text(_) | ElementKind::Embed(_) | ElementKind::Image(_) => {}
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

fn paint_rect(
    b: &BoxGeom,
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    fill: Option<Hsla>,
    window: &mut Window,
) {
    let z = cam.zoom.max(MIN_ZOOM);
    let c = box_padded_corners(b.x, b.y, b.w, b.h, b.rotation, 0.0);
    let trace = |pb: &mut PathBuilder| {
        pb.move_to(to_screen(c[0][0], c[0][1], cam, origin));
        pb.line_to(to_screen(c[1][0], c[1][1], cam, origin));
        pb.line_to(to_screen(c[2][0], c[2][1], cam, origin));
        pb.line_to(to_screen(c[3][0], c[3][1], cam, origin));
        pb.close();
    };
    if let Some(fill) = fill {
        let mut fb = PathBuilder::fill();
        trace(&mut fb);
        if let Ok(path) = fb.build() {
            window.paint_path(path, fill);
        }
    }
    let mut pb = PathBuilder::stroke(px((b.width * z).max(0.5)));
    trace(&mut pb);
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
}

fn paint_ellipse(
    b: &BoxGeom,
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    fill: Option<Hsla>,
    window: &mut Window,
) {
    let z = cam.zoom.max(MIN_ZOOM);
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let (rx, ry) = (b.w / 2.0, b.h / 2.0);
    const K: f32 = 0.552_284_8;
    let (kx, ky) = (rx * K, ry * K);
    // Every point is rotated about the box center before projection.
    let s = |wx: f32, wy: f32| {
        let (px_, py_) = rotate_pt(wx, wy, cx, cy, b.rotation);
        to_screen(px_, py_, cam, origin)
    };
    let trace = |pb: &mut PathBuilder| {
        pb.move_to(s(cx + rx, cy));
        pb.cubic_bezier_to(s(cx, cy + ry), s(cx + rx, cy + ky), s(cx + kx, cy + ry));
        pb.cubic_bezier_to(s(cx - rx, cy), s(cx - kx, cy + ry), s(cx - rx, cy + ky));
        pb.cubic_bezier_to(s(cx, cy - ry), s(cx - rx, cy - ky), s(cx - kx, cy - ry));
        pb.cubic_bezier_to(s(cx + rx, cy), s(cx + kx, cy - ry), s(cx + rx, cy - ky));
        pb.close();
    };
    if let Some(fill) = fill {
        let mut fb = PathBuilder::fill();
        trace(&mut fb);
        if let Ok(path) = fb.build() {
            window.paint_path(path, fill);
        }
    }
    let mut pb = PathBuilder::stroke(px((b.width * z).max(0.5)));
    trace(&mut pb);
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
}

/// Vertices of box-fitting polygons in box-relative coords: `(±1, ±1)` is the
/// box edge, `(0, 0)` the center. Scaled to the half-extents, rotated about the
/// center, and projected by [`paint_box_polygon`].
const DIAMOND_UNIT: [(f32, f32); 4] = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)];
const TRIANGLE_UNIT: [(f32, f32); 3] = [(0.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];

/// A 5-point star (outer radius 1, inner 0.382), point-up.
fn star_unit() -> [(f32, f32); 10] {
    use std::f32::consts::{FRAC_PI_2, PI};
    const INNER: f32 = 0.382;
    let mut pts = [(0.0, 0.0); 10];
    for (k, p) in pts.iter_mut().enumerate() {
        let a = -FRAC_PI_2 + k as f32 * (PI / 5.0);
        let r = if k % 2 == 0 { 1.0 } else { INNER };
        *p = (a.cos() * r, a.sin() * r);
    }
    pts
}

/// A pointy-top hexagon inscribed in the box's ellipse.
fn hexagon_unit() -> [(f32, f32); 6] {
    use std::f32::consts::{FRAC_PI_2, PI};
    let mut pts = [(0.0, 0.0); 6];
    for (k, p) in pts.iter_mut().enumerate() {
        let a = -FRAC_PI_2 + k as f32 * (PI / 3.0);
        *p = (a.cos(), a.sin());
    }
    pts
}

/// Stroke (and optionally fill) a closed polygon whose `unit` vertices are given
/// in box-relative coords (see [`DIAMOND_UNIT`]). Mirrors [`paint_rect`]: every
/// vertex is scaled to the half-extents, rotated about the box center, and
/// projected to screen.
fn paint_box_polygon(
    b: &BoxGeom,
    unit: &[(f32, f32)],
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    fill: Option<Hsla>,
    window: &mut Window,
) {
    let z = cam.zoom.max(MIN_ZOOM);
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let (rx, ry) = (b.w / 2.0, b.h / 2.0);
    let s = |u: &(f32, f32)| {
        let (wx, wy) = rotate_pt(cx + u.0 * rx, cy + u.1 * ry, cx, cy, b.rotation);
        to_screen(wx, wy, cam, origin)
    };
    let trace = |pb: &mut PathBuilder| {
        let mut it = unit.iter();
        if let Some(first) = it.next() {
            pb.move_to(s(first));
            for u in it {
                pb.line_to(s(u));
            }
            pb.close();
        }
    };
    if let Some(fill) = fill {
        let mut fb = PathBuilder::fill();
        trace(&mut fb);
        if let Ok(path) = fb.build() {
            window.paint_path(path, fill);
        }
    }
    let mut pb = PathBuilder::stroke(px((b.width * z).max(0.5)));
    trace(&mut pb);
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
}

/// A rounded rectangle: straight edges joined by quarter-circle corners (radius
/// = 20% of the shorter side), rotated about the center like [`paint_rect`].
fn paint_round_rect(
    b: &BoxGeom,
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    fill: Option<Hsla>,
    window: &mut Window,
) {
    let z = cam.zoom.max(MIN_ZOOM);
    let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
    let r = b.w.abs().min(b.h.abs()) * 0.2;
    let k = r * 0.552_284_8; // cubic control offset for a quarter circle
    let s = |wx: f32, wy: f32| {
        let (px_, py_) = rotate_pt(wx, wy, cx, cy, b.rotation);
        to_screen(px_, py_, cam, origin)
    };
    let (x0, y0, x1, y1) = (b.x, b.y, b.x + b.w, b.y + b.h);
    let trace = |pb: &mut PathBuilder| {
        // Clockwise from just past the top-left corner.
        pb.move_to(s(x0 + r, y0));
        pb.line_to(s(x1 - r, y0));
        pb.cubic_bezier_to(s(x1, y0 + r), s(x1 - r + k, y0), s(x1, y0 + r - k));
        pb.line_to(s(x1, y1 - r));
        pb.cubic_bezier_to(s(x1 - r, y1), s(x1, y1 - r + k), s(x1 - r + k, y1));
        pb.line_to(s(x0 + r, y1));
        pb.cubic_bezier_to(s(x0, y1 - r), s(x0 + r - k, y1), s(x0, y1 - r + k));
        pb.line_to(s(x0, y0 + r));
        pb.cubic_bezier_to(s(x0 + r, y0), s(x0, y0 + r - k), s(x0 + r - k, y0));
        pb.close();
    };
    if let Some(fill) = fill {
        let mut fb = PathBuilder::fill();
        trace(&mut fb);
        if let Ok(path) = fb.build() {
            window.paint_path(path, fill);
        }
    }
    let mut pb = PathBuilder::stroke(px((b.width * z).max(0.5)));
    trace(&mut pb);
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

/// A filled circular handle (distinct from the square resize handles) marking
/// the rotation grip, centered at a screen point.
fn draw_rotate_handle(hx: f32, hy: f32, color: Hsla, window: &mut Window) {
    let r = HANDLE_HALF + 0.5;
    const K: f32 = 0.552_284_8;
    let k = r * K;
    let p = |x: f32, y: f32| point(px(x), px(y));
    let mut pb = PathBuilder::fill();
    pb.move_to(p(hx + r, hy));
    pb.cubic_bezier_to(p(hx, hy + r), p(hx + r, hy + k), p(hx + k, hy + r));
    pb.cubic_bezier_to(p(hx - r, hy), p(hx - k, hy + r), p(hx - r, hy + k));
    pb.cubic_bezier_to(p(hx, hy - r), p(hx - r, hy - k), p(hx - k, hy - r));
    pb.cubic_bezier_to(p(hx + r, hy), p(hx + k, hy - r), p(hx + r, hy - k));
    pb.close();
    if let Ok(path) = pb.build() {
        window.paint_path(path, color);
    }
}

fn paint_selection(
    kind: &ElementKind,
    cam: Camera,
    origin: Point<Pixels>,
    color: Hsla,
    window: &mut Window,
) {
    // Lines/arrows: a handle at each endpoint (no box — its bbox is degenerate)
    // plus a rotate grip above.
    if let ElementKind::Line(s) | ElementKind::Arrow(s) = kind {
        for (wx, wy) in [(s.x1, s.y1), (s.x2, s.y2)] {
            let p = to_screen(wx, wy, cam, origin);
            draw_handle(f32::from(p.x), f32::from(p.y), color, window);
        }
        let (rx, ry) = rotate_handle_screen(kind, cam, origin);
        draw_rotate_handle(rx, ry, color, window);
        return;
    }
    // Box-like (rect/ellipse/text): the (possibly rotated) box outline + a
    // rotate grip. Corner resize handles show only when upright — a rotated box
    // hides them (rotated-frame resize is out of scope).
    if let Some((x, y, w, h, rot)) = box_like(kind) {
        let z = cam.zoom.max(MIN_ZOOM);
        let s = box_padded_corners(x, y, w, h, rot, SEL_PAD_PX / z)
            .map(|p| to_screen(p[0], p[1], cam, origin));
        let mut pb = PathBuilder::stroke(px(1.5));
        pb.move_to(s[0]);
        pb.line_to(s[1]);
        pb.line_to(s[2]);
        pb.line_to(s[3]);
        pb.close();
        if let Ok(path) = pb.build() {
            window.paint_path(path, color);
        }
        for p in &s {
            draw_handle(f32::from(p.x), f32::from(p.y), color, window);
        }
        let (rx, ry) = rotate_handle_screen(kind, cam, origin);
        draw_rotate_handle(rx, ry, color, window);
        return;
    }
    // Draw / Embed: a padded AABB box + four corner handles. Freehand strokes
    // (rotatable) also get a rotate grip; cards don't.
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
    if rotatable(kind) {
        let (rx, ry) = rotate_handle_screen(kind, cam, origin);
        draw_rotate_handle(rx, ry, color, window);
    }
}

/// A thin selection outline (no handles) — used for each element in a
/// multi-selection.
fn paint_box_outline(
    bb: (f32, f32, f32, f32),
    cam: Camera,
    origin: Point<Pixels>,
    color: Hsla,
    window: &mut Window,
) {
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
    if let Ok(p) = pb.build() {
        window.paint_path(p, color);
    }
}

/// The in-progress marquee box: a faint fill + thin outline.
fn paint_marquee(
    a: [f32; 2],
    b: [f32; 2],
    cam: Camera,
    origin: Point<Pixels>,
    color: Hsla,
    window: &mut Window,
) {
    let pa = to_screen(a[0], a[1], cam, origin);
    let pb = to_screen(b[0], b[1], cam, origin);
    let (x0, x1) = (
        f32::from(pa.x).min(f32::from(pb.x)),
        f32::from(pa.x).max(f32::from(pb.x)),
    );
    let (y0, y1) = (
        f32::from(pa.y).min(f32::from(pb.y)),
        f32::from(pa.y).max(f32::from(pb.y)),
    );
    let bounds = Bounds {
        origin: point(px(x0), px(y0)),
        size: size(px(x1 - x0), px(y1 - y0)),
    };
    let mut faint = color;
    faint.a *= 0.12;
    window.paint_quad(fill(bounds, faint));
    let mut pbld = PathBuilder::stroke(px(1.0));
    pbld.move_to(point(px(x0), px(y0)));
    pbld.line_to(point(px(x1), px(y0)));
    pbld.line_to(point(px(x1), px(y1)));
    pbld.line_to(point(px(x0), px(y1)));
    pbld.close();
    if let Ok(p) = pbld.build() {
        window.paint_path(p, color);
    }
}

impl Render for WhiteboardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let WhiteboardStyle {
            bg,
            grid,
            text,
            ink,
            panel,
            panel_strong,
            accent,
            selection,
            swatches,
        } = (self.style)();
        let cam = self.scene.camera;
        let zoom = cam.zoom.max(MIN_ZOOM);
        let bounds_cell = self.bounds.clone();

        // Decoded bitmaps for image elements, fetched from the host (which decodes
        // off-thread and re-renders when ready). Pre-fetched here — before the
        // element walk below — so the host callback can borrow `window`/`cx`
        // without clashing with the `iter_mut`.
        // Keyed by element id (not src) so two elements sharing a file but at
        // different angles don't collide. The rotation is snapped to a quarter
        // turn (images rotate in 90° steps), so a steady angle hits the host's
        // cache and only re-rotates as the drag crosses a 90° boundary.
        let img_sources: HashMap<u64, gpui::ImageSource> = {
            let items: Vec<(u64, String, f32)> = self
                .scene
                .elements
                .iter()
                .filter_map(|e| match &e.kind {
                    ElementKind::Image(im) => {
                        Some((e.id, im.src.clone(), snap_quarter(im.rotation)))
                    }
                    _ => None,
                })
                .collect();
            let mut map = HashMap::new();
            if let Some(f) = self.on_image.clone() {
                for (id, src, rot) in items {
                    if let Some(s) = f(&src, rot, window, cx) {
                        map.insert(id, s);
                    }
                }
            }
            map
        };

        // One ordered pass over the elements, building the paint stack as a list
        // of layers in `elements` order (later = on top). Canvas-drawn kinds
        // (shapes / lines / pen / text) accumulate into a "band" canvas; an image
        // or page-card flushes the band and adds its overlay div, so a shape can
        // sit above or below an image. Text is laid out here (measured extent for
        // selection/hit-test + outline segments) so it z-orders and rotates with
        // shapes. Re-laid each frame for now; see the path-cache note at the top.
        let font = self.font.clone();
        let editing = self.editing;
        let mut layers: Vec<Layer> = Vec::new();
        let mut band: Vec<ElemPaint> = Vec::new();
        for e in self.scene.elements.iter_mut() {
            let id = e.id;
            let stroke = e.stroke.map_or(ink, u32_to_hsla);
            let fill = e.fill.map(u32_to_hsla);
            match &mut e.kind {
                // Page-card: a titled box (top-aligned header + hint) that links
                // to a host page. Subtle border — the accent is the selection.
                ElementKind::Embed(em) => {
                    if !band.is_empty() {
                        layers.push(Layer::Band(std::mem::take(&mut band)));
                    }
                    layers.push(Layer::Overlay(
                        div()
                            .absolute()
                            .left(px((em.x - cam.x) * zoom))
                            .top(px((em.y - cam.y) * zoom))
                            .w(px(em.w * zoom))
                            .h(px(em.h * zoom))
                            .bg(panel)
                            .border_1()
                            .border_color(grid)
                            .rounded(px(8.0))
                            .overflow_hidden()
                            .p(px(10.0 * zoom))
                            .flex()
                            .flex_col()
                            .gap(px(3.0 * zoom))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0 * zoom))
                                    .text_size(px(14.0 * zoom))
                                    .text_color(ink)
                                    .child(div().text_color(accent).child("▤"))
                                    .child(SharedString::from(em.title.clone())),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0 * zoom))
                                    .text_color(text)
                                    .child("Double-click to open"),
                            )
                            .into_any_element(),
                    ));
                }
                // Image: the decoded bitmap (when the host has it ready), placed
                // in the element box's quarter-turn-rotated AABB; else a
                // placeholder while it loads.
                ElementKind::Image(im) => {
                    if !band.is_empty() {
                        layers.push(Layer::Band(std::mem::take(&mut band)));
                    }
                    let rot = snap_quarter(im.rotation);
                    let (bx, by, bw, bh) = if rot.abs() < ROT_EPS {
                        (im.x, im.y, im.w, im.h)
                    } else {
                        let c = box_padded_corners(im.x, im.y, im.w, im.h, rot, 0.0);
                        let (x0, y0, x1, y1) = aabb(&c);
                        (x0, y0, x1 - x0, y1 - y0)
                    };
                    let frame = div()
                        .absolute()
                        .left(px((bx - cam.x) * zoom))
                        .top(px((by - cam.y) * zoom))
                        .w(px(bw * zoom))
                        .h(px(bh * zoom))
                        .overflow_hidden()
                        .rounded(px(2.0));
                    let el = match img_sources.get(&id) {
                        // Set only the width and let gpui derive the height from the
                        // bitmap's aspect (its `Img` forces an `aspect_ratio` from the
                        // image, then ignores it unless a dimension is `Auto` — so
                        // `size_full` makes it overflow the box and clip). The bitmap is
                        // pre-rotated to the box's quarter-turn aspect, so width alone
                        // reproduces the rotated AABB exactly. `Contain` guards rounding.
                        Some(src) => frame.child(
                            gpui::img(src.clone())
                                .w(px(bw * zoom))
                                .object_fit(ObjectFit::Contain),
                        ),
                        None => frame
                            .bg(panel)
                            .border_1()
                            .border_color(grid)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(11.0 * zoom))
                                    .text_color(text)
                                    .child("Loading…"),
                            ),
                    };
                    layers.push(Layer::Overlay(el.into_any_element()));
                }
                // Canvas-drawn kinds: shapes / lines / pen / text.
                kind => {
                    let text = if let ElementKind::Text(t) = kind {
                        let layout = font.layout(&t.content, t.size);
                        t.measured_w = layout.width;
                        t.measured_h = layout.height;
                        Some(TextOutline {
                            segs: layout.segs,
                            x: t.x,
                            y: t.y,
                            rotation: t.rotation,
                            w: layout.width,
                            h: layout.height,
                            line_height: layout.line_height,
                            caret: (editing == Some(id)).then_some(layout.caret),
                        })
                    } else {
                        None
                    };
                    band.push(ElemPaint {
                        kind: kind.clone(),
                        stroke,
                        fill,
                        text,
                    });
                }
            }
        }
        if !band.is_empty() {
            layers.push(Layer::Band(band));
        }

        // The in-progress element previews in the current active color / fill.
        let pending_ink = self.active_stroke.map_or(ink, u32_to_hsla);
        let pending_fill = self.active_fill.map(u32_to_hsla);
        let pending = self.pending.as_ref().map(|p| p.kind.clone());
        // A single selection gets the full box + handles (unless it's the text
        // being edited — then just the caret). A multi-selection shows a single
        // enclosing group box instead of per-element outlines (one box stays
        // legible while rotating), with resize corners and — when at least one
        // member can rotate — a shared rotate grip.
        let single_sel = self
            .selected_single()
            .filter(|id| Some(*id) != self.editing)
            .and_then(|id| self.scene.elements.iter().find(|e| e.id == id))
            .map(|e| e.kind.clone());
        let group_sel = (self.selected.len() > 1)
            .then(|| self.selection_bbox())
            .flatten()
            .map(|bb| (bb, self.group_rotatable()));
        let marquee = self.marquee;

        // Tool palette + actions (top-center). The pill `occlude()`s so a press
        // on a button doesn't also act on the board beneath it. Layout, left→right:
        //   pan · select · color │ shapes&text▾ · pages&images▾ │ undo · redo · delete
        // The two bracketed buttons are categories: clicking one opens a flyout
        // (built below) of that group's tools, keeping the main bar trim.
        let active = self.tool;
        let open_group = self.open_group;

        // A bare tool button (icon + active highlight). The caller attaches the
        // tooltip and click handler, so this borrows nothing from `self`/`cx` and
        // can be reused for both the main bar and the flyout.
        let tool_btn = |t: Tool| {
            let icon: gpui::AnyElement = match t.icon() {
                Some((key, bytes)) => svg_icon(key, bytes, ink, 16.0).into_any_element(),
                None => t.glyph().into_any_element(),
            };
            let mut b = div()
                .id(("wb-tool", t as usize))
                .size(px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .text_size(px(15.0))
                .text_color(ink)
                .child(icon);
            // The hover tint also makes gpui repaint on hover transitions, which
            // is what lets a tooltip dismiss when the cursor leaves the button
            // (the canvas doesn't repaint on a bare mouse-move otherwise).
            if t == active {
                b = b.bg(accent);
            } else {
                b = b.hover(|s| s.bg(grid));
            }
            b
        };

        // A category button: shows the group's active tool (else a representative)
        // with a ▾ affordance, and highlights while its group owns the active tool
        // or its flyout is open.
        let cat_btn = |g: ToolGroup| {
            let shown = if g.contains(active) {
                active
            } else {
                g.representative()
            };
            let icon: gpui::AnyElement = match shown.icon() {
                Some((key, bytes)) => svg_icon(key, bytes, ink, 16.0).into_any_element(),
                None => shown.glyph().into_any_element(),
            };
            let mut b = div()
                .id(("wb-group", g as usize))
                .h(px(30.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(1.0))
                .rounded(px(6.0))
                .text_color(ink)
                .child(icon)
                .child(div().text_size(px(8.0)).text_color(text).child("▾"));
            if open_group == Some(g) || g.contains(active) {
                b = b.bg(accent);
            } else {
                b = b.hover(|s| s.bg(grid));
            }
            b
        };

        // The category buttons (one per `ToolGroup`), with the standalone Text
        // tool slotted in right after the Lines group.
        let mut cats: Vec<gpui::AnyElement> = Vec::with_capacity(ToolGroup::ALL.len() + 1);
        for &g in ToolGroup::ALL.iter() {
            cats.push(
                cat_btn(g)
                    .tooltip(self.tip(g.label()))
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        this.focus.focus(window, cx);
                        this.toggle_group(g, cx);
                    }))
                    .into_any_element(),
            );
            if g == ToolGroup::Lines {
                cats.push(
                    tool_btn(Tool::Text)
                        .tooltip(self.tip(Tool::Text.label()))
                        .on_click(cx.listener(|this, _ev, _w, cx| this.set_tool(Tool::Text, cx)))
                        .into_any_element(),
                );
            }
        }

        const UNDO_ICON: &[u8] = include_bytes!("../assets/icons/undo.svg");
        const REDO_ICON: &[u8] = include_bytes!("../assets/icons/redo.svg");
        const DELETE_ICON: &[u8] = include_bytes!("../assets/icons/delete.svg");
        let act = |id: usize, key: &'static str, bytes: &'static [u8]| {
            div()
                .id(("wb-act", id))
                .size(px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(|s| s.bg(grid))
                .child(svg_icon(key, bytes, ink, 16.0))
        };
        // Color button: a swatch of the current ink that toggles the picker.
        let cur_swatch = self.active_stroke.map_or(ink, u32_to_hsla);
        let mut color_btn = div()
            .id("wb-color")
            .size(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0));
        if self.picker.is_some() {
            color_btn = color_btn.bg(accent);
        } else {
            color_btn = color_btn.hover(|s| s.bg(grid));
        }
        let color_btn = color_btn
            .child(
                div()
                    .size(px(16.0))
                    .rounded(px(4.0))
                    .bg(cur_swatch)
                    .border_1()
                    .border_color(grid),
            )
            .tooltip(self.tip("Color"))
            .on_click(cx.listener(|this, _ev, window, cx| {
                this.focus.focus(window, cx);
                this.toggle_picker(cx);
            }));
        // Templates button: opens the gallery modal (its own toolbar item, since
        // a gallery of cards doesn't belong among the tool icons).
        const TEMPLATES_ICON: &[u8] = include_bytes!("../assets/icons/templates.svg");
        let mut templates_btn = div()
            .id("wb-templates")
            .size(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .child(svg_icon("wb-icon-templates", TEMPLATES_ICON, ink, 16.0));
        if self.templates_open {
            templates_btn = templates_btn.bg(accent);
        } else {
            templates_btn = templates_btn.hover(|s| s.bg(grid));
        }
        let templates_btn = templates_btn
            .tooltip(self.tip("Templates"))
            .on_click(cx.listener(|this, _ev, window, cx| {
                this.focus.focus(window, cx);
                this.toggle_templates(cx);
            }));
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
                    // navigate + color
                    .child(
                        tool_btn(Tool::Pan)
                            .tooltip(self.tip(Tool::Pan.label()))
                            .on_click(
                                cx.listener(|this, _ev, _w, cx| this.set_tool(Tool::Pan, cx)),
                            ),
                    )
                    .child(
                        tool_btn(Tool::Select)
                            .tooltip(self.tip(Tool::Select.label()))
                            .on_click(
                                cx.listener(|this, _ev, _w, cx| this.set_tool(Tool::Select, cx)),
                            ),
                    )
                    .child(color_btn)
                    .child(toolbar_divider(grid))
                    // tool categories (each opens a flyout of its tools)
                    .children(cats)
                    .child(templates_btn)
                    .child(toolbar_divider(grid))
                    // actions
                    .child(
                        act(0, "wb-icon-undo", UNDO_ICON)
                            .tooltip(self.tip("Undo (⌘Z)"))
                            .on_click(cx.listener(|this, _ev, window, cx| this.undo(window, cx))),
                    )
                    .child(
                        act(1, "wb-icon-redo", REDO_ICON)
                            .tooltip(self.tip("Redo (⌘⇧Z)"))
                            .on_click(cx.listener(|this, _ev, window, cx| this.redo(window, cx))),
                    )
                    .child(
                        act(2, "wb-icon-delete", DELETE_ICON)
                            .tooltip(self.tip("Delete selection (⌫)"))
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                this.delete_selected(window, cx)
                            })),
                    ),
            );

        // Tool-category flyout (centered below the toolbar), built only while a
        // group is open. Occluded like the main bar; picking a tool activates it
        // and closes the flyout (via `set_tool`), and a press elsewhere on the
        // canvas closes it (see `on_left_down`).
        let flyout = open_group.map(|g| {
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(2.0))
                .p(px(3.0))
                .rounded(px(9.0))
                .bg(panel_strong)
                .shadow_lg()
                .occlude();
            for &t in g.tools() {
                row = row.child(
                    tool_btn(t)
                        .tooltip(self.tip(t.label()))
                        .on_click(cx.listener(move |this, _ev, _w, cx| this.set_tool(t, cx))),
                );
            }
            div()
                .absolute()
                .top(px(52.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(row)
        });

        // Right-click context menu (a selection's "Save as template"), anchored at
        // the cursor. Occluded so its button doesn't fall through to the canvas;
        // any other press dismisses it (see `on_left_down`).
        let menu =
            self.context_menu.map(|pos| {
                // One clickable row; clicking runs `act` and closes the menu.
                let row = |id: &'static str, label: &'static str, shortcut: &'static str| {
                    div()
                        .id(id)
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(16.0))
                        .px(px(10.0))
                        .py(px(5.0))
                        .mx(px(4.0))
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .text_color(ink)
                        .hover(|s| s.bg(grid))
                        .child(label)
                        .child(div().text_size(px(11.0)).text_color(text).child(shortcut))
                };
                let divider = || div().my(px(4.0)).mx(px(8.0)).h(px(1.0)).bg(grid);
                let has_sel = !self.selected.is_empty();
                let mut panel = div()
                    .absolute()
                    .left(pos.x)
                    .top(pos.y)
                    .occlude()
                    .min_w(px(176.0))
                    .py(px(4.0))
                    .rounded(px(8.0))
                    .bg(panel_strong)
                    .shadow_lg()
                    .border_1()
                    .border_color(grid)
                    .flex()
                    .flex_col();
                // Z-order + copy / cut act on the selection, so they show only with one.
                if has_sel {
                    panel =
                        panel
                            .child(row("wb-ctx-front", "Bring to Front", "⌘⇧]").on_click(
                                cx.listener(|this, _ev, window, cx| {
                                    this.context_menu = None;
                                    this.reorder_selection(ZOrder::ToFront, window, cx);
                                }),
                            ))
                            .child(row("wb-ctx-forward", "Bring Forward", "⌘]").on_click(
                                cx.listener(|this, _ev, window, cx| {
                                    this.context_menu = None;
                                    this.reorder_selection(ZOrder::Forward, window, cx);
                                }),
                            ))
                            .child(row("wb-ctx-backward", "Send Backward", "⌘[").on_click(
                                cx.listener(|this, _ev, window, cx| {
                                    this.context_menu = None;
                                    this.reorder_selection(ZOrder::Backward, window, cx);
                                }),
                            ))
                            .child(
                                row("wb-ctx-back", "Send to Back", "⌘⇧[").on_click(cx.listener(
                                    |this, _ev, window, cx| {
                                        this.context_menu = None;
                                        this.reorder_selection(ZOrder::ToBack, window, cx);
                                    },
                                )),
                            )
                            .child(divider())
                            .child(row("wb-ctx-copy", "Copy", "⌘C").on_click(cx.listener(
                                |this, _ev, window, cx| {
                                    this.context_menu = None;
                                    this.copy_selection(window, cx);
                                },
                            )))
                            .child(row("wb-ctx-cut", "Cut", "⌘X").on_click(cx.listener(
                                |this, _ev, window, cx| {
                                    this.context_menu = None;
                                    if this.copy_selection(window, cx) {
                                        this.delete_selected(window, cx);
                                    }
                                },
                            )));
                }
                // Paste shows whenever the host wired it (so it works on empty canvas).
                if self.on_paste.is_some() {
                    panel = panel.child(row("wb-ctx-paste", "Paste", "⌘V").on_click(
                        cx.listener(|this, _ev, window, cx| this.paste_from_menu(window, cx)),
                    ));
                }
                // "Save as template" only with a selection and a wired host callback.
                if has_sel && self.on_save_template.is_some() {
                    panel = panel.child(divider()).child(
                        row("wb-ctx-save-template", "Save as template", "").on_click(cx.listener(
                            |this, _ev, window, cx| {
                                this.context_menu = None;
                                this.save_selection_as_template(window, cx);
                            },
                        )),
                    );
                }
                panel
            });

        // Templates gallery modal: a dimming scrim (click to dismiss) centering a
        // panel of preview cards. The panel `occlude()`s so clicks on it don't
        // reach the scrim; a card stamps its template and closes (see
        // `apply_template`), and Escape closes it (see `on_key`).
        let templates_modal = self.templates_open.then(|| {
            let body = if self.templates.is_empty() {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(28.0))
                    .child(
                        div()
                            .max_w(px(320.0))
                            .text_size(px(12.0))
                            .text_color(text)
                            .child(
                                "No templates yet. Select shapes on the canvas, right-click, \
                                 and choose “Save as template”.",
                            ),
                    )
                    .into_any_element()
            } else {
                let mut grid_el = div().flex().flex_wrap().gap(px(8.0)).justify_center();
                for i in 0..self.templates.len() {
                    grid_el = grid_el.child(self.template_card(i, ink, text, grid, bg, cx));
                }
                div()
                    .id("wb-tmpl-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(12.0))
                    .child(grid_el)
                    .into_any_element()
            };
            let panel = div()
                .w(px(540.0))
                .max_h(px(460.0))
                .flex()
                .flex_col()
                .rounded(px(12.0))
                .bg(panel_strong)
                .shadow_lg()
                .border_1()
                .border_color(grid)
                .occlude()
                // header
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(px(14.0))
                        .py(px(10.0))
                        .border_b_1()
                        .border_color(grid)
                        .child(div().text_size(px(14.0)).text_color(ink).child("Templates"))
                        .child(
                            div()
                                .id("wb-tmpl-close")
                                .size(px(22.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(6.0))
                                .text_size(px(15.0))
                                .text_color(text)
                                .hover(|s| s.bg(grid))
                                .child("✕")
                                .on_click(cx.listener(|this, _ev, _w, cx| {
                                    this.templates_open = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(body)
                // footer hint
                .child(
                    div()
                        .px(px(14.0))
                        .py(px(8.0))
                        .border_t_1()
                        .border_color(grid)
                        .text_size(px(10.0))
                        .text_color(text)
                        .child("Click to add · right-click to delete"),
                );
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(hsla(0.0, 0.0, 0.0, 0.35))
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _ev, _w, cx| {
                        this.templates_open = false;
                        cx.notify();
                    }),
                )
                .child(panel)
        });

        // Color picker panel (below the toolbar), built only while open. Not
        // occluded: presses fall through to `on_left_down`, which routes the SV
        // square / hue strip to drags (via the captured bounds), consumes presses
        // elsewhere on the panel, and closes on a press outside it.
        let sv_cell = self.sv_bounds.clone();
        let hue_cell = self.hue_bounds.clone();
        let alpha_cell = self.alpha_bounds.clone();
        let panel_cell = self.picker_bounds.clone();
        let swatch_list = swatches;
        let white = hsla(0.0, 0.0, 1.0, 1.0);
        // The stroke / fill colors backing the two target tabs (selection's, else
        // the active value). `None` = theme ink (stroke) or unfilled (fill).
        let stroke_disp = self
            .selected_single()
            .and_then(|id| self.scene.elements.iter().find(|e| e.id == id))
            .and_then(|e| e.stroke)
            .or(self.active_stroke);
        let fill_disp = self
            .selected_single()
            .and_then(|id| self.scene.elements.iter().find(|e| e.id == id))
            .and_then(|e| e.fill)
            .or(self.active_fill);
        let picker_panel = self.picker.map(|p| {
            let cur = hsva_to_u32(p.h, p.s, p.v, p.a);
            let hex = format!("#{:06X}", cur >> 8);
            let clear = hsla(0.0, 0.0, 0.0, 0.0);

            // Stroke / fill target tabs. The active one is highlighted; clicking
            // re-seeds the controls from that property's color.
            let tab = |active: bool, sw: Hsla, label: &'static str, id: &'static str| {
                let mut d = div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(ink);
                if active {
                    d = d.bg(accent);
                }
                d.child(
                    div()
                        .size(px(12.0))
                        .rounded(px(3.0))
                        .bg(sw)
                        .border_1()
                        .border_color(grid),
                )
                .child(label)
            };
            let tabs = div()
                .flex()
                .gap(px(6.0))
                .child(
                    tab(
                        p.target == PickerTarget::Stroke,
                        stroke_disp.map_or(ink, u32_to_hsla),
                        "Stroke",
                        "wb-tab-stroke",
                    )
                    .on_click(cx.listener(|this, _ev, _w, cx| {
                        this.set_picker_target(PickerTarget::Stroke, cx)
                    })),
                )
                .child(
                    tab(
                        p.target == PickerTarget::Fill,
                        fill_disp.map_or(clear, u32_to_hsla),
                        "Fill",
                        "wb-tab-fill",
                    )
                    .on_click(cx.listener(|this, _ev, _w, cx| {
                        this.set_picker_target(PickerTarget::Fill, cx)
                    })),
                );

            let sv_square = div()
                .relative()
                .w(px(SV_W))
                .h(px(SV_H))
                .rounded(px(5.0))
                .overflow_hidden()
                .bg(hsla(p.h, 1.0, 0.5, 1.0))
                .child(div().absolute().size_full().bg(linear_gradient(
                    90.0,
                    linear_color_stop(white, 0.0),
                    linear_color_stop(hsla(0.0, 0.0, 1.0, 0.0), 1.0),
                )))
                .child(div().absolute().size_full().bg(linear_gradient(
                    180.0,
                    linear_color_stop(hsla(0.0, 0.0, 0.0, 0.0), 0.0),
                    linear_color_stop(hsla(0.0, 0.0, 0.0, 1.0), 1.0),
                )))
                .child(
                    canvas(move |b, _, _| sv_cell.set(b), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(p.s * SV_W - 7.0))
                        .top(px((1.0 - p.v) * SV_H - 7.0))
                        .size(px(14.0))
                        .rounded_full()
                        .border_2()
                        .border_color(white),
                );

            let seg = |from: f32, to: f32| {
                div().flex_1().h_full().bg(linear_gradient(
                    90.0,
                    linear_color_stop(hsla(from, 1.0, 0.5, 1.0), 0.0),
                    linear_color_stop(hsla(to, 1.0, 0.5, 1.0), 1.0),
                ))
            };
            let hue_strip = div()
                .relative()
                .w(px(SV_W))
                .h(px(HUE_H))
                .rounded(px(4.0))
                .overflow_hidden()
                .flex()
                .child(seg(0.0, 1.0 / 6.0))
                .child(seg(1.0 / 6.0, 2.0 / 6.0))
                .child(seg(2.0 / 6.0, 3.0 / 6.0))
                .child(seg(3.0 / 6.0, 4.0 / 6.0))
                .child(seg(4.0 / 6.0, 5.0 / 6.0))
                .child(seg(5.0 / 6.0, 1.0))
                .child(
                    canvas(move |b, _, _| hue_cell.set(b), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(p.h * SV_W - 1.5))
                        .top(px(-2.0))
                        .w(px(3.0))
                        .h(px(HUE_H + 4.0))
                        .rounded(px(2.0))
                        .bg(white)
                        .border_1()
                        .border_color(hsla(0.0, 0.0, 0.0, 0.5)),
                );

            // Alpha (opacity) strip: transparent → the current color, opaque.
            let alpha_strip = div()
                .relative()
                .w(px(SV_W))
                .h(px(HUE_H))
                .rounded(px(4.0))
                .overflow_hidden()
                .bg(linear_gradient(
                    90.0,
                    linear_color_stop(clear, 0.0),
                    linear_color_stop(u32_to_hsla(hsv_to_u32(p.h, p.s, p.v)), 1.0),
                ))
                .child(
                    canvas(move |b, _, _| alpha_cell.set(b), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(p.a * SV_W - 1.5))
                        .top(px(-2.0))
                        .w(px(3.0))
                        .h(px(HUE_H + 4.0))
                        .rounded(px(2.0))
                        .bg(white)
                        .border_1()
                        .border_color(hsla(0.0, 0.0, 0.0, 0.5)),
                );

            // Reset means "back to theme ink" for stroke, "no fill" for fill.
            let reset_label = if p.target == PickerTarget::Fill {
                "None"
            } else {
                "Auto"
            };
            let info_row = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .size(px(22.0))
                        .rounded(px(4.0))
                        .bg(u32_to_hsla(cur))
                        .border_1()
                        .border_color(grid),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.0))
                        .text_color(text)
                        .child(SharedString::from(hex)),
                )
                .child(
                    div()
                        .id("wb-color-auto")
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(5.0))
                        .border_1()
                        .border_color(grid)
                        .text_size(px(12.0))
                        .text_color(ink)
                        .child(reset_label)
                        .on_click(
                            cx.listener(|this, _ev, window, cx| this.pick_color(None, window, cx)),
                        ),
                );

            let mut swatch_views = Vec::with_capacity(swatch_list.len());
            for (i, c) in swatch_list.iter().enumerate() {
                let col = *c;
                swatch_views.push(
                    div()
                        .id(("wb-swatch", i))
                        .size(px(20.0))
                        .rounded(px(4.0))
                        .bg(col)
                        .border_1()
                        .border_color(grid)
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            this.pick_color(Some(hsla_to_u32(col)), window, cx)
                        })),
                );
            }
            let swatch_grid = div().flex().flex_wrap().gap(px(6.0)).children(swatch_views);

            div()
                .absolute()
                .top(px(52.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .p(px(10.0))
                        .rounded(px(10.0))
                        .bg(panel_strong)
                        .shadow_lg()
                        .border_1()
                        .border_color(grid)
                        .child(
                            canvas(move |b, _, _| panel_cell.set(b), |_, _, _, _| {})
                                .absolute()
                                .size_full(),
                        )
                        .child(tabs)
                        .child(sv_square)
                        .child(hue_strip)
                        .child(alpha_strip)
                        .child(info_row)
                        .child(swatch_grid),
                )
        });

        // Pan tool shows a grab cursor (closed while dragging) to read as "drag
        // to move the canvas"; other tools use the default arrow.
        let board_cursor = if self.panning {
            CursorStyle::ClosedHand
        } else if self.tool == Tool::Pan {
            CursorStyle::OpenHand
        } else {
            CursorStyle::Arrow
        };

        // The board paints as a stack of layers (back → front): the grid /
        // background; then the element layers (canvas "bands" interleaved with
        // image / page-card overlays, in z-order); then a top "chrome" canvas for
        // the in-progress element, selection box, and marquee — kept above the
        // content so handles stay visible over images.
        let board_layer = canvas(
            move |bounds, _, _| bounds_cell.set(bounds),
            move |bounds, _, window, _| paint_board(bounds, cam, bg, grid, window),
        )
        .absolute()
        .size_full();
        let element_layers: Vec<gpui::AnyElement> = layers
            .into_iter()
            .map(|l| match l {
                Layer::Band(es) => band_canvas(es, cam).into_any_element(),
                Layer::Overlay(el) => el,
            })
            .collect();
        let chrome_layer = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if let Some(k) = &pending {
                    paint_element(k, cam, bounds.origin, pending_ink, pending_fill, window);
                }
                if let Some(k) = &single_sel {
                    paint_selection(k, cam, bounds.origin, selection, window);
                }
                // Group: an enclosing box with resize corners, plus a shared
                // rotate grip above it when the group can rotate.
                if let Some((bb, can_rotate)) = group_sel {
                    paint_box_outline(bb, cam, bounds.origin, selection, window);
                    let tl = to_screen(bb.0, bb.1, cam, bounds.origin);
                    let br = to_screen(bb.2, bb.3, cam, bounds.origin);
                    let m = SEL_PAD_PX;
                    let (x0, y0) = (f32::from(tl.x) - m, f32::from(tl.y) - m);
                    let (x1, y1) = (f32::from(br.x) + m, f32::from(br.y) + m);
                    for (hx, hy) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                        draw_handle(hx, hy, selection, window);
                    }
                    if can_rotate {
                        let (rx, ry) = rotate_handle_for_bbox(bb, cam, bounds.origin);
                        draw_rotate_handle(rx, ry, selection, window);
                    }
                }
                if let Some((a, b)) = marquee {
                    paint_marquee(a, b, cam, bounds.origin, selection, window);
                }
            },
        )
        .absolute()
        .size_full();

        div()
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .cursor(board_cursor)
            .child(board_layer)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_left_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_left_up))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_middle_down))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_middle_up))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
            .on_key_down(cx.listener(Self::on_key))
            // Files dragged from the OS land as `ExternalPaths`; hand them to the
            // host (which imports any images) at the drop point.
            .on_drop::<gpui::ExternalPaths>(cx.listener(
                |this, paths: &gpui::ExternalPaths, window, cx| {
                    if let Some(f) = this.on_drop_files.clone() {
                        let w = this.event_to_world(window.mouse_position());
                        f(paths.paths().to_vec(), w[0], w[1], window, cx);
                    }
                },
            ))
            .children(element_layers)
            .child(chrome_layer)
            .child(toolbar)
            .children(flyout)
            .children(menu)
            .children(picker_panel)
            .children(templates_modal)
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
                    stroke: None,
                    fill: None,
                },
                Element {
                    id: 2,
                    kind: ElementKind::Rect(BoxGeom {
                        x: 1.0,
                        y: 2.0,
                        w: 30.0,
                        h: 40.0,
                        width: 2.0,
                        rotation: 0.0,
                    }),
                    stroke: Some(0xff0000ff),
                    fill: Some(0x00ff0080),
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
                    stroke: None,
                    fill: None,
                },
            ],
        };
        let restored = Scene::from_json(&scene.to_json());
        assert_eq!(restored.elements.len(), 3);
        match &restored.elements[2].kind {
            ElementKind::Arrow(s) => assert_eq!(s.y2, 8.0),
            other => panic!("expected arrow, got {other:?}"),
        }
        // Per-element color round-trips; an uncolored element stays `None`.
        assert_eq!(restored.elements[1].stroke, Some(0xff0000ff));
        assert_eq!(restored.elements[1].fill, Some(0x00ff0080));
        assert_eq!(restored.elements[0].stroke, None);
        assert_eq!(restored.elements[0].fill, None);
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
    fn snap_grid_rounds_to_nearest_line() {
        // GRID is 24: values round to the nearest multiple, halves away from zero.
        assert_eq!(snap_grid(0.0), 0.0);
        assert_eq!(snap_grid(11.0), 0.0);
        assert_eq!(snap_grid(13.0), GRID);
        assert_eq!(snap_grid(GRID), GRID);
        assert_eq!(snap_grid(-13.0), -GRID);
        assert_eq!(snap_grid(1.5 * GRID), 2.0 * GRID);
    }

    #[test]
    fn move_target_drives_an_absolute_snapped_target() {
        // Origin off-grid (100 % 24 == 4); grab anchor at the cursor's start.
        let origin = [100.0, 100.0];
        let anchor = [0.0, 0.0];

        // Free move tracks the cursor exactly on both axes.
        assert_eq!(
            move_target(origin, anchor, [37.0, -11.0], false),
            [137.0, 89.0]
        );

        // Snapped: the target is `snap(origin + total)`, computed fresh each
        // frame — never the running position, so it can't stick. A 50,50 total
        // lands on snap(150) = 144 (150/24 = 6.25 → 6).
        assert_eq!(
            move_target(origin, anchor, [50.0, 50.0], true),
            [144.0, 144.0]
        );

        // Regression: twelve sub-threshold 4px steps (each < half a grid cell)
        // must still accumulate across grid lines on BOTH axes — the old logic
        // snapped each tiny step from the already-snapped spot and stuck.
        let mut cursor = [0.0, 0.0];
        for _ in 0..12 {
            cursor = [cursor[0] + 4.0, cursor[1] + 4.0];
        }
        // 48px total → snap(148) = 144 on each axis.
        assert_eq!(move_target(origin, anchor, cursor, true), [144.0, 144.0]);
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
            rotation: 0.0,
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
    fn color_round_trips_through_hsv_and_packed_ints() {
        // Pure primaries survive HSV → packed → HSV.
        for c in [0xff0000ff, 0x00ff00ff, 0x0000ffff, 0x808080ff, 0xffffffff] {
            let (h, s, v) = u32_to_hsv(c);
            assert_eq!(hsv_to_u32(h, s, v), c, "{c:#010x}");
        }
        // Hue endpoints both land on red.
        assert_eq!(hsv_to_u32(0.0, 1.0, 1.0), 0xff0000ff);
        assert_eq!(hsv_to_u32(1.0, 1.0, 1.0), 0xff0000ff);
        // A 2/3 hue is pure blue.
        assert_eq!(hsv_to_u32(2.0 / 3.0, 1.0, 1.0), 0x0000ffff);
        // pack clamps out-of-range and rounds to 0..255.
        assert_eq!(pack_rgba(1.5, -0.2, 0.5, 1.0), 0xff0080ff);
    }

    #[test]
    fn rotation_accumulates_on_boxes_and_bakes_into_segments() {
        use std::f32::consts::FRAC_PI_2;
        // A box stores the angle and its center-anchored bounds don't move.
        let mut k = ElementKind::Rect(BoxGeom {
            x: -10.0,
            y: -10.0,
            w: 20.0,
            h: 20.0,
            width: 1.0,
            rotation: 0.0,
        });
        rotate_element(&mut k, 0.0, 0.0, FRAC_PI_2);
        match &k {
            ElementKind::Rect(b) => assert!((b.rotation - FRAC_PI_2).abs() < 1e-5),
            other => panic!("expected rect, got {other:?}"),
        }
        // A square's bounds are unchanged by a 90° turn about its center.
        let bb = bbox(&k);
        assert!(
            (bb.0 + 10.0).abs() < 1e-3 && (bb.2 - 10.0).abs() < 1e-3,
            "{bb:?}"
        );

        // A line bakes the rotation into its endpoints: +90° about the origin
        // sends (10,0) → (0,10).
        let mut seg = ElementKind::Line(SegGeom {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 0.0,
            width: 1.0,
        });
        rotate_element(&mut seg, 0.0, 0.0, FRAC_PI_2);
        match seg {
            ElementKind::Line(s) => {
                assert!(s.x2.abs() < 1e-3 && (s.y2 - 10.0).abs() < 1e-3, "{s:?}");
            }
            other => panic!("expected line, got {other:?}"),
        }

        // Text rotates like a box: spun about its own center, it accumulates an
        // angle and stays put (centered on the pivot here, so no orbit).
        let mut txt = ElementKind::Text(TextGeom {
            x: -20.0,
            y: -8.0,
            content: "hi".into(),
            size: 16.0,
            rotation: 0.0,
            measured_w: 40.0,
            measured_h: 16.0,
        });
        rotate_element(&mut txt, 0.0, 0.0, FRAC_PI_2);
        match txt {
            ElementKind::Text(t) => {
                assert!((t.rotation - FRAC_PI_2).abs() < 1e-5);
                assert!(
                    (t.x + 20.0).abs() < 1e-3 && (t.y + 8.0).abs() < 1e-3,
                    "{t:?}"
                );
            }
            other => panic!("expected text, got {other:?}"),
        }

        // Orbiting: rotating a box about a *different* pivot moves its center
        // along the arc. A unit box at (1,0) turned 90° about the origin → (0,1).
        let mut orb = ElementKind::Rect(BoxGeom {
            x: 0.5,
            y: -0.5,
            w: 1.0,
            h: 1.0,
            width: 1.0,
            rotation: 0.0,
        });
        rotate_element(&mut orb, 0.0, 0.0, FRAC_PI_2);
        match orb {
            ElementKind::Rect(b) => {
                let (ccx, ccy) = (b.x + 0.5, b.y + 0.5);
                assert!(ccx.abs() < 1e-3 && (ccy - 1.0).abs() < 1e-3, "{b:?}");
            }
            other => panic!("expected rect, got {other:?}"),
        }
    }

    #[test]
    fn rotation_snaps_to_horizontal_and_vertical() {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
        let step = std::f32::consts::PI / 12.0;
        // Within the snap zone of a cardinal snaps onto it...
        assert!((snap_angle(FRAC_PI_2 - 0.05, false) - FRAC_PI_2).abs() < 1e-6);
        assert!(snap_angle(0.04, false).abs() < 1e-6);
        assert!((snap_angle(-FRAC_PI_2 + 0.03, false) + FRAC_PI_2).abs() < 1e-6);
        // ...but a hair outside it, and at 45°, the angle is left free.
        assert!((snap_angle(FRAC_PI_2 - 0.2, false) - (FRAC_PI_2 - 0.2)).abs() < 1e-6);
        assert!((snap_angle(FRAC_PI_4, false) - FRAC_PI_4).abs() < 1e-6);
        // Shift snaps to the nearest 15° everywhere.
        assert!((snap_angle(0.30, true) - step).abs() < 1e-4);
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
            rotation: 0.0,
            measured_w: 0.0,
            measured_h: 0.0,
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
            rotation: 0.0,
        })));
    }

    #[test]
    fn image_round_trips_and_behaves_like_a_box() {
        let kind = ElementKind::Image(ImageGeom {
            src: "images/x.png".into(),
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 60.0,
            rotation: 0.0,
        });
        // Bounds = the box; not a fillable closed shape.
        assert_eq!(bbox(&kind), (10.0, 20.0, 110.0, 80.0));
        assert!(!is_closed_shape(&kind));
        // Round-trips through JSON under the "image" tag, keeping its src.
        let elem = Element {
            id: 1,
            kind,
            stroke: None,
            fill: None,
        };
        let json = serde_json::to_string(&elem).unwrap();
        assert!(json.contains("\"image\""), "{json}");
        assert!(json.contains("images/x.png"));
        let mut back = serde_json::from_str::<Element>(&json).unwrap().kind;
        assert_eq!(bbox(&back), (10.0, 20.0, 110.0, 80.0));
        // Translates like the other box kinds.
        translate(&mut back, 5.0, -3.0);
        assert_eq!(bbox(&back), (15.0, 17.0, 115.0, 77.0));
    }

    #[test]
    fn new_box_shapes_share_box_behavior_and_round_trip() {
        let b = BoxGeom {
            x: 1.0,
            y: 2.0,
            w: 30.0,
            h: 40.0,
            width: 2.0,
            rotation: 0.5,
        };
        // (serde tag, kind) — the tag is what gets persisted in JSON.
        let cases = [
            ("diamond", ElementKind::Diamond(b)),
            ("triangle", ElementKind::Triangle(b)),
            ("round_rect", ElementKind::RoundRect(b)),
            ("star", ElementKind::Star(b)),
            ("hexagon", ElementKind::Hexagon(b)),
        ];
        for (tag, kind) in cases {
            // Every new shape is a fillable closed shape, commits like a box, and
            // flows through the shared `box_like` path (bounds / select / resize /
            // rotate) just like rect/ellipse.
            assert!(is_closed_shape(&kind), "{tag} should be fillable");
            assert!(committable(&kind), "{tag} should commit");
            assert_eq!(
                box_like(&kind),
                Some((1.0, 2.0, 30.0, 40.0, 0.5)),
                "{tag} box_like"
            );
            // Round-trips through JSON under its snake_case tag.
            let elem = Element {
                id: 7,
                kind,
                stroke: None,
                fill: None,
            };
            let json = serde_json::to_string(&elem).unwrap();
            assert!(json.contains(tag), "{tag} not in json: {json}");
            let back: Element = serde_json::from_str(&json).unwrap();
            assert_eq!(box_like(&back.kind), Some((1.0, 2.0, 30.0, 40.0, 0.5)));
        }
    }
}
