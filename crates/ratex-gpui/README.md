# ratex-gpui

A **structural, [MathQuill](https://mathquill.com/)-style math editor for [GPUI](https://www.gpui.rs/)**,
built on the [RaTeX](https://crates.io/crates/ratex-parser) typesetting engine. RaTeX is the
engine (parse → layout → rasterize); this crate is the **editor + display layer** for gpui.

Two halves, usable independently:

1. **Static rendering** (the `render` module) — turn a LaTeX string into a
   `gpui::RenderImage`, or a PNG / self-contained SVG for export.
2. **An interactive structural editor** — `MathEditor`, a gpui view that edits a formula
   **two-dimensionally** (a fraction is a real stacked box, the caret moves *into* a
   numerator), Casio-Natural-Display / MathQuill style — not by editing raw LaTeX text.
   It serializes back to LaTeX on demand.

The editing **core is GUI-free** (`editor::{model, cursor, geometry, input, latex}`) —
the gpui glue (`editor::view`) is layered on top.

**📖 Full reference:** every public item, with signatures, parameter tables, return
contracts, edge cases, and cost notes, lives in [API.md](API.md).

## Features

- **2-D structural editing** — fractions, roots (incl. nth-roots with an editable degree),
  super/subscripts, big operators with limits (∫ ∑ ∏ lim), and matrices (add/remove
  rows + columns)
- **Delimiters** that wrap a selection or insert an empty pair: `()` `[]` `{}` `||` `‖‖`
  `⟨⟩` `⌊⌋` `⌈⌉`
- **`\command` autocomplete** (~100 commands: Greek, relations, operators, arrows,
  set/logic) plus a click-to-insert **palette**
- **Mouse + keyboard** — click-to-place caret, drag / double / triple-click selection,
  arrow navigation that flows *out* of the formula at its edges
- **Undo / redo**, per-formula **alignment**, selection **wrapping** (into a fraction,
  root, or delimiter)
- **LaTeX in, LaTeX out** — seed from a string, serialize back with `to_latex()`
- **Export** — crisp `RenderImage`, PNG, or self-contained SVG (KaTeX glyph paths embedded)
- **Host-themed** — all chrome colors come from a host-supplied `MathTheme`

**Coverage:** rendering ≈ KaTeX (RaTeX's parser is a KaTeX port, ~660 commands), so
anything KaTeX renders, this renders. The 2-D *editor* models a practical subset —
constructs outside it (accents, math fonts, multi-line environments, …) still render
perfectly but **degrade if opened in the structural editor** (the wrapper is dropped on
commit); edit those as raw LaTeX instead. Details in [API.md](API.md) under `parse_latex`.

## Adding the dependency

```toml
# Full crate (default `editor` feature: MathEditor + the editing modules):
ratex-gpui = { version = "0.1" }

# Render-only build — LaTeX → image / PNG / SVG plus the parse/serialize core,
# without the gpui view + editing machinery:
ratex-gpui = { version = "0.1", default-features = false }
```

## Quick start

### Render a formula to an image

```rust
use gpui::{img, px};
use ratex_gpui::render;

let r = render::render_latex(
    r"\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
    22.0,                    // font size (px/em)
    window.scale_factor(),   // device-pixel ratio
    theme.text_color,        // tint
);
match r {
    Some(r) => img(r.image).w(px(r.width)).h(px(r.height)).into_any_element(),
    None => /* parse/layout failed — fall back to the raw source */ todo!(),
};
```

Rendering is **pure CPU** (no `Window`/GPU), so it's safe to run off the main thread and
cache — render once per formula, keyed by the LaTeX.

### Host the interactive editor

`MathEditor` is a normal gpui view. Create it, focus it, place it in your tree, and
listen for `MathNav` events:

```rust
use ratex_gpui::{MathAlign, MathEditor, MathNav, MathTheme};

let editor = cx.new(|cx| {
    MathEditor::from_latex(r"x^2 + 1", 22.0, true, MathAlign::Center, my_theme(), cx)
});
window.focus(&editor.read(cx).focus_handle(), cx);

cx.subscribe(&editor, |this, editor, ev: &MathNav, cx| match ev {
    // Arrowing past an edge (or Esc) — commit and take focus back.
    MathNav::Exit { after } => {
        let latex = editor.read(cx).to_latex();
        // splice `latex` back into your document, move your text caret out
    }
    // Right-click while editing — show your copy-LaTeX / export menu.
    MathNav::ContextMenu { position } => { /* … */ }
});
```

## Hosting notes

The crate is a leaf: it edits one formula and tells you (via `MathNav`) when to take
focus back. When hosting:

1. **Drop your key context** while the editor is focused — it lives *inside* your
   element, so your own keybindings would otherwise swallow arrows/typing before they
   reach it.
2. **Commit** on `MathNav::Exit`, on focus-loss (click-away), or when the user opens
   another formula: read `to_latex()` (+ `align()`) and splice it back.
3. On `MathNav::ContextMenu`, show your menu at the given window position;
   `set_align(…)` re-justifies live if you offer alignment there.

This is exactly how its host application (Zorite) drives it for both display `$$…$$`
blocks and inline `$…$` formulas.

## Built on RaTeX

Typesetting comes from the [RaTeX](https://crates.io/crates/ratex-parser) crates —
`ratex-parser` (parse), `ratex-layout` (box layout), `ratex-render` (raster, KaTeX fonts
embedded), and `ratex-svg` (vector export). This crate adds the editing model + cursor +
hit-testing and the gpui view/render adapters on top. `gpui` is a **git-only** dependency
(tracking Zed), so this isn't published to crates.io yet.

## License

GPL-3.0-or-later.
