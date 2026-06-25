# Design — a structural math editor on RaTeX

**Crate:** `ratex-gpui` — a single crate in zorite's workspace (`crates/ratex-gpui/`), beside the
other `gpui-*` crates. The math **editor is a module inside it**, not a separate crate.

## 0. Status

The `spike/ratex-probe` exploration proved the feasibility end-to-end:

- **Layout** (`ratex-layout`) exposes the math as a structured, dimensioned `LayoutBox`
  tree — every editable slot (sup/sub, numerator/denominator, radicand, limits) is a
  distinct sub-box. Nesting works.
- **Render** (`ratex-render` → PNG, `ratex-svg` → SVG) is KaTeX-grade and turnkey into a
  GUI as an image.
- **The editor loop** runs live in gpui: keystroke → mutate model → RaTeX re-typeset →
  reposition caret from the fresh layout → repaint. A 3-slot integral editor in ~250 lines.

So RaTeX is the **engine** (typeset + render, no editing). This crate is the **editor**.
RaTeX ships a GTK4 display widget (`ratex-gtk4`) but **nothing in it edits** — that part is
ours, and it's the unique, worth-building piece.

## 1. Goals / non-goals

**Goals**
- A 2-D structural ("Casio Natural Display" / MathQuill-class) math editor.
- **gpui-native, but the editor logic stays GUI-free** (its own module), so a lift to other
  GUIs later is mechanical rather than a rewrite.
- **WYSIWYG**: editing manipulates the *structure*, rendered in 2-D — not raw LaTeX text.
- Two insertion paths: a **palette** of starters and **`\command` typing** for the long tail.
- **Export**: copy a (sub-)expression as LaTeX / SVG / PNG.

**Non-goals (v1)**
- Not a full LaTeX *document* editor — math only.
- Not every TeX package — exactly what RaTeX/KaTeX supports.
- Not reimplementing layout or fonts — RaTeX owns those.

## 2. Architecture — one crate, modular inside

`ratex-gpui` (a zorite workspace member) has two faces, kept in separate modules:

```
RaTeX (external, MIT)    parse → LayoutBox → DisplayList / SVG / PNG    [typeset + render]
   │
ratex-gpui          render :  LaTeX → a gpui element (image)           (the ratex-gtk4 analog)
(crates/ratex-gpui) editor :  model + cursor + LaTeX-serialize + slot rects   <- gpui-free logic
                              view  + caret/selection + keys/mouse + palette   <- gpui glue
```

**Why a module, not a separate crate:** the editor's *logic* — model, cursor, LaTeX
serialization, slot-rect geometry — needs nothing from gpui, so it lives in gpui-free modules:
unit-testable without a window, and mechanically liftable into a standalone `*-core` crate later
**if** cross-GUI reuse ever becomes real. Only the `view`/input glue is gpui-coupled. No second
crate, no API ceremony now; the seam is a module boundary we can promote to a crate boundary later
for free. (RaTeX is the precedent that the *render* side is thin — `ratex-gtk4` is `set_latex()` +
draw; the editor is the part RaTeX has none of.)

## 3. Document model

A tree of edit nodes — a `Row` of `Atom`s, where an atom is a symbol or a structure with
child `Row`s:

```
Atom = Sym(char/command) | Frac{num: Row, den: Row} | Script{base, sup?: Row, sub?: Row}
     | Sqrt{rad: Row, index?: Row} | Delim{open, body: Row, close} | BigOp{op, lower?, upper?}
     | Matrix{rows: Vec<Vec<Row>>} | …
```

The **cursor** is a path to a `Row` + an index between its atoms (plus a selection anchor) —
MathQuill's model. Empty `Row`s render as a placeholder `□` and are the editable holes.

**Decided — our own edit-tree, serialized to LaTeX (not RaTeX's `ParseNode` directly).**
RaTeX's `ParseNode` is public + constructible, but it's a *typeset-oriented* AST tied to
0.1.x churn. A slim edit-tree that **serializes to LaTeX** keeps editing logic clean and
decouples us from RaTeX internals; LaTeX is also the natural interchange (export + round-trip
of pasted source). RaTeX stays a pure layout/render dependency: `model → LaTeX → RaTeX`.

## 4. Editing interactions

### 4.1 Navigation
- **←/→** move through the current `Row`; at a boundary, descend into / ascend out of adjacent
  slots (into a numerator, out to the parent) — real 2-D motion.
- **↑/↓** move between stacked slots (numerator ↔ denominator, base ↔ script).
- **Tab / Shift-Tab** jump to the next/previous empty slot (form-like).
- **Click** hit-tests the cursor to the nearest slot/gap (needs slot rects — §5).

### 4.2 WYSIWYG typing & structural conversions  _(your "as much WYSIWYG as possible")_
Typing a letter/digit/operator inserts an atom. Structure-creating keys build 2-D layout in
place (MathQuill-style):
- **`/`** → turns the preceding operand into a **fraction** numerator and opens a denominator
  box below, caret in the denominator. (Exactly your "horizontal line with a box top & bottom".)
- **`^` / `_`** → open a super/subscript slot on the preceding atom; caret enters it.
- **`(` `[` `|` …** → auto-growing delimiters (`\left…\right`).
- **Space / `}` / →** exit the current slot.

The "what becomes the numerator when I press `/`" rules are subtle; we borrow MathQuill's
well-tested conventions rather than invent them.

### 4.3 `\command` typing — the long tail  _(your point 2)_
Typing **`\`** opens an inline autocomplete that filters TeX commands as you type
(`\al` → `\alpha`, `\aleph`, `\alephsym`…). On completion:
- **Symbol command** (`\alpha`, `\nabla`, `\infty`) → inserts the glyph atom.
- **Template command** (`\frac`, `\sqrt`, `\sum`, `\binom`, `\begin{matrix}`) → inserts the
  structure with empty slots; caret in the first.

The command set is **derived from RaTeX's parser** (its supported functions/macros) so the
autocomplete covers exactly what will render. Unknown `\cmd` text is **passed through to
RaTeX** anyway — so anything RaTeX supports works even if it's not in the suggestion list.
This is the escape hatch the palette can't be: the palette is common starters, `\command` is
everything.

### 4.4 Slash menu + movable palette  _(your point 1)_
- In zorite, **`/math`** (and `/display`, `/inline`) in the slash palette inserts a math
  region and focuses it.
- A **floating, draggable palette** of starter glyphs/templates appears — tabbed categories
  (Common · Greek · Operators · Big operators · Delimiters · Arrows · Accents · Matrices…).
- **Click an item → insert at the caret** (same insert API as a `\command`); the structure
  is created with the caret in the first slot. The palette is repositionable and dismissable.
- The **`editor` logic owns the catalog** of insertable items (name → LaTeX template + slot
  count); the **`view` module renders the palette** and calls `insert`. So the palette is a
  gpui concern but the catalog comes from the GUI-free side and any future adapter reuses it.

### 4.5 Structural backspace
Backspace at a slot start, when the slot is empty, **deletes the enclosing template** and
merges remaining content into the parent (delete an empty fraction → its denominator content
flows up), MathQuill-style. Otherwise it deletes the atom before the caret.

### 4.6 Selection + right-click export  _(your point 4)_
- Shift+move / drag selects a contiguous range (or a sub-tree).
- **Right-click a glyph/selection → context menu:**
  - **Copy as LaTeX** — model → LaTeX serialization (the source of truth).
  - **Copy as SVG** — `ratex-svg` on the selected sub-expression.
  - **Copy as PNG** — `ratex-render`.
  - _Copy as MathML_ — only if RaTeX adds it (KaTeX has MathML output; RaTeX currently
    does SVG/PNG/PDF) → mark **future**.
  - Cut · Delete.
- Operates on the selection, or the whole formula when nothing is selected.

## 5. Render + geometry (the RaTeX integration — the one piece of real engineering)

Each model edit → serialize to LaTeX → `ratex_parser::parse` → `ratex_layout::layout`.

- **Render** — `ratex-render` → PNG/Pixmap → host image (gpui `RenderImage`, like zorite's
  Mermaid/PDF). Start here: turnkey + KaTeX-grade. (Native `to_display_list` drawing is a
  later optimization for crisp scaling / per-glyph theming.)
- **Geometry (caret + slot rects)** — the spike approximated limit positions from `SupSub`
  shift/kern fields (hence the slightly-off bottom caret). The correct, durable fix:
  - **Extend `ratex-layout`/`to_display_list` to emit slot rects** tagged with the
    `ParseNode.loc` source span. RaTeX *carries `loc` on the AST* and drops it at layout time
    — re-threading it is a bounded change to an MIT crate (fork → PR upstream). Our LaTeX
    serializer records each slot's source span, so `span → rect` is exact for every slot.
  - Interim (no fork): a positioned `LayoutBox` walk that mirrors `to_display_list`'s offset
    accumulation and reads box widths/shifts exactly (no kern guesses).
- **Theming** — render with the active text color (LayoutOptions color / recolor), like
  velotype and zorite's Mermaid.

This `loc`-threading is the same item flagged in the scoping memo; it's the crux feature and
the natural first upstream contribution.

## 6. zorite integration

- **Display math `$$…$$`** — a block widget in the gpui-editor, mirroring the existing
  **Mermaid block provider** (`set_block_mermaid_provider` + `markdown_syntax::mermaid_blocks`
  → a `set_block_math_provider`). Focused = the structural editor; unfocused = the rendered
  image; raw `$$…$$` under the caret. The easy, well-trodden win — and **all of v1**.
- **Inline math `$…$`** — **deferred to a later milestone (decided, M7).** v1 ships display
  `$$` only. Inline structural editing needs inline-widget support in the gpui-editor's
  flat-string `Element` model (today the inline layer is styled *text* only). As an interim it
  can render inline math as a read-only image with edit-as-raw on caret; full inline editing is
  the M7 piece.
- Slash menu + palette hook into zorite's slash system + a floating overlay.

## 7. Crate & repo

**Lives in zorite's workspace** — `crates/ratex-gpui/`, beside `gpui-editor`, `gpui-markdown`,
`gpui-pdf`, `gpui-whiteboard`, `os-spellcheck`. **Not** a separate repo. Modules per §2:
`render` (display) · `editor::{model, geometry}` (gpui-free) · `editor::view` (gpui glue + palette).
Optional cargo feature `editor` so a render-only consumer can skip it; zorite turns it on.

Depends on `ratex-*` (and, for exact slot rects, a forked/patched `ratex-layout` — §5).

**Future split — only if needed:** since the editor logic is already gpui-free, lifting
`editor::{model, geometry}` into a standalone `ratex-edit-core` + thin adapters (gpui / egui /
web) is mechanical, no rewrite. The option costs nothing to keep now.

License: MIT (matches RaTeX); KaTeX fonts (OFL) ride along in the render path. The name
`ratex-gpui` reads as "RaTeX's gpui companion" — apt. If it's ever published, decide then whether
to propose it upstream as RaTeX's official gpui adapter (they have gtk4/cairo, no gpui) or pick a
distinct name so it doesn't imply it's part of RaTeX.

## 8. Milestones

- **M0 — done.** Feasibility spike (layout / render / edit proven).
- **M1.** `editor::model` + LaTeX serializer + **exact slot-rect geometry** (`editor::geometry` —
  the `to_display_list` extension or a precise walk). Caret/slots pixel-correct. _(Fixes the
  bottom-caret wart for real.)_
- **M2.** Editing interactions — nav, typing, WYSIWYG `/` `^` `_`, structural backspace, selection.
- **M3.** The gpui `editor::view`/input — `Element`, draw, events. The demo becomes a real editor
  living in `crates/ratex-gpui/`.
- **M4.** `\command` autocomplete + the movable palette.
- **M5.** Right-click copy-as (LaTeX / SVG / PNG).
- **M6.** zorite integration — `/math`, the display-`$$` block widget. **← v1 done here.**
- **M7.** Inline `$…$` math (the deferred piece) — inline-widget support in the gpui-editor +
  inline structural editing.
- **M8.** Reach — more templates (matrices / cases / aligned), an egui adapter (proves the
  GUI-free split), publish to crates.io.

## 9. Risks & open questions

- **RaTeX maturity (0.1.x)** — API churn; the slot-rect feature likely needs a fork/PR. Pin a
  version; aim to contribute the `loc`-threading upstream.
- **Slot-rect computation** — the core engineering risk, but *bounded*: RaTeX computes the
  geometry; we only need it tagged to slots. The spike proved the data is all there.
- **WYSIWYG `/`→fraction rules** — borrow MathQuill's conventions; don't reinvent.
- **`\command` long tail** — enumerate RaTeX's commands for autocomplete; pass unknown `\cmd`
  straight through so coverage == RaTeX's coverage.

---

_Design decisions locked: **one `ratex-gpui` crate** in zorite's workspace, editor as a gpui-free
module (§2, §7); **our own edit-tree → LaTeX** model (§3); **inline math deferred to M7** (§6, §8).
Next concrete step when build starts: **M1** — the exact slot-rect geometry that retires the
approximate bottom-caret._
