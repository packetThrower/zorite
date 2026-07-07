# gpui-whiteboard

**An infinite, pannable/zoomable whiteboard canvas for [GPUI](https://www.gpui.rs/).**
Shapes, lines, arrows, freehand ink, text, images, and "page cards" on a boundless
board — with select / move / resize / rotate / z-order, a built-in toolbar + color
picker, templates, copy-paste, and undo/redo.

Host-agnostic: its only dependencies are `gpui`, `serde` / `serde_json`, `log`, and
`ttf-parser` (**no `gpui-component`, no native libraries**), so it drops into any
GPUI app on macOS, Linux, or Windows.

**📖 Full reference:** every public item — the scene model, all `WhiteboardView`
methods, every host callback — with signatures, parameter tables, return
contracts, and edge cases, lives in [API.md](API.md).

## Features

- **Full editor, not a bare canvas.** `WhiteboardView` renders its own toolbar
  (pan · select · color │ shapes & text ▾ · pages & images ▾ │ undo · redo · delete),
  a gradient color picker with host-supplied swatches, tool flyouts, a templates
  modal, and a right-click context menu. Drop the entity in and it's a working
  whiteboard.
- **Rich element set.** Freehand pen, rectangle, ellipse, diamond, triangle, rounded
  rectangle, hexagon, 5-point star, line, arrow, text, images, and page-cards — all
  share one select / move / resize / rotate / fill machinery.
- **Pan / zoom infinite canvas.** World-space coordinates with a camera (pan offset
  + zoom); drag to pan, scroll/pinch to zoom, snap-to-grid while holding ⌥.
- **Vector text.** Text is rendered as glyph **outlines** (via `ttf-parser`), not
  gpui overlay glyphs — so it rotates, scales, and z-orders exactly like shapes, and
  you can swap in a custom/user-uploaded face. JetBrains Mono ships bundled, so the
  crate works standalone.
- **Auto-fitting shape labels.** Double-click any closed shape to type a centered
  label; it word-wraps and auto-shrinks to fit the shape's *inscribed* area, rotates
  with the shape, and edits with full caret / selection / clipboard support.
- **Rich text formatting.** Per-character bold, italic, underline, strikethrough,
  and highlight on any text — via keyboard (⌘B / ⌘I / ⌘U / ⇧⌘X / ⇧⌘H), the
  right-click **Text ▸** fly-out, or the toolbar's **A** fly-out. Bold/italic are
  synthetic, so they work with any uploaded face; runs are stored in the scene.
- **True z-order.** Shapes and image/card overlays paint in one interleaved stack;
  reorder via the menu or `⌘]` / `⌘[` (± ⇧).
- **Copy / paste / templates.** `⌘C`/`⌘X`/`⌘V` plus reusable named templates — both
  serialize a selection to the same portable JSON, so groups move across boards.
- **Undo / redo**, multi-select (marquee + shift-click), group move/resize/rotate.
- **Theme-reactive.** Colors come from a `Fn() -> WhiteboardStyle` closure read at
  paint time, so the board follows live theme changes with no push from the host.
- **You own persistence, files, and navigation.** The crate never touches disk, the
  clipboard, or your page store — it calls back to you and hands you a plain JSON
  string to store however you like.

## Adding the dependency

The crate lives in the Zorite workspace (not on crates.io); depend on it by path:

```toml
[dependencies]
gpui-whiteboard = { path = "../gpui-whiteboard" }
```

It follows the workspace's `gpui` pin (`gpui = { workspace = true }`), so the app
and the crate unify to one gpui.

## Quick start

```rust
use std::rc::Rc;
use gpui_whiteboard::{Scene, WhiteboardStyle, WhiteboardView};

// Build the view over a scene (a fresh `Scene::default()` or `Scene::from_json`
// of a stored board). Call inside `cx.new(..)`.
let board = cx.new(|cx| {
    let mut v = WhiteboardView::new(
        Scene::from_json(&stored_json),     // empty board on "" / malformed input
        Rc::new(|| WhiteboardStyle {         // mapped from your theme, read each paint
            bg:           theme::bg(),
            grid:         theme::border_subtle(),
            text:         theme::muted(),    // HUD / placeholder text
            ink:          theme::text(),     // default stroke color
            panel:        theme::glass(),    // toolbar / flyout pills
            panel_strong: theme::sidebar(),  // color picker / menu (keep readable)
            accent:       theme::accent_tint(), // active-tool highlight
            selection:    theme::accent(),   // selection outline
            swatches:     theme::palette(),  // color-picker quick swatches
        }),
        cx,
    );
    // Persist on every change (the only hook most boards need):
    v.set_on_change(Rc::new(move |scene_json, _window, cx| {
        // store `scene_json` wherever this board lives
    }));
    v
});

// Render it like any entity:
div().size_full().child(board.clone())
```

That alone gives a fully usable board (every tool, color picker, undo/redo,
z-order). Wire the optional hooks to add page-cards, images, system-clipboard
copy/paste, templates, custom fonts, and toolbar-layout memory — each is a
`set_on_*` installer documented in [API.md](API.md).

## Host integration

- **Persistence.** Store the JSON string `on_change` hands you (or
  `view.scene().to_json()`); reload with `Scene::from_json`, which never panics —
  empty/garbage yields a blank board. The scene JSON is forward-leaning: new
  fields use serde defaults, so older boards keep loading.
- **Images and the clipboard stay yours.** The scene stores only a `src`
  reference; the crate asks for the decoded bitmap each paint (`ImageFn`) — you
  own the file store and the cache. Copy/paste routes through `CopyFn`/`PasteFn`
  so the system clipboard is the source of truth.
- **Pages, templates, fonts, toolbar layout** all round-trip the same way: a hook
  reports the change, you persist it, and push it back with the matching `set_*`
  method. Exact contracts per hook are in [API.md](API.md).
- **Coordinates** passed to hooks are world-space; see [`Camera`](API.md#camera).

## Keyboard & mouse

The view handles these when it has focus (it focuses on a canvas click):

| Input | Action |
| --- | --- |
| `H` `V` `P` `R` `O` `D` `G` `U` `S` `X` `L` `A` `T` `I` | pick a tool (pan, select, pen, rect, ellipse, diamond, triangle, rounded-rect, star, hexagon, line, arrow, text, image) |
| `⌫` / `Delete` | delete the selection |
| `⌘Z` / `⌘⇧Z` | undo / redo |
| `⌘C` / `⌘X` / `⌘V` | copy / cut / paste the selection |
| `⌘]` / `⌘[` | bring forward / send backward (add `⇧` for to-front / to-back) |
| `Esc` | deselect (or close the color picker / templates modal) |
| drag (Pan tool) · middle-drag | pan the canvas |
| scroll · pinch | zoom |
| hold `⌥` while dragging | snap to the grid |
| click / shift-click / marquee-drag (Select tool) | select one / add / box-select |
| drag a handle · the round grip above a selection | resize · rotate |
| double-click a page-card | open its page |
| double-click text or a closed shape | edit the text / centered label |
| `⌘B` `⌘I` `⌘U` `⇧⌘X` `⇧⌘H` (editing text) | toggle bold / italic / underline / strikethrough / highlight |
| drag the dotted grip (left of the toolbar) | move the toolbar (`R` mid-drag flips row ↔ column; double-click the grip resets) |
| right-click | context menu (z-order, copy/cut/paste, save as template, text formatting) |

While editing text it behaves like a normal text field: click to place the caret,
drag / double-click to select, arrows / Home / End (⇧ extends), ⌘A, and system
clipboard ⌘C / ⌘X / ⌘V; Esc (or a click away) commits.

## Status

Pre-1.0 (`0.1`). The API may still shift before 1.0. Performance note: elements
are re-tessellated each paint (as GPUI's own `painting` examples do); a built-`Path`
cache + viewport culling is the planned optimization once boards get large.

## License

GPL-3.0-or-later. The bundled default font (JetBrains Mono) is under the SIL Open
Font License — see `assets/JetBrainsMono-OFL.txt`.
