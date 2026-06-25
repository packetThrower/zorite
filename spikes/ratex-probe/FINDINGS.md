# RaTeX structural-editor probe — findings

**Question:** can RaTeX back a structural (Casio / MathQuill-style) math editor —
templates + 2D editable boxes — or is it only good for rendering a LaTeX string to
an image (velotype's use)?

**Answer: buildable.** Run it:

```sh
cargo run --manifest-path spikes/ratex-probe/Cargo.toml
```

It parses + lays out `∫_0^1 4x dx`, `∑_{i=1}^n i`, `\frac{a+b}{c}`, `√(x²+1)`, then
prints the `LayoutBox` tree (kind + em dimensions + the shift fields) and the absolute
`DisplayList`.

## What it proves

- **Every editable slot is a distinct, dimensioned `LayoutBox` sub-tree** —
  `SupSub{sup,sub}` (∫ limits / exponents), `OpLimits{sup,sub}` (∑ limits above/below),
  `Fraction{numer,denom}`, `Radical{body,index}`.
- **Nesting works** — `√(x²+1)` shows a `SupSub` *inside* the `Radical`'s radicand.
- **Absolute geometry is computed** — `to_display_list` gives every glyph an absolute
  `(x, y)`: e.g. the ∫ upper-limit `1` lands raised (y=0.451), the lower-limit `0`
  dropped (y=2.476), the fraction numerator/denominator above/below the bar.

## Implication

A structural editor walks the **structured `LayoutBox` tree** (not the flat display
list), so the "`layout()` drops `loc`" caveat barely applies — you own the model,
correlate it to the structured tree, and read each slot's box directly. The one
bounded task is a slot's **absolute** rect: a positioned walk replicating `to_display`'s
offset accumulation, or a small fork of `to_display_list` that emits slot rects next to
glyphs (reusing RaTeX's own positioning math). `loc` is only needed for a *secondary*
"paste raw rendered LaTeX → click-to-edit" flow.

RaTeX = KaTeX port, MIT, `github.com/erweixin/RaTeX`. Probed **v0.1.11** (the `0.1.9`
requirement resolved up; the API read against 0.1.9 compiled unchanged against 0.1.11,
so the surface is stable across the 0.1.x patch line).

## Update — RaTeX is multi-backend; rendering in gpui is turnkey

The full repo has 16 crates, including **`ratex-render`** (raster → PNG/Pixmap via
tiny-skia), `ratex-svg`, `ratex-pdf`, `ratex-cairo`, **`ratex-gtk4`** (a real GUI widget),
`ratex-wasm`, `ratex-ffi`. Run the raster probe:

```sh
cargo run --manifest-path spikes/ratex-probe/Cargo.toml --bin render   # → /tmp/ratex-out/*.png
```

- `render_to_png(&DisplayList, &RenderOptions) -> Vec<u8>` (fonts embeddable via
  `embed-fonts`). Output is **KaTeX-grade** → feeds straight into `gpui::RenderImage`,
  exactly zorite's Mermaid/PDF path. So *displaying* RaTeX math in gpui is a few lines.
- `ratex-gtk4` ("GTK4 widget for native RaTeX rendering") is a display-only widget — it
  proves the per-GUI-adapter model; a `ratex-gpui` would be its analog.
- **No editor anywhere in RaTeX** (grepped all crates + iOS/RN platforms + demos for
  cursor/caret/editable/keypress). Every crate is parse/layout/render/font/backend. So
  rendering is turnkey; the structural editor (model + interaction) is net-new + GUI-agnostic.
