/// World-space viewport used to skip elements that cannot affect the current
/// canvas. The screen-space margin keeps strokes, arrowheads, and handles from
/// popping at an edge while panning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WorldViewport {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl WorldViewport {
    pub(crate) fn from_canvas(
        width_px: f32,
        height_px: f32,
        camera_x: f32,
        camera_y: f32,
        zoom: f32,
        margin_px: f32,
    ) -> Option<Self> {
        if width_px <= 1.0 || height_px <= 1.0 || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        let margin = margin_px.max(0.0) / zoom;
        Some(Self {
            left: camera_x - margin,
            top: camera_y - margin,
            right: camera_x + width_px / zoom + margin,
            bottom: camera_y + height_px / zoom + margin,
        })
    }

    pub(crate) fn intersects(self, bounds: (f32, f32, f32, f32)) -> bool {
        let (left, top, right, bottom) = bounds;
        right >= self.left && left <= self.right && bottom >= self.top && top <= self.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_converts_screen_margin_to_world_units() {
        let viewport = WorldViewport::from_canvas(800.0, 600.0, 100.0, 200.0, 2.0, 80.0)
            .expect("valid viewport");

        assert!(viewport.intersects((60.0, 160.0, 61.0, 161.0)));
        assert!(viewport.intersects((539.0, 539.0, 540.0, 540.0)));
        assert!(!viewport.intersects((40.0, 100.0, 50.0, 150.0)));
        assert!(!viewport.intersects((541.0, 541.0, 600.0, 600.0)));
    }

    #[test]
    fn unknown_canvas_size_disables_culling_for_first_paint() {
        assert_eq!(
            WorldViewport::from_canvas(0.0, 600.0, 0.0, 0.0, 1.0, 80.0),
            None
        );
        assert_eq!(
            WorldViewport::from_canvas(800.0, 0.0, 0.0, 0.0, 1.0, 80.0),
            None
        );
    }
}
