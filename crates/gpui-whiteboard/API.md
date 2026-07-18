# gpui-whiteboard API

The complete public API of [`gpui-whiteboard`](README.md) — every exported item,
with its signature, parameters, return contract, edge cases, and cost. For the
what-and-why (features, quick start, keyboard map, host integration), see the
[README](README.md).

## Public API at a glance

Everything below is the complete public surface — if it isn't listed here, it
isn't public. Colors throughout are packed `0xRRGGBBAA` `u32`s; geometry is
world-space `f32` (see [`Camera`](#camera)).

| Item | Kind | Signature | Purpose |
| --- | --- | --- | --- |
| [`Scene`](#scene) | struct | `{ camera, elements }` | The board document — everything persisted |
| [`Scene::from_json`](#scenefrom_json) | assoc fn | `fn from_json(s: &str) -> Self` | Parse stored JSON; empty board on bad input |
| [`Scene::to_json`](#sceneto_json) | method | `fn to_json(&self) -> String` | Serialize for persistence |
| [`Element`](#element) | struct | `{ id, kind, stroke, fill, label, label_color, styles, mindmap }` | One board element |
| [`ElementKind`](#elementkind) | enum | 13 variants | What an element is (pen stroke, shape, text, …) |
| [`Stroke`](#stroke) | struct | `{ points, width }` | Freehand pen geometry |
| [`BoxGeom`](#boxgeom) | struct | `{ x, y, w, h, width, rotation }` | Box-shape geometry (rect/ellipse/…) |
| [`SegGeom`](#seggeom) | struct | `{ x1, y1, x2, y2, width, style, start_anchor, end_anchor }` | Line / arrow geometry |
| [`SegmentStyle`](#segmentstyle) | enum | `Solid \| Dashed` | A line/arrow's stroke style |
| [`SegmentAnchor`](#segmentanchor) | struct | `{ element_id, connector }` | A segment endpoint bound to a shape |
| [`MindMapNodeMeta`](#mindmapnodemeta) | struct | `{ parent, side, order, root_direction, connector_style }` | Mind-map metadata on an element |
| [`MindMapSide`](#mindmap-enums) | enum | `Left \| Right` | Which side of the root a branch hangs |
| [`MindMapRootDirection`](#mindmap-enums) | enum | `Both \| Left \| Right` | Root's growth direction |
| [`MindMapConnectorStyle`](#mindmap-enums) | enum | `Straight \| Bezier \| Orthogonal` | Mind-map connector rendering |
| [`LocalThumbnailMode`](#local-thumbnails) | enum | thumbnail framing modes | How a snapshot frames the scene |
| [`LocalThumbnailSpec`](#local-thumbnails) | struct | framing + size | A thumbnail's render parameters |
| [`LocalThumbnailSnapshot`](#local-thumbnails) | struct | `{ scene, camera, spec, … }` | A frozen scene + framing for a thumbnail |
| [`BoardEmbedView`](#boardembedview) | struct | entity | A read-only embedded board + Edit button |
| [`BoardThumbnailView`](#boardthumbnailview) | struct | entity | A static thumbnail rendering of a snapshot |
| [`ExpandEmbedFn`](#boardembedview) | type alias | `Rc<dyn Fn(&mut Window, &mut App)>` | Embed's Edit button clicked |
| [`TextGeom`](#textgeom) | struct | `{ x, y, content, size, rotation, … }` | Free-text geometry |
| [`EmbedGeom`](#embedgeom) | struct | `{ page_id, title, x, y, w, h }` | Page-card geometry |
| [`ImageGeom`](#imagegeom) | struct | `{ src, x, y, w, h, rotation }` | Image geometry (host-managed `src`) |
| [`Camera`](#camera) | struct | `{ x, y, zoom }` | The viewport: pan offset + zoom |
| [`StyleSpan`](#stylespan) | struct | `{ start, end, style }` | A formatting run over a byte range |
| [`RunStyle`](#runstyle) | struct | `{ bold, italic, underline, strike, highlight }` | The formatting of a run of characters |
| [`Format`](#format) | enum | `Bold \| Italic \| Underline \| Strike` | A toggleable boolean format |
| [`Tool`](#tool) | enum | 15 variants | The active tool (UI state, not persisted) |
| [`WhiteboardStyle`](#whiteboardstyle) | struct | 9 fields | Theme colors, read at paint time |
| [`WhiteboardStyleFn`](#whiteboardstylefn) | type alias | `Rc<dyn Fn() -> WhiteboardStyle>` | The host's theme closure |
| [`ChangeFn`](#changefn) | type alias | `Rc<dyn Fn(String, &mut Window, &mut App)>` | Persist-on-change hook |
| [`PlaceEmbedFn`](#placeembedfn) | type alias | `Rc<dyn Fn(f32, f32, &mut Window, &mut App)>` | Page-card tool clicked |
| [`OpenPageFn`](#openpagefn) | type alias | `Rc<dyn Fn(i64, &mut Window, &mut App)>` | Page-card double-clicked |
| [`ImageFn`](#imagefn) | type alias | `Rc<dyn Fn(&str, f32, &mut Window, &mut App) -> Option<ImageSource>>` | Fetch a decoded (rotated) bitmap |
| [`PlaceImageFn`](#placeimagefn) | type alias | `Rc<dyn Fn(f32, f32, &mut Window, &mut App)>` | Image tool clicked |
| [`DropFilesFn`](#dropfilesfn) | type alias | `Rc<dyn Fn(Vec<PathBuf>, f32, f32, &mut Window, &mut App)>` | Files dropped on the canvas |
| [`CopyFn`](#copyfn) | type alias | `Rc<dyn Fn(String, &mut Window, &mut App)>` | Write the selection to the clipboard |
| [`PasteFn`](#pastefn) | type alias | `Rc<dyn Fn(&mut Window, &mut App) -> Option<String>>` | Read board elements from the clipboard |
| [`SaveTemplateFn`](#savetemplatefn) | type alias | `Rc<dyn Fn(String, &mut Window, &mut App)>` | Save the selection as a template |
| [`DeleteTemplateFn`](#deletetemplatefn) | type alias | `Rc<dyn Fn(i64, &mut Window, &mut App)>` | Delete a stored template |
| [`SavedColorsFn`](#savedcolorsfn) | type alias | `Rc<dyn Fn(Vec<u32>, &mut Window, &mut App)>` | Persist the saved-color palette |
| [`PickFontFn`](#pickfontfn) | type alias | `Rc<dyn Fn(FontPick, &mut Window, &mut App)>` | Font flyout choice |
| [`MoveToolbarFn`](#movetoolbarfn) | type alias | `Rc<dyn Fn(Option<(f32, f32)>, bool, &mut Window, &mut App)>` | Persist the toolbar layout |
| [`FontPick`](#fontpick) | enum | `Upload \| Default` | Which face the Font flyout offers |
| [`Template`](#template) | struct | `{ id, name, elements }` | A reusable, stampable element group |
| [`Template::from_json`](#templatefrom_json) | assoc fn | `fn from_json(id: i64, name: impl Into<String>, elements_json: &str) -> Self` | Build from the host's stored row |
| [`WhiteboardView`](#whiteboardview) | struct | `impl Render` | The board entity: canvas + entire editing UI |
| [`WhiteboardView::new`](#whiteboardviewnew) | constructor | `fn new(scene, style, cx) -> Self` | Build a view over a scene |
| [`WhiteboardView::set_on_change`](#whiteboardviewset_on_change) | method | `fn set_on_change(&mut self, f: ChangeFn)` | Install the persistence hook |
| [`WhiteboardView::set_on_place_embed`](#whiteboardviewset_on_place_embed) | method | `fn set_on_place_embed(&mut self, f: PlaceEmbedFn)` | Install the page-card placement hook |
| [`WhiteboardView::set_on_open`](#whiteboardviewset_on_open) | method | `fn set_on_open(&mut self, f: OpenPageFn)` | Install the open-page hook |
| [`WhiteboardView::set_on_save_template`](#whiteboardviewset_on_save_template) | method | `fn set_on_save_template(&mut self, f: SaveTemplateFn)` | Install the save-template hook |
| [`WhiteboardView::set_on_delete_template`](#whiteboardviewset_on_delete_template) | method | `fn set_on_delete_template(&mut self, f: DeleteTemplateFn)` | Install the delete-template hook |
| [`WhiteboardView::set_on_image`](#whiteboardviewset_on_image) | method | `fn set_on_image(&mut self, f: ImageFn)` | Install the image-fetch hook |
| [`WhiteboardView::set_on_place_image`](#whiteboardviewset_on_place_image) | method | `fn set_on_place_image(&mut self, f: PlaceImageFn)` | Install the place-image hook |
| [`WhiteboardView::set_on_drop_files`](#whiteboardviewset_on_drop_files) | method | `fn set_on_drop_files(&mut self, f: DropFilesFn)` | Install the file-drop hook |
| [`WhiteboardView::set_on_copy`](#whiteboardviewset_on_copy) | method | `fn set_on_copy(&mut self, f: CopyFn)` | Install the copy hook |
| [`WhiteboardView::set_on_paste`](#whiteboardviewset_on_paste) | method | `fn set_on_paste(&mut self, f: PasteFn)` | Install the paste hook (menu item hidden without it) |
| [`WhiteboardView::set_on_save_colors`](#whiteboardviewset_on_save_colors) | method | `fn set_on_save_colors(&mut self, f: SavedColorsFn)` | Install the saved-colors hook |
| [`WhiteboardView::set_on_pick_font`](#whiteboardviewset_on_pick_font) | method | `fn set_on_pick_font(&mut self, f: PickFontFn)` | Install the font-picker hook (button hidden without it) |
| [`WhiteboardView::set_on_move_toolbar`](#whiteboardviewset_on_move_toolbar) | method | `fn set_on_move_toolbar(&mut self, f: MoveToolbarFn)` | Install the toolbar-moved hook |
| [`WhiteboardView::set_toolbar_pos`](#whiteboardviewset_toolbar_pos) | method | `fn set_toolbar_pos(&mut self, pos: Option<(f32, f32)>, cx)` | Push the persisted toolbar position |
| [`WhiteboardView::set_toolbar_vertical`](#whiteboardviewset_toolbar_vertical) | method | `fn set_toolbar_vertical(&mut self, vertical: bool, cx)` | Push the persisted toolbar orientation |
| [`WhiteboardView::set_saved_colors`](#whiteboardviewset_saved_colors) | method | `fn set_saved_colors(&mut self, colors: Vec<u32>, cx)` | Push the persisted saved-color palette |
| [`WhiteboardView::set_templates`](#whiteboardviewset_templates) | method | `fn set_templates(&mut self, templates: Vec<Template>, cx)` | Push the stored template list |
| [`WhiteboardView::set_font`](#whiteboardviewset_font) | method | `fn set_font(&mut self, font: Font, cx)` | Swap the text face |
| [`WhiteboardView::add_embed`](#whiteboardviewadd_embed) | method | `fn add_embed(&mut self, page_id, title, x, y, cx)` | Insert a page-card (host, after `PlaceEmbedFn`) |
| [`WhiteboardView::add_image_at`](#whiteboardviewadd_image_at) | method | `fn add_image_at(&mut self, src, px_w, px_h, cx_world, cy_world, cx)` | Insert an image (host, after `PlaceImageFn`/drop) |
| [`WhiteboardView::viewport_center`](#whiteboardviewviewport_center) | method | `fn viewport_center(&self) -> [f32; 2]` | World point at the viewport center |
| [`WhiteboardView::scene`](#whiteboardviewscene) | method | `fn scene(&self) -> &Scene` | Borrow the current board document |
| [`WhiteboardView::tool`](#whiteboardviewtool) | method | `fn tool(&self) -> Tool` | The active tool |
| [`WhiteboardView::set_tool`](#whiteboardviewset_tool) | method | `fn set_tool(&mut self, tool: Tool, cx)` | Switch the active tool |
| [`WhiteboardView::reset_view`](#whiteboardviewreset_view) | method | `fn reset_view(&mut self, cx)` | Back to the origin at 100% |
| [`WhiteboardView::zoom_in`](#whiteboardviewzoom_in--whiteboardviewzoom_out) | method | `fn zoom_in(&mut self, cx)` | Zoom a step about the canvas center |
| [`WhiteboardView::zoom_out`](#whiteboardviewzoom_in--whiteboardviewzoom_out) | method | `fn zoom_out(&mut self, cx)` | Zoom out a step about the canvas center |
| [`WhiteboardView::undo`](#whiteboardviewundo--whiteboardviewredo) | method | `fn undo(&mut self, window, cx)` | Revert the last change |
| [`WhiteboardView::redo`](#whiteboardviewundo--whiteboardviewredo) | method | `fn redo(&mut self, window, cx)` | Re-apply the last undone change |
| [`WhiteboardView::paste_elements`](#whiteboardviewpaste_elements) | method | `fn paste_elements(&mut self, json: &str, window, cx)` | Stamp a serialized selection onto the board |
| `impl Render for WhiteboardView` | trait impl | — | Renders the board + its whole editing UI |
| [`Font`](#font) | struct | `Clone` (cheap) | A TTF/OTF face backing whiteboard text |
| [`Font::from_bytes`](#fontfrom_bytes) | assoc fn | `fn from_bytes(bytes: Vec<u8>, index: u32) -> Option<Self>` | Build from raw face bytes |
| `impl Default for Font` | trait impl | `fn default() -> Self` | The bundled JetBrains Mono face |
| [`Font::measure`](#fontmeasure--fontmeasure_wrapped) | method | `fn measure(&self, content, font_size) -> (f32, f32)` | Text block extent, unwrapped |
| [`Font::measure_wrapped`](#fontmeasure--fontmeasure_wrapped) | method | `fn measure_wrapped(&self, content, font_size, max_width) -> (f32, f32)` | Text block extent with word-wrap |
| [`Font::fit_size`](#fontfit_size) | method | `fn fit_size(&self, content, max_w, max_h, max_size) -> f32` | Largest font size that fits a box |
| [`Font::caret_pos`](#fontcaret_pos--fontcaret_pos_wrapped) | method | `fn caret_pos(&self, content, font_size, at) -> [f32; 2]` | Caret top-left at a byte offset |
| [`Font::caret_pos_wrapped`](#fontcaret_pos--fontcaret_pos_wrapped) | method | `fn caret_pos_wrapped(&self, content, font_size, max_width, at) -> [f32; 2]` | Ditto, honoring a wrap width |
| [`Font::index_at_wrapped`](#fontindex_at_wrapped) | method | `fn index_at_wrapped(&self, content, font_size, max_width, p) -> usize` | Byte offset nearest a local point |
| [`Font::selection_rects`](#fontselection_rects--fontselection_rects_wrapped) | method | `fn selection_rects(&self, content, font_size, start, end) -> Vec<[f32; 4]>` | Selection highlight rects |
| [`Font::selection_rects_wrapped`](#fontselection_rects--fontselection_rects_wrapped) | method | `fn selection_rects_wrapped(&self, content, font_size, max_width, start, end) -> Vec<[f32; 4]>` | Ditto, honoring a wrap width |
| [`Font::layout_wrapped`](#fontlayout_wrapped--fontlayout_styled-crate-internal) | method | `fn layout_wrapped(…) -> TextLayout` | Glyph outlines (return type not exported — see note) |
| [`Font::layout_styled`](#fontlayout_wrapped--fontlayout_styled-crate-internal) | method | `fn layout_styled(…) -> StyledLayout` | Styled glyph outlines (not callable externally — see note) |

`Scene`, `Element`, `ElementKind`, the geometry structs, `Camera`, `RunStyle`,
and `StyleSpan` all derive `Clone`, `Debug`, `Serialize`, and `Deserialize`
(`Scene` and `RunStyle` also `Default`; `Camera` has a hand-written `Default` =
origin at zoom 1). `Tool`, `Format`, and `FontPick` are `Clone + Copy +
PartialEq + Eq + Debug`. Nothing here is `Send`: the view is a gpui entity and
the callbacks are `Rc`s — main-thread only, like all gpui UI ([`Font`](#font)
is the one exception).

---

## `Scene`

```rust
pub struct Scene {
    pub camera: Camera,
    pub elements: Vec<Element>,
}
```

The board document: everything persisted for a whiteboard. The host stores
[`to_json`](#sceneto_json) opaquely (Zorite keeps it in the `content` column of
a `kind = 'whiteboard'` page) and reloads with
[`from_json`](#scenefrom_json).

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `camera` | `Camera` | The viewport (pan + zoom). Persisted so a board reopens where you left it. |
| `elements` | `Vec<Element>` | The board's content, painted in order — later = on top. |

**Serialization** — plain serde JSON. Every field is `#[serde(default)]`, so
older boards keep loading as the model grows (missing fields take their
defaults). `Scene::default()` is an empty board with the default camera.

### `Scene::from_json`

```rust
pub fn from_json(s: &str) -> Self
```

Parse a board from its stored JSON.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `s` | `&str` | The stored scene JSON (what [`to_json`](#sceneto_json) / [`ChangeFn`](#changefn) produced). |

**Returns** — the parsed `Scene`. **Never panics, never errors**: empty or
whitespace-only input yields `Scene::default()`; malformed JSON logs a
`log::warn!` and yields `Scene::default()` — a corrupt row never blocks opening
the board.

**Guarantees & edge cases**

- The camera zoom is sanitized after parsing: a non-finite or `<= 0` zoom
  (e.g. from hand-edited JSON) is reset to `1.0`, so the world↔screen math
  can't divide by zero.
- Unknown element kinds or fields fail the whole parse (serde) → empty board;
  *missing* fields are fine (defaults).

### `Scene::to_json`

```rust
pub fn to_json(&self) -> String
```

Serialize for persistence.

**Parameters** — none.

**Returns** — the scene as a JSON string. Falls back to `"{}"` if
serialization fails (unreachable for well-formed scenes — no fallible types are
serialized).

**Guarantees & edge cases** — `Option` fields that are `None` and empty
`styles` vecs are omitted from the output (`skip_serializing_if`), keeping
stored boards compact and stable.

---

## `Element`

```rust
pub struct Element {
    pub id: u64,
    pub kind: ElementKind,
    pub stroke: Option<u32>,
    pub fill: Option<u32>,
    pub label: Option<String>,
    pub label_color: Option<u32>,
    pub styles: Vec<StyleSpan>,
}
```

One board element: a stable id plus its geometry/kind and appearance.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | `u64` | Stable per-board identity. [`WhiteboardView::new`](#whiteboardviewnew) seeds its id counter from `max(id) + 1`. |
| `kind` | `ElementKind` | What it is + its geometry. |
| `stroke` | `Option<u32>` | Stroke / ink color, packed `0xRRGGBBAA`. `None` follows the theme ink, so uncolored elements adapt to light/dark. |
| `fill` | `Option<u32>` | Fill for closed shapes; `None` = unfilled outline. Ignored by other kinds. |
| `label` | `Option<String>` | Centered text label for closed shapes, word-wrapped and auto-shrunk to the shape's *inscribed* area. `None`/empty = no label; ignored by non-shape kinds. |
| `label_color` | `Option<u32>` | Color of `label`; `None` follows `stroke` (theme ink if that's unset too). |
| `styles` | `Vec<StyleSpan>` | Rich-text runs over the element's text (a Text element's content, or a shape's label). Kept sorted, non-overlapping, non-plain; empty = unstyled. |

**Serialization** — `stroke` / `fill` / `label` / `label_color` are omitted
when `None`, `styles` when empty; all take defaults on load, so pre-feature
boards keep loading.

---

## `ElementKind`

```rust
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Draw(Stroke),
    Rect(BoxGeom),
    Ellipse(BoxGeom),
    Diamond(BoxGeom),
    Triangle(BoxGeom),
    RoundRect(BoxGeom),
    Star(BoxGeom),
    Hexagon(BoxGeom),
    Line(SegGeom),
    Arrow(SegGeom),
    Text(TextGeom),
    Embed(EmbedGeom),
    Image(ImageGeom),
}
```

The kinds of thing a board can hold.

**Variants**

| Variant | Geometry | Meaning |
| --- | --- | --- |
| `Draw` | [`Stroke`](#stroke) | Freehand pen stroke |
| `Rect` / `Ellipse` / `Diamond` / `Triangle` / `RoundRect` / `Star` / `Hexagon` | [`BoxGeom`](#boxgeom) | Closed shapes (fillable, labelable, rotatable) |
| `Line` / `Arrow` | [`SegGeom`](#seggeom) | Straight connectors (arrow has a head at `(x2, y2)`) |
| `Text` | [`TextGeom`](#textgeom) | Free text |
| `Embed` | [`EmbedGeom`](#embedgeom) | Page-card linking to a host page |
| `Image` | [`ImageGeom`](#imagegeom) | Image referencing a host-managed file |

**Serialization** — externally tagged with `snake_case` variant names:
`{"rect": {…}}`, `{"round_rect": {…}}`, `{"image": {…}}`, ….

---

## `Stroke`

```rust
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    pub width: f32,
}
```

A freehand pen stroke.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `points` | `Vec<[f32; 2]>` | World-space polyline points, input-thinned at draw time. |
| `width` | `f32` | Stroke width, **world units** (drawn as `NIB / zoom` so it scales with content). |

---

## `BoxGeom`

```rust
pub struct BoxGeom {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub width: f32,
    pub rotation: f32,
}
```

A box shape (rect / ellipse / diamond / …), world-space. `x, y, w, h` describe
the *unrotated* box; `rotation` spins it about its center at paint time.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `x`, `y` | `f32` | Top-left of the unrotated box, world space. |
| `w`, `h` | `f32` | Extent, world units. |
| `width` | `f32` | Outline stroke width, world units. |
| `rotation` | `f32` | Radians, clockwise, about the box center. `#[serde(default)]` — absent in older boards → 0. |

---

## `SegGeom`

```rust
pub struct SegGeom {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub width: f32,
    pub style: SegmentStyle,               // serde default: Solid
    pub start_anchor: Option<SegmentAnchor>, // serde default: None
    pub end_anchor: Option<SegmentAnchor>,   // serde default: None
}
```

A directed segment (line / arrow), world-space. An arrow's head sits at
`(x2, y2)`. All three newer fields are serde-defaulted, so boards saved before
they existed keep loading.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `x1`, `y1` | `f32` | Start point, world space. |
| `x2`, `y2` | `f32` | End point, world space. |
| `width` | `f32` | Stroke width, world units. |
| `style` | [`SegmentStyle`](#segmentstyle) | Solid (default) or dashed stroke. |
| `start_anchor`, `end_anchor` | `Option<SegmentAnchor>` | When set, the endpoint is BOUND to a shape's connector point and follows it as the shape moves/resizes/rotates; moving the segment itself detaches. Endpoints snap to hovered connector points while drawing. |

---

## `SegmentStyle`

```rust
pub enum SegmentStyle { Solid, Dashed }
```

A line/arrow's stroke rendering. Serde-defaults to `Solid` in older boards.

---

## `SegmentAnchor`

```rust
pub struct SegmentAnchor {
    pub element_id: u64,
    pub connector: usize,
}
```

A segment endpoint's binding: the target element's `id` and the index of one
of its connector points (edge midpoints; see the on-canvas connector dots that
appear when hovering a shape with the line/arrow tool). Dangling ids (element
deleted) detach harmlessly.

---

## `MindMapNodeMeta`

```rust
pub struct MindMapNodeMeta {
    pub parent: Option<u64>,
    pub side: MindMapSide,
    pub order: usize,
    pub root_direction: MindMapRootDirection,
    pub connector_style: MindMapConnectorStyle,
}
```

Mind-map bookkeeping carried by [`Element::mindmap`](#element) (`None` for
ordinary elements; serde-skipped when absent). A `parent` of `None` marks the
tree's ROOT node — `root_direction` and `connector_style` are read from the
root and apply to its whole tree. Child nodes hang on `side` at `order` among
their siblings. The view maintains the parent links, auto-layout, and the
"+" add-node buttons; the tree's connector segments re-sync whenever nodes
move or the tree changes.

---

## Mind-map enums

```rust
pub enum MindMapSide { Left, Right }
pub enum MindMapRootDirection { Both, Left, Right }   // Default: Both
pub enum MindMapConnectorStyle { Straight, Bezier, Orthogonal } // Default: Bezier
```

Selecting a root offers Direction and Connector pickers in the context UI;
`Both` alternates new branches left/right.

---

## Local thumbnails

```rust
pub enum LocalThumbnailMode { /* framing modes */ }
pub struct LocalThumbnailSpec { /* framing + pixel size */ }
pub struct LocalThumbnailSnapshot {
    pub scene: Scene,
    pub camera: Camera,
    pub spec: LocalThumbnailSpec,
}

impl LocalThumbnailSnapshot {
    pub fn for_scene_all_content(scene: Scene, width_px: f32, height_px: f32) -> Self
}
```

A frozen scene plus the framing needed to paint it small. Build one with
`for_scene_all_content` (frames the whole board's content into
`width_px × height_px`) and hand it to a
[`BoardThumbnailView`](#boardthumbnailview). Pure data — cheap to clone/store;
re-snapshot when the scene changes.

---

## `BoardEmbedView`

```rust
pub struct BoardEmbedView { /* entity */ }
pub type ExpandEmbedFn = Rc<dyn Fn(&mut Window, &mut App)>;

impl BoardEmbedView {
    pub fn new(scene: Scene, style: WhiteboardStyleFn, cx: &mut Context<Self>) -> Self
    pub fn board(&self) -> Entity<WhiteboardView>
    pub fn set_on_expand(&mut self, f: ExpandEmbedFn)
}
```

A read-only board embedded inside another surface (a document, a preview
pane): renders the scene with pan/zoom wheel input suppressed and an "Edit"
button overlaid; `set_on_expand` receives the click so the HOST runs its own
maximize / open-editor transition. `board()` exposes the inner
[`WhiteboardView`](#whiteboardview) (e.g. to `set_scene` on refresh).

---

## `BoardThumbnailView`

```rust
pub struct BoardThumbnailView { /* entity */ }

impl BoardThumbnailView {
    pub fn new(snapshot: LocalThumbnailSnapshot, style: WhiteboardStyleFn) -> Self
    pub fn snapshot(&self) -> &LocalThumbnailSnapshot
    pub fn set_snapshot(&mut self, snapshot: LocalThumbnailSnapshot)
}
```

A static, non-interactive rendering of a
[`LocalThumbnailSnapshot`](#local-thumbnails) — for board lists, cards, and
document blocks. No input handling at all; swap the snapshot to refresh.

---

## `TextGeom`

```rust
pub struct TextGeom {
    pub x: f32,
    pub y: f32,
    pub content: String,
    pub size: f32,
    pub rotation: f32,
    pub measured_w: f32,   // #[serde(skip)] — runtime cache
    pub measured_h: f32,   // #[serde(skip)] — runtime cache
}
```

A free-text label: a top-left anchor, its content, and a world-space font size.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `x`, `y` | `f32` | Top-left of the text block, world space. |
| `content` | `String` | The text. Newlines break lines. |
| `size` | `f32` | Font size, **world units** (created as 18 screen px ÷ zoom). |
| `rotation` | `f32` | Radians about the block's center. `#[serde(default)]` → 0 in older boards. |
| `measured_w`, `measured_h` | `f32` | Cached world-space extent, set each render from the font layout so selection/hit-test fit the real glyphs. **`#[serde(skip)]` — not persisted**; 0 means unmeasured (a rough estimate is used until first paint). |

---

## `EmbedGeom`

```rust
pub struct EmbedGeom {
    pub page_id: i64,
    pub title: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
```

A page-card: a titled box that links to a host page. The crate is
page-agnostic — the host supplies the id + title
([`WhiteboardView::add_embed`](#whiteboardviewadd_embed)) and handles opening
it ([`OpenPageFn`](#openpagefn)); the crate just stores and draws the card.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `page_id` | `i64` | Host page identity — opaque to the crate. |
| `title` | `String` | The card's displayed title. |
| `x`, `y`, `w`, `h` | `f32` | World-space box. |

---

## `ImageGeom`

```rust
pub struct ImageGeom {
    pub src: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rotation: f32,
}
```

An image referencing a host-managed file. The crate is storage-agnostic: the
host imports the file and serves the decoded bitmap via [`ImageFn`](#imagefn);
the scene stores only the reference + geometry.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `src` | `String` | Host file reference (e.g. `images/<name>`) — opaque to the crate. |
| `x`, `y`, `w`, `h` | `f32` | World-space box. |
| `rotation` | `f32` | Radians about the image center. `#[serde(default)]` → 0. The host re-rotates the bitmap to match (see [`ImageFn`](#imagefn)). |

---

## `Camera`

```rust
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}
```

The viewport: a world-space pan offset and a zoom factor. `(x, y)` is the world
point that maps to the canvas's top-left corner, so a canvas-relative screen
point `s` is the world point `offset + s / zoom` (and world → screen is
`(p - offset) * zoom`).

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `x`, `y` | `f32` | World point at the canvas top-left. `#[serde(default)]` → 0. |
| `zoom` | `f32` | Screen px per world unit. Serde default `1.0`; interaction clamps it to `[0.1, 8.0]`. |

**Guarantees & edge cases**

- `Camera::default()` = origin at 100% (`x: 0, y: 0, zoom: 1`).
- [`Scene::from_json`](#scenefrom_json) resets a non-finite or non-positive
  zoom to `1.0`; the internal math additionally floors the divisor at `0.1`,
  so hand-edited JSON can't produce NaN geometry.

---

## `StyleSpan`

```rust
pub struct StyleSpan {
    pub start: usize,
    pub end: usize,
    pub style: RunStyle,
}
```

A [`RunStyle`](#runstyle) over the byte range `[start, end)` of an element's
text (a Text element's `content` or a shape's `label`). See
[`Element::styles`](#element).

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `start`, `end` | `usize` | UTF-8 byte offsets, `[start, end)`. |
| `style` | `RunStyle` | The formatting of that run. |

**Guarantees & edge cases** — the view keeps an element's spans sorted,
non-overlapping, and non-plain (a plain run is an implicit gap), and re-aligns
them across every text edit. If you construct scenes by hand, keep the same
invariants.

---

## `RunStyle`

```rust
pub struct RunStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub highlight: Option<u32>,
}
```

The formatting of a run of characters; `RunStyle::default()` is plain text.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `bold` | `bool` | Synthetic bold (a doubled, offset outline pass — works with any face). |
| `italic` | `bool` | Synthetic oblique (~12° shear). |
| `underline` | `bool` | Underline decoration. |
| `strike` | `bool` | Strikethrough decoration. |
| `highlight` | `Option<u32>` | Highlight color behind the glyphs, packed `0xRRGGBBAA`; `None` = none. |

**Serialization** — every flag is omitted when unset (`false` / `None`), so
stored runs are minimal.

---

## `Format`

```rust
pub enum Format {
    Bold,
    Italic,
    Underline,
    Strike,
}
```

A toggleable boolean format — the four on/off axes of a [`RunStyle`](#runstyle).
(Highlight is a color, toggled on its own.) Exposed as part of the model
vocabulary; the view's keyboard/menu formatting drives it internally — hosts
normally never construct one.

---

## `Tool`

```rust
pub enum Tool {
    Pan, Select, Pen,
    Rect, Ellipse, Diamond, Triangle, RoundRect, Star, Hexagon,
    Line, Arrow, Text, Embed, Image,
}
```

The active tool. UI state — not part of the persisted scene. `Pan` is the
default (navigation before drawing). The view renders its own toolbar for
these and handles their single-key shortcuts (`H V P R O D G U S X L A T I`;
`Embed` has no key) — use
[`set_tool`](#whiteboardviewset_tool)/[`tool`](#whiteboardviewtool) only to
drive tools from your own chrome.

---

## `WhiteboardStyle`

```rust
pub struct WhiteboardStyle {
    pub bg: Hsla,
    pub grid: Hsla,
    pub text: Hsla,
    pub ink: Hsla,
    pub panel: Hsla,
    pub panel_strong: Hsla,
    pub accent: Hsla,
    pub selection: Hsla,
    pub swatches: Vec<Hsla>,
}
```

Theme colors, read at paint time (via [`WhiteboardStyleFn`](#whiteboardstylefn))
so the board follows live theme changes with no push from the host.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `bg` | `Hsla` | Canvas background. |
| `grid` | `Hsla` | Background grid dots. |
| `text` | `Hsla` | HUD / muted on-canvas text (zoom %, placeholders). |
| `ink` | `Hsla` | Default stroke/shape color; per-element colors override it. |
| `panel` | `Hsla` | Toolbar / flyout pill background — small pills, can be glassy. |
| `panel_strong` | `Hsla` | The larger color-picker / menu surface — keep much more opaque than `panel` so it reads over a busy canvas. |
| `accent` | `Hsla` | Active-tool highlight (subtle fill behind the current tool button). |
| `selection` | `Hsla` | Selection outline — use a strong, clearly visible color. |
| `swatches` | `Vec<Hsla>` | Quick swatches in the color picker (typically the host's theme palette). |

---

## `WhiteboardStyleFn`

```rust
pub type WhiteboardStyleFn = Rc<dyn Fn() -> WhiteboardStyle>;
```

The host's theme closure, passed to [`WhiteboardView::new`](#whiteboardviewnew).

**Fires** — every paint (and when tooltips/panels are built).

**The host must** — return the current theme mapped to a
[`WhiteboardStyle`](#whiteboardstyle). Return fresh values each call; the board
never caches them, which is what makes live light/dark switching free.

**Cost** — called at paint frequency: keep it a cheap struct build, not a
theme recomputation.

---

## `ChangeFn`

```rust
pub type ChangeFn = Rc<dyn Fn(String, &mut Window, &mut App)>;
```

The persistence hook — install with
[`set_on_change`](#whiteboardviewset_on_change).

**Fires** — when the board changes: an element committed / moved / resized /
deleted, text edited, z-order changed, undo/redo, paste, template stamp, or the
camera moved. Changes made during a drag are batched (a `dirty` flag) and
flushed on mouse-up, so you get one call per gesture, not per mouse-move.

**Arguments** — the full scene serialized as JSON
([`Scene::to_json`](#sceneto_json)).

**The host must** — store the string wherever this board lives. Nothing else;
don't parse it.

**Guarantees & edge cases** —
[`add_embed`](#whiteboardviewadd_embed) / [`add_image_at`](#whiteboardviewadd_image_at)
deliberately do **not** fire it (they run mid-host-update; a re-entrant save
would panic) — persist explicitly via [`scene`](#whiteboardviewscene) after
calling them.

---

## `PlaceEmbedFn`

```rust
pub type PlaceEmbedFn = Rc<dyn Fn(f32, f32, &mut Window, &mut App)>;
```

Install with [`set_on_place_embed`](#whiteboardviewset_on_place_embed).

**Fires** — when the page-card tool is clicked on the canvas, with the world
`(x, y)` of the click.

**The host must** — show its page picker, then call
[`WhiteboardView::add_embed`](#whiteboardviewadd_embed)`(page_id, title, x, y, cx)`
with the chosen page — and persist via [`scene`](#whiteboardviewscene)
afterward (see [`ChangeFn`](#changefn)).

---

## `OpenPageFn`

```rust
pub type OpenPageFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;
```

Install with [`set_on_open`](#whiteboardviewset_on_open).

**Fires** — when a page-card is double-clicked, with its `page_id`.

**The host must** — open that page (e.g. in a tab). Without the hook,
double-clicking a card does nothing.

---

## `ImageFn`

```rust
pub type ImageFn = Rc<dyn Fn(&str, f32, &mut Window, &mut App) -> Option<gpui::ImageSource>>;
```

Install with [`set_on_image`](#whiteboardviewset_on_image).

**Fires** — **each render, once per image element**, with the element's `src`
and its rotation in radians (0 = upright).

**The host must** — return the decoded bitmap for `src`, pre-rotated by the
given angle, from its own cache. Return `None` until the decode is ready (the
board shows a placeholder), then notify/re-render when it lands. Decode and
rotate **off-thread and cache by `(src, angle)`** — this is a per-paint call,
so a cache miss must not block the UI thread. A steady angle hits the cache;
re-rotation only happens when the angle changes.

**Guarantees & edge cases** — without the hook, image elements render as
placeholders; the scene still stores them.

---

## `PlaceImageFn`

```rust
pub type PlaceImageFn = Rc<dyn Fn(f32, f32, &mut Window, &mut App)>;
```

Install with [`set_on_place_image`](#whiteboardviewset_on_place_image).

**Fires** — when the image tool is clicked on the canvas, with the world
`(x, y)`.

**The host must** — show a file picker, import the chosen image into its own
store, then call
[`WhiteboardView::add_image_at`](#whiteboardviewadd_image_at) — and persist
via [`scene`](#whiteboardviewscene) afterward.

---

## `DropFilesFn`

```rust
pub type DropFilesFn = Rc<dyn Fn(Vec<std::path::PathBuf>, f32, f32, &mut Window, &mut App)>;
```

Install with [`set_on_drop_files`](#whiteboardviewset_on_drop_files).

**Fires** — when files are dropped onto the canvas, with the paths and the
world `(x, y)` of the drop point.

**The host must** — import any image files and place them via
[`add_image_at`](#whiteboardviewadd_image_at) (ignore non-images), then
persist via [`scene`](#whiteboardviewscene).

---

## `CopyFn`

```rust
pub type CopyFn = Rc<dyn Fn(String, &mut Window, &mut App)>;
```

Install with [`set_on_copy`](#whiteboardviewset_on_copy).

**Fires** — on ⌘C / ⌘X with a non-empty selection (⌘X then deletes), with the
selection serialized as a `Vec<Element>` JSON string, translated so the group's
bounding box starts at the origin — the **same portable format** as
[`SaveTemplateFn`](#savetemplatefn) and
[`paste_elements`](#whiteboardviewpaste_elements), so a copy pastes onto any
board.

**The host must** — write the string to the system clipboard (the crate never
touches the clipboard itself).

---

## `PasteFn`

```rust
pub type PasteFn = Rc<dyn Fn(&mut Window, &mut App) -> Option<String>>;
```

Install with [`set_on_paste`](#whiteboardviewset_on_paste).

**Fires** — on ⌘V, and when the context-menu **Paste** item is clicked.

**The host must** — read the system clipboard and return previously copied
whiteboard elements (the JSON a [`CopyFn`](#copyfn) wrote), or `None` if the
clipboard holds no board elements. The view passes the returned JSON to
[`paste_elements`](#whiteboardviewpaste_elements).

**Guarantees & edge cases**

- Returning `None` on ⌘V lets the keystroke propagate, so the host can handle
  a clipboard *image* instead (elements are preferred over images).
- Without the hook, the Paste menu item is **hidden** and ⌘V always
  propagates.

---

## `SaveTemplateFn`

```rust
pub type SaveTemplateFn = Rc<dyn Fn(String, &mut Window, &mut App)>;
```

Install with [`set_on_save_template`](#whiteboardviewset_on_save_template).

**Fires** — when the user saves the current selection as a template
(right-click → save), with the selected elements serialized and
origin-normalized (same format as [`CopyFn`](#copyfn)).

**The host must** — prompt for a name, store `(id, name, json)` in its own
store, then push the updated list back via
[`set_templates`](#whiteboardviewset_templates) (rebuild each row with
[`Template::from_json`](#templatefrom_json)).

---

## `DeleteTemplateFn`

```rust
pub type DeleteTemplateFn = Rc<dyn Fn(i64, &mut Window, &mut App)>;
```

Install with [`set_on_delete_template`](#whiteboardviewset_on_delete_template).

**Fires** — when a template card is right-clicked → delete, with the host id
the template was built with.

**The host must** — confirm if desired, remove the stored row, and push the
updated list back via [`set_templates`](#whiteboardviewset_templates).

---

## `SavedColorsFn`

```rust
pub type SavedColorsFn = Rc<dyn Fn(Vec<u32>, &mut Window, &mut App)>;
```

Install with [`set_on_save_colors`](#whiteboardviewset_on_save_colors).

**Fires** — when the user's saved-color palette changes (a swatch added via
the picker's `+`, or removed via right-click), with the **full** updated list
(packed `0xRRGGBBAA`).

**The host must** — persist the list and feed it back via
[`set_saved_colors`](#whiteboardviewset_saved_colors) on board open. Without
the hook, the palette still works but is per-session.

---

## `PickFontFn`

```rust
pub type PickFontFn = Rc<dyn Fn(FontPick, &mut Window, &mut App)>;
```

Install with [`set_on_pick_font`](#whiteboardviewset_on_pick_font).

**Fires** — when the user picks from the toolbar's **Aa** Font flyout:
[`FontPick::Upload`](#fontpick) (*Upload font…*) or `FontPick::Default`
(*Use default*).

**The host must** — for `Upload`, show a file dialog, build the face with
[`Font::from_bytes`](#fontfrom_bytes), and call
[`set_font`](#whiteboardviewset_font); for `Default`, call
`set_font(Font::default(), cx)`. Persist the per-board choice and restore it
on reopen (the crate doesn't store the font in the scene).

**Guarantees & edge cases** — without the hook, the Font toolbar button is
**hidden**.

---

## `MoveToolbarFn`

```rust
pub type MoveToolbarFn = Rc<dyn Fn(Option<(f32, f32)>, bool, &mut Window, &mut App)>;
```

Install with [`set_on_move_toolbar`](#whiteboardviewset_on_move_toolbar).

**Fires** — when the toolbar is dragged to a new spot, reset (double-click its
grip), or flipped row ↔ column (`R` mid-drag). Arguments: the new
board-relative top-left (`None` = the default top-center) and whether the bar
is vertical.

**The host must** — persist both values and feed them back via
[`set_toolbar_pos`](#whiteboardviewset_toolbar_pos) /
[`set_toolbar_vertical`](#whiteboardviewset_toolbar_vertical) on open. Without
the hook, the layout is per-session.

---

## `FontPick`

```rust
pub enum FontPick {
    Upload,
    Default,
}
```

Which face the Font flyout offers — handed to the host's
[`PickFontFn`](#pickfontfn).

| Variant | Meaning |
| --- | --- |
| `Upload` | Pick a `.ttf` / `.otf` from disk (the host shows the file dialog). |
| `Default` | Revert to the bundled default face (JetBrains Mono). |

---

## `Template`

```rust
pub struct Template {
    pub id: i64,
    pub name: String,
    pub elements: Vec<Element>,
}
```

A reusable group of elements the user can stamp onto a board (shown as preview
cards in the Pages & Images flyout's gallery). Element positions are normalized
so the group's bounding box starts at the origin; applying re-bases them
centered in the viewport, with fresh ids. **The host owns persistence and the
`id`**; the crate renders the preview and instantiates on click.

**Fields**

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | `i64` | Host storage identity — handed back verbatim by [`DeleteTemplateFn`](#deletetemplatefn). |
| `name` | `String` | Shown on the gallery card. |
| `elements` | `Vec<Element>` | Origin-normalized element group. |

### `Template::from_json`

```rust
pub fn from_json(id: i64, name: impl Into<String>, elements_json: &str) -> Self
```

Build a template from the host's stored row.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `id` | `i64` | Your storage id. |
| `name` | `impl Into<String>` | The display name. |
| `elements_json` | `&str` | A serialized `Vec<Element>` — the JSON a [`SaveTemplateFn`](#savetemplatefn) handed you. |

**Returns** — the `Template`. **Never panics**: malformed JSON yields an empty
(still-listable) template rather than an error.

---

## `WhiteboardView`

```rust
pub struct WhiteboardView { /* private */ }

impl Render for WhiteboardView { /* the board + its entire editing UI */ }
```

The whiteboard entity. It owns the scene, the active tool, selection,
in-progress edits, undo/redo history, and renders the whole editing UI —
toolbar, tool flyouts, color picker, thickness/font flyouts, templates gallery,
and right-click context menu. The host holds it in an `Entity<WhiteboardView>`
(keyed by board id) and renders it into a tab/panel like any entity. All
methods run on the UI thread inside entity updates (they take
`&mut Context<Self>`); methods taking `cx` call `cx.notify()` themselves.

With **no** callbacks installed it is still a working board (draw, select,
style, zoom, undo/redo; text editing even uses the system clipboard directly
via gpui) — the hooks add persistence, pages, images, element copy/paste
through the system clipboard, templates, fonts, and toolbar-layout memory.

### `WhiteboardView::new`

```rust
pub fn new(scene: Scene, style: WhiteboardStyleFn, cx: &mut Context<Self>) -> Self
```

Build a view over `scene`. Call inside `cx.new(|cx| WhiteboardView::new(..))`.

### Read-only mode

```rust
pub fn new_read_only(scene: Scene, style: WhiteboardStyleFn, cx: &mut Context<Self>) -> Self
pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>)
pub fn read_only(&self) -> bool
```

A read-only board renders without the toolbar or any editing affordances and
ignores editing input (wheel pan/zoom too — it sits quietly inside another
scroll surface). Used by [`BoardEmbedView`](#boardembedview); also togglable
live on any view.

### Mind-map & flowchart seeds

```rust
pub fn add_mindmap_seed(&mut self, center_x: f32, center_y: f32, cx: &mut Context<Self>)
pub fn add_mindmap_seed_at_viewport_center(&mut self, cx: &mut Context<Self>)
pub fn add_flowchart_seed(&mut self, center_x: f32, center_y: f32, cx: &mut Context<Self>)
pub fn add_flowchart_seed_at_viewport_center(&mut self, cx: &mut Context<Self>)
```

Insert a starter mind-map (a root + four labeled branches, with
[`MindMapNodeMeta`](#mindmapnodemeta) wired and bound connectors) or a starter
flowchart (Start → Process → Decision with Yes/No branches → End) centered at
the given world point / the current viewport center. Both are also on the
toolbar's shapes flyout; one undo step each. Mind-map trees keep their layout
and connectors in sync as nodes are added (the selection's "+" buttons),
moved, or deleted; a selected root offers Direction and Connector pickers.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `scene` | `Scene` | The board to edit — `Scene::default()` or [`Scene::from_json`](#scenefrom_json) of a stored board. Taken by value; the view owns it. |
| `style` | `WhiteboardStyleFn` | Theme closure, read each paint. |
| `cx` | `&mut Context<Self>` | Entity context (used to create the focus handle). |

**Returns** — the view. Infallible.

**Guarantees & edge cases**

- The internal id counter seeds from `max(element ids) + 1` (0 for an empty
  board), so new elements never collide with loaded ones.
- Initial state: tool `Pan`, nothing selected, no hooks installed, empty
  saved-colors and templates, default toolbar layout (top-center, horizontal),
  bundled font, empty undo history.

**Example**

```rust
let board = cx.new(|cx| {
    let mut v = WhiteboardView::new(Scene::from_json(&stored), style_fn.clone(), cx);
    v.set_on_change(Rc::new(|json, _w, _cx| store(json)));
    v
});
```

### Hook installers

Thirteen setters, one per [callback type](#changefn); each simply stores the
closure (no notify — the hook takes effect from the next event). All optional;
install after `new`, typically inside the same `cx.new` closure.

#### `WhiteboardView::set_on_change`

```rust
pub fn set_on_change(&mut self, f: ChangeFn)
```

Install the persistence hook. See [`ChangeFn`](#changefn) for when it fires
and the batching contract.

#### `WhiteboardView::set_on_place_embed`

```rust
pub fn set_on_place_embed(&mut self, f: PlaceEmbedFn)
```

Install the page-card placement hook (page-card tool click). See
[`PlaceEmbedFn`](#placeembedfn).

#### `WhiteboardView::set_on_open`

```rust
pub fn set_on_open(&mut self, f: OpenPageFn)
```

Install the open-page hook (double-click a card). See
[`OpenPageFn`](#openpagefn).

#### `WhiteboardView::set_on_save_template`

```rust
pub fn set_on_save_template(&mut self, f: SaveTemplateFn)
```

Install the save-template hook (right-click selection → save). See
[`SaveTemplateFn`](#savetemplatefn).

#### `WhiteboardView::set_on_delete_template`

```rust
pub fn set_on_delete_template(&mut self, f: DeleteTemplateFn)
```

Install the delete-template hook (right-click a template card → delete). See
[`DeleteTemplateFn`](#deletetemplatefn).

#### `WhiteboardView::set_on_image`

```rust
pub fn set_on_image(&mut self, f: ImageFn)
```

Install the image-fetch hook (decoded bitmap for an element's `src`, called
per paint). See [`ImageFn`](#imagefn) — the caching contract matters.

#### `WhiteboardView::set_on_place_image`

```rust
pub fn set_on_place_image(&mut self, f: PlaceImageFn)
```

Install the place-image hook (image tool click → host file picker). See
[`PlaceImageFn`](#placeimagefn).

#### `WhiteboardView::set_on_drop_files`

```rust
pub fn set_on_drop_files(&mut self, f: DropFilesFn)
```

Install the file-drop hook (files dropped on the canvas). See
[`DropFilesFn`](#dropfilesfn).

#### `WhiteboardView::set_on_copy`

```rust
pub fn set_on_copy(&mut self, f: CopyFn)
```

Install the copy hook (⌘C / ⌘X → write the selection to the system
clipboard). See [`CopyFn`](#copyfn).

#### `WhiteboardView::set_on_paste`

```rust
pub fn set_on_paste(&mut self, f: PasteFn)
```

Install the paste hook (context-menu **Paste** → read board elements from the
clipboard). **Without it, the Paste menu item is hidden.** See
[`PasteFn`](#pastefn).

#### `WhiteboardView::set_on_save_colors`

```rust
pub fn set_on_save_colors(&mut self, f: SavedColorsFn)
```

Install the saved-colors hook (the palette changed → host persists it). See
[`SavedColorsFn`](#savedcolorsfn).

#### `WhiteboardView::set_on_pick_font`

```rust
pub fn set_on_pick_font(&mut self, f: PickFontFn)
```

Install the font-picker hook. **Without it, the Font (Aa) toolbar button is
hidden.** See [`PickFontFn`](#pickfontfn).

#### `WhiteboardView::set_on_move_toolbar`

```rust
pub fn set_on_move_toolbar(&mut self, f: MoveToolbarFn)
```

Install the toolbar-moved hook (the host persists position + orientation).
See [`MoveToolbarFn`](#movetoolbarfn).

### Host-pushed state

The counterparts of the persistence hooks: the host pushes stored values on
open and after each change. Each replaces the current value and calls
`cx.notify()`.

#### `WhiteboardView::set_toolbar_pos`

```rust
pub fn set_toolbar_pos(&mut self, pos: Option<(f32, f32)>, cx: &mut Context<Self>)
```

Set the toolbar's board-relative top-left; `None` restores the default
top-center. Feed back what a [`MoveToolbarFn`](#movetoolbarfn) reported.

#### `WhiteboardView::set_toolbar_vertical`

```rust
pub fn set_toolbar_vertical(&mut self, vertical: bool, cx: &mut Context<Self>)
```

Set the toolbar orientation (`true` = a vertical column).

#### `WhiteboardView::set_saved_colors`

```rust
pub fn set_saved_colors(&mut self, colors: Vec<u32>, cx: &mut Context<Self>)
```

Replace the user's saved-color palette (packed `0xRRGGBBAA`), shown in the
color picker. Feed back what a [`SavedColorsFn`](#savedcolorsfn) reported.

#### `WhiteboardView::set_templates`

```rust
pub fn set_templates(&mut self, templates: Vec<Template>, cx: &mut Context<Self>)
```

Replace the stored templates shown in the Pages & Images flyout / gallery.
Call on open and after any save/delete round-trip.

#### `WhiteboardView::set_font`

```rust
pub fn set_font(&mut self, font: Font, cx: &mut Context<Self>)
```

Swap the face used to render all board text (see [`Font`](#font)). Takes
effect next paint; existing elements re-render in the new face (layout is
per-paint, nothing is baked).

### Inserting elements from the host

Both insert, select the new element, and switch to the Select tool — and
**neither fires [`ChangeFn`](#changefn)**: they're called from inside a host
update (re-entrant persistence would panic), so persist explicitly with
[`scene`](#whiteboardviewscene)`.to_json()` afterward. Both push an undo step.

#### `WhiteboardView::add_embed`

```rust
pub fn add_embed(
    &mut self,
    page_id: i64,
    title: impl Into<String>,
    x: f32,
    y: f32,
    cx: &mut Context<Self>,
)
```

Insert a page-card at world `(x, y)` — the coordinates a
[`PlaceEmbedFn`](#placeembedfn) handed you.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `page_id` | `i64` | Your page identity (round-trips through [`OpenPageFn`](#openpagefn)). |
| `title` | `impl Into<String>` | Card title. |
| `x`, `y` | `f32` | World-space top-left. |
| `cx` | `&mut Context<Self>` | Entity context. |

**Guarantees & edge cases** — the card is created at 210 × 76 *screen* px
(divided by the current zoom), so it appears the same size regardless of zoom
level.

#### `WhiteboardView::add_image_at`

```rust
pub fn add_image_at(
    &mut self,
    src: impl Into<String>,
    px_w: f32,
    px_h: f32,
    cx_world: f32,
    cy_world: f32,
    cx: &mut Context<Self>,
)
```

Insert an image element referencing `src`, **centered** at world
`(cx_world, cy_world)`.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `src` | `impl Into<String>` | Your file reference (what [`ImageFn`](#imagefn) will be asked for). |
| `px_w`, `px_h` | `f32` | The image's pixel dimensions — used only for the aspect ratio / default size. |
| `cx_world`, `cy_world` | `f32` | World-space center (e.g. the drop point, or [`viewport_center`](#whiteboardviewviewport_center) for a paste). |
| `cx` | `&mut Context<Self>` | Entity context. |

**Guarantees & edge cases** — sized so the longest edge is 280 *screen* px at
the current zoom, aspect preserved; degenerate pixel dimensions are floored at
1 so the math can't divide by zero. Rotation starts at 0.

### Reading state

#### `WhiteboardView::viewport_center`

```rust
pub fn viewport_center(&self) -> [f32; 2]
```

The world point at the center of the current viewport — where a host-initiated
paste should drop an image (the host has no other access to the camera).
Returns `[x, y]` world coordinates. Before first paint the canvas bounds are
zero, so this degenerates to the camera offset.

#### `WhiteboardView::scene`

```rust
pub fn scene(&self) -> &Scene
```

Borrow the current board document — e.g. `view.scene().to_json()` to persist
after an [`add_embed`](#whiteboardviewadd_embed) /
[`add_image_at`](#whiteboardviewadd_image_at).

#### `WhiteboardView::tool`

```rust
pub fn tool(&self) -> Tool
```

The active tool (for host-driven chrome).

### Imperative controls

Most boards never need these — the built-in toolbar and shortcuts drive them —
but they exist for custom chrome.

#### `WhiteboardView::set_tool`

```rust
pub fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>)
```

Switch the active drawing tool. Switching to anything other than
`Tool::Select` clears the selection; always closes an open tool flyout.

#### `WhiteboardView::reset_view`

```rust
pub fn reset_view(&mut self, cx: &mut Context<Self>)
```

Reset the viewport to the origin at 100% (`Camera::default()`). Also bound to
double-click on empty canvas. The camera change is marked dirty and persisted
at the next flush (mouse-up on the canvas).

#### `WhiteboardView::zoom_in` / `WhiteboardView::zoom_out`

```rust
pub fn zoom_in(&mut self, cx: &mut Context<Self>)
pub fn zoom_out(&mut self, cx: &mut Context<Self>)
```

Zoom by a 1.2× step (in / out), keeping the world point at the canvas center
fixed. Clamped to zoom `[0.1, 8.0]`; a step at the clamp edge is a no-op.
Persisted at the next flush, like `reset_view`.

#### `WhiteboardView::undo` / `WhiteboardView::redo`

```rust
pub fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>)
pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>)
```

Step the history (`⌘Z` / `⌘⇧Z` do this already). Each history step is a full
scene snapshot; the stack keeps at most 50. A step clears the selection and
**fires [`ChangeFn`](#changefn) immediately**. Empty stack → no-op. Any new
mutation clears the redo stack.

#### `WhiteboardView::paste_elements`

```rust
pub fn paste_elements(&mut self, json: &str, window: &mut Window, cx: &mut Context<Self>)
```

Stamp a serialized `Vec<Element>` (the origin-normalized JSON a
[`CopyFn`](#copyfn) wrote) onto the board: centered in the viewport, with
fresh ids, selected, tool switched to Select. Call this from your
[`PasteFn`](#pastefn) flow, or directly when the system clipboard holds board
elements.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `json` | `&str` | A serialized `Vec<Element>` (from [`CopyFn`](#copyfn) / [`SaveTemplateFn`](#savetemplatefn)). |
| `window`, `cx` | — | Window + entity context. |

**Guarantees & edge cases**

- Invalid JSON is silently ignored (no-op); an empty element list is a no-op.
- Unlike `add_embed`/`add_image_at`, this **does** push undo *and* fire
  [`ChangeFn`](#changefn) — it's driven by user gestures, not mid-host-update.

---

## `Font`

```rust
pub struct Font { /* Arc<Vec<u8>> + face index */ }

impl Default for Font { /* the bundled JetBrains Mono Regular (OFL) */ }
```

A font backing whiteboard text. Board text is rendered as **vector outlines**
(via `ttf-parser`), not gpui glyph sprites, so it rotates/scales with the
camera; a `Font` is just the raw face bytes, parsed on demand. `Clone` is an
`Arc` bump — cheap. Unlike everything else in this crate, `Font` holds no
`Rc`/gpui state and its methods are pure — safe to use from any thread.

All coordinates and sizes in the methods below are **unit-agnostic** (the
whiteboard passes world units): text-local space with the origin at the
block's top-left, x right, y **down**. `content` newlines break lines; byte
offsets are UTF-8 offsets into `content`.

**Cost** — every method re-parses the face from bytes
(`ttf_parser::Face::parse`) per call. That's a header parse, not a decode —
fine at UI frequency, but don't call in a tight per-glyph loop.

### `Font::from_bytes`

```rust
pub fn from_bytes(bytes: Vec<u8>, index: u32) -> Option<Self>
```

Build a font from raw face bytes (e.g. a user-uploaded `.ttf`/`.otf`).

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `bytes` | `Vec<u8>` | The font file's contents. |
| `index` | `u32` | Face index within a collection (`.ttc`); pass 0 otherwise. |

**Returns** — `Some(Font)` if the bytes parse as a valid face at `index`,
`None` otherwise (never panics). Validation happens here, so the layout
methods can assume a parseable face (and still degrade gracefully if not).

### `Font::measure` / `Font::measure_wrapped`

```rust
pub fn measure(&self, content: &str, font_size: f32) -> (f32, f32)
pub fn measure_wrapped(&self, content: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32)
```

The block's `(width, height)` at `font_size` without building outlines — for
selection bounds and hit-testing (no `Window` needed). `measure` is
`measure_wrapped(…, None)`; with `Some(max_width)` the text word-wraps and the
height reflects the wrapped line count.

**Returns** — `(width, height)`: width of the widest line, and
`line_count.max(1) × line_height` — so `""` still measures one line high.

### `Font::fit_size`

```rust
pub fn fit_size(&self, content: &str, max_w: f32, max_h: f32, max_size: f32) -> f32
```

The largest font size in `(0, max_size]` at which `content`, wrapped to
`max_w`, fits within a `max_w × max_h` box — how shape labels auto-shrink.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `content` | `&str` | The label text. |
| `max_w`, `max_h` | `f32` | The box to fit (world units in board use). |
| `max_size` | `f32` | The preferred (starting) font size. |

**Returns** — the fitting size. Returns `max_size` unchanged for empty
content or a non-positive box; never returns below `1.0` world unit (a tiny
box may then still overflow slightly).

**Cost** — early-out if `max_size` already fits; otherwise a 20-iteration
bisection, each step a `measure_wrapped`.

### `Font::caret_pos` / `Font::caret_pos_wrapped`

```rust
pub fn caret_pos(&self, content: &str, font_size: f32, at: usize) -> [f32; 2]
pub fn caret_pos_wrapped(&self, content: &str, font_size: f32, max_width: Option<f32>, at: usize) -> [f32; 2]
```

Text-local top-left of the caret at content byte offset `at` (the `_wrapped`
variant honors a wrap width — label editing). Out-of-range or non-boundary
offsets clamp to the end of the text. Caret stops come from the same layout
pass as the glyphs, so the caret always lands exactly between rendered
characters.

### `Font::index_at_wrapped`

```rust
pub fn index_at_wrapped(&self, content: &str, font_size: f32, max_width: Option<f32>, p: [f32; 2]) -> usize
```

The content byte offset whose caret sits nearest the text-local point `p` —
how a click lands the caret between letters. Picks the line by `p[1]`
(clamped to the first/last line), then the closest caret stop by `p[0]`.
Returns 0 for empty layouts. Always a valid char boundary of `content`.

### `Font::selection_rects` / `Font::selection_rects_wrapped`

```rust
pub fn selection_rects(&self, content: &str, font_size: f32, start: usize, end: usize) -> Vec<[f32; 4]>
pub fn selection_rects_wrapped(&self, content: &str, font_size: f32, max_width: Option<f32>, start: usize, end: usize) -> Vec<[f32; 4]>
```

Text-local highlight rectangles `[x, y, w, h]` for the selection
`[start, end)` (byte offsets) — one rect per line the selection touches.
`start >= end` returns an empty vec. A line whose trailing newline is inside
the selection gets a small stub (0.3 × font size) so selected newlines and
empty lines still read.

### `Font::layout_wrapped` / `Font::layout_styled` (crate-internal)

```rust
pub fn layout_wrapped(&self, content: &str, font_size: f32, max_width: Option<f32>) -> TextLayout
pub fn layout_styled(&self, content: &str, font_size: f32, max_width: Option<f32>,
                     style_at: impl Fn(usize) -> GlyphStyle) -> StyledLayout
```

The outline-producing layout passes the whiteboard's renderer feeds to gpui's
`PathBuilder`. They are `pub`, **but their types (`TextLayout`,
`StyledLayout`, `GlyphStyle`, `Seg`, `Decoration`, `DecoKind`) live in the
crate's private `font` module and are not re-exported** — so outside the crate
`layout_wrapped`'s result can't be named (usable only via inference), and
`layout_styled` can't be called at all (you can't produce a `GlyphStyle`).
Treat both as internal; the supported external surface is the
measure/caret/selection methods above. `layout_styled` bakes synthetic italic
(shear) and bold (doubled, offset pass) into the outlines and emits
underline/strike/highlight runs as decorations; wrapping and caret geometry
are identical to the unstyled path.
