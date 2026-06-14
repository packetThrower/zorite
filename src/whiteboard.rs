//! Whiteboard support. The infinite-canvas view lives in the [`gpui_whiteboard`]
//! crate; this module re-exports what the app uses and supplies the theme.
//!
//! A whiteboard is a `kind = 'whiteboard'` page — its canvas JSON stored in the
//! page `content` — opened in its own [`WhiteboardView`] tab (see
//! [`crate::app::AppView::open_whiteboard`]). The design lives in
//! `docs/whiteboard-architecture.md`.

use std::rc::Rc;

pub use gpui_whiteboard::{
    ElementKind, Font, FontPick, Scene, Template, WhiteboardStyle, WhiteboardStyleFn,
    WhiteboardView,
};

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
        // Readable surface for popovers (color picker, flyouts, right-click
        // menu). `elevated` would do, but in light themes it equals the white
        // canvas (`bg_content`) and the panel vanishes — `bg_sidebar` is a
        // distinct light grey there, and in dark themes it's the same raised tone
        // `elevated` already uses. A hint of transparency keeps the overlay feel;
        // a drop shadow (in the crate) lifts it off a same-colored background.
        panel_strong: gpui::Hsla {
            a: 0.92,
            ..theme::bg_sidebar()
        },
        accent: theme::accent_tint(),
        selection: theme::accent(),
        // Quick swatches for the color picker: neutrals (ink → white) plus a
        // spread of hues. The picker's gradient area covers everything else.
        swatches: vec![
            theme::text_primary(),
            gpui::hsla(0.0, 0.0, 0.45, 1.0),
            gpui::hsla(0.0, 0.0, 1.0, 1.0),
            theme::accent(),
            gpui::hsla(0.00, 0.72, 0.55, 1.0), // red
            gpui::hsla(0.08, 0.85, 0.55, 1.0), // orange
            gpui::hsla(0.14, 0.80, 0.52, 1.0), // amber
            gpui::hsla(0.33, 0.55, 0.45, 1.0), // green
            gpui::hsla(0.52, 0.65, 0.50, 1.0), // teal
            gpui::hsla(0.60, 0.70, 0.55, 1.0), // blue
            gpui::hsla(0.75, 0.55, 0.58, 1.0), // violet
            gpui::hsla(0.90, 0.65, 0.60, 1.0), // pink
        ],
    })
}
