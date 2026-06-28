//! Note images: importing (copy pasted/dropped files into the managed data-dir
//! folders) and rendering (decode at display resolution into GPU-ready bitmaps,
//! cached and explicitly freed).
//!
//! Imported files are copied into [`paths::images_dir`] / [`paths::pdf_dir`] and
//! referenced from markdown relatively (`images/<name>` / `pdf/<name>`, resolved
//! against the data dir), so notes stay portable. For rendering, [`ImageStore`]
//! decodes each image **downscaled to display size** and holds the bitmap until
//! the view changes — see its docs for why both halves matter.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{App, RenderImage, SharedString, Window};
use image::{Frame, RgbaImage};

use crate::paths;

/// Extensions the renderer can decode: gpui's `image` stack for the common
/// formats, plus the pure-Rust `heic_decoder` for HEIC/HEIF/AVIF — see
/// [`decode_heif`].
const SUPPORTED: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "ico", "svg", "heic", "heif", "avif",
];

/// Whether `path` looks like an image we can render (by extension).
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Copy an external image file into the images dir; return its `images/<name>`
/// reference for the markdown.
pub fn import_file(src: &Path) -> std::io::Result<String> {
    import_into(src, &paths::images_dir(), "images", "png")
}

/// Copy `src` into `dir`, giving it a unique, sanitized name; return the relative
/// `<rel_prefix>/<name>` reference. `default_ext` is used if `src` has none.
pub fn import_into(
    src: &Path,
    dir: &Path,
    rel_prefix: &str,
    default_ext: &str,
) -> std::io::Result<String> {
    fs::create_dir_all(dir)?;
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_prefix);
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or(default_ext)
        .to_lowercase();
    let name = unique_name(dir, &sanitize(stem), &ext);
    fs::copy(src, dir.join(&name))?;
    Ok(format!("{rel_prefix}/{name}"))
}

/// Save pasted image bytes into the images dir; return its `images/<name>` ref.
pub fn import_bytes(bytes: &[u8], ext: &str) -> std::io::Result<String> {
    let dir = paths::images_dir();
    fs::create_dir_all(&dir)?;
    let name = unique_name(&dir, "pasted", ext);
    fs::write(dir.join(&name), bytes)?;
    Ok(format!("images/{name}"))
}

/// A filename in `dir` that doesn't collide (`stem.ext`, then `stem-1.ext`, …).
fn unique_name(dir: &Path, stem: &str, ext: &str) -> String {
    let stem = if stem.is_empty() { "image" } else { stem };
    let mut name = format!("{stem}.{ext}");
    let mut i = 1;
    while dir.join(&name).exists() {
        name = format!("{stem}-{i}.{ext}");
        i += 1;
    }
    name
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

// --- Rendering: decode-at-display-size cache ---

/// Cap on a decoded image's longest edge. Phone photos are ~4000 px but a note
/// shows them at ~1000 px even on a Retina display; decoding at native size
/// costs ~11× the RAM for no visible gain (a 3024×4032 photo is 47 MB of RGBA
/// versus ~4 MB downscaled). 2048 stays crisp at any realistic note width.
const MAX_IMAGE_EDGE: u32 = 1280;

/// Decoded bitmaps handed from one window to another when a tab moves (drag a
/// tab out / into another window, right-click → "Open in new window"), so the
/// receiving window paints them immediately instead of re-decoding from disk.
/// `Arc` clones — no bitmap is copied.
pub type ImageSeed = Vec<(SharedString, Arc<RenderImage>)>;

/// A cache slot for one image source.
enum Slot {
    /// A decode is in flight (off-thread).
    Loading,
    /// Decoded, GPU-ready.
    Ready(Arc<RenderImage>),
    /// The file is missing or couldn't be decoded.
    Failed,
}

/// Decodes note images at **display** resolution and caches the GPU-ready
/// bitmaps. Two problems this solves, both of which surfaced once real phone
/// photos were imported (synthetic perf DBs had none):
///
/// 1. **Size** — gpui decodes an image to full-native-resolution RGBA
///    regardless of how small it's shown, so a 12-megapixel photo eats ~47 MB
///    of RAM to display at under 1 MP. [`decode_scaled`] downscales first.
/// 2. **Lifetime** — gpui never auto-evicts a `RenderImage` (CPU buffer *or*
///    its GPU atlas texture), so images accumulate for the life of the window.
///    [`ImageStore::release`] frees them when the view changes (the PDF viewer
///    does the same for its rasterized pages).
#[derive(Default)]
pub struct ImageStore {
    cache: HashMap<SharedString, Slot>,
}

impl ImageStore {
    /// The decoded bitmap for `src`, or `None` while it loads / on failure.
    pub fn get(&self, src: &str) -> Option<Arc<RenderImage>> {
        match self.cache.get(src) {
            Some(Slot::Ready(arc)) => Some(arc.clone()),
            _ => None,
        }
    }

    /// Whether `src` failed to decode — the caller shows a fallback, not a
    /// loading placeholder that would never resolve.
    pub fn failed(&self, src: &str) -> bool {
        matches!(self.cache.get(src), Some(Slot::Failed))
    }

    /// Claim `src` for loading. Returns `false` if it's already known (loading,
    /// ready, or failed), so the caller kicks off a decode exactly once.
    pub fn begin(&mut self, src: SharedString) -> bool {
        if self.cache.contains_key(&src) {
            return false;
        }
        self.cache.insert(src, Slot::Loading);
        true
    }

    /// Record a finished decode (`None` → failed).
    pub fn finish(&mut self, src: SharedString, image: Option<Arc<RenderImage>>) {
        self.cache
            .insert(src, image.map_or(Slot::Failed, Slot::Ready));
    }

    /// The decoded bitmaps, for seeding another window's store (see [`ImageSeed`]).
    pub fn snapshot(&self) -> ImageSeed {
        self.cache
            .iter()
            .filter_map(|(src, slot)| match slot {
                Slot::Ready(arc) => Some((src.clone(), arc.clone())),
                _ => None,
            })
            .collect()
    }

    /// Adopt bitmaps decoded by another window. Existing entries win — a decode
    /// already in flight here will land over its `Loading` slot, not be clobbered.
    pub fn adopt(&mut self, seed: ImageSeed) {
        for (src, arc) in seed {
            self.cache.entry(src).or_insert(Slot::Ready(arc));
        }
    }

    /// Free every cached bitmap — CPU buffer (dropping the `Arc`) and GPU atlas
    /// texture (`drop_image`). gpui caches one atlas texture per `RenderImage`
    /// on paint and only releases it here, so this must run before the bitmaps
    /// are forgotten or the textures leak.
    pub fn release(&mut self, window: &mut Window, cx: &mut App) {
        for (_, slot) in self.cache.drain() {
            if let Slot::Ready(arc) = slot {
                cx.drop_image(arc, Some(window));
            }
        }
    }
}

/// Decode `path` to a GPU-ready bitmap, downscaled so its longest edge is at
/// most [`MAX_IMAGE_EDGE`]. Runs off the UI thread. `None` if the file can't be
/// decoded.
pub fn decode_scaled(path: &Path) -> Option<Arc<RenderImage>> {
    // Fast path for JPEGs: decode at a reduced size (DCT scaling at the decoder),
    // a fraction of the work + memory of a full-resolution decode. Then the HEIF
    // decoder for HEIC/HEIF/AVIF (gpui's `image` stack can't read those). Else a
    // full `image::open`; the first to yield a bitmap wins, any miss falls through.
    let img = decode_jpeg_reduced(path)
        .or_else(|| decode_heif(path))
        .or_else(|| image::open(path).ok())?;
    let buf = scale_and_bgra(img);
    Some(Arc::new(RenderImage::new(vec![Frame::new(buf)])))
}

/// Decode a JPEG **downscaled at the decoder** (DCT scaling to ~[`MAX_IMAGE_EDGE`]
/// on the long edge), so a 12-megapixel phone photo costs a fraction of a
/// full-resolution decode + buffer (which then gets thumbnailed away anyway).
/// `None` for non-JPEGs, already-small images, uncommon pixel formats
/// (CMYK / 16-bit), or any decode error — the caller falls back to `image::open`.
fn decode_jpeg_reduced(path: &Path) -> Option<image::DynamicImage> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext != "jpg" && ext != "jpeg" {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut dec = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
    dec.read_info().ok()?;
    let info = dec.info()?;
    let (ow, oh) = (info.width as u32, info.height as u32);
    if ow.max(oh) <= MAX_IMAGE_EDGE {
        return None; // already small — the normal path is exact + cheap enough
    }
    let scale = MAX_IMAGE_EDGE as f32 / ow.max(oh) as f32;
    let req_w = ((ow as f32 * scale).round() as u16).max(1);
    let req_h = ((oh as f32 * scale).round() as u16).max(1);
    let (sw, sh) = dec.scale(req_w, req_h).ok()?;
    let pixels = dec.decode().ok()?;
    let (sw, sh) = (sw as u32, sh as u32);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            image::RgbImage::from_raw(sw, sh, pixels).map(image::DynamicImage::ImageRgb8)
        }
        jpeg_decoder::PixelFormat::L8 => {
            image::GrayImage::from_raw(sw, sh, pixels).map(image::DynamicImage::ImageLuma8)
        }
        _ => None, // CMYK32 / L16 — let image::open handle it
    }
}

/// Decode a HEIF-family image (`.heic`/`.heif`/`.avif`) via the pure-Rust
/// `heic_decoder` — HEVC through `scuffle-h265`, AV1 through `rav1d` — since
/// gpui's `image` stack can't. The decoder applies the container's own
/// `clap`/`irot`/`imir` transforms but not EXIF rotation, so we apply that here;
/// the hint declines when the primary item already carries a transform, so there
/// is no double-rotation. `None` for non-HEIF extensions, oversized input
/// (guardrails), or any decode error — including grid-tiled primary items, which
/// aren't supported — so the caller falls back and the note shows a placeholder.
fn decode_heif(path: &Path) -> Option<image::DynamicImage> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "heic" | "heif" | "avif") {
        return None;
    }
    let guardrails = heic_decoder::DecodeGuardrails {
        max_input_bytes: Some(256 * 1024 * 1024),
        max_pixels: Some(100_000_000),
        max_temp_spool_bytes: Some(256 * 1024 * 1024),
        temp_spool_directory: None,
    };
    // rav1d (the AVIF/AV1 decoder) builds its large frame-context state on the stack, which
    // overflows the background-executor pool thread's modest stack — a hard SIGILL/bus-error
    // crash on any AVIF/HEIC image. Run the decode on a dedicated thread with a generous
    // stack; the inputs and the decoded result are both `Send`.
    let owned_path = path.to_path_buf();
    let decoded = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            heic_decoder::decode_path_to_rgba_with_guardrails(&owned_path, guardrails).ok()
        })
        .ok()?
        .join()
        .ok()
        .flatten()?;
    // The decoder leaves EXIF orientation unapplied (libheif parity); apply it so
    // portrait phone photos aren't sideways. `orientation_to_apply` returns `None`
    // when the primary item already carries the rotation, avoiding double-turns.
    let decoded = match heic_decoder::exif_orientation_hint_from_path(path)
        .ok()
        .and_then(|hint| hint.orientation_to_apply())
    {
        Some(orientation) => decoded.apply_exif_orientation(orientation).ok()?,
        None => decoded,
    };
    let (w, h) = (decoded.width, decoded.height);
    match decoded.pixels {
        heic_decoder::DecodedRgbaPixels::U8(p) => {
            image::RgbaImage::from_raw(w, h, p).map(image::DynamicImage::ImageRgba8)
        }
        heic_decoder::DecodedRgbaPixels::U16(p) => {
            image::ImageBuffer::from_raw(w, h, p).map(image::DynamicImage::ImageRgba16)
        }
    }
}

/// Rotate a decoded bitmap by a multiple of 90° clockwise — `deg` is rounded to
/// the nearest quarter-turn. Exact (rows/columns are swapped, no resampling and
/// no bounding-box growth), so it's cheap and never enlarges the bitmap. gpui
/// can't transform a raster sprite, so a rotated image is rendered by
/// pre-rotating its pixels here (cached by the caller). Returns `None` for an
/// upright turn (the caller serves the original) or if the bytes are missing.
pub fn rotate_render_image(src: &RenderImage, deg: i32) -> Option<Arc<RenderImage>> {
    let size = src.size(0);
    let base = RgbaImage::from_raw(
        size.width.0 as u32,
        size.height.0 as u32,
        src.as_bytes(0)?.to_vec(),
    )?;
    let turned = match (deg.rem_euclid(360) + 45) / 90 % 4 {
        1 => image::imageops::rotate90(&base),
        2 => image::imageops::rotate180(&base),
        3 => image::imageops::rotate270(&base),
        _ => return None, // upright — caller uses the original bitmap
    };
    Some(Arc::new(RenderImage::new(vec![Frame::new(turned)])))
}

/// Downscale `img` so its longest edge is at most [`MAX_IMAGE_EDGE`], then
/// convert to the BGRA byte order [`RenderImage`] expects. Split out so the
/// scaling + channel swap are unit-testable without building a `RenderImage`.
fn scale_and_bgra(img: image::DynamicImage) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let small = if w.max(h) > MAX_IMAGE_EDGE {
        let scale = MAX_IMAGE_EDGE as f32 / w.max(h) as f32;
        let (nw, nh) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
        // `DynamicImage::thumbnail` box-samples in the image's native format
        // (no RGBA blow-up, no f32 inflation that the filtered `resize` would
        // do — ~146 MB for a 12-megapixel photo), so only the decoder's own
        // full-res buffer is ever allocated.
        img.thumbnail(nw.max(1), nh.max(1))
    } else {
        img
    };
    let mut rgba = small.into_rgba8();
    // gpui's `RenderImage` holds straight (non-premultiplied) BGRA; the decoded
    // buffer is RGBA, so swap R and B to match.
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    #[test]
    fn large_images_downscale_to_the_cap() {
        // A 4000×3000 photo (like an imported phone shot) is capped on its
        // longest edge, keeping aspect ratio — ~11× fewer pixels = ~11× less RAM.
        let big = DynamicImage::ImageRgba8(RgbaImage::new(4000, 3000));
        let out = scale_and_bgra(big);
        assert_eq!(out.width(), MAX_IMAGE_EDGE);
        assert_eq!(out.height(), MAX_IMAGE_EDGE * 3 / 4);
        assert!(out.width().max(out.height()) <= MAX_IMAGE_EDGE);
    }

    #[test]
    fn seed_adopts_into_empty_store_and_blocks_redecode() {
        let bitmap = Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]));
        let mut a = ImageStore::default();
        a.finish("images/x.png".into(), Some(bitmap));
        let seed = a.snapshot();
        assert_eq!(seed.len(), 1);

        let mut b = ImageStore::default();
        b.adopt(seed);
        // The adopted bitmap is served, and a placeholder's `begin` won't claim
        // the slot for a fresh decode.
        assert!(b.get("images/x.png").is_some());
        assert!(!b.begin("images/x.png".into()));
    }

    #[test]
    fn adopt_never_replaces_an_existing_entry() {
        // Replacing a Ready slot would orphan the old bitmap's GPU texture (only
        // `release` frees it), so adopt must keep what's already there.
        let old = Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]));
        let new = Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(2, 2))]));
        let mut store = ImageStore::default();
        store.finish("images/x.png".into(), Some(old.clone()));
        store.adopt(vec![("images/x.png".into(), new)]);
        assert_eq!(store.get("images/x.png").unwrap().id, old.id);
    }

    #[test]
    fn small_images_are_untouched_but_swapped_to_bgra() {
        // Under the cap: original size kept, only RGBA→BGRA channel swap applied.
        let mut src = RgbaImage::new(2, 1);
        src.put_pixel(0, 0, Rgba([10, 20, 30, 255])); // R,G,B,A
        let out = scale_and_bgra(DynamicImage::ImageRgba8(src));
        assert_eq!((out.width(), out.height()), (2, 1));
        assert_eq!(out.get_pixel(0, 0).0, [30, 20, 10, 255]); // B,G,R,A
    }

    #[test]
    fn rotate_quarter_turns_swap_dimensions() {
        let src = RenderImage::new(vec![Frame::new(RgbaImage::from_pixel(
            40,
            20,
            Rgba([10, 20, 30, 255]),
        ))]);
        let dims = |arc: &RenderImage| (arc.size(0).width.0, arc.size(0).height.0);
        // 90° / 270° swap exactly; 180° keeps the dimensions.
        assert_eq!(dims(&rotate_render_image(&src, 90).unwrap()), (20, 40));
        assert_eq!(dims(&rotate_render_image(&src, 180).unwrap()), (40, 20));
        assert_eq!(dims(&rotate_render_image(&src, 270).unwrap()), (20, 40));
        // Off-axis angles snap to the nearest quarter turn (no bbox growth).
        assert_eq!(dims(&rotate_render_image(&src, 80).unwrap()), (20, 40));
        // An ~upright turn returns None — the caller serves the original.
        assert!(rotate_render_image(&src, 5).is_none());
        assert!(rotate_render_image(&src, 0).is_none());
    }
}
