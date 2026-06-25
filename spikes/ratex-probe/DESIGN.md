# Design — a structural math editor on RaTeX

Working name: **`mathcaret`** _(TBD — alts: `nabla`, `equate`, `glyphwright`)_

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
- **GUI-agnostic core**; thin per-GUI adapters (gpui first, for zorite).
- **WYSIWYG**: editing manipulates the *structure*, rendered in 2-D — not raw LaTeX text.
- Two insertion paths: a **palette** of starters and **`\command` typing** for the long tail.
- **Export**: copy a (sub-)expression as LaTeX / SVG / PNG.

**Non-goals (v1)**
- Not a full LaTeX *document* editor — math only.
- Not every TeX package — exactly what RaTeX/KaTeX supports.
- Not reimplementing layout or fonts — RaTeX owns those.

## 2. Architecture — three layers

```
RaTeX (external, MIT)         parse → LayoutBox → DisplayList / SVG / PNG     [typeset + render]
   │
mathcaret-core (no GUI dep)   model + interaction + RaTeX integration         [the editor]
   │   apply(event) → mutate model → re-typeset → produce a `View`
   ▼
mathcaret-gpui (thin)         draw the View's image + caret/selection,        [the adapter]
                              forward key/mouse events                         (≈ ratex-gtk4)
```

**The seam (why it stays agnostic):** the core takes input *events* and emits a **`View`** —
a render artifact (image or display list) plus the **caret rect, selection rects, and the
active-slot rect**, all in one coordinate space. The host just paints the View and forwards
events. That's the same agnostic shape RaTeX itself has, and it's what lets the gpui binding
be ~thin while the editor logic stays portable (egui / web adapters later, same core).

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

**Decision — our own edit-tree, serialized to LaTeX (not RaTeX's `ParseNode` directly).**
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
- The **core owns the catalog** of insertable items (name → LaTeX template + slot count); the
  **adapter renders the palette** and calls `insert`. So the palette is a gpui concern but
  every other adapter gets the same catalog for free.

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
  image; raw `$$…$$` under the caret. The easy, well-trodden win.
- **Inline math `$…$`** — the harder case: the gpui-editor's inline layer is styled *text*
  only, no inline widgets. v1 = render inline math as an image with edit-as-raw on caret (a
  stepping stone); full inline structural editing later (needs inline-widget support in the
  flat-string `Element` model).
- Slash menu + palette hook into zorite's slash system + a floating overlay.

## 7. Crate & repo

**Own repo** (per your plan), a small workspace:
- **`mathcaret-core`** — model + interaction + RaTeX integration. GUI-agnostic. Depends on
  `ratex-*` (and, for slot rects, a forked/patched `ratex-layout`).
- **`mathcaret-gpui`** — the gpui adapter (an `Element` + the palette + event plumbing). zorite
  consumes this.
- **`examples/`** — the standalone demo (this spike, evolved); optionally an **egui** adapter
  to prove the core is genuinely agnostic.

License: MIT (matches RaTeX); KaTeX fonts (OFL) ride along in the render path.

**Incubation:** start the core + gpui adapter here in `spikes/` for fast iteration against
zorite, **extract to its own repo once the core API stabilizes** (≈ after M3). Avoids premature
repo overhead while the shape is still moving.

## 8. Milestones

- **M0 — done.** Feasibility spike (layout / render / edit proven).
- **M1.** Core model + LaTeX serializer + **exact slot-rect geometry** (the `to_display_list`
  extension or a precise walk). Caret/slots pixel-correct. _(Fixes the bottom-caret wart for real.)_
- **M2.** Editing interactions — nav, typing, WYSIWYG `/` `^` `_`, structural backspace, selection.
- **M3.** gpui adapter — `Element`, draw, events. The demo becomes a real editor. **← extract to own repo around here.**
- **M4.** `\command` autocomplete + the movable palette.
- **M5.** Right-click copy-as (LaTeX / SVG / PNG).
- **M6.** zorite integration — `/math`, the display-math block widget.
- **M7+.** Inline math, more templates (matrices / cases / aligned), an egui adapter, publish to crates.io.

## 9. Risks & open questions

- **RaTeX maturity (0.1.x)** — API churn; the slot-rect feature likely needs a fork/PR. Pin a
  version; aim to contribute the `loc`-threading upstream.
- **Slot-rect computation** — the core engineering risk, but *bounded*: RaTeX computes the
  geometry; we only need it tagged to slots. The spike proved the data is all there.
- **Inline math in zorite** — gated on inline-widget support in the editor; defer or stepping-stone.
- **WYSIWYG `/`→fraction rules** — borrow MathQuill's conventions; don't reinvent.
- **`\command` long tail** — enumerate RaTeX's commands for autocomplete; pass unknown `\cmd`
  straight through so coverage == RaTeX's coverage.
- **Name** — decide it before the repo extraction (M3).

---

_Decisions that want your call: (a) the edit-tree-vs-`ParseNode` model choice (§3), (b) incubate-then-extract vs fresh-repo-now (§7), (c) the name, (d) whether inline math is in scope for v1 or deferred (§6)._
