//! Element geometry: bounding boxes, hit-testing, rotation, snapping,
//! translate/resize math, and connector anchor points — pure world-space
//! helpers over [`ElementKind`], split from `lib.rs`.

use super::*;

/// Whether an in-progress element is big enough to keep (a click that doesn't
/// drag leaves nothing).
pub(crate) fn committable(kind: &ElementKind) -> bool {
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
pub(crate) fn is_closed_shape(kind: &ElementKind) -> bool {
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

/// Set the stroke width (world-space) on kinds that have one — pen / box-like
/// shapes / lines / arrows. A no-op for text, page-cards, and images.
pub(crate) fn set_kind_width(kind: &mut ElementKind, w: f32) {
    match kind {
        ElementKind::Draw(s) => s.width = w,
        ElementKind::Rect(b)
        | ElementKind::Ellipse(b)
        | ElementKind::Diamond(b)
        | ElementKind::Triangle(b)
        | ElementKind::RoundRect(b)
        | ElementKind::Star(b)
        | ElementKind::Hexagon(b) => b.width = w,
        ElementKind::Line(s) | ElementKind::Arrow(s) => s.width = w,
        ElementKind::Text(_) | ElementKind::Embed(_) | ElementKind::Image(_) => {}
    }
}

/// Rotate `(x, y)` by `a` radians about `(cx, cy)`.
pub(crate) fn rotate_pt(x: f32, y: f32, cx: f32, cy: f32, a: f32) -> (f32, f32) {
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
pub(crate) fn box_like(kind: &ElementKind) -> Option<(f32, f32, f32, f32, f32)> {
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
pub(crate) fn box_padded_corners(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rotation: f32,
    pad: f32,
) -> [[f32; 2]; 4] {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (x0, y0) = (x - pad, y - pad);
    let (x1, y1) = (x + w + pad, y + h + pad);
    [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|[px_, py_]| {
        let (rx, ry) = rotate_pt(px_, py_, cx, cy, rotation);
        [rx, ry]
    })
}

/// Axis-aligned bounds of a set of points (empty → a zero box at the origin).
pub(crate) fn aabb(pts: &[[f32; 2]]) -> (f32, f32, f32, f32) {
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
pub(crate) fn rotatable(kind: &ElementKind) -> bool {
    !matches!(kind, ElementKind::Embed(_))
}

/// Snap an absolute orientation (radians) while rotating: with `shift`, to the
/// nearest 15°; otherwise to horizontal/vertical when within [`ROT_SNAP`], else
/// left free so any angle is still reachable away from the cardinals.
pub(crate) fn snap_angle(abs: f32, shift: bool) -> f32 {
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
pub(crate) fn reference_angle(kind: &ElementKind) -> Option<f32> {
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
pub(crate) fn rotate_element(kind: &mut ElementKind, cx: f32, cy: f32, delta: f32) {
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

/// Screen position of a group's rotate button: the intersection of the top
/// connector's horizontal guide and the right connector's vertical guide.
pub(crate) fn rotate_handle_for_bbox(
    bb: (f32, f32, f32, f32),
    cam: Camera,
    origin: Point<Pixels>,
) -> (f32, f32) {
    let top_right = to_screen(bb.2, bb.1, cam, origin);
    (
        f32::from(top_right.x) + CONNECTOR_BUTTON_GAP,
        f32::from(top_right.y) - CONNECTOR_BUTTON_GAP,
    )
}

/// Screen position of a single element's rotate button. For a rotated box the
/// offset follows its local top/right outward directions.
pub(crate) fn rotate_handle_screen(
    kind: &ElementKind,
    cam: Camera,
    origin: Point<Pixels>,
) -> (f32, f32) {
    if let Some((x, y, w, h, rotation)) = box_like(kind) {
        let corners = box_padded_corners(x, y, w, h, rotation, 0.0);
        let top_right = to_screen(corners[1][0], corners[1][1], cam, origin);
        let connectors = connector_points(kind);
        let top = to_screen(connectors[0][0], connectors[0][1], cam, origin);
        let right = to_screen(connectors[1][0], connectors[1][1], cam, origin);
        let center = to_screen(x + w / 2.0, y + h / 2.0, cam, origin);
        let unit = |point: Point<Pixels>| {
            let dx = f32::from(point.x - center.x);
            let dy = f32::from(point.y - center.y);
            let length = (dx * dx + dy * dy).sqrt().max(1.0);
            (dx / length, dy / length)
        };
        let up = unit(top);
        let right = unit(right);
        return (
            f32::from(top_right.x) + (up.0 + right.0) * CONNECTOR_BUTTON_GAP,
            f32::from(top_right.y) + (up.1 + right.1) * CONNECTOR_BUTTON_GAP,
        );
    }
    rotate_handle_for_bbox(bbox(kind), cam, origin)
}

/// An element's world-space bounding box `(min_x, min_y, max_x, max_y)`.
pub(crate) fn bbox(kind: &ElementKind) -> (f32, f32, f32, f32) {
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
pub(crate) fn elements_extent(elems: &[Element]) -> (f32, f32) {
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

/// Whether an element exposes connector points while hovered.
pub(crate) fn connector_capable(kind: &ElementKind) -> bool {
    matches!(kind, ElementKind::Text(_) | ElementKind::Image(_)) || is_closed_shape(kind)
}

/// Connector points for a box-like element: top, right, bottom, left midpoints,
/// rotated with the element. Empty for non-connectable kinds.
pub(crate) fn connector_points(kind: &ElementKind) -> Vec<[f32; 2]> {
    if !connector_capable(kind) {
        return Vec::new();
    }
    let Some((x, y, w, h, rot)) = box_like(kind) else {
        return Vec::new();
    };
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    [
        (x + w / 2.0, y),
        (x + w, y + h / 2.0),
        (x + w / 2.0, y + h),
        (x, y + h / 2.0),
    ]
    .into_iter()
    .map(|(px_, py_)| {
        let (rx, ry) = rotate_pt(px_, py_, cx, cy, rot);
        [rx, ry]
    })
    .collect()
}

pub(crate) fn connector_world_pos_in(
    elements: &[Element],
    anchor: SegmentAnchor,
) -> Option<[f32; 2]> {
    elements
        .iter()
        .find(|element| element.id == anchor.element_id)
        .and_then(|element| {
            connector_points(&element.kind)
                .get(anchor.connector)
                .copied()
        })
}

/// Whether `(wx, wy)` falls within an element's bounds, padded by `pad` (world).
pub(crate) fn hit_test(kind: &ElementKind, wx: f32, wy: f32, pad: f32) -> bool {
    let (x0, y0, x1, y1) = bbox(kind);
    wx >= x0 - pad && wx <= x1 + pad && wy >= y0 - pad && wy <= y1 + pad
}

/// Translate an element by a world-space delta.
pub(crate) fn translate(kind: &mut ElementKind, dx: f32, dy: f32) {
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
pub(crate) fn diagonal_scale(anchor: [f32; 2], from: [f32; 2], target: [f32; 2]) -> f32 {
    let d = [from[0] - anchor[0], from[1] - anchor[1]];
    let c = [target[0] - anchor[0], target[1] - anchor[1]];
    let dd = d[0] * d[0] + d[1] * d[1];
    if dd < 1e-6 {
        return 1.0;
    }
    (c[0] * d[0] + c[1] * d[1]) / dd
}

/// Scale factor along one axis: how far `target` sits from `anchor` relative to
/// `from` (per-axis edge group resize). Degenerate (`anchor == from`) → 1.0.
pub(crate) fn axis_scale(anchor: f32, from: f32, target: f32) -> f32 {
    let d = from - anchor;
    if d.abs() < 1e-6 {
        1.0
    } else {
        (target - anchor) / d
    }
}

/// World point `p` → block-local space (origin = the block's top-left `(x, y)`),
/// undoing rotation about `pivot` (the shape's center) — maps a click to a caret.
pub(crate) fn block_local(x: f32, y: f32, rotation: f32, pivot: [f32; 2], p: [f32; 2]) -> [f32; 2] {
    let (rx, ry) = rotate_pt(p[0], p[1], pivot[0], pivot[1], -rotation);
    [rx - x, ry - y]
}

/// Block-local point → world space, applying rotation about the block/shape pivot.
pub(crate) fn block_world(
    x: f32,
    y: f32,
    rotation: f32,
    pivot: [f32; 2],
    p: [f32; 2],
) -> (f32, f32) {
    rotate_pt(x + p[0], y + p[1], pivot[0], pivot[1], rotation)
}

/// Snap target `(tx, ty)` so its angle from `(ox, oy)` is a multiple of 45°,
/// preserving the distance (the line-drawing constraint for endpoint drags).
pub(crate) fn snap_45(ox: f32, oy: f32, tx: f32, ty: f32) -> (f32, f32) {
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
pub(crate) fn snap_grid(v: f32) -> f32 {
    (v / GRID).round() * GRID
}

/// Round an angle (radians) to the nearest quarter turn. Images rotate only in
/// 90° steps — gpui can't transform a raster sprite, so the host re-rotates the
/// pixels, and quarter turns keep that exact (no resampling) and cheap.
pub(crate) fn snap_quarter(rad: f32) -> f32 {
    let q = std::f32::consts::FRAC_PI_2;
    (rad / q).round() * q
}

/// Where a move-drag's primary element should sit: its grab-time top-left
/// (`origin`) plus the *total* cursor delta since the grab `anchor`, optionally
/// snapped to the grid. Driving an absolute target from the total delta (rather
/// than snapping each frame's increment) keeps the shape under the cursor and
/// lets sub-grid motion accumulate across frames instead of sticking.
pub(crate) fn move_target(
    origin: [f32; 2],
    anchor: [f32; 2],
    cursor: [f32; 2],
    snap: bool,
) -> [f32; 2] {
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

/// Approximate world-space (width, height) of a text element — enough for
/// hit-testing and the selection box (real shaping happens at paint time).
pub(crate) fn text_extent(t: &TextGeom) -> (f32, f32) {
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
pub(crate) fn resize_about(kind: &mut ElementKind, ax: f32, ay: f32, sx: f32, sy: f32) {
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
            // Position follows the (possibly per-axis) scale, but a glyph has a
            // single size — never stretched. The geometric mean keeps a
            // proportional resize (sx == sy) exact and an edge drag uniform
            // (scaling by the average of the two factors).
            t.x = fx(t.x);
            t.y = fy(t.y);
            t.size = (t.size * (sx.abs() * sy.abs()).sqrt()).max(0.5);
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
