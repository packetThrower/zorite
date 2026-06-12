# Whiteboard architecture

> Status: **design** — not yet implemented. Pairs with the planned-feature note
> in [TODO.md](../TODO.md). This doc captures the research so the feasibility
> work doesn't have to be re-derived, and so decisions A and B (below) can be
> settled in writing before any code lands.

A whiteboard is a freeform spatial canvas (Logseq-style): an infinite,
pannable/zoomable board you can sketch on (freehand strokes, shapes, arrows,
text) and onto which you can drop page/note cards that link back into the
graph. It's a distinct surface from the text journal, opened in a tab like a
page or PDF.

It ships as a reusable, host-agnostic **`gpui-whiteboard`** crate — a third
sibling to [`gpui-markdown`](../crates/gpui-markdown) and
[`gpui-pdf`](../crates/gpui-pdf).

---

## Feasibility verdict

**No blockers.** GPUI's `canvas()` escape hatch hands you a `&mut Window` with
`paint_quad`, `paint_path`, `paint_image`, `paint_svg`, `paint_glyph`. The
make-or-break question was vector paths, and `gpui::PathBuilder` (lyon-backed)
supports **arbitrary filled paths and arbitrary-width stroked polylines with
joins, caps, miter, and dash arrays** — that covers freehand pen strokes,
geometric shapes, and arrow shafts, all antialiased.

Two working references to crib from:

- **`gpui-component/crates/story/examples/brush.rs`** — variable-width freehand
  strokes via `PathBuilder::stroke(px(size))`, mouse down/move/up, grid,
  opacity, clear. The natural starting skeleton.
- **`gpui/examples/painting.rs`** — fills, dashed strokes, arcs, polygons,
  cubic/quadratic Béziers, gradients, shift-to-constrain.

**The one structural reality to design around:** GPUI has **no global / scene
transform matrix**. `TransformationMatrix` exists but rides only on *sprites*
(glyphs, monochrome SVG, raster images) — never on quads or paths. So
**zoom/pan is userland math**: the view keeps a `Camera { offset, zoom }` and
applies `screen = (world − offset) × zoom` to every coordinate before painting,
and the inverse for hit-testing. This isn't exotic — it's exactly what
gpui-component's chart/plot layer already does for data→pixel mapping — but it
touches every draw call and is the bulk of the non-drawing work.

### What GPUI gives you for free

- `canvas(prepaint, paint)` — an imperative draw surface; the paint closure
  receives `&mut Window` with all the `paint_*` primitives. (`gpui`
  `crates/gpui/src/elements/canvas.rs`)
- Full vector paths via lyon: `PathBuilder::fill()` / `PathBuilder::stroke(px)`
  → `build()` → `window.paint_path(path, color)`. Gradient fills included.
  (`crates/gpui/src/path_builder.rs`)
- Rectangle clipping for the viewport: `with_content_mask`.
- Rich, modifier-aware input including **native pinch-to-zoom**: `on_mouse_down`
  / `on_mouse_move` / `on_mouse_up`, `on_scroll_wheel` (pixel & line deltas),
  `on_pinch` (`PinchEvent.delta` + focal point), `on_drag`/`on_drag_move`,
  force-touch pressure, and a global `Window::on_mouse_event` for drag capture.
  (`crates/gpui/src/interactive.rs`, `…/elements/div.rs`)

### What you build yourself

- **The entire camera.** No scene transform exists; maintain `{ offset, zoom }`
  and map world→screen on every primitive each frame; inverse-map for
  hit-testing.
- **Path caching & invalidation.** Both reference examples re-tessellate every
  stroke every frame, and `gpui::Path` exposes only `scale()` (no translate
  after build). For a board with many strokes: cache built `Path`s,
  re-tessellate only the in-progress stroke, and rebuild on zoom change. This is
  the main perf concern (`gpui/examples/paths_bench.rs` exists to measure it).
- **Infinite-canvas bookkeeping:** world-space storage, viewport culling, a
  world-space background grid.
- **Arrows/connectors:** no arrow primitive — compose a stroked shaft + a filled
  arrowhead polygon.
- **Embedded interactive sub-views** — see Decision B; `canvas()` has no
  children, and the only child-positioning hook (`with_element_offset`) is
  translation-only and prepaint-only, so live child widgets can track pan but
  **cannot be scaled** with zoom.

---

## Architecture — three layers

### 1. `gpui-whiteboard` crate (the engine, host-agnostic)

A board owns mutable interactive state (elements, selection, camera, many
sub-element textures), so it follows the **stateful `PdfView` mold**, not the
stateless `MarkdownView` one. (See the two crate "shapes" table below.)

```rust
pub struct WhiteboardView { /* scene, camera, selection, tool, bitmap_slots,
                               generation, focus, … */ }

impl WhiteboardView {
    pub fn new(scene: Scene, style: WhiteboardStyleFn, cx: &mut Context<Self>) -> Self;

    // setters — host pushes data / installs callbacks after construction
    pub fn set_scene(&mut self, scene: Scene, cx: &mut Context<Self>);
    pub fn set_on_change(&mut self, f: ChangeFn);       // crate→host: persist edits
    pub fn set_on_open_link(&mut self, f: LinkFn);      // crate→host: click a card → open page
    pub fn set_embed_renderer(&mut self, f: EmbedFn);   // crate→host: build an AnyElement for a card

    // commands the host drives
    pub fn zoom_in(&mut self, cx);  pub fn zoom_out(&mut self, cx);
    pub fn reset_zoom(&mut self, cx);  pub fn fit_to_content(&mut self, cx);

    // GPU lifecycle — host MUST call (textures are never auto-evicted)
    pub fn release(&mut self, window: &mut Window, cx: &mut Context<Self>);
    pub fn detach_textures(&mut self, window: &mut Window, cx: &mut Context<Self>);
}

impl Render for WhiteboardView { /* … */ }
impl EventEmitter<WhiteboardEvent> for WhiteboardView {}   // e.g. SelectionChanged
```

The contract mirrors `gpui-pdf` exactly: `new(data, style_fn, cx)` + setters for
callbacks + an `EventEmitter` for surrounding chrome + `release` /
`detach_textures` for GPU hygiene.

- **Input:** an owned, serializable `Scene` value the host loads from the DB
  (like `MarkdownView`'s source string, not `PdfView`'s file path), with an
  `on_change` callback writing edits back.
- **Theme:** `WhiteboardStyleFn = Rc<dyn Fn() -> WhiteboardStyle>` read **at
  paint time**, exactly like `PdfStyleFn` — so the board follows live theme
  changes per window without push updates.
- **Dependencies:** `gpui` only (no `gpui-component`, no `zorite`), matching its
  siblings. It embeds rich host content (page-cards) through the markdown
  `Rc<dyn Fn(…) -> AnyElement>` delegation trick
  ([gpui-markdown/src/lib.rs:199](../crates/gpui-markdown/src/lib.rs:199)), so it
  never takes a dependency on `gpui-markdown` or `gpui-pdf`.

**The two crate shapes in this repo, and why a whiteboard is the second:**

| | `gpui-markdown` (`MarkdownView`) | `gpui-pdf` (`PdfView`) → **whiteboard** |
|---|---|---|
| GPUI primitive | `RenderOnce` (`IntoElement`) | `struct` + `impl Render` in an `Entity<…>` |
| State | stateless — rebuilt every frame | **stateful** — owns scroll/zoom/parsed doc/GPU bitmaps |
| Constructed | inline in `render()` each frame | once, in `cx.new(…)`; stored in a host `HashMap` |
| Config in | builder methods | constructor args + setter methods |
| Behavior out | `Rc<dyn Fn(…)>` typedefs | same, installed via setters; + `EventEmitter` |
| GPU lifecycle | none | `release()` / `detach_textures()` — host must call |

### 2. zorite host integration

The `Pdf` tab kind is a complete, recent worked example of every touch-point.
`TabKind` is exhaustively matched in ~12 places, so the compiler enumerates the
full list once `Whiteboard` is added.

| Touch-point | Location | Note |
|---|---|---|
| Add `Whiteboard(i64)` variant | [src/app.rs:61](../src/app.rs:61) | a board id, like `Page(i64)` |
| Entity store `whiteboard_views: HashMap<i64, Entity<WhiteboardView>>` | beside `pdf_views`, [src/app.rs:298](../src/app.rs:298) | entity outlives tab-vector churn |
| `open_whiteboard(id)` constructor | model on `open_pdf` [src/app.rs:1821](../src/app.rs:1821) | dedupe → push `OpenTab` → activate → `cx.new(WhiteboardView::new(…))` → install setters → insert |
| `activate_tab` rebuild arm | [src/app.rs:1043](../src/app.rs:1043) | |
| `close_tab` release arm | [src/app.rs:1072](../src/app.rs:1072) | `whiteboard_views.remove(id)` then `v.release(window, cx)` |
| Render dispatch arm | [src/app.rs:4103](../src/app.rs:4103) | `Some(v) => v.into_any_element()` |
| Window hand-off (`take_tab_seed` / adopt / `receive_tab`) | [src/app.rs:2977](../src/app.rs:2977) | `detach_textures` + ship the entity, OR just reload from DB |
| Window-close free loop | [src/app.rs:2717](../src/app.rs:2717) | |
| `NewWhiteboard` action + handler + menu | [src/actions.rs](../src/actions.rs), register near `on_new_page` [src/app.rs:3989](../src/app.rs:3989) | only needed to *create* boards |

Sidebar/tab context-menu opens (`OpenInNewWindow` / `OpenInNewTab`) already flow
through a generic `context_target: Option<TabKind>`
([src/app.rs:3185](../src/app.rs:3185),
[src/ui/sidebar.rs:520](../src/ui/sidebar.rs:520)) — **zero board-specific
code** there. That's the payoff of the typed-enum design.

GPU-texture discipline is the load-bearing part: GPUI never auto-evicts a
`RenderImage` (CPU buffer or GPU atlas) — only `cx.drop_image(arc, Some(window))`
frees one. The repo already does this in three places with the same pattern
(`PdfView::release` [gpui-pdf/src/lib.rs:1058](../crates/gpui-pdf/src/lib.rs:1058),
`detach_textures` [:1074](../crates/gpui-pdf/src/lib.rs:1074), and the host
`ImageStore` [src/images.rs](../src/images.rs)). A board holding N
image/sketch sub-elements adopts the same `Loading / Ready / Failed` slot cache
+ `release()` / `detach_textures()` pair, plus a `generation` guard if it
re-rasterizes on zoom.

### 3. Data model

Owned and serialized by the crate; the host just persists the blob.

```
Scene   { elements: Vec<Element>, camera: Camera }
Element { id, kind, transform: { x, y, w, h, rotation }, z, style }
Camera  { offset: Point, zoom: f32 }

ElementKind =
    | Draw  { points: Vec<Point>, width }   // freehand, world-space points
    | Rect | Ellipse | Line
    | Arrow { from, to, binding }           // optional endpoint bindings to other elements
    | Text  { content }
    | Embed { page_id, title }              // a page/note card
```

World-space coordinates throughout; the camera is the only thing that maps to
screen.

---

## Decisions to settle

### A. Persistence — `kind` column vs. separate table  ·  *recommended: kind column*

Migrations are linear, transactional, and `user_version`-gated
([src/db.rs:137](../src/db.rs:137)); adding a v6 step is a few lines. The
`pages` table already stores an opaque document string in `content` (markdown
today). Links/backlinks are FK-bound to `pages(id)`
([src/db.rs:31](../src/db.rs:31)).

**Recommended (b): add a `kind` column to `pages`, store the canvas JSON in
`content`.**

```sql
-- v6
ALTER TABLE pages ADD COLUMN kind TEXT NOT NULL DEFAULT 'page';
PRAGMA user_version = 6;
```

- **Wins:** boards instantly inherit `page_links` / backlinks, sidebar listing,
  rename, delete-cascade, recent, and favorites. A board can *be* a `[[link]]`
  target and can link to pages with **zero new link plumbing** — directly
  serving the brief's "linkable to pages / drop pages on the board." A board
  just stores JSON in `content` where a page stores markdown.
- **Cost:** `pages_fts` indexes `content` ([src/db.rs:189](../src/db.rs:189)),
  so the v6 step must also exclude `kind='whiteboard'` from the FTS triggers, or
  the JSON pollutes full-text search.

**Alternative (a): a separate `whiteboard` table.** Cleaner separation and the
element-model version can live inside the JSON (`"schema": N`), but `page_links`
can't reference it, so board↔page links need a new `board_links` table or
mirrored rows — more plumbing for the exact feature we want. **(c) normalized
element tables** is deferred: every element-model change would become a DB
migration, fighting the "model lives in an opaque column" grain.

### B. Embedded page-cards under zoom  ·  *recommended: self-drawn cards in v1*

GPUI's catch: live child elements (`div` / `Entity`) can only be *translated*,
**not scaled** — only self-drawn primitives (paths/quads/text) and sprites scale
with zoom. So "a live, editable markdown card on the board that shrinks as you
zoom out" isn't directly achievable.

**Recommended:** v1 page-cards are **self-drawn** — a titled box (`paint_quad` +
`paint_glyph`) showing the page title and a few preview lines, which scales
natively because we draw it. Click opens the page in a tab. *Live in-place
editing* becomes a later enhancement via a snapshot/hybrid: show a scaled
rasterized snapshot when zoomed away from 1:1, swap in a real editor at 1:1
(tldraw-style edit-vs-view mode). This sidesteps the limitation entirely for v1.

---

## Build outline — phased, each independently shippable

| Phase | Scope | Verify |
|---|---|---|
| **0 — Walking skeleton** | Scaffold `crates/gpui-whiteboard` (Cargo wiring, empty `WhiteboardView` rendering a blank `canvas()`); `TabKind::Whiteboard`, `open_whiteboard`, render arm, entity map; v6 migration + load/save the JSON stub. | Open a board tab, it renders, survives restart. |
| **1 — Camera & grid** | Userland `{ offset, zoom }`; pan (drag + scroll-wheel); zoom (native `on_pinch` + ⌘-scroll about the cursor); world-space dot grid; `fit_to_content`. | Pan/zoom feels right; content stays under the cursor while zooming. |
| **2 — Freehand pen** | Capture stroke points in world space; render with `PathBuilder::stroke`; persist into the scene. **Build the path cache here** (re-tessellate only the in-progress stroke + on zoom). | Draw, zoom, reload — strokes persist and stay crisp. |
| **3 — Shapes, arrows, text** | Rect/ellipse/line tools; arrows as stroked shaft + filled head; text labels; a minimal tool state-machine. | Each kind draws, persists, scales. |
| **4 — Selection & manipulation** | Hit-testing via inverse camera; single + marquee select; move/resize/rotate handles (`anchored()` overlay); delete; z-order; undo/redo. | Select/move/resize/undo across element kinds. |
| **5 — Page-card embeds + linking** | Drop a `[[page]]` as a self-drawn card; click → `open_page`; populate `page_links` with the board as source so backlinks light up. (Decision B's live-edit hybrid is an optional sub-phase.) | Card opens the page; the page's backlinks panel shows the board. |
| **6 — Scale & multi-window polish** | Viewport culling; the `release` / `detach_textures` texture discipline; cross-window tab tear-off; sidebar "Whiteboards" affordance. | A ~500-element board stays smooth; tear-off preserves state. |

**Shippable v1 = Phases 0–2** ("a sketch layer that persists"). 3–4 is the real
editor; 5–6 is the integration that makes it *zorite's* whiteboard rather than a
generic canvas.

---

## Risks & notes

- **Path caching across zoom** is the main perf work. `gpui::Path` has only
  `scale()` (no translate after build), so panning a cached path means
  re-emitting with offset; zoom invalidates cached vertices. Cache aggressively,
  cull to the viewport, and benchmark with `paths_bench.rs` early.
- **Text at arbitrary zoom** — font size scales with the camera; watch crispness
  and re-shaping cost at extreme zoom.
- **Hit-testing freehand** — stroke selection needs distance-to-polyline against
  the inverse-mapped cursor, not just bounding boxes.
- **Verification constraint** (carried from prior work): synthetic keyboard
  input never reaches the GPUI window, so interaction phases get verified via the
  running app + DB inspection + **unit tests on the camera/hit-test math** (it's
  pure and very testable), not scripted keypresses.

---

## References

- Crate patterns: [gpui-pdf/src/lib.rs](../crates/gpui-pdf/src/lib.rs)
  (stateful entity: ctor `:536`, setters `:705`–`:738`, lifecycle `:1058` /
  `:1074`, style typedefs `:349`–`:359`, `Render` `:1387`, `EventEmitter`
  `:1385`), [gpui-markdown/src/lib.rs:199](../crates/gpui-markdown/src/lib.rs:199)
  (the `Fn(…) -> AnyElement` delegation for host-owned sub-elements).
- Host plumbing: [src/app.rs](../src/app.rs) (`TabKind` `:61`, `pdf_views`
  `:298`, `open_pdf` `:1821`, close/release `:1072`, seed hand-off `:2977`,
  render dispatch `:4103`).
- Persistence: [src/db.rs](../src/db.rs) (migrations `:137`, `pages` `:21`,
  `page_links` `:31`, FTS `:189`, settings `:215`),
  [src/models.rs:6](../src/models.rs:6) (`Page`),
  [src/ui/links.rs:7](../src/ui/links.rs:7) (`[[link]]` parsing).
- The slot cache blueprint: [src/images.rs](../src/images.rs)
  (`Loading/Ready/Failed`, `release`, `snapshot`/`adopt`).
- GPUI source (Zed monorepo checkout): `canvas.rs`, `path_builder.rs`,
  `scene.rs`, `interactive.rs`, `elements/div.rs`. Drawing references:
  `gpui/examples/painting.rs` and
  `gpui-component/crates/story/examples/brush.rs`.
