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
