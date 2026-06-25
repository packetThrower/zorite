//! Interactive gpui demo — a 3-slot structural editor for ∫.
//!
//! Three editable slots: lower limit, upper limit, integrand. Tab / ←/→ move the
//! caret between them; type into whichever is active; Backspace deletes; Esc clears
//! the active slot. Each keystroke mutates the model → RaTeX re-layouts + re-renders
//! → caret repositioned from the fresh `LayoutBox` geometry (limits from the `SupSub`
//! box's shift/scale fields). When the current input isn't parseable yet (e.g. a lone
//! `^`), the caret turns red and the last good render holds until you complete it.
//!
//!   cargo run --manifest-path spikes/ratex-probe/Cargo.toml --bin demo

use gpui::*;
use ratex_layout::layout_box::BoxContent;
use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_parser::parse;
use ratex_render::{render_to_png, RenderOptions};
use std::path::PathBuf;

const FONT: f32 = 72.0;
const PAD: f32 = 16.0;
const DPR: f32 = 2.0;

// slot indices
const LOWER: usize = 0;
const UPPER: usize = 1;
const INTEGRAND: usize = 2;

struct MathDemo {
    slots: [String; 3],
    active: usize,
    valid: bool,
    focus: FocusHandle,
    render_n: u64,
    png: PathBuf,
    img_w: f32,
    img_h: f32,
    /// (x, top, height) in logical px, per slot.
    carets: [(f32, f32, f32); 3],
}

impl MathDemo {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            slots: ["0".into(), "1".into(), String::new()],
            active: INTEGRAND,
            valid: true,
            focus: cx.focus_handle(),
            render_n: 0,
            png: PathBuf::new(),
            img_w: 0.0,
            img_h: 0.0,
            carets: [(0.0, 0.0, 0.0); 3],
        };
        this.rerender();
        this
    }

    fn body(s: &str) -> String {
        if s.is_empty() {
            r"\square".to_string()
        } else {
            format!("{{{}}}", s)
        }
    }

    fn rerender(&mut self) {
        let latex = format!(
            r"\int_{}^{} {} \, dx",
            Self::body(&self.slots[LOWER]),
            Self::body(&self.slots[UPPER]),
            Self::body(&self.slots[INTEGRAND]),
        );

        let Ok(nodes) = parse(&latex) else {
            self.valid = false;
            return;
        };
        let root = layout(&nodes, &LayoutOptions::default());
        let dl = to_display_list(&root);
        let opts = RenderOptions {
            font_size: FONT,
            padding: PAD,
            device_pixel_ratio: DPR,
            ..Default::default()
        };
        let Ok(png_bytes) = render_to_png(&dl, &opts) else {
            self.valid = false;
            return;
        };
        self.valid = true;

        self.render_n += 1;
        let png = std::env::temp_dir().join(format!("ratex-demo-{}.png", self.render_n));
        if std::fs::write(&png, &png_bytes).is_err() {
            return;
        }
        self.png = png;
        self.img_w = dl.width as f32 * FONT + 2.0 * PAD;
        self.img_h = (dl.height + dl.depth) as f32 * FONT + 2.0 * PAD;
        self.carets = self.compute_carets(&root, dl.height as f32);
    }

    /// Caret rect (x, top, h) for each slot, in logical px.
    fn compute_carets(&self, root: &ratex_layout::layout_box::LayoutBox, baseline_em: f32) -> [(f32, f32, f32); 3] {
        let mut carets = [(PAD, PAD, FONT); 3];
        let BoxContent::HBox(kids) = &root.content else {
            return carets;
        };

        // Limits live in the leading SupSub (∫ with scripts).
        if let Some(first) = kids.first() {
            if let BoxContent::SupSub {
                base,
                sup,
                sub,
                sup_shift,
                sub_shift,
                sup_scale,
                sub_scale,
                italic_correction,
                sub_h_kern,
                ..
            } = &first.content
            {
                let bw = base.width as f32;
                if let Some(s) = sup {
                    let sc = *sup_scale as f32;
                    let x = bw + *italic_correction as f32 + s.width as f32 * sc;
                    let by = baseline_em - *sup_shift as f32;
                    let h = ((s.height + s.depth) as f32 * sc).max(0.5) * FONT;
                    carets[UPPER] = (x * FONT + PAD, (by - s.height as f32 * sc) * FONT + PAD, h);
                }
                if let Some(s) = sub {
                    let sc = *sub_scale as f32;
                    let x = bw - *sub_h_kern as f32 + s.width as f32 * sc;
                    let by = baseline_em + *sub_shift as f32;
                    let h = ((s.height + s.depth) as f32 * sc).max(0.5) * FONT;
                    carets[LOWER] = (x * FONT + PAD, (by - s.height as f32 * sc) * FONT + PAD, h);
                }
            }
        }

        // Integrand: first non-(SupSub/OpLimits/Kern) child after the operator.
        let mut x_em = 0.0_f32;
        let mut iw = 0.0_f32;
        for k in kids {
            match &k.content {
                BoxContent::SupSub { .. } | BoxContent::OpLimits { .. } | BoxContent::Kern => {
                    x_em += k.width as f32;
                }
                _ => {
                    iw = k.width as f32;
                    break;
                }
            }
        }
        let cx_em = if self.slots[INTEGRAND].is_empty() {
            x_em
        } else {
            x_em + iw
        };
        carets[INTEGRAND] = (
            cx_em * FONT + PAD,
            (baseline_em - 0.82) * FONT + PAD,
            1.08 * FONT,
        );

        carets
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        if ks.modifiers.platform || ks.modifiers.control {
            return;
        }
        match ks.key.as_str() {
            "tab" => {
                self.active = if ks.modifiers.shift {
                    (self.active + 2) % 3
                } else {
                    (self.active + 1) % 3
                };
                cx.notify();
                return;
            }
            "right" => {
                self.active = (self.active + 1) % 3;
                cx.notify();
                return;
            }
            "left" => {
                self.active = (self.active + 2) % 3;
                cx.notify();
                return;
            }
            "backspace" => {
                self.slots[self.active].pop();
            }
            "escape" => {
                self.slots[self.active].clear();
            }
            _ => match ks.key_char.as_ref() {
                Some(c) => self.slots[self.active].push_str(c),
                None => return,
            },
        }
        self.rerender();
        cx.notify();
    }
}

impl Render for MathDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (cx_px, top, h) = self.carets[self.active];
        let caret_color = if self.valid {
            rgb(0x2563eb)
        } else {
            rgb(0xdc2626)
        };
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0xffffff))
            .child(
                div()
                    .relative()
                    .w(px(self.img_w))
                    .h(px(self.img_h))
                    .child(
                        img(self.png.clone())
                            .w(px(self.img_w))
                            .h(px(self.img_h))
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(cx_px))
                            .top(px(top))
                            .w(px(2.5))
                            .h(px(h))
                            .bg(caret_color),
                    ),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(900.0), px(520.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(MathDemo::new);
                let handle = view.read(cx).focus.clone();
                window.focus(&handle);
                view
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
