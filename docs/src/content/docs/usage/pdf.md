---
title: PDF & images
description: 'Link or drop PDFs into the page-virtualized viewer — zoom, navigation, a TOC, drag-to-highlight markup with note↔PDF jump links, fillable forms, and password-protected files. Plus inline images you can paste, drop, and resize.'
---

Reference material lives next to your notes. Link a PDF with a wiki-link or drop
one onto a note and it opens in a dedicated viewer tab; paste or drag an image
into a note and it renders inline.

## PDFs

Link a PDF with `[[file.pdf]]` or `![](file.pdf)`, or **drop one onto a note**,
to open it in a dedicated viewer tab.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-pdf-light.png" />
  <img
    src="/zorite/screenshots/zorite-pdf-dark.png"
    alt="The Zorite PDF viewer — a page-virtualized document with a table of contents and drag-to-highlight markup"
  />
</picture>

### A viewer that scales

The viewer is **page-virtualized**: only pages near the viewport are rasterized,
and scrolled-away pages free both their memory *and* their GPU texture. An
800-page document stays light. Rendering is DPI-aware, so pages stay crisp on
high-resolution displays.

Zoom, page navigation, and a table of contents for jumping around a long
document.

### Highlight markup

**Drag to highlight** a passage in a PDF. Each highlight becomes a markup with
**note↔PDF jump links**: follow it from your note to land on the right page, or
from the PDF back to where you wrote about it.

### Password-protected PDFs

Encrypted PDFs are supported — both **RC4** and **AES**. Zorite prompts for the
password and renders the document once it's unlocked.

### Fill in forms

A PDF with form fields (AcroForm) is fillable right in the viewer. Click a
**checkbox or radio button** to toggle it. Click a **text field** and a small
input appears seated under it — type, then **Enter** (or click away) saves,
**Esc** cancels, and **Tab / Shift-Tab** hop field to field, scrolling each
into view. Edits are written back into the stored PDF with proper appearance
streams, so the filled file renders correctly in every other PDF viewer too.
Signature and read-only fields stay untouched; choice fields currently edit
as free text.

### Under the hood

PDF rendering is pure-Rust via [`hayro`](https://crates.io/crates/hayro), so
there are **no native dependencies**. The viewer is its own reusable
[`gpui-pdf`](https://github.com/packetThrower/zorite/blob/main/crates/gpui-pdf/README.md)
crate.

## Inline images

`![](path-or-url)` images render for real in your notes. To add one, **paste**
or **drag-and-drop** a file — it's copied into the data directory's `images/`
folder, so the note doesn't depend on where the original lived. Importing the
same image twice (paste or drag, any page) reuses the existing copy instead of
duplicating it, and **Settings → General → Unused images** sweeps files nothing
references anymore into the system trash (with a confirmation listing them
first).

Drag the corner handle to **resize** an image; the new size is saved back into
the Markdown as `{width=N}`, so it stays the size you set the next time the note
renders.

## Export a note to PDF

Right-click a tab or a sidebar page and pick **Export as PDF…** (or press
`⌘P` / `Ctrl+P` on the active note). The export renders like the reading view
— wrapped styled text, tables, inline images, alerts, typeset math, and
mermaid diagrams — written by a pure-Rust PDF writer, no browser or print
dialog involved.
