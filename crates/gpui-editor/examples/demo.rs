//! Standalone demo for the `gpui-editor` crate.
//!
//! Run with: `cargo run -p gpui-editor --example demo`.
//!
//! Wires the editor to the real OS spell checker (M6): misspelled words get red
//! squiggles, and right-clicking one offers the system's suggestions. Type to
//! watch the squiggles update live — the editor emits [`EditorEvent::Changed`]
//! on each edit, and we re-run the checker in response.

use gpui::{
    App, AppContext, Bounds, Context, Entity, Focusable, IntoElement, KeyBinding, ParentElement,
    Render, Styled, Subscription, Window, WindowBounds, WindowOptions, actions, div, font, hsla,
    px, rgb, size,
};
use gpui_editor::{Diagnostic, EditorEvent, EditorState, SyntaxStyle};
use spellcheck::SpellChecker;

actions!(demo, [Quit]);

struct Demo {
    editor: Entity<EditorState>,
    /// Held so the change subscription keeps firing for the window's lifetime.
    _spell_sub: Subscription,
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

/// Run the OS spell checker over `text` and turn the misspellings into editor
/// diagnostics (the byte ranges + suggestions map across one-to-one).
/// A dark-theme palette for the live-preview markdown styling.
fn demo_markdown_style() -> SyntaxStyle {
    SyntaxStyle {
        marker: hsla(0., 0., 0.5, 0.55),  // dimmed gray syntax markers
        code: hsla(0.09, 0.6, 0.72, 1.),  // warm inline code text
        code_bg: hsla(0., 0., 1., 0.06),  // faint code chip background
        link: hsla(0.58, 0.75, 0.66, 1.), // blue links / wiki-links
        tag: hsla(0.33, 0.45, 0.62, 1.),  // green tags
        mono: font("Menlo"),
    }
}

fn diagnostics_for(text: &str) -> Vec<Diagnostic> {
    SpellChecker::new()
        .check(text)
        .into_iter()
        .map(|range| Diagnostic { range })
        .collect()
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
                let text = "# Heading 1\n## Heading 2\n\nHeadings get bigger as you type (W2). \
                            **Bold**, *italic*, ~~strike~~, `inline code`, a \
                            [link](https://example.com), a [[Wiki Page]], and a #tag style \
                            inline.\n\nA fenced code block (W4b):\n\n```rust\nfn main() {\n    \
                            println!(\"hello, world\");\n}\n```\n\nA table (W4c):\n\n| Name | \
                            Role | Score |\n| :-- | :--: | --: |\n| Ada | Engineer | 99 |\n\
                            | Linus | Kernel | 88 |\n\nSpell-check still flags mispelled wrds; \
                            right-click one for suggestions.";
                let editor = cx.new(|cx| {
                    EditorState::new(window, cx)
                        .with_placeholder("Type here…")
                        .with_text(text)
                });
                window.focus(&editor.read(cx).focus_handle(cx), cx);

                // Lazy suggestion provider (consulted on right-click) + an
                // initial detection pass over the seeded text.
                editor.update(cx, |editor, cx| {
                    editor.on_suggest(|word| SpellChecker::new().suggestions(word));
                    editor.set_markdown_style(demo_markdown_style(), cx);
                    editor.set_diagnostics(diagnostics_for(text), cx);
                });

                // Re-check on every edit.
                let editor_handle = editor.clone();
                cx.new(|cx| {
                    let _spell_sub = cx.subscribe(
                        &editor_handle,
                        |_demo: &mut Demo, editor, _: &EditorEvent, cx| {
                            let text = editor.read(cx).text().to_string();
                            let diagnostics = diagnostics_for(&text);
                            editor.update(cx, |editor, cx| editor.set_diagnostics(diagnostics, cx));
                        },
                    );
                    Demo { editor, _spell_sub }
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
