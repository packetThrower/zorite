//! Standalone demo for the `gpui-editor` crate.
//!
//! Run with: `cargo run -p gpui-editor --example demo`

use gpui::{
    App, AppContext, Bounds, Context, Entity, Focusable, IntoElement, KeyBinding, ParentElement,
    Render, Styled, Window, WindowBounds, WindowOptions, actions, div, px, rgb, size,
};
use gpui_editor::EditorState;

actions!(demo, [Quit]);

struct Demo {
    editor: Entity<EditorState>,
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1e22))
            .text_color(rgb(0xe6e6e6))
            .text_size(px(16.))
            .p(px(28.))
            .child(
                div()
                    .bg(rgb(0x111114))
                    .border_1()
                    .border_color(rgb(0x333338))
                    .rounded(px(8.))
                    .p(px(14.))
                    .child(self.editor.clone()),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        gpui_editor::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(760.), px(520.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let editor = cx.new(|cx| {
                    EditorState::new(window, cx)
                        .with_placeholder("Type here…")
                        .with_text(
                            "gpui-editor demo\n\nA from-scratch multi-line editor.\n\
                             Type, select, arrow around, ⌘A / ⌘C / ⌘V / ⌘X.",
                        )
                });
                window.focus(&editor.read(cx).focus_handle(cx), cx);
                cx.new(|_| Demo { editor })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
