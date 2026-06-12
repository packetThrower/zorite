//! Whiteboard support. The infinite-canvas view lives in the [`gpui_whiteboard`]
//! crate; this module re-exports what the app uses and supplies the theme.
//!
//! A whiteboard is a `kind = 'whiteboard'` page — its canvas JSON stored in the
//! page `content` — opened in its own [`WhiteboardView`] tab (see
//! [`crate::app::AppView::open_whiteboard`]). The design lives in
//! `docs/whiteboard-architecture.md`.

use std::rc::Rc;

pub use gpui_whiteboard::{Scene, WhiteboardStyle, WhiteboardStyleFn, WhiteboardView};

use crate::theme;

/// The board's theme colors, pulled live (read each paint) so the canvas follows
/// theme changes and can differ per window — the same pattern as [`crate::pdf`].
pub fn style() -> WhiteboardStyleFn {
    Rc::new(|| WhiteboardStyle {
        bg: theme::bg_content(),
        grid: theme::border_subtle(),
        text: theme::text_tertiary(),
        ink: theme::text_primary(),
        panel: theme::glass_strong(),
        accent: theme::accent_tint(),
        selection: theme::accent(),
    })
}
