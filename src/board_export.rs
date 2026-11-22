//! Whiteboards out of the app: SVG (images embedded), PNG (rasterized with
//! tiny-skia from the same drawing), and the PNG on the clipboard. The crate
//! turns a board into a [`Drawing`]; this renders it on white with dark ink,
//! whatever the theme, so an export reads anywhere.

use base64::Engine as _;
use gpui_whiteboard::{DrawItem, Drawing, Seg};
use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, StrokeDash,
    Transform,
};

/// Ink for elements with no color of their own.
pub const INK: u32 = 0x1F1F23FF;
const BACKGROUND: u32 = 0xFFFFFFFF;
/// PNG pixels per world unit: crisp on a high-DPI screen.
const PNG_SCALE: f32 = 2.0;

fn image_path(src: &str) -> Option<std::path::PathBuf> {
    crate::paths::resolve_local(src).filter(|p| p.is_file())
}

/// The board as SVG, images inlined as `data:` URIs so the file stands alone.
pub fn svg(d: &Drawing) -> String {
    d.to_svg(Some(BACKGROUND), |src| {
        let path = image_path(src)?;
        let bytes = std::fs::read(&path).ok()?;
        let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            _ => "image/png",
        };
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        Some(format!("data:{mime};base64,{data}"))
    })
}

fn color(c: u32) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8((c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn path(segs: &[Seg]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for s in segs {
        match *s {
            Seg::Move(p) => pb.move_to(p[0], p[1]),
            Seg::Line(p) => pb.line_to(p[0], p[1]),
            Seg::Quad(c, p) => pb.quad_to(c[0], c[1], p[0], p[1]),
            Seg::Cubic(a, b, p) => pb.cubic_to(a[0], a[1], b[0], b[1], p[0], p[1]),
            Seg::Close => pb.close(),
        }
    }
    pb.finish()
}

/// The board as a PNG at [`PNG_SCALE`]. `None` if it's too large to allocate.
pub fn png(d: &Drawing) -> Option<Vec<u8>> {
    let [bx, by, bw, bh] = d.bounds;
    let (w, h) = (
        (bw * PNG_SCALE).ceil() as u32,
        (bh * PNG_SCALE).ceil() as u32,
    );
    let mut pixmap = Pixmap::new(w.max(1), h.max(1))?;
    pixmap.fill(color(BACKGROUND));
    // World → pixels.
    let view = Transform::from_row(
        PNG_SCALE,
        0.0,
        0.0,
        PNG_SCALE,
        -bx * PNG_SCALE,
        -by * PNG_SCALE,
    );
    for item in &d.items {
        match item {
            DrawItem::Path {
                segs,
                fill,
                stroke,
                transform: t,
            } => {
                let Some(p) = path(segs) else { continue };
                let place =
                    view.pre_concat(Transform::from_row(t[0], t[1], t[2], t[3], t[4], t[5]));
                let mut paint = Paint {
                    anti_alias: true,
                    ..Paint::default()
                };
                if let Some(f) = fill {
                    paint.set_color(color(*f));
                    pixmap.fill_path(&p, &paint, FillRule::Winding, place, None);
                }
                if let Some((s, width, dashed)) = stroke {
                    paint.set_color(color(*s));
                    let dash = dashed
                        .then(|| {
                            StrokeDash::new(
                                vec![(width * 4.5).max(10.0), (width * 2.5).max(6.0)],
                                0.0,
                            )
                        })
                        .flatten();
                    let st = Stroke {
                        width: *width,
                        line_cap: LineCap::Round,
                        line_join: LineJoin::Round,
                        dash,
                        ..Stroke::default()
                    };
                    pixmap.stroke_path(&p, &paint, &st, place, None);
                }
            }
            DrawItem::Image {
                src,
                rect,
                transform: t,
            } => {
                let Some(img) = image_path(src)
                    .and_then(|p| image::open(p).ok())
                    .map(|i| i.to_rgba8())
                else {
                    continue;
                };
                let (iw, ih) = img.dimensions();
                let mut data = img.into_raw();
                // tiny-skia wants premultiplied alpha.
                for px in data.as_chunks_mut::<4>().0 {
                    let a = px[3] as u16;
                    for c in &mut px[..3] {
                        *c = ((*c as u16 * a + 127) / 255) as u8;
                    }
                }
                let Some(src_px) = tiny_skia::IntSize::from_wh(iw, ih)
                    .and_then(|size| Pixmap::from_vec(data, size))
                else {
                    continue;
                };
                let place = view
                    .pre_concat(Transform::from_row(t[0], t[1], t[2], t[3], t[4], t[5]))
                    .pre_translate(rect[0], rect[1])
                    .pre_scale(rect[2] / iw as f32, rect[3] / ih as f32);
                pixmap.draw_pixmap(0, 0, src_px.as_ref(), &PixmapPaint::default(), place, None);
            }
        }
    }
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_draws_shapes_on_white() {
        let scene = gpui_whiteboard::Scene::from_json(
            r#"{"elements":[
                {"id":1,"kind":{"rect":{"x":0,"y":0,"w":200,"h":100,"width":2}},"label":"Hello","fill":4294156447},
                {"id":2,"kind":{"ellipse":{"x":260,"y":0,"w":120,"h":100,"width":3}},"stroke":3815994623},
                {"id":3,"kind":{"sticky":{"x":420,"y":0,"w":140,"h":140,"width":1}},"label":"Call the plumber","fill":4291420927}
            ]}"#,
        );
        let d = scene
            .to_drawing(&Default::default(), gpui::rgba(INK).into())
            .expect("a drawing");
        let png = png(&d).expect("png");
        if let Ok(dir) = std::env::var("BOARD_PNG_OUT") {
            std::fs::write(format!("{dir}/board.png"), &png).unwrap();
            std::fs::write(format!("{dir}/board.svg"), svg(&d)).unwrap();
        }
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(img.width(), (d.bounds[2] * PNG_SCALE).ceil() as u32);
        // White background at the corner, ink somewhere.
        assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert!(img.pixels().any(|p| p.0[0] < 128 && p.0[1] < 128));
        assert!(svg(&d).starts_with("<svg"));
    }
}
