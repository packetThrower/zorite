---
title: Whiteboards
description: 'The freeform infinite canvas — pen, shapes, curved arrows, text, sticky notes, checklists, images, and page-card embeds; select, move, resize, rotate, colour, fill, opacity, z-order, snap-to-grid, copy/paste, duplicate, groups, PNG / SVG export, and reusable templates.'
---

Whiteboards are a freeform infinite canvas that lives alongside the text
journal. They're first-class pages — each board has a title, its own sidebar
section, and the words on it show up in search.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-whiteboard-light.png" />
  <img
    src="/zorite/screenshots/zorite-whiteboard-dark.png"
    alt="A Zorite whiteboard with sticky notes in several colors, a checklist, shapes joined by curved arrows, and stickies written in Arabic and Hebrew"
  />
</picture>

## Tools

- **Pen** — freehand drawing.
- **Shapes** — rectangle, ellipse, line, arrow, diamond, triangle,
  rounded-rectangle, star, and hexagon.
- **Curved lines and arrows** — three curved tools in the Lines menu. With any
  line tool, drag from a shape to draw an arrow attached to it.
- **Text** — type directly on the canvas.
- **Sticky notes** — the Sticky note tool (`N`) drops a paper note with a
  folded corner, in one of five colors, ready for typing. Long text wraps and
  shrinks to fit.
- **Images** — drop or paste an image file onto the board.
- **Page cards** — embed a card that links back to a note, so a board can be a
  visual index of your written pages.

## Editing elements

Select one element or rubber-band a group, then **move**, **resize**, or
**rotate** it. Per element you can set the **colour**, **fill**, a **gradient**,
and **opacity**, and adjust **z-order** (bring forward / send back) so overlapping
shapes stack the way you want.

A selected line or arrow has a handle in its middle: drag it to bend the line
into a curve, or back onto the line to straighten it. Hold **Shift** while
dragging the handle to move the curve's peak along the line.

**Duplicate** the selection with `⌘D` / `Ctrl+D`, or Option / Alt-drag it to
drag out a copy; arrows between copied shapes stay attached to the copies. **Group** a
selection with `⌘G` / `Ctrl+G` (ungroup with `⌘⇧G` / `Ctrl+Shift+G`), or from
the right-click menu; a group selects, moves and copies as one.

## Writing on a board

Double-click a text box, shape or sticky note to type in it, or select it and
press **Enter**. While typing, `⌘↑` / `⌘↓` (`Ctrl+Home` / `Ctrl+End` on
Windows and Linux) jump to the start or end of the text.

Start a line with `* ` or `- ` for a bullet, or `[ ] ` for a checkbox. Enter
continues the list, and clicking a box ticks it.

Arabic and Hebrew read right to left: Arabic letters join, lines that mix them
with English come out in the right order, and the caret and selection follow.
A sticky note's right-to-left text starts at its right edge.

**Snap-to-grid** keeps things aligned as you place and resize. **Copy and paste**
works within a board and across boards, so you can lift a diagram from one canvas
onto another.

## Export

Right-click a board (in the sidebar, on its tab, in search or in All pages) to
**Export as PNG…**, **Export as SVG…**, or **Copy as image**. Exports are on
white, in the board's own font, with images included. On Linux, Copy as image
needs wl-clipboard (Wayland) or xclip (X11); Zorite tells you which to install
if it's missing.

## Templates

Save a board's contents as a **reusable template** and start new boards from it —
handy for a recurring layout like a weekly planner, a retro grid, or a system
diagram skeleton.

## Multiple boards

Boards are first-class pages: each one has its own entry in the sidebar's
whiteboards section, and you can search for a board by its title or the words
on it (or filter search with the `wb:` chip — see [Search](/zorite/usage/search/)). Open a board
in its own tab, drag it to another window, or tear it off into a new one, just
like any other page.

## From Logseq

Importing a Logseq graph brings its tldraw whiteboards across as native Zorite
boards — images and all. See [Import from Logseq](/zorite/usage/import/).
