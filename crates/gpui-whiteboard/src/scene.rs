//! The persisted scene model: [`Scene`] / [`Element`] / [`ElementKind`] and their
//! geometry payloads, rich-text style spans, [`Camera`], and the local-thumbnail
//! spec — the JSON-(de)serialized layer, split from `lib.rs`.

use super::*;

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
    /// Centered text label for closed shapes (rect / ellipse / …). Rendered at a
    /// font size auto-shrunk so the wrapped text never crosses the shape's
    /// border (see the shape paint path). `None` / empty = no label; ignored by
    /// non-shape kinds. Absent in older boards → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Color of the shape's [`label`](Self::label), packed `0xRRGGBBAA`. `None`
    /// inks it with the shape's stroke (theme ink if that's unset too). Ignored by
    /// non-shape kinds. Absent in older boards → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_color: Option<u32>,
    /// Rich-text formatting runs over the element's text (a Text element's
    /// content, or a shape's label): bold / italic / underline / strikethrough /
    /// highlight, each over a byte range. Empty = unstyled. Kept sorted +
    /// non-overlapping and maintained across edits; absent in older boards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub styles: Vec<StyleSpan>,
    /// Optional whiteboard-native mind-map metadata. Only mind-map nodes carry
    /// this; lines stay regular anchored arrows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mindmap: Option<MindMapNodeMeta>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MindMapSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MindMapRootDirection {
    #[default]
    Both,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MindMapConnectorStyle {
    Straight,
    #[default]
    Bezier,
    Orthogonal,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MindMapNodeMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
    pub side: MindMapSide,
    #[serde(default)]
    pub order: usize,
    #[serde(default)]
    pub root_direction: MindMapRootDirection,
    #[serde(default)]
    pub connector_style: MindMapConnectorStyle,
}

/// Whether a [`RunStyle`] flag is unset — its default, kept out of the JSON.
fn is_false(b: &bool) -> bool {
    !*b
}

/// The formatting of a run of characters; `default()` is plain text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct RunStyle {
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub strike: bool,
    /// Highlight color behind the glyphs, packed `0xRRGGBBAA`; `None` = none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<u32>,
}

impl RunStyle {
    /// No formatting — needn't be stored (plain runs are implicit gaps).
    fn is_plain(&self) -> bool {
        *self == RunStyle::default()
    }
}

/// A toggleable boolean format. (Highlight is a color, toggled on its own.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl Format {
    pub(crate) fn get(self, s: &RunStyle) -> bool {
        match self {
            Format::Bold => s.bold,
            Format::Italic => s.italic,
            Format::Underline => s.underline,
            Format::Strike => s.strike,
        }
    }
    pub(crate) fn set(self, s: &mut RunStyle, on: bool) {
        match self {
            Format::Bold => s.bold = on,
            Format::Italic => s.italic = on,
            Format::Underline => s.underline = on,
            Format::Strike => s.strike = on,
        }
    }
}

/// A [`RunStyle`] over the byte range `[start, end)` of an element's text. Runs
/// are kept sorted, non-overlapping, and non-plain. See [`Element::styles`].
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StyleSpan {
    pub start: usize,
    pub end: usize,
    pub style: RunStyle,
}

/// `[s, e)` as contiguous `(start, end, style)` segments — existing runs plus
/// plain gaps. Assumes `spans` is sorted + non-overlapping.
fn style_segments(spans: &[StyleSpan], s: usize, e: usize) -> Vec<(usize, usize, RunStyle)> {
    let mut out = Vec::new();
    let mut pos = s;
    for sp in spans.iter().filter(|sp| sp.start < e && sp.end > s) {
        let a = sp.start.max(s);
        if a > pos {
            out.push((pos, a, RunStyle::default()));
        }
        out.push((a, sp.end.min(e), sp.style));
        pos = sp.end.min(e);
    }
    if pos < e {
        out.push((pos, e, RunStyle::default()));
    }
    out
}

/// Re-sort, merge touching equal runs, and drop plain runs.
fn normalize_styles(mut segs: Vec<(usize, usize, RunStyle)>) -> Vec<StyleSpan> {
    segs.retain(|(a, b, _)| a < b);
    segs.sort_by_key(|(a, _, _)| *a);
    let mut out: Vec<StyleSpan> = Vec::new();
    for (a, b, st) in segs {
        if st.is_plain() {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.end == a
            && last.style == st
        {
            last.end = b;
            continue;
        }
        out.push(StyleSpan {
            start: a,
            end: b,
            style: st,
        });
    }
    out
}

/// The style covering byte `offset` (the char starting there), else plain.
pub(crate) fn style_at(spans: &[StyleSpan], offset: usize) -> RunStyle {
    spans
        .iter()
        .find(|sp| sp.start <= offset && offset < sp.end)
        .map_or(RunStyle::default(), |sp| sp.style)
}

/// The formatting common to *every* char in `[s, e)` — for menu checkmarks. A
/// collapsed range reports the style just left of the caret (what typing
/// inherits).
pub(crate) fn active_style(spans: &[StyleSpan], s: usize, e: usize) -> RunStyle {
    if s >= e {
        return style_at(spans, s.saturating_sub(1));
    }
    let segs = style_segments(spans, s, e);
    let mut it = segs.iter().map(|&(_, _, st)| st);
    let Some(mut acc) = it.next() else {
        return RunStyle::default();
    };
    for st in it {
        acc.bold &= st.bold;
        acc.italic &= st.italic;
        acc.underline &= st.underline;
        acc.strike &= st.strike;
        acc.highlight = match (acc.highlight, st.highlight) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ => None,
        };
    }
    acc
}

/// Replace the styling of `[s, e)` with `mid` (which covers `[s, e)`), keeping
/// runs outside (splitting any that straddle), then normalize.
fn replace_segment(
    spans: &[StyleSpan],
    s: usize,
    e: usize,
    mid: Vec<(usize, usize, RunStyle)>,
) -> Vec<StyleSpan> {
    let mut segs: Vec<(usize, usize, RunStyle)> = Vec::new();
    for sp in spans {
        if sp.start < s {
            segs.push((sp.start, sp.end.min(s), sp.style));
        }
        if sp.end > e {
            segs.push((sp.start.max(e), sp.end, sp.style));
        }
    }
    segs.extend(mid);
    normalize_styles(segs)
}

/// Toggle `format` over `[s, e)`: removed if every char already has it, else
/// added.
pub(crate) fn toggle_format(
    spans: &[StyleSpan],
    s: usize,
    e: usize,
    format: Format,
) -> Vec<StyleSpan> {
    if s >= e {
        return spans.to_vec();
    }
    let segs = style_segments(spans, s, e);
    let add = !segs.iter().all(|(_, _, st)| format.get(st));
    let mid = segs
        .into_iter()
        .map(|(a, b, mut st)| {
            format.set(&mut st, add);
            (a, b, st)
        })
        .collect();
    replace_segment(spans, s, e, mid)
}

/// Set/clear the highlight color over `[s, e)`: cleared if every char already
/// has `color`, else set to it.
pub(crate) fn toggle_highlight(
    spans: &[StyleSpan],
    s: usize,
    e: usize,
    color: u32,
) -> Vec<StyleSpan> {
    if s >= e {
        return spans.to_vec();
    }
    let segs = style_segments(spans, s, e);
    let on = !segs.iter().all(|(_, _, st)| st.highlight == Some(color));
    let mid = segs
        .into_iter()
        .map(|(a, b, mut st)| {
            st.highlight = on.then_some(color);
            (a, b, st)
        })
        .collect();
    replace_segment(spans, s, e, mid)
}

/// Shift/clip runs when the text `[s, e)` becomes `ins_len` bytes; the inserted
/// bytes take `insert_style`. Keeps runs aligned to the edited text.
pub(crate) fn splice_styles(
    spans: &[StyleSpan],
    s: usize,
    e: usize,
    ins_len: usize,
    insert_style: RunStyle,
) -> Vec<StyleSpan> {
    let delta = ins_len as isize - (e - s) as isize;
    let after = |p: usize| (p as isize + delta).max(0) as usize;
    let mut segs: Vec<(usize, usize, RunStyle)> = Vec::new();
    for sp in spans {
        if sp.start < s {
            segs.push((sp.start, sp.end.min(s), sp.style));
        }
        if sp.end > e {
            segs.push((after(sp.start.max(e)), after(sp.end), sp.style));
        }
    }
    if ins_len > 0 && !insert_style.is_plain() {
        segs.push((s, s + ins_len, insert_style));
    }
    normalize_styles(segs)
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
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentAnchor {
    /// The connected shape/text/image element.
    pub element_id: u64,
    /// Which connector on that element: 0/1/2/3 = top/right/bottom/left.
    pub connector: usize,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SegGeom {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub width: f32,
    #[serde(default = "default_segment_style")]
    pub style: SegmentStyle,
    /// Attachment for the first endpoint. Absent in older boards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_anchor: Option<SegmentAnchor>,
    /// Attachment for the second endpoint. Absent in older boards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_anchor: Option<SegmentAnchor>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentStyle {
    Solid,
    Dashed,
}

fn default_segment_style() -> SegmentStyle {
    SegmentStyle::Solid
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
    pub(crate) fn screen_to_world(&self, sx: f32, sy: f32) -> (f32, f32) {
        let z = self.zoom.max(MIN_ZOOM);
        (self.x + sx / z, self.y + sy / z)
    }

    /// Pan by a screen-space delta (px): the content follows the gesture.
    pub(crate) fn pan_by(&mut self, dx: f32, dy: f32) {
        let z = self.zoom.max(MIN_ZOOM);
        self.x -= dx / z;
        self.y -= dy / z;
    }

    /// Multiply the zoom by `factor`, keeping the world point under the
    /// canvas-relative screen point `(rx, ry)` fixed (zoom-about-cursor).
    pub(crate) fn zoom_about(&mut self, rx: f32, ry: f32, factor: f32) {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalThumbnailMode {
    Auto,
    Selection,
    Viewport,
    AllContent,
    Element(u64),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct LocalThumbnailSpec {
    pub anchor_element_id: Option<u64>,
    /// Focus bounds in world coordinates: `[x0, y0, x1, y1]`.
    pub focus_bounds: [f32; 4],
    /// Whole-scene bounds in world coordinates, when available.
    pub scene_bounds: Option<[f32; 4]>,
    /// Recommended camera to render this thumbnail into the requested size.
    pub camera: Camera,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalThumbnailSnapshot {
    pub scene: Scene,
    pub spec: LocalThumbnailSpec,
}

impl LocalThumbnailSnapshot {
    /// Build a chrome-free all-content thumbnail directly from a persisted scene.
    /// Hosts can use this without mounting a full [`WhiteboardView`] entity first.
    pub fn for_scene_all_content(scene: Scene, width_px: f32, height_px: f32) -> Self {
        let width_px = width_px.max(1.0);
        let height_px = height_px.max(1.0);
        let scene_bounds = scene_bbox_for_local_thumbnail(&scene);
        let spec = if let Some(focus) = scene_bounds {
            local_thumbnail_spec_from_bbox(None, focus, scene_bounds, width_px, height_px)
        } else {
            let zoom = scene.camera.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
            LocalThumbnailSpec {
                anchor_element_id: None,
                focus_bounds: [
                    scene.camera.x,
                    scene.camera.y,
                    scene.camera.x + width_px / zoom,
                    scene.camera.y + height_px / zoom,
                ],
                scene_bounds: None,
                camera: Camera {
                    x: scene.camera.x,
                    y: scene.camera.y,
                    zoom,
                },
            }
        };
        Self { scene, spec }
    }
}

pub(crate) fn scene_bbox_for_local_thumbnail(scene: &Scene) -> Option<(f32, f32, f32, f32)> {
    let mut bounds = scene.elements.iter().map(|element| bbox(&element.kind));
    let first = bounds.next()?;
    Some(bounds.fold(first, |current, next| {
        (
            current.0.min(next.0),
            current.1.min(next.1),
            current.2.max(next.2),
            current.3.max(next.3),
        )
    }))
}

pub(crate) fn local_thumbnail_spec_from_bbox(
    anchor_element_id: Option<u64>,
    focus: (f32, f32, f32, f32),
    scene_bounds: Option<(f32, f32, f32, f32)>,
    width_px: f32,
    height_px: f32,
) -> LocalThumbnailSpec {
    let width_px = width_px.max(1.0);
    let height_px = height_px.max(1.0);
    let focus_w = (focus.2 - focus.0).abs().max(1.0);
    let focus_h = (focus.3 - focus.1).abs().max(1.0);
    let pad_x = (focus_w * 0.18).max(48.0);
    let pad_y = (focus_h * 0.18).max(36.0);
    let padded_w = (focus_w + pad_x * 2.0).max(1.0);
    let padded_h = (focus_h + pad_y * 2.0).max(1.0);
    let zoom = (width_px / padded_w)
        .min(height_px / padded_h)
        .clamp(MIN_ZOOM, MAX_ZOOM);
    let center_x = (focus.0 + focus.2) * 0.5;
    let center_y = (focus.1 + focus.3) * 0.5;
    LocalThumbnailSpec {
        anchor_element_id,
        focus_bounds: [focus.0, focus.1, focus.2, focus.3],
        scene_bounds: scene_bounds.map(|bounds| [bounds.0, bounds.1, bounds.2, bounds.3]),
        camera: Camera {
            x: center_x - width_px / (2.0 * zoom),
            y: center_y - height_px / (2.0 * zoom),
            zoom,
        },
    }
}
