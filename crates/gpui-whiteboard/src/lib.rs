//! An infinite, pannable/zoomable whiteboard canvas for GPUI.
//!
//! Host-agnostic — depends only on `gpui` + `serde`. Modeled on the
//! `gpui-pdf` stateful-entity pattern: the host builds a [`WhiteboardView`]
//! from a [`Scene`], stores the entity, and renders it in a tab. This crate
//! owns the scene model and its (de)serialization; the host owns persistence,
//! theme, and navigation.
//!
//! **Phase 0** (this version): renders a blank board surface and round-trips a
//! [`Scene`] through JSON. The world→screen camera (pan/zoom), a background
//! grid, freehand strokes, shapes, arrows, and embedded page-cards arrive in
//! later phases — see `docs/whiteboard-architecture.md` in the host repo.

use std::rc::Rc;

use gpui::{
    Context, Hsla, IntoElement, ParentElement, Render, SharedString, Styled, Window, canvas, div,
    fill, px,
};
use serde::{Deserialize, Serialize};

/// The board document: everything persisted for a whiteboard. Owned and
/// (de)serialized here; the host stores [`Scene::to_json`] opaquely (for zorite,
/// in the `content` column of a `kind = 'whiteboard'` page).
///
/// Every field is `#[serde(default)]` so older boards keep loading as the model
/// grows — e.g. when an `elements` vector is added with the drawing phase, a
/// board saved today still deserializes.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scene {
    /// The viewport (pan + zoom). Persisted so a board reopens where you left it.
    #[serde(default)]
    pub camera: Camera,
}

impl Scene {
    /// Parse a board from its stored JSON, falling back to an empty board on
    /// empty or malformed input — a corrupt row never blocks opening the tab.
    pub fn from_json(s: &str) -> Self {
        if s.trim().is_empty() {
            return Self::default();
        }
        serde_json::from_str(s).unwrap_or_else(|e| {
            log::warn!("whiteboard: ignoring bad scene JSON ({e}); starting empty");
            Self::default()
        })
    }

    /// Serialize for persistence.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// The viewport: a world-space pan offset and a zoom factor. Once panning and
/// zooming land (Phase 1), every element coordinate maps to the screen as
/// `screen = (world - offset) * zoom`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Camera {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default = "one")]
    pub zoom: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

/// Theme colors, read at paint time (via [`WhiteboardStyleFn`]) so the board
/// follows live theme changes per window.
#[derive(Clone, Copy, Debug)]
pub struct WhiteboardStyle {
    /// The canvas background.
    pub bg: Hsla,
    /// The background grid / hairlines (used from Phase 1).
    pub grid: Hsla,
    /// Placeholder / muted on-canvas text.
    pub text: Hsla,
}

/// A `() -> WhiteboardStyle` the host supplies; called each paint so the board
/// tracks theme changes without the host pushing updates.
pub type WhiteboardStyleFn = Rc<dyn Fn() -> WhiteboardStyle>;

/// The whiteboard view entity. The host holds it in an `Entity<WhiteboardView>`
/// (keyed by board id) and renders it into a tab.
pub struct WhiteboardView {
    scene: Scene,
    style: WhiteboardStyleFn,
}

impl WhiteboardView {
    /// Build a view over `scene`. Call inside `cx.new(|cx| WhiteboardView::new(..))`.
    pub fn new(scene: Scene, style: WhiteboardStyleFn, _cx: &mut Context<Self>) -> Self {
        Self { scene, style }
    }

    /// The current board document (for the host to persist).
    pub fn scene(&self) -> &Scene {
        &self.scene
    }
}

impl Render for WhiteboardView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let style = (self.style)();
        let bg = style.bg;
        let zoom = self.scene.camera.zoom;
        div()
            .size_full()
            .relative()
            // The board surface. Phase 0 paints just the background fill through a
            // `canvas()` — proving the custom-paint hook end to end; the grid and
            // elements paint into this same canvas in later phases.
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        window.paint_quad(fill(bounds, bg));
                    },
                )
                .absolute()
                .size_full(),
            )
            // Placeholder label, over the canvas. Shows the loaded camera zoom so
            // the JSON round-trip is visible; replaced by real chrome later.
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(style.text)
                    .text_size(px(15.0))
                    .child(SharedString::from(format!(
                        "Whiteboard · {:.0}%",
                        zoom * 100.0
                    ))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_garbage_loads_a_blank_board() {
        for s in ["", "   ", "not json", "{}"] {
            assert_eq!(Scene::from_json(s).camera.zoom, 1.0, "input {s:?}");
        }
    }

    #[test]
    fn camera_round_trips_through_json() {
        let scene = Scene {
            camera: Camera {
                x: 12.5,
                y: -4.0,
                zoom: 2.0,
            },
        };
        let restored = Scene::from_json(&scene.to_json());
        assert_eq!(restored.camera.x, 12.5);
        assert_eq!(restored.camera.y, -4.0);
        assert_eq!(restored.camera.zoom, 2.0);
    }

    #[test]
    fn missing_camera_fields_fall_back_to_defaults() {
        // zoom present, x/y absent: x/y default to 0, zoom is honored. This is
        // what keeps old boards loading as the model grows.
        let scene = Scene::from_json(r#"{"camera":{"zoom":3.0}}"#);
        assert_eq!(scene.camera.x, 0.0);
        assert_eq!(scene.camera.y, 0.0);
        assert_eq!(scene.camera.zoom, 3.0);
    }
}
