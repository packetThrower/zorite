//! The XCursor binary format — parse and write, pure Rust.
//!
//! Layout (all little-endian u32): a 16-byte header (`"Xcur"`, header size,
//! version, TOC count), a TOC of (type, subtype, offset) triples, then chunks.
//! Image chunks (type `0xfffd_0002`) carry a 36-byte header — chunk-header
//! size, type, subtype (= nominal size), version, width, height, xhot, yhot,
//! delay — followed by `width * height` premultiplied ARGB pixels, which in
//! little-endian byte order is exactly the BGRA of [`crate::Image`].

use crate::Image;

const MAGIC: &[u8; 4] = b"Xcur";
const IMAGE_TYPE: u32 = 0xfffd_0002;
const IMAGE_HEADER: u32 = 36;
/// Sanity cap — no real cursor is larger (Bibata's biggest is 96px).
const MAX_DIM: u32 = 1024;

fn u32_at(data: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?))
}

/// Parse every image frame in an XCursor file, TOC order (sizes ascending,
/// animation frames consecutive). `None` on anything malformed.
pub fn parse(data: &[u8]) -> Option<Vec<Image>> {
    if data.get(..4)? != MAGIC {
        return None;
    }
    let ntoc = u32_at(data, 12)?;
    let mut images = Vec::new();
    for i in 0..ntoc as usize {
        let toc = 16 + i * 12;
        if u32_at(data, toc)? != IMAGE_TYPE {
            continue;
        }
        let pos = u32_at(data, toc + 8)? as usize;
        let (width, height) = (u32_at(data, pos + 16)?, u32_at(data, pos + 20)?);
        if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
            return None;
        }
        let pixels = pos + IMAGE_HEADER as usize;
        let len = width as usize * height as usize * 4;
        images.push(Image {
            size: u32_at(data, pos + 8)?,
            width,
            height,
            hotspot: (u32_at(data, pos + 24)?, u32_at(data, pos + 28)?),
            delay: u32_at(data, pos + 32)?,
            bgra: data.get(pixels..pixels + len)?.to_vec(),
        });
    }
    Some(images)
}

/// Encode frames as an XCursor file (give them in TOC order — sizes
/// ascending; [`parse`]'s output order round-trips).
pub fn write(images: &[Image]) -> Vec<u8> {
    let ntoc = images.len() as u32;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    for v in [16u32, 0x1_0000, ntoc] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut pos = 16 + ntoc * 12;
    for img in images {
        for v in [IMAGE_TYPE, img.size, pos] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        pos += IMAGE_HEADER + img.bgra.len() as u32;
    }
    for img in images {
        for v in [
            IMAGE_HEADER,
            IMAGE_TYPE,
            img.size,
            1,
            img.width,
            img.height,
            img.hotspot.0,
            img.hotspot.1,
            img.delay,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&img.bgra);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(size: u32, delay: u32) -> Image {
        Image {
            size,
            width: size,
            height: size,
            hotspot: (size / 4, size / 3),
            delay,
            bgra: (0..size * size * 4).map(|i| i as u8).collect(),
        }
    }

    #[test]
    fn roundtrip() {
        let frames = vec![frame(2, 0), frame(4, 50), frame(4, 50)];
        assert_eq!(parse(&write(&frames)).unwrap(), frames);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse(b"notacursor"), None);
        assert_eq!(parse(&[]), None);
        let mut truncated = write(&[frame(4, 0)]);
        truncated.truncate(truncated.len() - 1);
        assert_eq!(parse(&truncated), None);
    }

    #[test]
    fn best_image_picks_smallest_covering_size() {
        let frames = vec![frame(24, 0), frame(48, 0), frame(96, 0)];
        assert_eq!(crate::best_image(&frames, 40).unwrap().size, 48);
        assert_eq!(crate::best_image(&frames, 48).unwrap().size, 48);
        assert_eq!(crate::best_image(&frames, 200).unwrap().size, 96);
        assert!(crate::best_image(&[], 32).is_none());
    }
}
