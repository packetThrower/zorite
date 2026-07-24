//! The paint path: world→screen mapping, board/grid painting, per-element
//! painters, selection/marquee/guide overlays, thumbnail layer building, the
//! text-layout caches, and the packed-color ↔ HSV/Hsla conversions — split
//! from `lib.rs`.

use super::*;

// --- color ----------------------------------------------------------------
//
// Element colors are stored as packed `0xRRGGBBAA` so the scene JSON stays
// dependency-free. The picker works in HSV (the usual hue / saturation /
// brightness controls); these convert between HSV, packed ints, and gpui's
// `Hsla` for painting.

/// Pack 0..1 RGBA components into `0xRRGGBBAA`.
pub(crate) fn pack_rgba(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let q = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u32;
    (q(r) << 24) | (q(g) << 16) | (q(b) << 8) | q(a)
}

/// A packed color as a gpui `Hsla`, for painting.
pub(crate) fn u32_to_hsla(c: u32) -> Hsla {
    rgba(c).into()
}

/// A gpui `Hsla` packed into `0xRRGGBBAA` (used to store theme swatches).
pub(crate) fn hsla_to_u32(c: Hsla) -> u32 {
    let r = Rgba::from(c);
    pack_rgba(r.r, r.g, r.b, r.a)
}

/// HSV (each 0..1) → RGB (each 0..1).
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
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
pub(crate) fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
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
pub(crate) fn hsv_to_u32(h: f32, s: f32, v: f32) -> u32 {
    hsva_to_u32(h, s, v, 1.0)
}

/// HSVA (each 0..1) packed into `0xRRGGBBAA`.
pub(crate) fn hsva_to_u32(h: f32, s: f32, v: f32, a: f32) -> u32 {
    let (r, g, b) = hsv_to_rgb(h, s, v);
    pack_rgba(r, g, b, a)
}

/// A packed color's HSV (alpha dropped).
pub(crate) fn u32_to_hsv(c: u32) -> (f32, f32, f32) {
    let p = rgba(c);
    rgb_to_hsv(p.r, p.g, p.b)
}

/// A packed color's alpha as 0..1.
pub(crate) fn u32_alpha(c: u32) -> f32 {
    (c & 0xff) as f32 / 255.0
}

/// World → absolute screen point at the current camera.
pub(crate) fn to_screen(wx: f32, wy: f32, cam: Camera, origin: Point<Pixels>) -> Point<Pixels> {
    let z = cam.zoom.max(MIN_ZOOM);
    point(
        px(f32::from(origin.x) + (wx - cam.x) * z),
        px(f32::from(origin.y) + (wy - cam.y) * z),
    )
}

/// Paint the board background + the world-space dot grid into `bounds`.
pub(crate) fn paint_board(
    bounds: Bounds<Pixels>,
    cam: Camera,
    bg: Hsla,
    grid: Hsla,
    window: &mut Window,
) {
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
pub(crate) struct ElemPaint {
    pub(crate) kind: ElementKind,
    pub(crate) stroke: Hsla,
    pub(crate) fill: Option<Hsla>,
    pub(crate) text: Option<TextOutline>,
    pub(crate) mindmap_connector_style: Option<MindMapConnectorStyle>,
}

/// One slice of the board's z-order paint stack. Canvas-drawn elements collect
/// into a `Band` (one canvas); an image or page-card is an `Overlay` div between
/// bands. `render` builds these in `elements` order so paint order = z-order,
/// which lets a shape sit above or below an image. See [`band_canvas`].
pub(crate) enum Layer {
    Band(Vec<ElemPaint>),
    Overlay(gpui::AnyElement),
}

/// A transparent, full-size canvas painting one run of canvas-drawn elements
/// (shapes / lines / pen / text) in order. Stacked between [`Layer::Overlay`]
/// divs so paint order follows the element list.
pub(crate) fn band_canvas(elems: Vec<ElemPaint>, cam: Camera) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            for ep in &elems {
                // Shapes / lines / pen paint here; Text elements are a no-op in
                // `paint_element`. Any text outline — a Text element's content or
                // a shape's label — then paints on top.
                paint_element(
                    &ep.kind,
                    ep.mindmap_connector_style,
                    cam,
                    bounds.origin,
                    ep.stroke,
                    ep.fill,
                    window,
                );
                if let Some(t) = &ep.text {
                    paint_text(t, cam, bounds.origin, window);
                }
            }
        },
    )
    .absolute()
    .size_full()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_thumbnail_layers(
    scene: &Scene,
    font: &Font,
    cam: Camera,
    ink: Hsla,
    text: Hsla,
    grid: Hsla,
    panel: Hsla,
    viewport: Option<WorldViewport>,
    mut text_layout_cache: Option<&mut HashMap<u64, CachedTextLayout>>,
    mut label_layout_cache: Option<&mut HashMap<u64, CachedLabelLayout>>,
) -> Vec<Layer> {
    let mindmap_connector_styles: HashMap<u64, MindMapConnectorStyle> = scene
        .elements
        .iter()
        .filter_map(|element| {
            thumbnail_mindmap_connector_style_for_element(scene, &element.kind)
                .map(|style| (element.id, style))
        })
        .collect();
    let mut layers: Vec<Layer> = Vec::new();
    let mut band: Vec<ElemPaint> = Vec::new();
    for e in &scene.elements {
        if viewport.is_some_and(|viewport| !viewport.intersects(bbox(&e.kind))) {
            continue;
        }
        let id = e.id;
        let stroke = e.stroke.map_or(ink, u32_to_hsla);
        let fill = e.fill.map(u32_to_hsla);
        let label = e.label.as_deref();
        let label_color = e.label_color;
        let styles = e.styles.as_slice();
        match &e.kind {
            ElementKind::Embed(em) => {
                if !band.is_empty() {
                    layers.push(Layer::Band(std::mem::take(&mut band)));
                }
                layers.push(Layer::Overlay(
                    div()
                        .absolute()
                        .left(px((em.x - cam.x) * cam.zoom.max(MIN_ZOOM)))
                        .top(px((em.y - cam.y) * cam.zoom.max(MIN_ZOOM)))
                        .w(px(em.w * cam.zoom.max(MIN_ZOOM)))
                        .h(px(em.h * cam.zoom.max(MIN_ZOOM)))
                        .bg(panel)
                        .border_1()
                        .border_color(grid)
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .p(px(10.0 * cam.zoom.max(MIN_ZOOM)))
                        .flex()
                        .flex_col()
                        .gap(px(3.0 * cam.zoom.max(MIN_ZOOM)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0 * cam.zoom.max(MIN_ZOOM)))
                                .text_size(px(14.0 * cam.zoom.max(MIN_ZOOM)))
                                .text_color(ink)
                                .child(div().child("▤"))
                                .child(SharedString::from(em.title.clone())),
                        )
                        .child(
                            div()
                                .text_size(px(11.0 * cam.zoom.max(MIN_ZOOM)))
                                .text_color(text)
                                .child("Page"),
                        )
                        .into_any_element(),
                ));
            }
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
                let zoom = cam.zoom.max(MIN_ZOOM);
                layers.push(Layer::Overlay(
                    div()
                        .absolute()
                        .left(px((bx - cam.x) * zoom))
                        .top(px((by - cam.y) * zoom))
                        .w(px(bw * zoom))
                        .h(px(bh * zoom))
                        .rounded(px(2.0))
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
                                .child("Image"),
                        )
                        .into_any_element(),
                ));
            }
            kind => {
                let text_outline = thumbnail_text_outline(
                    font,
                    kind,
                    stroke,
                    label,
                    label_color,
                    styles,
                    id,
                    text_layout_cache.as_deref_mut(),
                    label_layout_cache.as_deref_mut(),
                );
                band.push(ElemPaint {
                    kind: kind.clone(),
                    stroke,
                    fill,
                    text: text_outline,
                    mindmap_connector_style: mindmap_connector_styles.get(&id).copied(),
                });
            }
        }
    }
    if !band.is_empty() {
        layers.push(Layer::Band(band));
    }
    layers
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn thumbnail_text_outline(
    font: &Font,
    kind: &ElementKind,
    stroke: Hsla,
    label: Option<&str>,
    label_color: Option<u32>,
    styles: &[StyleSpan],
    element_id: u64,
    mut text_layout_cache: Option<&mut HashMap<u64, CachedTextLayout>>,
    label_layout_cache: Option<&mut HashMap<u64, CachedLabelLayout>>,
) -> Option<TextOutline> {
    if let ElementKind::Text(t) = kind {
        let layout = match text_layout_cache.as_deref_mut() {
            Some(cache) => {
                cached_text_layout(cache, font, element_id, &t.content, t.size, None, styles)
            }
            None => prepare_text_layout(font, &t.content, t.size, None, styles, 0),
        };
        return Some(TextOutline {
            segs: layout.segs.clone(),
            bold_segs: layout.bold_segs.clone(),
            bold_width: layout.bold_width,
            color: stroke,
            x: t.x,
            y: t.y,
            rotation: t.rotation,
            pivot: [t.x + layout.width / 2.0, t.y + layout.height / 2.0],
            line_height: layout.line_height,
            caret: None,
            selection: Vec::new(),
            sel_color: hsla(0.0, 0.0, 0.0, 0.0),
            decorations: layout.decorations.clone(),
        });
    }
    if is_closed_shape(kind)
        && let Some((bx, by, bw, bh, rot)) = box_like(kind)
        && label.is_some_and(|s| !s.trim().is_empty())
    {
        let text = label.map_or("", str::trim);
        let cached_label = label_layout_cache.map(|cache| {
            cached_label_layout(cache, font, element_id, kind, bx, by, bw, bh, text, styles)
        });
        let (x, y, layout) = if let Some(label) = cached_label {
            (bx + label.offset_x, by + label.offset_y, label.text)
        } else {
            let block = shape_label_block(font, kind, bx, by, bw, bh, text);
            let layout = match text_layout_cache {
                Some(cache) => cached_text_layout(
                    cache,
                    font,
                    element_id,
                    text,
                    block.size,
                    Some(block.wrap),
                    styles,
                ),
                None => prepare_text_layout(font, text, block.size, Some(block.wrap), styles, 0),
            };
            (block.x, block.y, layout)
        };
        return Some(TextOutline {
            segs: layout.segs.clone(),
            bold_segs: layout.bold_segs.clone(),
            bold_width: layout.bold_width,
            color: label_color.map_or(stroke, u32_to_hsla),
            x,
            y,
            rotation: rot,
            pivot: [bx + bw / 2.0, by + bh / 2.0],
            line_height: layout.line_height,
            caret: None,
            selection: Vec::new(),
            sel_color: hsla(0.0, 0.0, 0.0, 0.0),
            decorations: layout.decorations.clone(),
        });
    }
    None
}

pub(crate) fn thumbnail_mindmap_meta(scene: &Scene, id: u64) -> Option<MindMapNodeMeta> {
    scene
        .elements
        .iter()
        .find(|e| e.id == id)
        .and_then(|e| e.mindmap)
}

pub(crate) fn thumbnail_mindmap_root_of(scene: &Scene, id: u64) -> Option<u64> {
    let mut current = id;
    loop {
        let meta = thumbnail_mindmap_meta(scene, current)?;
        match meta.parent {
            Some(parent) => current = parent,
            None => return Some(current),
        }
    }
}

pub(crate) fn thumbnail_mindmap_connector_style_for_root(
    scene: &Scene,
    root_id: u64,
) -> MindMapConnectorStyle {
    thumbnail_mindmap_meta(scene, root_id)
        .map(|meta| meta.connector_style)
        .unwrap_or_default()
}

pub(crate) fn thumbnail_mindmap_connector_style_for_element(
    scene: &Scene,
    kind: &ElementKind,
) -> Option<MindMapConnectorStyle> {
    let seg = match kind {
        ElementKind::Line(seg) | ElementKind::Arrow(seg) => seg,
        _ => return None,
    };
    let start_root = seg
        .start_anchor
        .and_then(|anchor| thumbnail_mindmap_root_of(scene, anchor.element_id));
    let end_root = seg
        .end_anchor
        .and_then(|anchor| thumbnail_mindmap_root_of(scene, anchor.element_id));
    match (start_root, end_root) {
        (Some(a), Some(b)) if a == b => Some(thumbnail_mindmap_connector_style_for_root(scene, a)),
        _ => None,
    }
}

/// A text element's glyph outlines (text-local space) plus placement, captured
/// for the paint closure to transform (camera + rotation) and fill.
#[derive(Clone)]
pub(crate) struct CachedTextLayout {
    pub(crate) signature: u64,
    pub(crate) segs: Arc<[font::Seg]>,
    pub(crate) bold_segs: Arc<[font::Seg]>,
    pub(crate) bold_width: f32,
    pub(crate) decorations: Arc<[font::Decoration]>,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) line_height: f32,
}

#[derive(Clone)]
pub(crate) struct CachedLabelLayout {
    pub(crate) signature: u64,
    pub(crate) offset_x: f32,
    pub(crate) offset_y: f32,
    pub(crate) size: f32,
    pub(crate) wrap: f32,
    pub(crate) text: CachedTextLayout,
}

pub(crate) fn hash_text_styles(styles: &[StyleSpan], hasher: &mut impl Hasher) {
    for span in styles {
        span.start.hash(hasher);
        span.end.hash(hasher);
        span.style.bold.hash(hasher);
        span.style.italic.hash(hasher);
        span.style.underline.hash(hasher);
        span.style.strike.hash(hasher);
        span.style.highlight.hash(hasher);
    }
}

pub(crate) fn text_layout_signature(
    content: &str,
    size: f32,
    max_width: Option<f32>,
    styles: &[StyleSpan],
) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    size.to_bits().hash(&mut hasher);
    max_width.map(f32::to_bits).hash(&mut hasher);
    hash_text_styles(styles, &mut hasher);
    hasher.finish()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cached_label_layout(
    cache: &mut HashMap<u64, CachedLabelLayout>,
    font: &Font,
    element_id: u64,
    kind: &ElementKind,
    bx: f32,
    by: f32,
    bw: f32,
    bh: f32,
    content: &str,
    styles: &[StyleSpan],
) -> CachedLabelLayout {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    bw.to_bits().hash(&mut hasher);
    bh.to_bits().hash(&mut hasher);
    std::mem::discriminant(kind).hash(&mut hasher);
    hash_text_styles(styles, &mut hasher);
    let signature = hasher.finish();
    if let Some(layout) = cache
        .get(&element_id)
        .filter(|layout| layout.signature == signature)
    {
        return layout.clone();
    }

    let block = shape_label_block(font, kind, bx, by, bw, bh, content);
    let text = prepare_text_layout(
        font,
        content,
        block.size,
        Some(block.wrap),
        styles,
        signature,
    );
    let cached = CachedLabelLayout {
        signature,
        offset_x: block.x - bx,
        offset_y: block.y - by,
        size: block.size,
        wrap: block.wrap,
        text,
    };
    cache.insert(element_id, cached.clone());
    cached
}

pub(crate) fn cached_text_layout(
    cache: &mut HashMap<u64, CachedTextLayout>,
    font: &Font,
    element_id: u64,
    content: &str,
    size: f32,
    max_width: Option<f32>,
    styles: &[StyleSpan],
) -> CachedTextLayout {
    let signature = text_layout_signature(content, size, max_width, styles);
    if let Some(layout) = cache
        .get(&element_id)
        .filter(|layout| layout.signature == signature)
    {
        return layout.clone();
    }
    let cached = prepare_text_layout(font, content, size, max_width, styles, signature);
    cache.insert(element_id, cached.clone());
    cached
}

pub(crate) fn prepare_text_layout(
    font: &Font,
    content: &str,
    size: f32,
    max_width: Option<f32>,
    styles: &[StyleSpan],
    signature: u64,
) -> CachedTextLayout {
    let layout = font.layout_styled(content, size, max_width, |byte| {
        glyph_style(style_at(styles, byte))
    });
    CachedTextLayout {
        signature,
        segs: layout.segs.into(),
        bold_segs: layout.bold_segs.into(),
        bold_width: layout.bold_width,
        decorations: layout.decorations.into(),
        width: layout.width,
        height: layout.height,
        line_height: layout.line_height,
    }
}

pub(crate) struct TextOutline {
    pub(crate) segs: Arc<[font::Seg]>,
    /// Bold glyphs' outlines, stroked over the fill (synthetic bold), + the
    /// local stroke width.
    pub(crate) bold_segs: Arc<[font::Seg]>,
    pub(crate) bold_width: f32,
    /// Glyph fill color — a Text element's ink, or a shape label's color.
    pub(crate) color: Hsla,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) rotation: f32,
    /// Rotation pivot (world): the shape's center, so an off-center label (a
    /// triangle's base-anchored text) still rotates with the shape.
    pub(crate) pivot: [f32; 2],
    pub(crate) line_height: f32,
    /// Caret's text-local top, when this text is being edited.
    pub(crate) caret: Option<[f32; 2]>,
    /// Selection highlight rects (text-local `[x, y, w, h]`), when editing.
    pub(crate) selection: Vec<[f32; 4]>,
    /// Fill color for the selection highlight.
    pub(crate) sel_color: Hsla,
    /// Underline / strikethrough / highlight runs (text-local), from the styling.
    pub(crate) decorations: Arc<[font::Decoration]>,
}

/// Paint a text element's vector outlines (and, when editing, its caret). Local
/// glyph points are placed at `(x, y)`, rotated about the block's center, then
/// projected to the screen — so text rotates and scales like the shapes.
pub(crate) fn paint_text(t: &TextOutline, cam: Camera, origin: Point<Pixels>, window: &mut Window) {
    let color = t.color;
    let (cx, cy) = (t.pivot[0], t.pivot[1]);
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
    // A text-local `[x, y, w, h]` rect → a screen-space fill path (rotated like
    // the glyphs). Shared by highlights, the selection, and under/strike bars.
    let rect_path = |r: [f32; 4]| {
        let (x, y, w, h) = (r[0], r[1], r[2], r[3]);
        let mut pb = PathBuilder::fill();
        pb.move_to(tf([x, y]));
        pb.line_to(tf([x + w, y]));
        pb.line_to(tf([x + w, y + h]));
        pb.line_to(tf([x, y + h]));
        pb.close();
        pb.build().ok()
    };
    // Highlights, then the editing selection — both behind the glyphs.
    for d in t.decorations.iter() {
        if let font::DecoKind::Highlight(c) = d.kind
            && let Some(path) = rect_path(d.rect)
        {
            window.paint_path(path, u32_to_hsla(c));
        }
    }
    for r in &t.selection {
        if let Some(path) = rect_path(*r) {
            window.paint_path(path, t.sel_color);
        }
    }
    // Walk glyph segments into `pb` (a fill or a stroke path).
    let emit = |pb: &mut PathBuilder, segs: &[font::Seg]| {
        let mut cur = point(px(0.0), px(0.0));
        for seg in segs {
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
    };
    if !t.segs.is_empty() {
        let mut pb = PathBuilder::fill();
        emit(&mut pb, &t.segs);
        if let Ok(path) = pb.build() {
            window.paint_path(path, color);
        }
    }
    // Synthetic bold: stroke the bold glyphs' outlines over the solid fill (a
    // doubled fill would cancel under even-odd winding and read as hollow).
    if !t.bold_segs.is_empty() {
        let zoom = cam.zoom.max(MIN_ZOOM);
        let mut pb = PathBuilder::stroke(px((t.bold_width * zoom).max(0.5)));
        emit(&mut pb, &t.bold_segs);
        if let Ok(path) = pb.build() {
            window.paint_path(path, color);
        }
    }
    // Underline / strikethrough bars, in the text color, over the glyphs.
    for d in t.decorations.iter() {
        if matches!(d.kind, font::DecoKind::Underline | font::DecoKind::Strike)
            && let Some(path) = rect_path(d.rect)
        {
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
pub(crate) fn paint_element(
    kind: &ElementKind,
    mindmap_connector_style: Option<MindMapConnectorStyle>,
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
        ElementKind::Line(s) => paint_segment(
            s,
            false,
            mindmap_connector_style.unwrap_or(MindMapConnectorStyle::Straight),
            cam,
            origin,
            ink,
            window,
        ),
        ElementKind::Arrow(s) => paint_segment(
            s,
            true,
            mindmap_connector_style.unwrap_or(MindMapConnectorStyle::Straight),
            cam,
            origin,
            ink,
            window,
        ),
        // Text / cards / images are drawn as overlay elements in render(), not here.
        ElementKind::Text(_) | ElementKind::Embed(_) | ElementKind::Image(_) => {}
    }
}

pub(crate) fn paint_stroke(
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

pub(crate) fn paint_rect(
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

pub(crate) fn paint_ellipse(
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
pub(crate) fn star_unit() -> [(f32, f32); 10] {
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
pub(crate) fn hexagon_unit() -> [(f32, f32); 6] {
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
pub(crate) fn paint_box_polygon(
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
pub(crate) fn paint_round_rect(
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

pub(crate) fn paint_segment(
    seg: &SegGeom,
    arrow: bool,
    style: MindMapConnectorStyle,
    cam: Camera,
    origin: Point<Pixels>,
    ink: Hsla,
    window: &mut Window,
) {
    let z = cam.zoom.max(MIN_ZOOM);
    let p1 = to_screen(seg.x1, seg.y1, cam, origin);
    let p2 = to_screen(seg.x2, seg.y2, cam, origin);
    let p1f = [f32::from(p1.x), f32::from(p1.y)];
    let p2f = [f32::from(p2.x), f32::from(p2.y)];
    let stroke_px = (seg.width * z).max(0.5);
    let mut points = vec![p1f];
    let (dxw, _dyw) = (seg.x2 - seg.x1, seg.y2 - seg.y1);
    let (end_dx, end_dy) = match style {
        MindMapConnectorStyle::Straight => {
            points.push(p2f);
            (p2f[0] - p1f[0], p2f[1] - p1f[1])
        }
        MindMapConnectorStyle::Bezier => {
            let cx1 = seg.x1 + dxw * 0.35;
            let cy1 = seg.y1;
            let cx2 = seg.x2 - dxw * 0.35;
            let cy2 = seg.y2;
            let c1 = to_screen(cx1, cy1, cam, origin);
            let c2 = to_screen(cx2, cy2, cam, origin);
            let c1f = [f32::from(c1.x), f32::from(c1.y)];
            let c2f = [f32::from(c2.x), f32::from(c2.y)];
            for i in 1..=24 {
                let t = i as f32 / 24.0;
                points.push(cubic_point(p1f, c1f, c2f, p2f, t));
            }
            (3.0 * (p2f[0] - c2f[0]), 3.0 * (p2f[1] - c2f[1]))
        }
        MindMapConnectorStyle::Orthogonal => {
            let mid_x = seg.x1 + dxw * 0.5;
            let m1 = to_screen(mid_x, seg.y1, cam, origin);
            let m2 = to_screen(mid_x, seg.y2, cam, origin);
            let m1f = [f32::from(m1.x), f32::from(m1.y)];
            let m2f = [f32::from(m2.x), f32::from(m2.y)];
            points.push(m1f);
            points.push(m2f);
            points.push(p2f);
            (p2f[0] - m2f[0], p2f[1] - m2f[1])
        }
    };
    paint_polyline(points.as_slice(), stroke_px, seg.style, ink, window);
    if !arrow {
        return;
    }
    let (dx, dy) = (end_dx, end_dy);
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

pub(crate) fn cubic_point(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    p3: [f32; 2],
    t: f32,
) -> [f32; 2] {
    let mt = 1.0 - t;
    let a = mt * mt * mt;
    let b = 3.0 * mt * mt * t;
    let c = 3.0 * mt * t * t;
    let d = t * t * t;
    [
        a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
        a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
    ]
}

pub(crate) fn paint_polyline(
    points: &[[f32; 2]],
    stroke_px: f32,
    style: SegmentStyle,
    ink: Hsla,
    window: &mut Window,
) {
    if points.len() < 2 {
        return;
    }
    let mut pb = PathBuilder::stroke(px(stroke_px));
    match style {
        SegmentStyle::Solid => {
            pb.move_to(point(px(points[0][0]), px(points[0][1])));
            for p in &points[1..] {
                pb.line_to(point(px(p[0]), px(p[1])));
            }
        }
        SegmentStyle::Dashed => {
            let dash = (stroke_px * 4.5).max(10.0);
            let gap = (stroke_px * 2.5).max(6.0);
            let cycle = dash + gap;
            let mut traveled = 0.0;
            for seg in points.windows(2) {
                let a = seg[0];
                let b = seg[1];
                let dx = b[0] - a[0];
                let dy = b[1] - a[1];
                let len = (dx * dx + dy * dy).sqrt();
                if len <= 0.01 {
                    continue;
                }
                let ux = dx / len;
                let uy = dy / len;
                let mut local = 0.0;
                while local < len {
                    let at = traveled + local;
                    let phase = at % cycle;
                    let draw = if phase < dash { dash - phase } else { 0.0 };
                    if draw > 0.0 {
                        let s = local;
                        let e = (local + draw).min(len);
                        let p0 = [a[0] + ux * s, a[1] + uy * s];
                        let p1 = [a[0] + ux * e, a[1] + uy * e];
                        pb.move_to(point(px(p0[0]), px(p0[1])));
                        pb.line_to(point(px(p1[0]), px(p1[1])));
                        local = e;
                    } else {
                        local = (local + (cycle - phase)).min(len);
                    }
                }
                traveled += len;
            }
        }
    }
    if let Ok(path) = pb.build() {
        window.paint_path(path, ink);
    }
}

pub(crate) fn draw_filled_circle(hx: f32, hy: f32, radius: f32, color: Hsla, window: &mut Window) {
    const K: f32 = 0.552_284_8;
    let k = radius * K;
    let p = |x: f32, y: f32| point(px(x), px(y));
    let mut path = PathBuilder::fill();
    path.move_to(p(hx + radius, hy));
    path.cubic_bezier_to(
        p(hx, hy + radius),
        p(hx + radius, hy + k),
        p(hx + k, hy + radius),
    );
    path.cubic_bezier_to(
        p(hx - radius, hy),
        p(hx - k, hy + radius),
        p(hx - radius, hy + k),
    );
    path.cubic_bezier_to(
        p(hx, hy - radius),
        p(hx - radius, hy - k),
        p(hx - k, hy - radius),
    );
    path.cubic_bezier_to(
        p(hx + radius, hy),
        p(hx + k, hy - radius),
        p(hx + radius, hy - k),
    );
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// Compact circular resize handle matching the whiteboard connector controls.
pub(crate) fn draw_handle(hx: f32, hy: f32, color: Hsla, window: &mut Window) {
    draw_filled_circle(hx, hy, HANDLE_HALF + 1.0, hsla(0.0, 0.0, 1.0, 1.0), window);
    draw_filled_circle(hx, hy, HANDLE_HALF, color, window);
}

/// Screen-space centers for the top/right/bottom/left connector buttons. Each
/// button is pushed outward from the selected element while its line still
/// starts at the true edge connector point.
pub(crate) fn connector_button_centers(
    kind: &ElementKind,
    cam: Camera,
    origin: Point<Pixels>,
) -> [Point<Pixels>; 4] {
    let points = connector_points(kind);
    let edges = [
        to_screen(points[0][0], points[0][1], cam, origin),
        to_screen(points[1][0], points[1][1], cam, origin),
        to_screen(points[2][0], points[2][1], cam, origin),
        to_screen(points[3][0], points[3][1], cam, origin),
    ];
    let center = edges.iter().fold((0.0, 0.0), |(x, y), point| {
        (x + f32::from(point.x), y + f32::from(point.y))
    });
    let center = (center.0 / 4.0, center.1 / 4.0);
    edges.map(|edge| {
        let dx = f32::from(edge.x) - center.0;
        let dy = f32::from(edge.y) - center.1;
        let length = (dx * dx + dy * dy).sqrt().max(1.0);
        point(
            edge.x + px(dx / length * CONNECTOR_BUTTON_GAP),
            edge.y + px(dy / length * CONNECTOR_BUTTON_GAP),
        )
    })
}

pub(crate) fn paint_snap_points(
    kind: &ElementKind,
    active: usize,
    cam: Camera,
    origin: Point<Pixels>,
    color: Hsla,
    window: &mut Window,
) {
    for (index, point) in connector_points(kind).into_iter().enumerate() {
        let screen = to_screen(point[0], point[1], cam, origin);
        let radius = if index == active {
            HANDLE_HALF + 1.5
        } else {
            HANDLE_HALF
        };
        let (x, y) = (f32::from(screen.x), f32::from(screen.y));
        draw_filled_circle(x, y, radius + 1.0, hsla(0.0, 0.0, 1.0, 1.0), window);
        draw_filled_circle(x, y, radius, color, window);
    }
}

pub(crate) fn paint_selection(
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
        return;
    }
    // Box-like (rect/ellipse/text): the (possibly rotated) box outline, four
    // corner handles, and a rotate grip. Edge-midpoint handles (per-axis stretch)
    // show only when upright — a rotated box's edges aren't world-axis-aligned.
    if let Some((x, y, w, h, rot)) = box_like(kind) {
        let s =
            box_padded_corners(x, y, w, h, rot, 0.0).map(|p| to_screen(p[0], p[1], cam, origin));
        for p in &s {
            draw_handle(f32::from(p.x), f32::from(p.y), color, window);
        }
        if rot.abs() <= ROT_EPS && !matches!(kind, ElementKind::Text(_)) {
            let mid = |a: Point<Pixels>, b: Point<Pixels>| {
                (
                    (f32::from(a.x) + f32::from(b.x)) / 2.0,
                    (f32::from(a.y) + f32::from(b.y)) / 2.0,
                )
            };
            for (hx, hy) in [
                mid(s[0], s[1]),
                mid(s[1], s[2]),
                mid(s[2], s[3]),
                mid(s[3], s[0]),
            ] {
                draw_handle(hx, hy, color, window);
            }
        }
        return;
    }
    // Draw / Embed: a padded AABB box + four corner handles. Freehand strokes
    // (rotatable) also get a rotate grip; cards don't.
    let bb = bbox(kind);
    let tl = to_screen(bb.0, bb.1, cam, origin);
    let br = to_screen(bb.2, bb.3, cam, origin);
    let m = 0.0;
    let (x0, y0) = (f32::from(tl.x) - m, f32::from(tl.y) - m);
    let (x1, y1) = (f32::from(br.x) + m, f32::from(br.y) + m);
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    for (hx, hy) in [
        (x0, y0),
        (x1, y0),
        (x0, y1),
        (x1, y1),
        (mx, y0),
        (x1, my),
        (mx, y1),
        (x0, my),
    ] {
        draw_handle(hx, hy, color, window);
    }
}

/// The in-progress marquee box: a faint fill + thin outline.
pub(crate) fn paint_marquee(
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

pub(crate) fn paint_alignment_guides(
    guides: AlignmentGuides,
    bounds: Bounds<Pixels>,
    cam: Camera,
    color: Hsla,
    window: &mut Window,
) {
    let mut color = color;
    color.a = 0.8;
    if let Some(x) = guides.vertical {
        let screen = to_screen(x, 0.0, cam, bounds.origin);
        let mut path = PathBuilder::stroke(px(1.0));
        path.move_to(point(screen.x, bounds.top()));
        path.line_to(point(screen.x, bounds.bottom()));
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
    if let Some(y) = guides.horizontal {
        let screen = to_screen(0.0, y, cam, bounds.origin);
        let mut path = PathBuilder::stroke(px(1.0));
        path.move_to(point(bounds.left(), screen.y));
        path.line_to(point(bounds.right(), screen.y));
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}
