# ratex-gpui

A **structural, [MathQuill](https://mathquill.com/)-style math editor for [GPUI](https://www.gpui.rs/)**,
built on the [RaTeX](https://crates.io/crates/ratex-parser) typesetting engine. RaTeX is the
engine (parse → layout → rasterize); this crate is the **editor + display layer** for gpui.

It gives you two things, usable independently:

1. **Static rendering** — turn a LaTeX string into a `gpui::RenderImage` (or a PNG / SVG for
   export). See the [`render`](#rendering-the-render-module) module.
2. **An interactive structural editor** — [`MathEditor`](#matheditor), a gpui view that edits a
   formula **two-dimensionally** (a fraction is a real stacked box, a caret moves *into* a
   numerator), Casio-Natural-Display / MathQuill style — not by editing raw LaTeX text. It
   serializes back to LaTeX on demand. Behind the default **`editor`** feature, so a
   render-only dependency can drop it (see [Cargo features](#cargo-features)).

The editing **core is GUI-free** (`editor::model`, `editor::cursor`, `editor::geometry`,
`editor::input`, `editor::latex`) — the gpui glue (`editor::view`) is layered on top, so the
editor could move to another GUI toolkit with a thin adapter swap.

## Features

- **2-D structural editing** — fractions, square roots, **nth roots** (editable degree),
  super/subscripts, big operators with limits (∫ ∑ ∏ lim), and **matrices** (add/remove
  rows + columns)
- **Delimiters** that wrap a selection or insert an empty pair: `()` `[]` `{}` `||` `‖‖` `⟨⟩`
  `⌊⌋` `⌈⌉`
- **`\command` autocomplete** — type `\` and a prefix for a scrollable menu of ~100 commands
  (Greek, relations, operators, arrows, set/logic symbols), plus a **symbol/structure palette**
- **Mouse + keyboard** — click to place the caret anywhere inside the formula, double/triple-click
  to select, drag-select, arrow-key navigation that flows *out* of the formula at its edges
- **Undo / redo**, **per-formula alignment** (left / center / right), and **selection wrapping**
  (wrap a sub-expression in a fraction, root, or any delimiter)
- **LaTeX in, LaTeX out** — seed from a LaTeX string, serialize back with `to_latex()`
- **Export** — render to a crisp `RenderImage`, a **PNG**, or a self-contained **SVG**
  (KaTeX glyph paths embedded, so it renders correctly anywhere)
- **Host-themed** — all chrome colors come from a host-supplied [`MathTheme`](#maththeme)

## Quick start

### Render a formula to an image

```rust
use gpui::{img, px};
use ratex_gpui::render;

// In a render method, with `window`/theme available:
let r = render::render_latex(
    r"\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
    22.0,                    // font size (px/em)
    window.scale_factor(),   // device-pixel ratio — render at 2× for a crisp 2× display
    theme.text_color,        // tint
);
match r {
    Some(r) => img(r.image).w(px(r.width)).h(px(r.height)).into_any_element(),
    None => /* parse/layout failed — fall back to the raw source */ todo!(),
};
```

Rendering is **pure CPU** (no `Window`/GPU needed), so it is safe to run off the main thread and
cache — the host typically renders once into a store keyed by the LaTeX and reads the bitmap back
on paint.

### Host the interactive editor

`MathEditor` is a normal gpui view (`Entity<MathEditor>`). Create it, focus it, place it in your
tree, and listen for [`MathNav`](#mathnav) events:

```rust
use ratex_gpui::{MathAlign, MathEditor, MathNav, MathTheme};

let editor = cx.new(|cx| {
    MathEditor::from_latex(
        r"x^2 + 1",
        22.0,                 // font size (px/em)
        true,                 // place caret at the end (entered from the right/below)
        MathAlign::Center,
        MathTheme { /* map your theme */ ..my_math_theme() },
        cx,
    )
});
let focus = editor.read(cx).focus_handle();
window.focus(&focus, cx);

// Arrowing past an edge (or Esc) asks to hand focus back to your text; a right-click
// while editing asks for the formula's context menu.
cx.subscribe(&editor, |this, editor, ev: &MathNav, cx| match ev {
    MathNav::Exit { after } => {
        let latex = editor.read(cx).to_latex();   // serialize the edited formula
        // commit `latex` back into your document, then move your text caret out
    }
    MathNav::ContextMenu { position } => { /* show copy/export menu at `position` */ }
});
```

`MathEditor` implements `Render`, so `editor.clone()` drops into any element tree. When the host
also commits on focus-loss (click-away), read `to_latex()` in the blur handler.

## API

### Rendering (the `render` module)

Pure functions — no `Window`/GPU, safe off-thread. Each returns `None` if the LaTeX fails to
parse, lay out, or rasterize (so the host can fall back to showing the raw source).

| Item | Signature | Purpose |
| --- | --- | --- |
| `render_latex` | `fn render_latex(latex: &str, font_size: f32, dpr: f32, color: Hsla) -> Option<Rendered>` | Typeset `latex` to a `gpui::RenderImage` tinted `color`, rasterized at `dpr` device-pixels (display at the returned logical size; `dpr = 2` → crisp on retina). |
| `render_row` | `fn render_row(row: &Row, font_size: f32, dpr: f32, color: Hsla) -> Option<Rendered>` | Same, from an already-parsed `Row` (e.g. a live editor's `to_latex()` avoided). |
| `render_latex_to_png` | `fn render_latex_to_png(latex: &str, font_size: f32, dpr: f32) -> Option<Vec<u8>>` | Encode the formula as **PNG** bytes — for an "Export PNG" action / the clipboard. |
| `render_latex_to_svg` | `fn render_latex_to_svg(latex: &str, font_size: f32) -> Option<String>` | The formula as a self-contained **SVG** with **glyph paths embedded** (KaTeX fonts), so it renders correctly in viewers that don't have those fonts. |
| `PAD` | `const PAD: f32 = 8.0;` | Padding (px) the rasterizer reserves around the glyphs. |

```rust
pub struct Rendered {
    pub image: Arc<RenderImage>,  // BGRA bitmap, ready for `gpui::img(...)`
    pub width: f32,               // logical (pre-DPR) px — what to size the element to
    pub height: f32,
}
```

### `MathEditor`

The interactive editor — a gpui view (`impl Render`, `EventEmitter<MathNav>`).

| Method | Signature | Purpose |
| --- | --- | --- |
| `new` | `fn new(cx: &mut Context<Self>) -> Self` | An empty editor (48 px/em, centered, default theme). |
| `from_latex` | `fn from_latex(latex: &str, font_size: f32, at_end: bool, align: MathAlign, theme: MathTheme, cx: &mut Context<Self>) -> Self` | Seed from `latex` at `font_size`. `at_end` puts the caret at the end of the top row (entered from the right/below) vs the start (from the left/above), so arrowing *into* a formula lands naturally. `align` matches the host block's justification so entering edit doesn't shift it. |
| `to_latex` | `fn to_latex(&self) -> String` | Serialize the current formula to LaTeX — call on commit. |
| `align` | `fn align(&self) -> MathAlign` | The current horizontal alignment (the host persists it on commit). |
| `set_align` | `fn set_align(&mut self, align: MathAlign, cx: &mut Context<Self>)` | Re-justify the formula live (e.g. from an "Align" menu). |
| `focus_handle` | `fn focus_handle(&self) -> FocusHandle` | Focus it so keys reach the editor. **The host must drop its own key context while this is focused** (see [Hosting](#hosting-the-editor)). |

### `MathNav`

Events the editor emits (`EventEmitter<MathNav>`) so focus can flow back to the host:

```rust
pub enum MathNav {
    /// The caret tried to move past a boundary of the formula (or Esc was pressed). `after` =
    /// past the end → seat the host's text caret after the formula, else before it. The host
    /// commits `to_latex()` and re-focuses its own editor — like arrowing out of a table cell.
    Exit { after: bool },
    /// The formula was right-clicked while being edited. The hosted editor occludes the
    /// formula, so the host's own right-click can't fire; this routes it back out so the host
    /// can show its copy-LaTeX / export menu at `position` (window-space).
    ContextMenu { position: Point<Pixels> },
}
```

### `MathTheme`

All editor chrome colors, supplied by the host (`#[derive(Clone)]`, has a `Default`):

```rust
pub struct MathTheme {
    pub fg: Hsla,        // formula glyphs + primary text (button labels)
    pub muted: Hsla,     // secondary text — grips, dropdown rows
    pub panel: Hsla,     // panel + button surfaces (palette, toolbar, dropdown)
    pub border: Hsla,    // panel + button borders
    pub accent: Hsla,    // the caret and active/selected highlights
    pub accent_bg: Hsla, // a subtle accent fill — hover, selected row, command preview
}
```

### `MathAlign`

```rust
pub enum MathAlign { Left, #[default] Center, Right }
```

Horizontal justification of a display formula. `Center` is the default (LaTeX display
convention). The host typically persists a non-default choice alongside the formula.

### `parse_latex`

```rust
pub fn parse_latex(latex: &str) -> Row
```

Parse a LaTeX string into the editor's structural `Row` model (the inverse of
`Row::to_latex`). Unrecognized constructs degrade to literal symbols rather than failing, so a
round-trip never loses the source outright. Used internally by `from_latex` /
`render::render_latex`; exposed for hosts that want to inspect or pre-build the model.

## Interacting with the editor

Once focused, the editor handles:

- **Typing** — letters, digits, and operators insert as symbols; `^` starts a superscript and
  `_` a subscript (the caret descends into the new box).
- **`\command`** — type `\` then a name (e.g. `\alpha`, `\sqrt`, `\frac`) for a scrollable
  autocomplete; Enter/Tab commits the highlighted command. ~100 commands (see [Coverage](#coverage)).
- **Palette** — a floating panel of one-click structures and symbols (`x/y`, `√`, `ⁿ√`, `▦`, the
  delimiters, big operators, Greek, relations, …).
- **Navigation** — arrow keys move the caret through the 2-D structure; arrowing past the
  formula's outer edge emits [`MathNav::Exit`](#mathnav) so focus flows back to the host's text.
- **Selection + wrapping** — Shift-arrows or a mouse drag select a sub-expression;
  double-click selects a cell, triple-click a row. A fraction, root, or delimiter command then
  **wraps the selection** instead of inserting an empty one.
- **Matrices** — insert a 2×2 grid; add or remove rows and columns as you edit.
- **Undo / redo** — `Cmd/Ctrl+Z`, `Cmd/Ctrl+Shift+Z`, `Cmd/Ctrl+Y`.
- **Mouse** — click to place the caret precisely; right-click emits [`MathNav::ContextMenu`](#mathnav).
- **Esc** — leave the editor (emits [`MathNav::Exit`](#mathnav)).

## Coverage

Rendering and structural editing cover **different amounts**: display is near-complete (a
KaTeX-grade engine), while the 2-D editor models a practical core. A construct outside the
editor's model still **renders** — it just can't be *structurally* edited.

| Construct | Render | 2-D edit |
| --- | :---: | :---: |
| Fractions, roots (incl. **nth-root**), super/subscripts, matrices, delimiters `()` `[]` `{}` `\|\|` `‖‖` `⟨⟩` `⌊⌋` `⌈⌉` | ✅ | ✅ |
| Big operators with limits (`\int \iint \oint \sum \prod \lim`) + functions (`\log \ln \sin \cos \tan`) | ✅ | ✅ |
| Symbols — Greek, relations, binary ops, arrows, set/logic (`\in \subset \cup \forall …`) | ✅ | ✅ |
| Accents (`\hat \bar \vec \tilde \widehat`), `\overline` / `\underline`, `\overbrace` / `\underbrace` | ✅ | ⚠️ degrades |
| Math fonts — `\mathbb` `\mathcal` `\mathfrak` `\mathbf`, `\text` | ✅ | ⚠️ degrades |
| Multi-line environments (`align`, `cases`, …), `\binom`, `\color`, manual spacing | ✅ | ⚠️ degrades |

**Rendering ≈ KaTeX.** `render_latex` parses with **RaTeX's parser — a KaTeX port** (~660
commands), so if [KaTeX](https://katex.org/docs/support_table) renders a formula, so does this
crate: in the reading view, and whenever a formula isn't being structurally edited.

**2-D editing models a subset** — six structure kinds (fraction, root, super/subscript, matrix,
delimiter) plus ~100 named commands (≈80 symbols + the structures) via the 40-button palette and
`\`-autocomplete. A formula using a ⚠️ construct **renders perfectly**, but opening it in the
structural editor drops that wrapper: `parse_latex` keeps the inner content and `to_latex()`
re-emits without it, so a commit would lose it. Edit those as raw `$…$` LaTeX instead (the
`\`-menu still helps) — everything simple-to-moderate edits fully two-dimensionally.

### What the 2-D editor builds

- **Structures** — `\frac{}{}`, `\sqrt{}`, `\sqrt[n]{}`, super/subscripts (`^`, `_`),
  `\begin{matrix}…\end{matrix}`, and the delimiter pairs `()` `[]` `\{\}` `||` `\|\|`
  `\langle\rangle` `\lfloor\rfloor` `\lceil\rceil` (each wraps a selection or inserts an empty
  pair).
- **Operators** — `\int \iint \oint \sum \prod` (lower + upper limits), `\lim` (lower); function
  names `\log \ln \sin \cos \tan`.
- **Symbols (~80)** — relations (`\le \ge \ne \approx \equiv \sim \propto`), binary ops
  (`\times \div \cdot \pm \mp \ast \circ`), arrows (`\to \rightarrow \Rightarrow \leftrightarrow
  \mapsto`), set/logic (`\in \notin \subset \subseteq \cup \cap \emptyset \forall \exists
  \neg`), misc (`\infty \partial \nabla \angle \cdots \ldots`), and the full lower- and
  upper-case **Greek** alphabet.

## Hosting the editor

The crate is a leaf: it edits one formula and tells you (via [`MathNav`](#mathnav)) when to take focus
back. A typical host (e.g. editing a `$$…$$` block in a Markdown document) does:

1. **Open** — create the editor with `from_latex(source, font_size, at_end, align, theme, cx)`,
   place it where the formula sits, and `window.focus(&editor.read(cx).focus_handle(), cx)`.
2. **Drop your key context** while it's focused — the editor lives *inside* your element, so
   your own keybindings would otherwise swallow arrows/typing before they reach it. Render your
   host element with an empty `key_context` for the duration.
3. **Commit** on [`MathNav::Exit`](#mathnav) (arrow-out / Esc), on focus-loss (click-away), or when the
   user opens another formula: read `to_latex()` (+ `align()`), splice it back into your
   document, and move your text caret out (`after` says which side).
4. **Context menu** — on [`MathNav::ContextMenu`](#mathnav), show your copy-LaTeX / export menu at the
   given window position; `set_align(…)` re-justifies live if you offer alignment there.

This is exactly how its host application (Zorite) drives it for both display `$$…$$` blocks and
inline `$…$` formulas.

## Architecture

| Module | Role |
| --- | --- |
| `editor::model` | The structural model: `Row` (a sequence of `Atom`s) ↔ LaTeX (`to_latex`). GUI-free. |
| `editor::cursor` | The 2-D cursor: insert/backspace, move L/R/U/D into and out of boxes, selection ranges, matrix row/column ops, and selection-wrapping. GUI-free. |
| `editor::geometry` | Lays a `Row` out to boxes and reads rects back — caret/selection rectangles, and **hit-testing** a click `(x, y)` to a cursor position. GUI-free. |
| `editor::input` | The `\command` table + symbol/structure **palette**, and the autocomplete prefix match. GUI-free. |
| `editor::latex` | `parse_latex` — LaTeX → `Row`. GUI-free. |
| `editor::view` | The gpui glue: `MathEditor` view, key/mouse handling, the floating palette + dropdowns, and theming via `MathTheme`. |
| `render` | Display: RaTeX raster → `gpui::RenderImage` / PNG / SVG. |

## Cargo features

| Feature | Default | Enables |
| --- | :---: | --- |
| `editor` | ✅ | The interactive structural editor — [`MathEditor`](#matheditor) and the `cursor` / `geometry` / `input` / `view` modules. |

For a **render-only** build — LaTeX → image / PNG / SVG (the
[`render`](#rendering-the-render-module) module) plus the parse/serialize core (`parse_latex`,
`editor::model`), without the gpui view + editing machinery — turn the default feature off:

```toml
ratex-gpui = { version = "0.1", default-features = false }
```

Everything under [Rendering](#rendering-the-render-module) (and `parse_latex`) stays; everything
under [`MathEditor`](#matheditor) / [Interacting](#interacting-with-the-editor) requires `editor`.

## Built on RaTeX

Typesetting comes from the [RaTeX](https://crates.io/crates/ratex-parser) crates — `ratex-parser`
(parse), `ratex-layout` (box layout), `ratex-render` (raster, KaTeX fonts embedded), and
`ratex-svg` (vector export). This crate adds the **editing** model + cursor + hit-testing and the
**gpui** view/render adapters on top.

## Status

Early but functional: an interactive 2-D editor with autocomplete, selection-wrapping, matrices,
undo/redo, alignment, mouse editing, and PNG/SVG export. `gpui` is a **git-only** dependency
(tracking Zed), so this isn't published to crates.io yet.

## License

GPL-3.0-or-later.
