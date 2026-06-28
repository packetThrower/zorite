---
title: Math & equations
description: 'Write LaTeX math inline or as display blocks, edit it visually in a 2-D structural editor (palette + \command autocomplete), align it, and copy or export to PNG / SVG.'
---

Zorite renders LaTeX math and lets you edit it **visually** — a fraction is a real
stacked box you click into, not a string you hand-edit. Both inline and display
math work anywhere you write Markdown.

## Writing math

Wrap math in dollar signs:

- **Inline** — `$…$` sits in a line of prose, e.g. `the area is $\pi r^2$`.
- **Display** — `$$…$$` on their own lines render as a centered block.

Both typeset to crisp formulas with KaTeX-grade coverage. A lone `$` in prose
(like `it cost $5`) stays literal. You can also insert a block from the `/` command
palette — type `/` and pick **Math**.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-math-light.png" />
  <img
    src="/zorite/screenshots/zorite-math-dark.png"
    alt="Zorite editing a formula in its 2-D structural math editor, with the symbol palette open"
  />
</picture>

## The structural editor

**Click or arrow into a formula** to edit it in place with a two-dimensional editor
(Casio-Natural-Display / MathQuill style). The caret moves *through* the structure —
into a numerator, under a root, between matrix cells — rather than along a line of
LaTeX.

- **Type naturally** — letters and operators insert as symbols; `^` starts a
  superscript and `_` a subscript, and the caret descends into the new box.
- **`\command` autocomplete** — type `\` and a name (`\alpha`, `\sqrt`, `\frac`, …)
  for a scrollable menu of ~100 commands; `Enter` or `Tab` inserts the highlighted
  one.
- **Symbol palette** — a floating panel of one-click structures and symbols
  (fractions, roots, matrices, the Greek alphabet, relations, big operators, …).
- **Select and wrap** — select a sub-expression (Shift-arrows or drag; double-click
  a cell, triple-click a row), then apply a fraction, root, or delimiter to **wrap
  it** instead of inserting an empty one.
- **Matrices** — insert a grid and add or remove rows and columns as you go.
- **Undo / redo** — `⌘Z`, `⌘⇧Z`, `⌘Y` (`Ctrl` on Windows and Linux).

Arrow past a formula's edge (or press `Esc`) to flow the caret back into the
surrounding text, the way arrowing out of a table cell leaves the table.

## Alignment

A display formula is **centered** by default. Right-click it for **Align → Left /
Center / Right** — the choice is saved per-formula, so it stays put when you edit.

## Copy & export

Right-click any rendered formula for:

- **Copy LaTeX** — the formula's source, to paste elsewhere.
- **Export PNG** — a transparent raster at display resolution.
- **Export SVG** — a self-contained vector with the glyph outlines embedded, so it
  renders correctly even where the math fonts aren't installed.

## What's supported

**Rendering is essentially complete** — anything
[KaTeX](https://katex.org/docs/support_table) can typeset, Zorite renders: every
symbol, accents, blackboard-bold and script fonts, `align` / `cases` environments,
and more.

The **2-D editor** handles the common core — fractions, roots, super/subscripts,
matrices, delimiters, and ~100 symbols. A formula that uses something outside that
(an accent like `\hat`, a font like `\mathbb`, a multi-line environment) still
**renders perfectly**; you just edit it as raw `$…$` LaTeX rather than in the
structural editor. See the
[`ratex-gpui` reference](/zorite/reference/crates/ratex-gpui/) for the full coverage
breakdown.
