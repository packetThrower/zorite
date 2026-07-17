# gpui-whiteboard

**An infinite, pannable/zoomable whiteboard canvas for [GPUI](https://www.gpui.rs/).**
Shapes, lines, arrows, freehand ink, text, images, and "page cards" on a boundless
board — with select / move / resize / rotate / z-order, a built-in toolbar + color
picker, templates, copy-paste, and undo/redo.

Host-agnostic: its only dependencies are `gpui`, `serde` / `serde_json`, `log`, and
`ttf-parser` (**no `gpui-component`, no native libraries**), so it drops into any
GPUI app on macOS, Linux, or Windows. It comes in two layers:

- a plain, serializable **scene model** ([`Scene`](#the-scene-model) / [`Element`] / …)
  that you persist as an opaque JSON string in your own store, and
- a ready-made **[`WhiteboardView`](#whiteboardview)** entity that renders the board
  *and its whole editing UI* (toolbar, flyouts, color picker, templates gallery,
  right-click menu) and drives all interaction — you supply a theme and a handful of
  optional callbacks.

## Features

- **Full editor, not a bare canvas.** `WhiteboardView` renders its own toolbar
  (pan · select · color │ shapes & text ▾ · pages & images ▾ │ undo · redo · delete),
  a gradient color picker with host-supplied swatches, tool flyouts, a templates
  modal, and a right-click context menu. Drop the entity in and it's a working
  whiteboard.
- **Rich element set.** Freehand pen, rectangle, ellipse, diamond, triangle, rounded
  rectangle, hexagon, 5-point star, line, arrow, text, images, and page-cards — all
  share one select / move / resize / rotate / fill machinery.
- **Pan / zoom infinite canvas.** World-space coordinates with a [`Camera`] (pan
  offset + zoom); drag to pan, scroll/pinch to zoom, snap-to-grid while holding ⌥.
- **Vector text.** Text is rendered as glyph **outlines** (via `ttf-parser`), not
  gpui overlay glyphs — so it rotates, scales, and z-orders exactly like shapes, and
  you can swap in a [custom/user-uploaded face](#custom-fonts). JetBrains Mono ships
  bundled, so the crate works standalone.
- **Auto-fitting shape labels.** Double-click any closed shape (rect / ellipse /
  diamond / triangle / rounded-rect / star / hexagon) to type a centered label. The
  text word-wraps and auto-shrinks to fit the shape's *inscribed* area — staying
  inside slanted or round outlines (a diamond's center, a triangle's lower wedge),
  not just the bounding box — and rotates with the shape, with the same caret /
  selection / clipboard editing as free text. Color it independently of the outline
  via the picker's **Text** tab (alongside Stroke / Fill).
- **Rich text formatting.** Per-character **bold**, italic, underline,
  strikethrough, and highlight on any text — free text or shape labels. Toggle via
  keyboard (⌘B / ⌘I / ⌘U / ⇧⌘X / ⇧⌘H), a right-click **Text ▸** fly-out, or the
  toolbar's **A** fly-out — each showing a ✓ on the formats active across the
  selection. Bold and italic are synthetic (so they work with any uploaded face);
  the styling is stored as runs in the scene and survives edits.
- **True z-order.** Canvas shapes and image/card overlays paint in one interleaved
  stack, so a shape can sit *above or below* an image. Bring to Front / Forward /
  Backward / Send to Back via the menu or `⌘]` / `⌘[` (± ⇧).
- **Copy / paste / templates.** `⌘C`/`⌘X`/`⌘V` and a right-click Copy/Cut/Paste, plus
  reusable named templates — both serialize a selection to the same portable JSON, so
  groups move across boards and windows.
- **Undo / redo**, multi-select (marquee + shift-click), group move/resize, and a
  rotate grip on the selection.
- **Theme-reactive.** Colors come from a `Fn() -> WhiteboardStyle` closure read at
  paint time, so the board follows live theme / light-dark changes (and can differ
  per window) with no push from the host.
- **You own persistence, files, and navigation.** The crate never touches disk, the
  clipboard, or your page store. It calls back to you ([hooks](#host-hooks)) to fetch
  an image bitmap, open a page, read/write the clipboard, or persist the scene — and
  hands you a plain JSON string to store however you like.

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

That alone gives a fully usable board (every tool, color picker, undo/redo, z-order,
copy/paste between boards). Wire the [optional hooks](#host-hooks) to add page-cards,
images, templates, and system-clipboard paste.

## Embedding in a rich-text editor

When you embed the board inside a larger editor, the most common integration mode is
read-only preview + viewport movement.

Use the dedicated constructor:

```rust
use std::rc::Rc;
use gpui_whiteboard::{Scene, WhiteboardStyle, WhiteboardView};

let board = cx.new(|cx| {
    WhiteboardView::new_read_only(
        Scene::from_json(&stored_json),
        Rc::new(light_style),
        cx,
    )
});
```

Or switch an existing board at runtime:

```rust
board.update(cx, |view, cx| {
    view.set_read_only(true, cx);
});
```

`read_only = true` means the board behaves like a forced move tool:

- left-drag pans the canvas
- selection / editing / creation are ignored
- external `set_tool(..)` calls are coerced back to pan mode

If you want a ready-made embed surface with an "edit / maximize" affordance, use
`BoardEmbedView`:

```rust
use std::rc::Rc;
use gpui_whiteboard::{BoardEmbedView, Scene, WhiteboardStyle};

let embed = cx.new(|cx| {
    let mut v = BoardEmbedView::new(
        Scene::from_json(&stored_json),
        Rc::new(light_style),
        cx,
    );
    v.set_on_expand(Rc::new(|window, cx| {
        // Host/editor decides what "maximize for editing" means:
        // - open a modal
        // - switch to a dedicated editor tab
        // - expand the current block
    }));
    v
});
```

Recommended responsibility split:

- `gpui-whiteboard` owns the embedded preview surface and the "Edit" button
- your editor owns the actual maximize / modal / split-pane transition

For local thumbnails inside a document block, use the snapshot + thumbnail view
pair:

```rust
use std::rc::Rc;
use gpui_whiteboard::{BoardThumbnailView, Scene, WhiteboardStyle, WhiteboardView};

let snapshot = board.read(cx).local_thumbnail_snapshot(320.0, 180.0);

let thumb = snapshot.map(|snapshot| {
    cx.new(|_| BoardThumbnailView::new(snapshot, Rc::new(light_style)))
});
```

This renders a chrome-free local preview:

- background grid
- shapes / text / connectors
- page cards
- image placeholders

It intentionally does not render the toolbar, selection handles, or editing
chrome.

## API

### `WhiteboardView`

A gpui entity (`impl Render`) that owns the scene, the current tool, selection,
in-progress edits, undo history, and the entire editing UI. Store the
`Entity<WhiteboardView>` and render it in a tab/panel.

**Construction**

| Method | Signature | Purpose |
| --- | --- | --- |
| `new` | `fn new(scene: Scene, style: WhiteboardStyleFn, cx: &mut Context<Self>) -> Self` | Build a view over `scene`. `style` is read at paint time (see [`WhiteboardStyle`](#whiteboardstyle)). Call inside `cx.new(\|cx\| …)`. |
| `new_read_only` | `fn new_read_only(scene: Scene, style: WhiteboardStyleFn, cx: &mut Context<Self>) -> Self` | Build a read-only board view for embedding/preview. |

**Imperative controls** (most boards never need these — the built-in toolbar/keys
drive them — but they're here for custom chrome):

| Method | Signature | Purpose |
| --- | --- | --- |
| `tool` / `set_tool` | `fn tool(&self) -> Tool` · `fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>)` | Read / set the active [`Tool`](#tool). |
| `read_only` / `set_read_only` | `fn read_only(&self) -> bool` · `fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>)` | Read / toggle forced-pan read-only mode. |
| `zoom_in` / `zoom_out` / `reset_view` | `fn …(&mut self, cx: &mut Context<Self>)` | Zoom about the viewport center; `reset_view` returns to 100% at the origin. |
| `undo` / `redo` | `fn …(&mut self, window: &mut Window, cx: &mut Context<Self>)` | Step the history. (`⌘Z` / `⌘⇧Z` do this already.) |
| `scene` | `fn scene(&self) -> &Scene` | Borrow the current model — e.g. to persist after an `add_embed`/`add_image_at` (which don't auto-fire `on_change`). |
| `viewport_center` | `fn viewport_center(&self) -> [f32; 2]` | The world point at the center of the viewport — where pastes/templates land. |
| `local_thumbnail_spec` | `fn local_thumbnail_spec(&self, width_px: f32, height_px: f32) -> Option<LocalThumbnailSpec>` | Build the default local-thumbnail focus spec. |
| `local_thumbnail_spec_for_mode` | `fn local_thumbnail_spec_for_mode(&self, mode: LocalThumbnailMode, width_px: f32, height_px: f32) -> Option<LocalThumbnailSpec>` | Build a local-thumbnail focus spec for an explicit focus mode. |
| `local_thumbnail_snapshot` | `fn local_thumbnail_snapshot(&self, width_px: f32, height_px: f32) -> Option<LocalThumbnailSnapshot>` | Capture a serializable thumbnail snapshot (`scene + spec`). |
| `local_thumbnail_snapshot_for_mode` | `fn local_thumbnail_snapshot_for_mode(&self, mode: LocalThumbnailMode, width_px: f32, height_px: f32) -> Option<LocalThumbnailSnapshot>` | Capture a snapshot for an explicit focus mode. |

### `BoardEmbedView`

A small host-facing wrapper around a read-only `WhiteboardView`, intended for rich
text / document embedding.

| Method | Signature | Purpose |
| --- | --- | --- |
| `new` | `fn new(scene: Scene, style: WhiteboardStyleFn, cx: &mut Context<Self>) -> Self` | Build a read-only embedded board preview. |
| `board` | `fn board(&self) -> Entity<WhiteboardView>` | Access the inner board entity for host-driven inspection or updates. |
| `set_on_expand` | `fn set_on_expand(&mut self, f: ExpandEmbedFn)` | Install the callback fired when the embed's "编辑" button is clicked. |

### `BoardThumbnailView`

A lightweight, read-only local thumbnail renderer built from a
`LocalThumbnailSnapshot`.

| Method | Signature | Purpose |
| --- | --- | --- |
| `new` | `fn new(snapshot: LocalThumbnailSnapshot, style: WhiteboardStyleFn) -> Self` | Build a chrome-free thumbnail view. |
| `snapshot` | `fn snapshot(&self) -> &LocalThumbnailSnapshot` | Read the current thumbnail snapshot. |
| `set_snapshot` | `fn set_snapshot(&mut self, snapshot: LocalThumbnailSnapshot)` | Replace the rendered snapshot without rebuilding the surrounding host UI. |

### Thumbnail focus types

`LocalThumbnailMode` controls what the board focuses when building a local
thumbnail:

- `Auto`: selected content first, otherwise current viewport, otherwise all content
- `Selection`: current selection only
- `Viewport`: current camera viewport
- `AllContent`: all scene elements
- `Element(id)`: one explicit element

`LocalThumbnailSpec` returns:

- `anchor_element_id`
- `focus_bounds`
- `scene_bounds`
- `camera`

`LocalThumbnailSnapshot` packages:

- `scene`
- `spec`

Hosts that only have persisted scene JSON can build a document thumbnail without
mounting a full board entity first:

```rust
let scene = Scene::from_json(&stored_json);
let snapshot = LocalThumbnailSnapshot::for_scene_all_content(scene, 320.0, 180.0);
let thumbnail = cx.new(|_| BoardThumbnailView::new(snapshot, style));
```

**Building elements from the host** (called *after* a place-hook fires; see
[hooks](#host-hooks)). These run mid-host-update and so do **not** fire
`on_change` — persist explicitly via `scene()` afterward:

| Method | Signature |
| --- | --- |
| `add_embed` | `fn add_embed(&mut self, page_id: i64, title: impl Into<String>, x: f32, y: f32, cx: &mut Context<Self>)` |
| `add_image_at` | `fn add_image_at(&mut self, src: impl Into<String>, px_w: f32, px_h: f32, cx_world: f32, cy_world: f32, cx: &mut Context<Self>)` |
| `paste_elements` | `fn paste_elements(&mut self, json: &str, window: &mut Window, cx: &mut Context<Self>)` |

`add_image_at` sizes the element from the image's pixel dimensions (`px_w`/`px_h`,
aspect preserved) centered on `(cx_world, cy_world)`. `paste_elements` stamps a
serialized selection (from a clipboard read) centered in the viewport.

### `WhiteboardStyle`

The board reads its palette through a `Fn() -> WhiteboardStyle` each paint (not
stored), so returning fresh values tracks live theme changes.

```rust
pub struct WhiteboardStyle {
    pub bg: Hsla,           // canvas background
    pub grid: Hsla,         // background grid dots
    pub text: Hsla,         // HUD / muted on-canvas text (zoom %, "Loading…")
    pub ink: Hsla,          // default stroke/shape color (per-element color overrides it)
    pub panel: Hsla,        // toolbar / flyout pills (can be glassy)
    pub panel_strong: Hsla, // color picker / menu surface (keep opaque + readable)
    pub accent: Hsla,       // active-tool highlight
    pub selection: Hsla,    // selection outline (use a strong, visible color)
    pub swatches: Vec<Hsla>,// quick swatches in the color picker (your theme colors)
}

pub type WhiteboardStyleFn = Rc<dyn Fn() -> WhiteboardStyle>;
```

### Host hooks

All optional — install with the matching `set_*` method after `new`. Each is an
`Rc<dyn Fn(...)>`; the board works with none installed (you just lose that feature).
Coordinates passed to hooks are **world-space** (see [`Camera`]).

| Setter | Type | Fires when… | You should… |
| --- | --- | --- | --- |
| `set_on_change` | `ChangeFn` = `Fn(String, &mut Window, &mut App)` | the board changes (element committed/moved/deleted, camera moved) | persist the scene JSON string |
| `set_on_place_embed` | `PlaceEmbedFn` = `Fn(f32, f32, &mut Window, &mut App)` | the page-card tool is clicked at `(x, y)` | pick a page, then call `add_embed(page_id, title, x, y, cx)` |
| `set_on_open` | `OpenPageFn` = `Fn(i64, &mut Window, &mut App)` | a page-card is double-clicked | open that page (`page_id`) in your app |
| `set_on_image` | `ImageFn` = `Fn(&str, f32, &mut Window, &mut App) -> Option<ImageSource>` | each paint, per image element | return the decoded bitmap for `src` rotated by the `f32` radians (decode off-thread; `None` until ready, then re-render) |
| `set_on_place_image` | `PlaceImageFn` = `Fn(f32, f32, &mut Window, &mut App)` | the image tool is clicked at `(x, y)` | pick a file, import it, then call `add_image_at(...)` |
| `set_on_drop_files` | `DropFilesFn` = `Fn(Vec<PathBuf>, f32, f32, &mut Window, &mut App)` | files are dropped on the canvas at `(x, y)` | import any images and place them via `add_image_at(...)` |
| `set_on_copy` | `CopyFn` = `Fn(String, &mut Window, &mut App)` | `⌘C` / `⌘X` with a selection | write the serialized selection to the system clipboard |
| `set_on_paste` | `PasteFn` = `Fn(&mut Window, &mut App) -> Option<String>` | the context-menu **Paste** | read the clipboard; return previously copied board JSON, or `None` |
| `set_on_save_template` | `SaveTemplateFn` = `Fn(String, &mut Window, &mut App)` | the user saves a selection as a template | name + store it, then feed the list back via `set_templates` |
| `set_on_delete_template` | `DeleteTemplateFn` = `Fn(i64, &mut Window, &mut App)` | a template card is right-clicked → delete | remove it (by id), then `set_templates` |
| `set_on_save_colors` | `SavedColorsFn` = `Fn(Vec<u32>, &mut Window, &mut App)` | the user adds/removes a swatch in the picker's **Saved** palette | persist the packed `0xRRGGBBAA` list, then push it back via `set_saved_colors` |
| `set_on_pick_font` | `PickFontFn` = `Fn(FontPick, &mut Window, &mut App)` | the **Aa** Font flyout's *Upload* / *Use default* is clicked | load the `.ttf`/`.otf` (or the default) and call `set_font` — and persist the per-board choice |
| `set_on_move_toolbar` | `MoveToolbarFn` = `Fn(Option<(f32, f32)>, bool, &mut Window, &mut App)` | the toolbar is dragged, reset (double-click the grip), or flipped row↔column | persist its new board-relative top-left (`None` = default top-center) and orientation (`bool` = vertical) |
| `set_templates` | `fn(&mut self, Vec<Template>, &mut Context<Self>)` | — | push the current template list (on open and after any save/delete) |
| `set_saved_colors` | `fn(&mut self, Vec<u32>, &mut Context<Self>)` | — | push the user's saved-color palette (on open and after a change) |
| `set_toolbar_pos` | `fn(&mut self, Option<(f32, f32)>, &mut Context<Self>)` | — | push the saved toolbar position (`None` = default top-center) on open and after a change |
| `set_toolbar_vertical` | `fn(&mut self, bool, &mut Context<Self>)` | — | push the saved toolbar orientation (vertical = a column) on open and after a change |
| `set_font` | `fn(&mut self, Font, &mut Context<Self>)` | — | swap the text face (see [Custom fonts](#custom-fonts)) |

> **Image & clipboard flow.** Images aren't stored in the scene — only a `src`
> reference is. The crate asks for the bitmap via `ImageFn` each paint; you own the
> file store and the cache (decode off-thread, downscale, manage the GPU texture).
> Copy/paste likewise routes the bytes through `CopyFn`/`PasteFn` so the system
> clipboard stays the source of truth (and `⌘V` prefers copied elements over a
> clipboard image). Templates persist through `SaveTemplateFn`/`set_templates`.

### The scene model

A `Scene` is the board's persisted state — a [`Camera`] plus a `Vec<Element>` in
paint order (earlier = behind). It's plain serde data: store
`view.scene().to_json()` and reload with `Scene::from_json(&s)` (which never panics —
empty/garbage yields a blank board). Element colors are packed `0xRRGGBBAA` `u32`s.

```rust
pub struct Scene { pub camera: Camera, pub elements: Vec<Element> }
impl Scene {
    pub fn from_json(s: &str) -> Self;   // empty board on "" / malformed input
    pub fn to_json(&self) -> String;
}

pub struct Camera { pub x: f32, pub y: f32, pub zoom: f32 } // pan offset + zoom
// world point under a canvas point `s`:  camera.offset + s / zoom

pub struct Element {
    pub id: u64,
    pub kind: ElementKind,
    pub stroke: Option<u32>,    // packed 0xRRGGBBAA; None = follow theme ink
    pub fill:   Option<u32>,    // closed shapes only; None = unfilled outline
    pub label:  Option<String>, // closed shapes only; centered, word-wrapped + auto-shrunk to fit
    pub label_color: Option<u32>, // shape label color; None = follow the stroke / theme ink
    pub styles: Vec<StyleSpan>,   // per-character bold/italic/underline/strike/highlight runs
}

pub enum ElementKind {       // serialized snake_case: {"rect": {...}}, {"image": {...}}, …
    Draw(Stroke),                                        // freehand pen
    Rect(BoxGeom), Ellipse(BoxGeom), Diamond(BoxGeom),   // box-like shapes
    Triangle(BoxGeom), RoundRect(BoxGeom),
    Star(BoxGeom), Hexagon(BoxGeom),
    Line(SegGeom), Arrow(SegGeom),
    Text(TextGeom),
    Embed(EmbedGeom),                                    // page-card
    Image(ImageGeom),
}

pub struct Stroke   { pub points: Vec<[f32; 2]>, pub width: f32 }
pub struct BoxGeom  { pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub width: f32, pub rotation: f32 }
pub struct SegGeom  { pub x1: f32, pub y1: f32, pub x2: f32, pub y2: f32, pub width: f32 }
pub struct TextGeom { pub x: f32, pub y: f32, pub content: String, pub size: f32, pub rotation: f32, /* measured_* cached */ }
pub struct EmbedGeom{ pub page_id: i64, pub title: String, pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
pub struct ImageGeom{ pub src: String, pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub rotation: f32 }
```

`rotation` is radians clockwise about the element's center. All geometry is
world-space; multiply by `camera.zoom` and subtract the pan offset for screen space.

### `Tool`

```rust
pub enum Tool {
    Pan, Select, Pen, Rect, Ellipse, Diamond, Triangle,
    RoundRect, Star, Hexagon, Line, Arrow, Text, Embed, Image,
}
```

`Pan` is the default. The view renders a toolbar for these and handles their
single-key shortcuts itself; use `set_tool` only if you drive tools from your own UI.

### Templates

A reusable group of elements, stamped centered in the viewport. The crate renders the
preview gallery and instantiates on click; **you own storage and the `id`.**

```rust
pub struct Template { pub id: i64, pub name: String, pub elements: Vec<Element> }
impl Template {
    // `elements_json` is a serialized `Vec<Element>` (what `SaveTemplateFn` hands you);
    // malformed JSON yields an empty (still-listable) template.
    pub fn from_json(id: i64, name: impl Into<String>, elements_json: &str) -> Self;
}
```

### Custom fonts

Text is drawn from glyph outlines, so any TrueType/OpenType face works. The default is
bundled (JetBrains Mono, OFL); swap one in directly with `set_font`:

```rust
use gpui_whiteboard::Font;
if let Some(face) = Font::from_bytes(ttf_bytes, /* face index */ 0) {
    board.update(cx, |v, cx| v.set_font(face, cx));
}
// Font::default() is the bundled face.
```

For a user-facing picker, install `set_on_pick_font`: the toolbar then shows an **Aa**
button whose flyout offers *Upload font…* and *Use default*. The crate hands you a
`FontPick` (`Upload` / `Default`); you run the file dialog, build the face, call
`set_font`, and persist the choice however you like (the host app keeps one face per
board, restored on reopen).

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
| drag a handle · the round grip above a selection | resize (corners scale; edge handles stretch one axis, on a single element or a group) · rotate |
| double-click a page-card | open its page (via `OpenPageFn`) |
| double-click text · `T`-click text | edit it — click a letter for the caret, drag / double-click to select |
| double-click a closed shape | edit its centered label — wraps + auto-shrinks to fit; full caret / selection / clipboard |
| `⌘B` `⌘I` `⌘U` `⇧⌘X` `⇧⌘H` (editing text) | bold / italic / underline / strikethrough / highlight the selection (toggle) |
| drag the dotted grip (left of the toolbar) | move the toolbar (tap `R` mid-drag to flip row ↔ column; double-click the grip resets it) |
| right-click | context menu (z-order, copy/cut/paste, save as template); while editing text, a **Text ▸** fly-out toggles bold / italic / underline / strike / highlight (✓ marks active) |

While editing a text element it behaves like a normal text field: click to place the
caret, click-drag or double-click to select, arrows / Home / End (⇧ extends), ⌘A, and
⌘C / ⌘X / ⌘V on the system clipboard; Esc (or a click away) commits.

## Persistence

The crate is storage-agnostic. Persist a board by storing the string from `on_change`
(or `view.scene().to_json()`); reload with `WhiteboardView::new(Scene::from_json(&s), …)`.
Images and templates live in *your* store — the scene only references images by `src`,
and templates round-trip through your `SaveTemplateFn` + `set_templates`.

## Status

Pre-1.0 (`0.1`). The scene JSON is forward-leaning — new fields use serde defaults, so
older boards keep loading — but the API may still shift before 1.0. Performance note:
elements are re-tessellated each paint (as GPUI's own `painting` examples do); a
built-`Path` cache + viewport culling is the planned optimization once boards get
large.

## License

GPL-3.0-or-later. The bundled default font (JetBrains Mono) is under the SIL Open Font
License — see `assets/JetBrainsMono-OFL.txt`.
