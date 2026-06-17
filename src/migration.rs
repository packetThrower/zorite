//! The startup "Moving your data…" progress window, shown while a scheduled
//! data-location move runs (see [`crate::paths::set_location`]). The move
//! happens before the main window opens — so without this, a large cross-volume
//! move would leave the app showing nothing until it finished.

use std::sync::Arc;

use gpui::{
    Context, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px,
};
use gpui_component::{TitleBar, progress::Progress};

use crate::paths::MigrationProgress;
use crate::theme;

/// A small window that renders a determinate progress bar from a shared
/// [`MigrationProgress`]. The startup loop re-renders it on a timer and closes
/// it once the move finishes.
pub struct MigrationView {
    progress: Arc<MigrationProgress>,
    target: SharedString,
}

impl MigrationView {
    pub fn new(progress: Arc<MigrationProgress>, target: String) -> Self {
        Self {
            progress,
            target: target.into(),
        }
    }
}

impl Render for MigrationView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let pct = self.progress.fraction() * 100.0;
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme::bg_window())
            .text_color(theme::text_primary())
            .child(TitleBar::new())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(14.0))
                    .px(px(40.0))
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Moving your data…"),
                    )
                    .child(
                        div()
                            .max_w(px(380.0))
                            .text_size(px(12.0))
                            .text_color(theme::text_secondary())
                            .child(self.target.clone()),
                    )
                    .child(
                        div().w(px(360.0)).child(
                            Progress::new("migration-progress")
                                .value(pct)
                                .color(theme::accent()),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::text_tertiary())
                            .child(format!("{}%", pct.round() as i32)),
                    ),
            )
    }
}
