//! Importing images that are pasted or dropped into notes. Files are copied
//! into [`paths::images_dir`] and referenced from markdown relatively as
//! `images/<name>` (resolved against the data dir by the image renderer), so
//! notes stay portable.

use std::fs;
use std::path::Path;

use crate::paths;

/// Extensions the renderer (gpui's image stack) can actually decode. AVIF is
/// deliberately absent — gpui can't decode it.
const SUPPORTED: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "ico", "svg",
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
    let dir = paths::images_dir();
    fs::create_dir_all(&dir)?;
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    let name = unique_name(&dir, &sanitize(stem), &ext);
    fs::copy(src, dir.join(&name))?;
    Ok(format!("images/{name}"))
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
