//! Text-input (IME) plumbing: the [`EntityInputHandler`] impl that routes
//! platform text input into the active text-edit target, and the invisible
//! [`WhiteboardInputElement`] that registers it each frame — split from `lib.rs`.

use super::*;

impl EntityInputHandler for WhiteboardView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.editing_content()?;
        let range = Self::utf16_range_to_utf8_in(&text, &range_utf16);
        actual_range.replace(Self::utf8_range_to_utf16_in(&text, &range));
        Some(text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let text = self.editing_content()?;
        let range = {
            let (s, e) = self.sel_range();
            s..e
        };
        Some(UTF16Selection {
            range: Self::utf8_range_to_utf16_in(&text, &range),
            reversed: self.caret < self.sel_anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let text = self.editing_content()?;
        self.marked_range
            .as_ref()
            .map(|range| Self::utf8_range_to_utf16_in(&text, range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.editing_content() else {
            return;
        };
        let visible_range = range_utf16
            .as_ref()
            .map(|range| Self::utf16_range_to_utf8_in(&text, range))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| {
                let (start, end) = self.sel_range();
                start..end
            });
        self.replace_text_in_visible_range(visible_range, new_text, None, false, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.editing_content() else {
            return;
        };
        let visible_range = range_utf16
            .as_ref()
            .map(|range| Self::utf16_range_to_utf8_in(&text, range))
            .or(self.marked_range.clone())
            .unwrap_or_else(|| {
                let (start, end) = self.sel_range();
                start..end
            });
        let selected_range_relative = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| Self::utf16_range_to_utf8_in(new_text, range_utf16))
            .map(|relative| relative.start..relative.end);

        self.replace_text_in_visible_range(
            visible_range,
            new_text,
            selected_range_relative,
            !new_text.is_empty(),
            cx,
        );
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let id = self.editing?;
        let tg = self.edit_target(id)?;
        let range = Self::utf16_range_to_utf8_in(&tg.content, &range_utf16);
        let caret = self
            .font
            .caret_pos_wrapped(&tg.content, tg.size, tg.wrap, range.end);
        let (wx, wy) = block_world(tg.x, tg.y, tg.rotation, tg.pivot, caret);
        let sp = to_screen(wx, wy, self.scene.camera, bounds.origin);
        Some(Bounds {
            origin: sp,
            size: size(
                px(1.0),
                px((tg.size * self.scene.camera.zoom.max(MIN_ZOOM)).max(12.0)),
            ),
        })
    }

    fn character_index_for_point(
        &mut self,
        pt: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let id = self.editing?;
        let tg = self.edit_target(id)?;
        let p = self.event_to_world(pt);
        let local = block_local(tg.x, tg.y, tg.rotation, tg.pivot, p);
        let idx = self
            .font
            .index_at_wrapped(&tg.content, tg.size, tg.wrap, local);
        Some(Self::utf8_to_utf16_in(&tg.content, idx))
    }
}

pub(crate) struct WhiteboardInputElement {
    input: Entity<WhiteboardView>,
}

impl WhiteboardInputElement {
    pub(crate) fn new(input: Entity<WhiteboardView>) -> Self {
        Self { input }
    }
}

impl IntoElement for WhiteboardInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for WhiteboardInputElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus.clone();
        if focus_handle.is_focused(window) {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.input.clone()),
                cx,
            );
        }
    }
}
