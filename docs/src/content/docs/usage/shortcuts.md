---
title: Keyboard shortcuts
description: 'Keyboard shortcuts in Zorite — new tab, new window, close tab, tab switching, settings, find in page, search all notes, and the slash command menu.'
---

One binding works on every OS: the modifier shown as **⌘** below is **Cmd** on
macOS and **Ctrl** on Windows and Linux.

## App-wide

| Action | macOS | Windows / Linux |
|---|---|---|
| New tab (new page) | `⌘T` | `Ctrl+T` |
| New window | `⌘N` | `Ctrl+N` |
| Close tab | `⌘W` | `Ctrl+W` |
| Next tab | `Ctrl+Tab` | `Ctrl+Tab` |
| Previous tab | `Ctrl+Shift+Tab` | `Ctrl+Shift+Tab` |
| Find in page | `⌘F` | `Ctrl+F` |
| Search all notes | `⌘⇧F` | `Ctrl+Shift+F` |
| Fit oversized images to view | `⌘⇧I` | `Ctrl+Shift+I` |
| Open settings | `⌘,` | `Ctrl+,` |
| Quit | `⌘Q` | `Ctrl+Q` |

**Find in page** searches the current page (reading view or editor); on the
Journal it opens the feed's find bar, matching across every loaded day.
**Search all notes** opens the note-wide search (see
[Search](/zorite/usage/search/)). PDF tabs keep their own find — `⌘F` inside a
PDF searches that document.

The tab-switch chords are `Ctrl+Tab` / `Ctrl+Shift+Tab` on every platform
(including macOS), matching the convention most apps use for cycling tabs.

## While editing

| Action | Key |
|---|---|
| Open the slash command menu | `/` |
| Move up / down in the menu | `↑` / `↓` |
| Insert the selected item | `Enter` |
| Close the slash menu | `Esc` |
| Indent / nest list item | `Tab` |
| Outdent | `Shift+Tab` |
| Bold / italic / inline code | `⌘B` / `⌘I` / `⌘E` |
| Copy | `⌘C` / `Ctrl+C` |
| Cut | `⌘X` / `Ctrl+X` |
| Paste (image-aware) | `⌘V` / `Ctrl+V` |
| Undo | `⌘Z` / `Ctrl+Z` |
| Redo | `⌘⇧Z` / `Ctrl+Y` |
| Select all | `⌘A` / `Ctrl+A` |

Click into a note to edit its raw Markdown; click away (or press `Esc` out of the
slash menu and click out) to re-render. `⌘V` / `Ctrl+V` pastes an image from the
clipboard if there is one, and otherwise pastes text as usual.

**Enter** continues a list or to-do — the `-`, `1.`, or `- [ ]` marker carries to
the next line, and an empty item ends the list. **Inside a table**, `Tab` /
`Shift+Tab` and the arrow keys move from cell to cell (the arrows keep your
column) and `Enter` drops to the row below.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-edit-light.png" />
  <img
    src="/zorite/screenshots/zorite-edit-dark.png"
    alt="A journal day in edit mode — raw Markdown on click, rendered on click-away"
  />
</picture>

## Whiteboard

These work while a board tab is focused. A single letter picks a tool; the
editing chords act on the current selection.

### Tools

| Tool | Key | | Tool | Key |
|---|---|---|---|---|
| Select | `V` | | Star | `S` |
| Pan | `H` | | Hexagon | `X` |
| Pen | `P` | | Line | `L` |
| Rectangle | `R` | | Arrow | `A` |
| Ellipse | `O` | | Text | `T` |
| Diamond | `D` | | Image | `I` |
| Triangle | `G` | | Rounded rectangle | `U` |

### Editing

| Action | Key |
|---|---|
| Undo | `⌘Z` / `Ctrl+Z` |
| Redo | `⌘⇧Z` / `Ctrl+Y` |
| Copy / Cut / Paste | `⌘C` · `⌘X` · `⌘V` |
| Bring forward / to front | `⌘]` / `⌘⇧]` |
| Send backward / to back | `⌘[` / `⌘⇧[` |
| Delete selection | `Delete` / `Backspace` |
| Deselect | `Esc` |

Hold **Option / Alt** while dragging to snap to the grid, and **Shift** while
rotating to snap to 45°.

## PDF viewer

These work while a PDF tab is focused.

| Action | Key |
|---|---|
| Next / previous page | `PageDown` / `PageUp` |
| First / last page | `Home` / `End` |
| Zoom in / out | `⌘=` / `⌘−` |
| Reset zoom | `⌘0` |
| Find in PDF | `⌘F` |
| Next / previous match | `⌘G` / `⌘⇧G` |
| Go to page… | `⌘⌥G` |
| Toggle highlight mode | `⌘⇧H` |

A PDF's own find (`⌘F`) is separate from the note-wide search — see
[Search](/zorite/usage/search/).

## Mouse

A few things are mouse-driven rather than keyboard-bound:

- **Drag a tab** to reorder it, drop it on another window to move it there, or
  release it on empty space to tear it off into a new window.
- **Drag a corner handle** on an inline image to resize it.
- **Click a to-do checkbox** to toggle it.
- **Hover a table** for `+` / `−` handles to add or delete a row or column, or
  **right-click** it for insert / delete, column alignment, and **Delete table**.
- **Drag to highlight** a passage in the PDF viewer.
- The **⚙ Settings** button and a quick **light/dark toggle** live in the title
  bar.

> **Import from Logseq** lives in the **File** menu — the native menu bar on
> macOS, the in-titlebar **File / Edit / View** menu on Windows and Linux — and
> has no keyboard shortcut.
