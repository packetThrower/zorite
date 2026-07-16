//! Rich-text clipboard writes: markdown goes out as BOTH the raw source
//! (the plain-text flavor) and rendered HTML, so Word/Mail/Docs paste
//! formatted text while code editors and terminals get the markdown.
//!
//! gpui's clipboard has no HTML flavor (`ClipboardEntry` is string / image /
//! paths only), so the write goes through `arboard` — NSPasteboard on macOS,
//! CF_HTML on Windows, the X11/Wayland selections on Linux. If the platform
//! write fails (e.g. a Wayland compositor without the data-control
//! protocol), it falls back to gpui's plain-string copy so a copy never
//! silently does nothing.

use gpui::{App, ClipboardItem};

/// Put `markdown` on the clipboard as plain text + rendered HTML.
pub fn copy_rich(markdown: &str, cx: &mut App) {
    let html =
        markdown::to_html_with_options(markdown, &markdown::Options::gfm()).unwrap_or_default();
    let wrote = !html.is_empty()
        && arboard::Clipboard::new()
            .and_then(|mut c| c.set_html(html, Some(markdown.to_string())))
            .is_ok();
    if !wrote {
        cx.write_to_clipboard(ClipboardItem::new_string(markdown.to_string()));
    }
}
