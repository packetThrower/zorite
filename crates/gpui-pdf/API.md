# gpui-pdf API

The complete public API of [`gpui-pdf`](README.md) — every exported item, with its
signature, parameters, return contract, edge cases, and cost. For the what-and-why
(the two layers, quick start, password flow, feature overview), see the
[README](README.md).

## Public API at a glance

Everything below is the complete public surface — if it isn't listed here, it isn't
public. Items in the **Feature** column require that Cargo feature (`search` implies
`markup`); "—" means always available.

| Item | Kind | Feature | Signature | Purpose |
| --- | --- | --- | --- | --- |
| [`Document`](#type-document) | type alias | — | `type Document = hayro::Pdf` | A parsed PDF, shared via `Arc` |
| [`LoadError`](#enum-loaderror) | enum | — | `Locked \| Other(String)` | Why loading a PDF failed |
| [`parse`](#parse) | fn | — | `fn parse(bytes: Arc<Vec<u8>>) -> Result<Arc<Document>, LoadError>` | Parse PDF bytes once, reuse per page |
| [`normalize_form_appearances`](#normalize_form_appearances) | fn | `forms` | `fn normalize_form_appearances(bytes: &[u8]) -> Option<Vec<u8>>` | Rewrite form-widget appearances so they render |
| [`form_fields`](#form_fields) | fn | `forms` | `fn form_fields(bytes: &[u8]) -> Vec<FormField>` | Every form widget: name, kind, page, rect, value |
| [`set_form_value`](#set_form_value) | fn | `forms` | `fn set_form_value(bytes: &[u8], name: &str, value: &str) -> Option<Vec<u8>>` | Write a field's value + regenerate its appearance |
| [`FormField`](#struct-formfield) | struct | `forms` | — | One widget, described for a host UI |
| [`FieldKind`](#enum-fieldkind) | enum | `forms` | `Text \| Checkbox \| Radio \| Choice \| Signature` | What input a field takes |
| [`PdfView::form_fields`](#pdfviewform_fields) | method | `forms` | `fn form_fields(&self) -> &[FormField]` | The loaded document's fields (Tab order) |
| [`PdfView::reveal_field`](#pdfviewreveal_field) | method | `forms` | `fn reveal_field(&mut self, field: &FormField, cx) -> Option<Bounds<Pixels>>` | Scroll a field on-screen, return its window bounds |
| [`PdfView::replace_bytes`](#pdfviewreplace_bytes) | method | — | `fn replace_bytes(&mut self, bytes: Vec<u8>, cx)` | Hot-swap the document (scroll/zoom kept, no blanking) |
| `PdfEvent::FieldClicked` | event variant | `forms` | `{ field: FormField, bounds: Bounds<Pixels> }` | A form widget was clicked — toggle or seat an input |
| [`parse_with_password`](#parse_with_password) | fn | — | `fn parse_with_password(bytes: Arc<Vec<u8>>, password: &str) -> Result<Arc<Document>, LoadError>` | Parse an encrypted PDF |
| [`page_dims`](#page_dims) | fn | — | `fn page_dims(doc: &Document) -> Vec<(f32, f32)>` | Per-page `(w, h)` in points, no rasterization |
| [`render_page`](#render_page) | fn | — | `fn render_page(doc: &Document, idx: usize, scale: f32) -> Result<Arc<RenderImage>, String>` | Rasterize one page to a BGRA bitmap |
| [`is_pdf`](#is_pdf) | fn | — | `fn is_pdf(src: &str) -> bool` | `.pdf` extension check |
| [`PAGE_WIDTH`](#const-page_width) | const | — | `const PAGE_WIDTH: f32 = 820.0` | Base on-screen page width at zoom 1.0 |
| [`keep_window`](#keep_window) | fn | — | `fn keep_window(dims: &[(f32, f32)], page_width: f32, scroll_y: f32, viewport_h: f32) -> (usize, usize)` | Which pages to keep rasterized |
| [`PdfStyle`](#struct-pdfstyle) | struct | — | 6 `pub Hsla` fields; `impl Default` | Colors for the viewer chrome |
| [`PdfStyleFn`](#type-pdfstylefn) | type alias | — | `Rc<dyn Fn() -> PdfStyle>` | Live style source, read at paint time |
| [`PdfQualityFn`](#type-pdfqualityfn) | type alias | — | `Rc<dyn Fn() -> f32>` | Live render-quality source |
| [`PdfEvent`](#enum-pdfevent) | enum | — | `LockChanged` | Emitted on lock-state transitions |
| [`PdfView`](#struct-pdfview) | struct | — | `impl Render + EventEmitter<PdfEvent>` | The ready-made page-virtualized viewer |
| [`PdfView::new`](#pdfviewnew) | constructor | — | `fn new(path: PathBuf, style: PdfStyleFn, quality: PdfQualityFn, cx: &mut Context<Self>) -> Self` | Create a viewer; loads off-thread |
| [`PdfView::is_locked`](#pdfviewis_locked) | method | — | `fn is_locked(&self) -> bool` | Encrypted and awaiting a password |
| [`PdfView::unlock_failed`](#pdfviewunlock_failed) | method | — | `fn unlock_failed(&self) -> bool` | Last unlock used a wrong password |
| [`PdfView::unlock`](#pdfviewunlock) | method | — | `fn unlock(&mut self, password: String, cx: &mut Context<Self>)` | Retry an encrypted PDF with a password |
| [`PdfView::release`](#pdfviewrelease) | method | — | `fn release(&mut self, window: &mut Window, cx: &mut Context<Self>)` | Free all bitmaps + GPU textures before drop |
| [`PdfView::detach_textures`](#pdfviewdetach_textures) | method | — | `fn detach_textures(&mut self, window: &mut Window, cx: &mut Context<Self>)` | Free GPU textures, keep bitmaps (window move) |
| [`PdfView::set_zoom`](#pdfviewset_zoom) | method | — | `fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>)` | Set zoom (clamped 0.5–3.0) |
| [`PdfView::zoom_in`](#pdfviewzoom_in--zoom_out--reset_zoom) | method | — | `fn zoom_in(&mut self, cx: &mut Context<Self>)` | Zoom in one step (×1.25) |
| [`PdfView::zoom_out`](#pdfviewzoom_in--zoom_out--reset_zoom) | method | — | `fn zoom_out(&mut self, cx: &mut Context<Self>)` | Zoom out one step (÷1.25) |
| [`PdfView::reset_zoom`](#pdfviewzoom_in--zoom_out--reset_zoom) | method | — | `fn reset_zoom(&mut self, cx: &mut Context<Self>)` | Back to 100% |
| [`PdfView::go_to_page`](#pdfviewgo_to_page) | method | — | `fn go_to_page(&mut self, index: usize, cx: &mut Context<Self>)` | Scroll a page to the viewport top |
| [`PdfView::next_page`](#pdfviewnext_page--prev_page) | method | — | `fn next_page(&mut self, cx: &mut Context<Self>)` | Go to the next page |
| [`PdfView::prev_page`](#pdfviewnext_page--prev_page) | method | — | `fn prev_page(&mut self, cx: &mut Context<Self>)` | Go to the previous page |
| [`PdfView::toggle_toc`](#pdfviewtoggle_toc) | method | — | `fn toggle_toc(&mut self, cx: &mut Context<Self>)` | Toggle the table-of-contents panel |
| [`PdfView::has_outline`](#pdfviewhas_outline) | method | — | `fn has_outline(&self) -> bool` | Whether the PDF has bookmarks |
| [`PdfView::set_highlights`](#pdfviewset_highlights) | method | markup | `fn set_highlights(&mut self, highlights: Vec<Highlight>, cx: &mut Context<Self>)` | Hand the viewer highlights to draw |
| [`PdfView::set_on_highlight`](#pdfviewset_on_highlight) | method | markup | `fn set_on_highlight(&mut self, handler: HighlightClickFn)` | Click handler for a highlight |
| [`PdfView::set_on_create_highlight`](#pdfviewset_on_create_highlight) | method | markup | `fn set_on_create_highlight(&mut self, handler: CreateHighlightFn)` | Handler for a finished drag-selection |
| [`PdfView::toggle_select_mode`](#pdfviewtoggle_select_mode) | method | markup | `fn toggle_select_mode(&mut self, cx: &mut Context<Self>)` | Toggle drag-to-highlight mode |
| [`PdfView::set_highlight_palette`](#pdfviewset_highlight_palette) | method | markup | `fn set_highlight_palette(&mut self, palette: Vec<(SharedString, Hsla)>, cx: &mut Context<Self>)` | Colors for the picker |
| [`PdfView::reveal_highlight`](#pdfviewreveal_highlight) | method | markup | `fn reveal_highlight(&mut self, page: usize, cx: &mut Context<Self>)` | Scroll to a page's highlight and flash it |
| [`PdfView::toggle_search`](#pdfviewtoggle_search) | method | search | `fn toggle_search(&mut self, cx: &mut Context<Self>)` | Open/close the find bar |
| [`PdfView::close_search`](#pdfviewclose_search) | method | search | `fn close_search(&mut self, cx: &mut Context<Self>)` | Close the find bar, clear matches |
| [`PdfView::next_match`](#pdfviewnext_match--prev_match) | method | search | `fn next_match(&mut self, cx: &mut Context<Self>)` | Focus the next match (wraps) |
| [`PdfView::prev_match`](#pdfviewnext_match--prev_match) | method | search | `fn prev_match(&mut self, cx: &mut Context<Self>)` | Focus the previous match (wraps) |
| [`OutlineItem`](#struct-outlineitem) | struct | — | `{ title: String, level: usize, page: Option<usize> }` | One flattened outline (bookmark) entry |
| [`LinkTarget`](#enum-linktarget) | enum | — | `Page(usize) \| Uri(String)` | Where a clickable PDF link points |
| [`PdfLink`](#struct-pdflink) | struct | — | `{ x, y, w, h: f32, target: LinkTarget }` | A `/Link` annotation, normalized rect |
| [`outline`](#outline) | fn | — | `fn outline(doc: &Document) -> Vec<OutlineItem>` | Extract the document outline |
| [`page_links`](#page_links) | fn | — | `fn page_links(doc: &Document) -> Vec<Vec<PdfLink>>` | Extract link annotations per page |
| [`Highlight`](#struct-highlight) | struct | markup | `{ id: u64, page: usize, quote: String, occurrence: usize, color: Hsla }` | A quote-anchored highlight to draw |
| [`HighlightClickFn`](#type-highlightclickfn) | type alias | markup | `Rc<dyn Fn(u64, &mut Window, &mut App)>` | Highlight click callback |
| [`CreateHighlightFn`](#type-createhighlightfn) | type alias | markup | `Rc<dyn Fn(usize, String, usize, SharedString, &mut Window, &mut App)>` | Drag-selection-finished callback |
| [`NormRect`](#struct-normrect) | struct | markup | `{ x, y, w, h: f32 }` | Rect in normalized (0..1) page coords |
| [`NormPoint`](#struct-normpoint) | struct | markup | `{ x, y: f32 }` | Point in normalized page coords |
| [`Selection`](#struct-selection) | struct | markup | `{ quote: String, occurrence: usize, rects: Vec<NormRect> }` | A resolved drag selection |
| [`extract_page_text`](#extract_page_text) | fn | markup | `fn extract_page_text(doc: &Document, index: usize) -> Option<PageText>` | Extract a page's text layer |
| [`PageText`](#struct-pagetext) | struct | markup | — | A page's text + per-glyph rects |
| [`PageText::is_empty`](#pagetextis_empty) | method | markup | `fn is_empty(&self) -> bool` | No extractable text (pure scan) |
| [`PageText::text`](#pagetexttext) | method | markup | `fn text(&self) -> String` | Readable reconstruction of the page |
| [`PageText::locate`](#pagetextlocate) | method | markup | `fn locate(&self, needle: &str, occurrence: usize) -> Vec<NormRect>` | Find the nth occurrence of a quote |
| [`PageText::find_matches`](#pagetextfind_matches) | method | search | `fn find_matches(&self, needle: &str) -> Vec<Vec<NormRect>>` | All occurrences, reading order |
| [`PageText::select`](#pagetextselect) | method | markup | `fn select(&self, from: NormPoint, to: NormPoint) -> Option<Selection>` | Resolve a drag into a quote |
| `impl Default for PdfStyle` | trait impl | — | `fn default() -> Self` | Neutral dark palette |
| `impl EventEmitter<PdfEvent> for PdfView` | trait impl | — | — | Subscribe via `cx.subscribe` |
| `impl Render for PdfView` | trait impl | — | — | Render the `Entity<PdfView>` as a child view |

---

## `type Document`

```rust
pub type Document = hayro::hayro_syntax::Pdf;
```

A parsed PDF. Parse **once** (not per page) — re-parsing a large file for every page
is slow and churns the allocator. hayro's `Pdf` is `Send + Sync` and caches pages
internally, so share it via `Arc` across background render tasks. The `Document`
owns the file bytes it was parsed from.

---

## `enum LoadError`

```rust
pub enum LoadError {
    Locked,
    Other(String),
}
```

Why loading a PDF failed.

- `Locked` — the PDF is encrypted and the supplied password was missing or wrong.
  The caller can prompt for a password and retry via
  [`parse_with_password`](#parse_with_password).
- `Other(String)` — any other failure: malformed file, or an encryption scheme
  hayro's standard security handler doesn't cover (public-key / certificate
  handlers, non-standard crypt filters). The string is a debug-formatted hayro
  error, for logging — not for display to users.

`#[derive(Debug)]` only — no `std::error::Error` impl, no `Display`.

---

## `parse`

```rust
pub fn parse(bytes: Arc<Vec<u8>>) -> Result<Arc<Document>, LoadError>
```

Parse a PDF's bytes into a reusable [`Document`](#type-document). Exactly
`parse_with_password(bytes, "")`.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `bytes` | `Arc<Vec<u8>>` | The whole file's bytes. The `Document` keeps the `Arc`, so the caller can drop its copy. |

**Returns** — `Ok(Arc<Document>)` ready for [`page_dims`](#page_dims) /
[`render_page`](#render_page), or a [`LoadError`](#enum-loaderror).

**Guarantees & edge cases**

- A password-protected file returns `Err(LoadError::Locked)` — retry with
  [`parse_with_password`](#parse_with_password). This is the *only* condition mapped
  to `Locked` (hayro's `DecryptionError::PasswordProtected`); every other parse or
  decryption failure is `Other`.
- Never panics; malformed input is `Err(Other)`.

**Cost & threading** — parses the cross-reference table and document structure
(pages are lazy). Cheap for small files, but run it off the UI thread for large
ones; the result is `Send + Sync`.

---

## `parse_with_password`

```rust
pub fn parse_with_password(
    bytes: Arc<Vec<u8>>,
    password: &str,
) -> Result<Arc<Document>, LoadError>
```

Like [`parse`](#parse), but supplies a decryption `password` for an encrypted PDF.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `bytes` | `Arc<Vec<u8>>` | The whole file's bytes. |
| `password` | `&str` | The password that opens the document. `""` for an unencrypted file. |

**Returns** — `Ok(Arc<Document>)`, or `Err(LoadError::Locked)` when the file is
password-protected and `password` is missing or incorrect, or `Err(LoadError::Other)`
otherwise.

**Guarantees & edge cases**

- Decryption is hayro's, via the PDF **standard security handler** (the
  password-based scheme):

  | Algorithm | PDF `/V` | Notes |
  | --- | --- | --- |
  | RC4, 40-bit | 1 | legacy |
  | RC4, 40–128-bit | 2 | key length from `/Length` |
  | AES-128 | 4 | `AESV2` crypt filter (RC4 via a `V2` filter also works) |
  | AES-256 | 5 / 6 | `AESV3` crypt filter; PDF 2.0 (revision 6) |

- Anything else — public-key / certificate handlers (`/Filter` ≠ `/Standard`), any
  non-standard crypt filter — surfaces as `Err(Other)`, **not** `Locked`: don't show
  a password prompt for it.
- With the `forms` feature, the bytes first pass through
  [`normalize_form_appearances`](#normalize_form_appearances) (both here and in
  [`parse`](#parse)) so form values and checkbox states display; an encrypted or
  form-free file passes through untouched.
- Never panics.

**Example**

```rust
match gpui_pdf::parse_with_password(bytes, password) {
    Ok(doc) => { /* render */ }
    Err(gpui_pdf::LoadError::Locked) => { /* prompt for a password and retry */ }
    Err(gpui_pdf::LoadError::Other(e)) => { /* malformed / unsupported encryption */ }
}
```

---

## `normalize_form_appearances`

*Feature: `forms`.*

```rust
pub fn normalize_form_appearances(bytes: &[u8]) -> Option<Vec<u8>>
```

Rewrite a document's bytes so every AcroForm widget has a directly-renderable
appearance stream. hayro composites annotation `/AP /N` streams but (a) skips a
checkbox/radio whose `/N` is a **dictionary of states** selected by `/AS`, and
(b) never **synthesizes** an appearance for a valued text field that has none
(the `NeedAppearances` case). This pass fixes both at the byte level — resolve
`/N` through `/AS`; synthesize a Helvetica appearance for the field's `/V` —
so form PDFs *display* what they carry. [`parse`](#parse) /
[`parse_with_password`](#parse_with_password) call it automatically under this
feature; it's public for hosts that manage bytes themselves.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `bytes` | `&[u8]` | The whole file's bytes. |

**Returns** — `Some(rewritten_bytes)` **only when something changed**; `None`
means keep the originals (no form widgets, nothing to fix, an encrypted file,
or bytes lopdf can't parse).

**Guarantees & edge cases**

- Idempotent: running it on its own output returns `None`.
- Encrypted documents are left untouched (`None`) — hayro decrypts on its own;
  rewriting would need the password.
- `/FT` and `/V` are resolved up the `/Parent` chain (split field/widget
  pairs), bounded against cyclic chains.
- Synthesized text is single-line, left-aligned, ~12 pt Helvetica
  (WinAnsi-lossy: characters outside Latin-1 become `?`), clipped to the
  widget rect by the XObject's BBox. `/DA` fonts, `/Q` quadding, comb fields,
  multiline layout, and rich-text values are **not** honored — this is display
  correctness, not a form engine.
- Only `/AP /N` (the normal appearance) is touched; `/D`/`/R` states and
  non-widget annotations pass through unchanged.
- Never panics; any structural surprise inside a widget just skips that widget.

**Cost** — a full lopdf parse + serialize when changes are made (one-time, at
load). A document with no `/Subtype /Widget` objects short-circuits after the
parse.

---

## `form_fields`

*Feature: `forms`.*

```rust
pub fn form_fields(bytes: &[u8]) -> Vec<FormField>
```

Every form-field widget in the document, in page order — what a host needs to
overlay inputs on the viewer.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `bytes` | `&[u8]` | The whole file's bytes. |

**Returns** — one [`FormField`](#struct-formfield) per *widget* (a radio group
is one field name across several widgets, each reported with its own rect and
on-state). Pushbuttons carry no value and are skipped.

**Guarantees & edge cases**

- An encrypted or unparseable file yields an empty `Vec` — never an error.
- `rect` is in PDF points with a bottom-left origin, corners normalized
  (`x0 < x1`, `y0 < y1`); to overlay on a rendered page, flip y against the
  page height and multiply by the render scale.
- Names are fully qualified (`/T` joined root-first with `.` up the `/Parent`
  chain) — exactly the key [`set_form_value`](#set_form_value) expects.
- `/FT`, `/V`, `/Ff`, and `/Opt` resolve through field inheritance.

---

## `set_form_value`

*Feature: `forms`.*

```rust
pub fn set_form_value(bytes: &[u8], name: &str, value: &str) -> Option<Vec<u8>>
```

Set the field named `name` and **regenerate its appearance stream**, so the
written file renders correctly in every viewer — not just this crate's.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `bytes` | `&[u8]` | The whole file's bytes. |
| `name` | `&str` | A fully-qualified name from [`form_fields`](#form_fields). |
| `value` | `&str` | `Text`/`Choice`: the literal text. `Checkbox`/`Radio`: an on-state from [`FormField::options`](#struct-formfield), or `"Off"` to clear. |

**Returns** — `Some(rewritten_bytes)` on success; `None` when nothing matched
(unknown name, read-only or signature field, encrypted/unparseable file).

**Guarantees & edge cases**

- `/V` is written on the dict that owns `/FT` (the parent for split
  field/widget pairs), so sibling widgets stay consistent; a radio group's
  every widget gets its `/AS` set (`value` where that widget carries the
  state, `Off` elsewhere).
- Text appearances are regenerated with the same synthesis as
  [`normalize_form_appearances`](#normalize_form_appearances) — same ceilings
  (single-line, WinAnsi-lossy).
- Read-only fields (`/Ff` bit 1) and signature fields are refused.
- The caller owns persistence: write the returned bytes wherever the document
  lives, and re-[`parse`](#parse) to refresh a viewer.

---

## `struct FormField`

*Feature: `forms`.*

```rust
pub struct FormField {
    pub name: String,           // fully-qualified field name (the set_form_value key)
    pub kind: FieldKind,
    pub page: usize,            // 0-based page index
    pub rect: (f32, f32, f32, f32), // PDF points, bottom-left origin, corners ordered
    pub value: String,          // text, or the on-state name / "Off" for buttons
    pub read_only: bool,        // /Ff bit 1
    pub options: Vec<String>,   // Choice: /Opt entries; Checkbox/Radio: on-state names
}
```

One widget, described for a host UI. See [`form_fields`](#form_fields) for the
coordinate and naming contracts.

---

## `enum FieldKind`

*Feature: `forms`.*

```rust
pub enum FieldKind { Text, Checkbox, Radio, Choice, Signature }
```

What input a field takes. `Radio` is one widget of a group (same `name`,
several widgets). `Signature` is display-only — [`set_form_value`](#set_form_value)
refuses it. Pushbuttons never appear (no value to hold).

---

## `PdfView::form_fields`

*Feature: `forms`.*

```rust
pub fn form_fields(&self) -> &[FormField]
```

The loaded document's form fields, enumerated at load from the original bytes
(not the display-normalized ones) — for a host driving Tab navigation. The
order is [`form_fields`](#form_fields)' page-then-document order. Empty before
the load finishes and for form-free documents.

---

## `PdfView::reveal_field`

*Feature: `forms`.*

```rust
pub fn reveal_field(&mut self, field: &FormField, cx: &mut Context<Self>) -> Option<Bounds<Pixels>>
```

Scroll so `field`'s widget sits comfortably on-screen (a 56 px margin), then
return its fresh window-space bounds — what a host needs to seat an input on a
field reached by Tab rather than by click. `None` before the first layout.
Every form widget is rendered as a transparent overlay with a hover tint and a
pointer cursor; clicking one emits `PdfEvent::FieldClicked` with the same
bounds shape.

---

## `PdfView::replace_bytes`

```rust
pub fn replace_bytes(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>)
```

Swap in a new version of the document — e.g. after
[`set_form_value`](#set_form_value) rewrote the file — keeping scroll, zoom,
and view state. Re-parses off-thread. When the page count is unchanged, the
old page bitmaps keep painting until their crisp replacements land (the same
no-blanking swap as zoom/quality changes); a different page count resets the
slots. Highlights' cached text layers are dropped and rebuilt lazily. A parse
failure is logged and the old document stays.

---

## `page_dims`

```rust
pub fn page_dims(doc: &Document) -> Vec<(f32, f32)>
```

Each page's `(width, height)` in PDF points.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `doc` | `&Document` | A parsed PDF. |

**Returns** — `Vec<(f32, f32)>`, one `(width, height)` per page, in page order
(hayro's `render_dimensions`, i.e. the size a render would produce — rotation
already applied).

**Guarantees & edge cases**

- A zero-page document returns an empty vec.
- No rasterization — cheap enough to call on load, so a viewer can lay out
  correctly-sized page slots (and a correct scrollbar) before any page renders.

---

## `render_page`

```rust
pub fn render_page(doc: &Document, idx: usize, scale: f32) -> Result<Arc<RenderImage>, String>
```

Rasterize a single page of an already-parsed [`Document`](#type-document).

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `doc` | `&Document` | A parsed PDF. |
| `idx` | `usize` | 0-based page index. |
| `scale` | `f32` | Pixels per PDF point (page point-size × this = bitmap size). |

**Returns** — `Arc<gpui::RenderImage>` (a single frame), or `Err(String)` with a
short human-readable message.

**Guarantees & edge cases**

- The bitmap is **BGRA, fully opaque, composited onto white**: hayro produces
  premultiplied RGBA; each pixel is alpha-composited over white and channel-swapped,
  and the output alpha is forced to 255. Transparent PDF backgrounds therefore
  render white, not transparent.
- An out-of-range `idx` returns `Err` (never panics).
- Higher `scale` = sharper but more memory (pixels grow quadratically). `PdfView`
  derives its scale from display pixel ratio × zoom × quality and clamps it to
  0.5–4.0; do something similar if you call this directly.

**Cost & threading** — CPU-bound rasterization of the whole page; run it on a
background thread (`Document` is `Send + Sync`, so clone the `Arc` into the task).

---

## `is_pdf`

```rust
pub fn is_pdf(src: &str) -> bool
```

True if a link/image `src` points at a PDF.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `src` | `&str` | A path or URL string. |

**Returns** — `bool`: whether `src`, lowercased and with trailing whitespace
trimmed, ends with `.pdf`.

**Guarantees & edge cases**

- Pure string check — no filesystem access, no content sniffing.
- A URL with a query string or fragment (`report.pdf?v=2`) is **not** detected;
  only a trailing extension is.

---

## `const PAGE_WIDTH`

```rust
pub const PAGE_WIDTH: f32 = 820.0;
```

The base on-screen page width in points at zoom 1.0. `PdfView` lays pages out at
`PAGE_WIDTH × zoom` wide (heights follow each page's aspect ratio); pass the same
value to [`keep_window`](#keep_window) if you reuse its math.

---

## `keep_window`

```rust
pub fn keep_window(
    dims: &[(f32, f32)],
    page_width: f32,
    scroll_y: f32,
    viewport_h: f32,
) -> (usize, usize)
```

The inclusive page-index range `(start, end)` to keep rasterized for a scroll
position: the pages intersecting the viewport, padded by 3 pages on each side (so
scrolling finds neighbors already rendered and small wiggles don't thrash
render/evict). This is the pure, unit-tested core of `PdfView`'s virtualization —
use it to build your own viewer.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `dims` | `&[(f32, f32)]` | Per-page `(w, h)` in points (from [`page_dims`](#page_dims)). |
| `page_width` | `f32` | On-screen column width in px (base × zoom). |
| `scroll_y` | `f32` | How far the content is scrolled down (px, ≥ 0; negatives are clamped). |
| `viewport_h` | `f32` | Visible height (px). |

**Returns** — `(start, end)`, an **inclusive** 0-based index range, always within
`0..dims.len()`.

**Guarantees & edge cases**

- Empty `dims` → `(0, 0)`.
- `viewport_h ≤ 1.0` (first frame, before layout) → assumes a 900 px viewport so
  the first pages still render.
- Mirrors `PdfView`'s exact slot layout: 16 px top padding, then each page's
  aspect-scaled height, with a 10 px gap between pages. If your layout differs, the
  window will be offset.
- Pure and cheap (one linear scan); never panics.

---

## `struct PdfStyle`

```rust
pub struct PdfStyle {
    pub bg: Hsla,             // viewer background
    pub border: Hsla,         // page-slot border + header divider
    pub placeholder_bg: Hsla, // unrendered page slot + control hover
    pub placeholder_fg: Hsla, // "Page N" / "Loading…" text
    pub header_fg: Hsla,      // header filename + control text
    pub header_muted: Hsla,   // "· N pages" / page counter text
}
```

Colors for the [`PdfView`](#struct-pdfview) chrome. `Clone + Copy`.
`PdfStyle::default()` is a neutral dark palette (near-black background, faint white
borders/text).

---

## `type PdfStyleFn`

```rust
pub type PdfStyleFn = Rc<dyn Fn() -> PdfStyle>;
```

Supplies the current [`PdfStyle`](#struct-pdfstyle) **at paint time**. `PdfView` is
a persistent entity (not rebuilt by its parent each frame), so it reads its colors
through this closure on every render — return fresh colors each call and the viewer
follows live theme changes (and can differ per window) with no push from the host.

---

## `type PdfQualityFn`

```rust
pub type PdfQualityFn = Rc<dyn Fn() -> f32>;
```

Supplies the current render-quality multiplier at paint time: `1.0` = native DPI;
lower is faster and softer, higher supersamples. Clamped internally to 0.25–3.0.
Read like [`PdfStyleFn`](#type-pdfstylefn) — when the returned value changes by more
than 0.01, every open viewer bumps its render generation and re-rasterizes visible
pages (showing the old bitmaps, rescaled, until the crisp ones land). This is why
there is no `set_quality` method.

---

## `enum PdfEvent`

```rust
pub enum PdfEvent {
    LockChanged,
}
```

Emitted (via `impl EventEmitter<PdfEvent> for PdfView`) when the view's lock state
transitions: the load discovered an encrypted file, an [`unlock`](#pdfviewunlock)
succeeded, or an unlock failed with a wrong password. Fired only on these
transitions — not on every redraw. A host rendering a password prompt around the
viewer subscribes to know when to re-render (see
[`PdfView::is_locked`](#pdfviewis_locked)).

---

## `struct PdfView`

```rust
pub struct PdfView { /* private */ }

impl Render for PdfView { /* … */ }
impl EventEmitter<PdfEvent> for PdfView { /* … */ }
```

A self-contained, page-virtualized PDF viewer entity. Every page gets a
correctly-sized slot up front (so the scrollbar reflects the whole document), but
only the pages within the visible range ±3 are rasterized; pages scrolled away are
freed — CPU pixel buffer *and* GPU atlas texture — so memory is bounded by what's on
screen, not the page count.

Built-in chrome: a header with the filename, page navigation (‹ / ›, a
click-to-edit page counter you can type a number into), zoom controls (−, %, +), a
table-of-contents side panel (when the PDF has an outline), clickable link
annotations overlaid on each page (internal → jump to page, external →
`cx.open_url`), an overlay scrollbar, and a scroll-to-top button. With `markup`: a
highlight pen + color picker; with `search`: a find bar (🔍).

Keyboard shortcuts (handled when the viewer is focused — it focuses itself on
click): PageUp / PageDown / Home / End navigate; ⌘= / ⌘- / ⌘0 zoom; ⌘⌥G jumps to a
page; ⌘⇧H toggles highlight mode (`markup`); ⌘F toggles find and ⌘G / ⌘⇧G step
matches (`search`). "⌘" is the platform secondary modifier (Ctrl elsewhere).

Each `PdfView` owns its own scroll handle, zoom, and document, so multiple viewers
operate independently.

**Rendering states** — while loading (or after a failed load) the view renders a
"Loading PDF…" placeholder. A file-read error or malformed PDF is **logged and the
view stays on that placeholder indefinitely** — there is no error state or event
for it. An encrypted file flips to locked instead (see
[`is_locked`](#pdfviewis_locked)).

### `PdfView::new`

```rust
pub fn new(
    path: PathBuf,
    style: PdfStyleFn,
    quality: PdfQualityFn,
    cx: &mut Context<Self>,
) -> Self
```

Create a viewer for `path`, kicking off the off-thread read + parse + measure.
Call inside `cx.new(|cx| PdfView::new(path, style, quality, cx))`.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `path` | `PathBuf` | A local `.pdf` file. Read once on the background executor. |
| `style` | [`PdfStyleFn`](#type-pdfstylefn) | Chrome colors, read at paint time. |
| `quality` | [`PdfQualityFn`](#type-pdfqualityfn) | Render-quality multiplier, read at paint time. |
| `cx` | `&mut Context<Self>` | The entity context (from `cx.new`). |

**Returns** — the viewer, immediately renderable (it shows "Loading PDF…" until the
load lands).

**Guarantees & edge cases**

- The file is read, parsed, and measured on the background executor; the outline
  and link annotations are extracted in the same pass. The UI never blocks.
- Encrypted file → the view becomes [locked](#pdfviewis_locked) and emits
  [`PdfEvent::LockChanged`](#enum-pdfevent); the raw bytes are kept so
  [`unlock`](#pdfviewunlock) retries without re-reading the disk.
- Unreadable or malformed file → `log::error!` and the view stays on the loading
  placeholder. No panic, no event.

**Cost & threading** — construction is cheap; the heavy work is spawned. Main
thread (it's a gpui entity).

**Example**

```rust
let view = cx.new(|cx| {
    PdfView::new(path, Rc::new(PdfStyle::default), Rc::new(|| 1.0), cx)
});
// then `view.clone()` into your element tree; call `release` before dropping.
```

### `PdfView::is_locked`

```rust
pub fn is_locked(&self) -> bool
```

Whether the PDF is encrypted and awaiting a password — the host should render its
own password prompt and call [`unlock`](#pdfviewunlock) instead of rendering the
viewer. Flips true when the initial load hits a password-protected file; flips
false on a successful unlock. **Parameters** — none (`&self`).

### `PdfView::unlock_failed`

```rust
pub fn unlock_failed(&self) -> bool
```

Whether the most recent [`unlock`](#pdfviewunlock) used a wrong password — drives
the prompt's "incorrect password" message. Cleared at the start of the next
attempt (and on success). **Parameters** — none (`&self`).

### `PdfView::unlock`

```rust
pub fn unlock(&mut self, password: String, cx: &mut Context<Self>)
```

Retry an encrypted PDF with `password`, reusing the bytes already read.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `password` | `String` | The user's password attempt. |
| `cx` | `&mut Context<Self>` | Entity context. |

**Returns** — nothing; the result arrives asynchronously.

**Guarantees & edge cases**

- Asynchronous: re-parses off-thread, then either installs the document (viewer
  renders, [`is_locked`](#pdfviewis_locked) → false) or sets
  [`unlock_failed`](#pdfviewunlock_failed) and stays locked. Emits
  [`PdfEvent::LockChanged`](#enum-pdfevent) on **either** outcome, so the prompt
  can react.
- Exception: if the retry fails with `LoadError::Other` (e.g. an unsupported
  encryption scheme), it is only logged — no event, no state change.
- Silently a no-op if called before the initial load has read the file bytes.

### `PdfView::release`

```rust
pub fn release(&mut self, window: &mut Window, cx: &mut Context<Self>)
```

Free every rasterized page — CPU pixel buffers (by dropping the `Arc`s) **and** the
GPU atlas textures. **Call this before dropping the view** (e.g. when its tab
closes): gpui caches one atlas texture per `RenderImage` on paint and only frees it
via `drop_image`; a plain drop leaks the textures.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `window` | `&mut Window` | The window whose atlas holds the textures. |
| `cx` | `&mut Context<Self>` | Entity context. |

**Returns** — nothing. Idempotent (the slots are emptied). The view is unusable
for display afterwards only in the sense that all pages re-rasterize if it renders
again.

### `PdfView::detach_textures`

```rust
pub fn detach_textures(&mut self, window: &mut Window, cx: &mut Context<Self>)
```

Free the viewer's GPU textures in `window` but **keep** its rendered page bitmaps —
for a host moving the view to a different window (e.g. a tab drag). The kept
bitmaps re-upload wherever the view next paints, so its pages appear there
immediately, with scroll, zoom, and (for an encrypted file) the unlocked state
intact.

**Parameters** — same as [`release`](#pdfviewrelease). **Returns** — nothing.

### `PdfView::set_zoom`

```rust
pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>)
```

Set the zoom factor, keeping the current page in view.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `zoom` | `f32` | Desired factor; clamped to **0.5–3.0**. `1.0` = base width ([`PAGE_WIDTH`](#const-page_width)). |
| `cx` | `&mut Context<Self>` | Entity context. |

**Returns** — nothing.

**Guarantees & edge cases**

- No-op if the clamped value is within 0.001 of the current zoom.
- The current top page is re-anchored to the viewport top after the layout change.
- Visible pages re-rasterize crisp at the new scale; their current bitmaps stay on
  screen (rescaled) until the fresh ones land, so nothing blanks. In-flight renders
  from the old scale are discarded.

### `PdfView::zoom_in` / `zoom_out` / `reset_zoom`

```rust
pub fn zoom_in(&mut self, cx: &mut Context<Self>)
pub fn zoom_out(&mut self, cx: &mut Context<Self>)
pub fn reset_zoom(&mut self, cx: &mut Context<Self>)
```

One multiplicative step (×1.25 / ÷1.25) or back to 100% — all delegate to
[`set_zoom`](#pdfviewset_zoom) (same clamping and no-blank behavior).
**Parameters** — `cx` only.

### `PdfView::go_to_page`

```rust
pub fn go_to_page(&mut self, index: usize, cx: &mut Context<Self>)
```

Scroll so page `index`'s top sits at the viewport top.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `index` | `usize` | 0-based page; clamped to the last page. |
| `cx` | `&mut Context<Self>` | Entity context. |

**Guarantees & edge cases**

- No-op while the document is still loading (no page dims yet).
- Page 0 scrolls to the true document top (keeping the column's top padding).

### `PdfView::next_page` / `prev_page`

```rust
pub fn next_page(&mut self, cx: &mut Context<Self>)
pub fn prev_page(&mut self, cx: &mut Context<Self>)
```

Step from the current page (the topmost page intersecting the viewport top) via
[`go_to_page`](#pdfviewgo_to_page); clamped at both ends, no wrap. **Parameters** —
`cx` only.

### `PdfView::toggle_toc`

```rust
pub fn toggle_toc(&mut self, cx: &mut Context<Self>)
```

Toggle the table-of-contents (outline) side panel. The panel only actually shows
when [`has_outline`](#pdfviewhas_outline) is true; entries with an unresolved
destination (named destinations) render muted and inert. **Parameters** — `cx`
only.

### `PdfView::has_outline`

```rust
pub fn has_outline(&self) -> bool
```

Whether the document has an outline (bookmarks) — false until the load finishes,
and false for PDFs without `/Outlines`. Use it to hide a TOC control.
**Parameters** — none (`&self`).

### `PdfView::set_highlights`

*(`markup` feature)*

```rust
pub fn set_highlights(&mut self, highlights: Vec<Highlight>, cx: &mut Context<Self>)
```

Set the highlights to draw. The host derives these from its own store (e.g. the
markdown blocks that quote this PDF); the viewer owns no highlight storage.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `highlights` | `Vec<Highlight>` | Replaces the current set entirely. |
| `cx` | `&mut Context<Self>` | Entity context. |

**Guarantees & edge cases**

- Pages with highlights extract their text layer lazily — off-thread, cached — as
  they scroll into view; then each quote is [located](#pagetextlocate) and drawn as
  a translucent box per line it spans.
- A quote that isn't found on its page simply draws nothing (no error).
- Coordinates are normalized, so highlights track zoom and DPI for free.

### `PdfView::set_on_highlight`

*(`markup` feature)*

```rust
pub fn set_on_highlight(&mut self, handler: HighlightClickFn)
```

Set the handler invoked with a highlight's `id` when it's clicked (e.g. to jump to
the source note). Replaces any previous handler; unset means clicks do nothing.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `handler` | [`HighlightClickFn`](#type-highlightclickfn) | `Rc<dyn Fn(u64, &mut Window, &mut App)>` — receives the clicked `Highlight.id`. |

### `PdfView::set_on_create_highlight`

*(`markup` feature)*

```rust
pub fn set_on_create_highlight(&mut self, handler: CreateHighlightFn)
```

Set the handler invoked when a drag-selection finishes in highlight mode. Without
one, selections resolve but nothing is stored.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `handler` | [`CreateHighlightFn`](#type-createhighlightfn) | Receives `(page, quote, occurrence, color_label, &mut Window, &mut App)`. |

**Guarantees & edge cases**

- Only fires for a real drag: a bare click or tiny jitter (< 0.005 of the page in
  both axes) never creates a highlight.
- The quote is a single-line join of the selected text; `occurrence` disambiguates
  a repeated quote so it re-locates to the right match; `color_label` is the label
  of the active palette swatch (empty if no palette was set).

### `PdfView::toggle_select_mode`

*(`markup` feature)*

```rust
pub fn toggle_select_mode(&mut self, cx: &mut Context<Self>)
```

Toggle "highlight mode": when on, dragging over text selects it and fires the
create handler; the color picker pops down (if a palette is set). Turning it off
cancels any in-progress selection and hides the picker. While on, every visible
page extracts its text layer so drags can select anywhere. Also bound to ⌘⇧H.
**Parameters** — `cx` only.

### `PdfView::set_highlight_palette`

*(`markup` feature)*

```rust
pub fn set_highlight_palette(
    &mut self,
    palette: Vec<(SharedString, Hsla)>,
    cx: &mut Context<Self>,
)
```

Set the highlight colors the picker offers, as `(label, fill)` pairs.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `palette` | `Vec<(SharedString, Hsla)>` | Swatches, in display order. The label is opaque to the viewer — echoed back via [`CreateHighlightFn`](#type-createhighlightfn) so the host can store it and map it back to a fill for [`set_highlights`](#pdfviewset_highlights). |
| `cx` | `&mut Context<Self>` | Entity context. |

**Guarantees & edge cases**

- If the active index falls outside the new palette, it resets to 0.
- Empty palette → no picker; new highlights select with a default yellow and an
  empty label.

### `PdfView::reveal_highlight`

*(`markup` feature)*

```rust
pub fn reveal_highlight(&mut self, page: usize, cx: &mut Context<Self>)
```

Jump to a highlight from its note: scroll `page` into view and briefly flash the
page's highlights (brighter fill + outline for ~1.2 s) so the eye finds them.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `page` | `usize` | 0-based page; clamped to the last page. |
| `cx` | `&mut Context<Self>` | Entity context. |

**Guarantees & edge cases**

- Called before the document finishes loading → the jump is queued and applied once
  the document is measured.
- If the page's text layer is already extracted, the scroll lands the page's first
  highlight just below the viewport top; otherwise it lands on the page top.
- A newer reveal supersedes an in-flight flash (no early clear).

### `PdfView::toggle_search`

*(`search` feature)*

```rust
pub fn toggle_search(&mut self, cx: &mut Context<Self>)
```

Toggle the find-in-PDF bar (also bound to ⌘F). Opening kicks off text extraction
for **every page** of the document (off-thread, cached — subsequent opens are
instant) and computes matches for the current query; the match list refreshes once
the last page's text lands. Closing clears the matches. The query is typed
directly into the bar (the viewer captures keystrokes while it's open); Enter /
⇧Enter and ⌘G / ⌘⇧G step matches. A fresh query starts from the page currently
being read, not the document top. **Parameters** — `cx` only.

### `PdfView::close_search`

*(`search` feature)*

```rust
pub fn close_search(&mut self, cx: &mut Context<Self>)
```

Close the find bar and clear the matches (the query text is kept for the next
open; extracted page text stays cached). Also bound to Esc while the bar is open.
**Parameters** — `cx` only.

### `PdfView::next_match` / `prev_match`

*(`search` feature)*

```rust
pub fn next_match(&mut self, cx: &mut Context<Self>)
pub fn prev_match(&mut self, cx: &mut Context<Self>)
```

Focus the next / previous match in reading order (page, then top-to-bottom),
wrapping at the ends, and scroll it into view — but only if it isn't already
comfortably visible, so stepping doesn't yank the page around. No-op when there
are no matches. **Parameters** — `cx` only.

---

## `struct OutlineItem`

```rust
pub struct OutlineItem {
    pub title: String,       // the bookmark label
    pub level: usize,        // nesting depth, 0 = top level
    pub page: Option<usize>, // 0-based target page, or None if unresolved
}
```

One entry in a PDF's outline (bookmarks), flattened depth-first by
[`outline`](#outline). `page` is `None` when the destination couldn't be resolved —
currently that's **named destinations** (the title still shows; `PdfView` renders
these muted and inert). `Clone + Debug + PartialEq + Eq`.

---

## `enum LinkTarget`

```rust
pub enum LinkTarget {
    Page(usize),  // a 0-based page index within this document
    Uri(String),  // an external URI
}
```

Where a clickable PDF link points. `Clone + Debug + PartialEq + Eq`.

---

## `struct PdfLink`

```rust
pub struct PdfLink {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub target: LinkTarget,
}
```

A clickable `/Link` annotation: its rectangle in **normalized page coordinates**
(0..1 of the crop box, top-left origin — matching the rendered image, so multiply
by the on-screen page size to overlay it) and its target. All components are
clamped to 0..1. `Clone + Debug + PartialEq`.

---

## `outline`

```rust
pub fn outline(doc: &Document) -> Vec<OutlineItem>
```

Extract the document outline (bookmarks), flattened depth-first.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `doc` | `&Document` | A parsed PDF. |

**Returns** — `Vec<OutlineItem>` in depth-first document order; **empty** when the
PDF has no `/Outlines`.

**Guarantees & edge cases**

- Destinations given as an explicit `[pageRef /XYZ …]` array — directly in `/Dest`
  or inside an `/A` GoTo action — resolve to a page index; **named destinations**
  are left unresolved (`page: None`).
- Titles decode as UTF-16BE when they carry the BOM, otherwise as Latin-1 (a
  close-enough stand-in for PDFDocEncoding); leading/trailing whitespace trimmed.
- Malformed or hostile trees can't hang or OOM: a visited-set breaks cycles, and
  hard caps bound the walk (10 000 items, depth 32).
- Never panics.

**Cost & threading** — walks the outline and the page tree (to map page object
refs to indices); no rasterization. Pure and thread-safe; `PdfView` runs it once
in the off-thread load.

---

## `page_links`

```rust
pub fn page_links(doc: &Document) -> Vec<Vec<PdfLink>>
```

Extract the clickable `/Link` annotations for every page.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `doc` | `&Document` | A parsed PDF. |

**Returns** — one `Vec<PdfLink>` per page, indexed by page; pages with no links get
an empty vec (the outer vec always has exactly one entry per page).

**Guarantees & edge cases**

- Handles `/Dest` explicit destination arrays and `/A` actions of type `/URI`
  (external) and `/GoTo` (internal). Other action types, named destinations, and
  empty URIs are skipped.
- **Rotated pages return an empty vec** (their annotation rectangles would need
  rotating to line up with the render); so do degenerate (zero-area) crop boxes.
- `/Rect` is converted from PDF user space (bottom-left origin) to the normalized
  top-left-origin crop-box coordinates of [`PdfLink`](#struct-pdflink).
- Never panics.

**Cost & threading** — one pass over each page's annotation array plus a page-tree
walk; no rasterization. Pure and thread-safe.

---

## `struct Highlight`

*(`markup` feature)*

```rust
pub struct Highlight {
    pub id: u64,           // host identifier, echoed back on click
    pub page: usize,       // 0-based page the quote is on
    pub quote: String,     // the text to locate (case-/whitespace-insensitive)
    pub occurrence: usize, // which occurrence on the page (0-based)
    pub color: Hsla,       // fill color; drawn translucent (alpha overridden)
}
```

A highlight to draw on the PDF, located by its quote. Hand these to
[`PdfView::set_highlights`](#pdfviewset_highlights); the viewer finds the quote via
the text layer and draws a translucent box over each line it spans (`color` at
alpha 0.35 normally, 0.6 while flashing). `Clone`.

---

## `type HighlightClickFn`

*(`markup` feature)*

```rust
pub type HighlightClickFn = Rc<dyn Fn(u64, &mut Window, &mut gpui::App)>;
```

Invoked with a [`Highlight`](#struct-highlight)'s `id` when the user clicks it.
Install via [`PdfView::set_on_highlight`](#pdfviewset_on_highlight).

---

## `type CreateHighlightFn`

*(`markup` feature)*

```rust
pub type CreateHighlightFn =
    Rc<dyn Fn(usize, String, usize, SharedString, &mut Window, &mut gpui::App)>;
```

Invoked when the user finishes a drag-selection in highlight mode, with `(page,
quote, occurrence, color_label, window, app)`: the 0-based page, the selected
one-line quote, which occurrence of it on the page (so it re-locates
unambiguously), and the label of the picked palette color (the opaque tag from
[`set_highlight_palette`](#pdfviewset_highlight_palette), for the host to store).
Install via [`PdfView::set_on_create_highlight`](#pdfviewset_on_create_highlight).

---

## `struct NormRect`

*(`markup` feature)*

```rust
pub struct NormRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
```

A rectangle in normalized page coordinates: each component is a fraction (0..1) of
the page's width/height, origin at the top-left. Resolution- and zoom-independent —
multiply by the on-screen page rect at paint time. `Clone + Copy + Debug +
PartialEq`.

---

## `struct NormPoint`

*(`markup` feature)*

```rust
pub struct NormPoint {
    pub x: f32,
    pub y: f32,
}
```

A point in normalized page coordinates (0..1 of width/height, top-left origin).
`Clone + Copy + Debug + PartialEq`.

---

## `struct Selection`

*(`markup` feature)*

```rust
pub struct Selection {
    pub quote: String,       // the selected text, as a single-line quote
    pub occurrence: usize,   // which occurrence of that quote on the page
    pub rects: Vec<NormRect>,// one rect per line, to draw while selecting
}
```

The result of a drag selection ([`PageText::select`](#pagetextselect)). The quote is
single-spaced and trimmed (line breaks and gaps become single spaces), so it stores
cleanly and re-[locates](#pagetextlocate) with the whitespace-insensitive matcher.
`Clone + Debug`.

---

## `extract_page_text`

*(`markup` feature)*

```rust
pub fn extract_page_text(doc: &Document, index: usize) -> Option<PageText>
```

Extract the text layer of one page by running a non-rasterizing hayro interpret
pass with a glyph-collecting device — no heavyweight PDF library, only `kurbo`
geometry.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `doc` | `&Document` | A parsed PDF. |
| `index` | `usize` | 0-based page index. |

**Returns** — `Some(PageText)`, or `None` if the page doesn't exist. A page that
exists but has no text still returns `Some` (check
[`is_empty`](#pagetextis_empty)).

**Guarantees & edge cases**

- Records **all** glyphs, including invisible ones — that's the searchable OCR text
  layer over scanned page images. Glyphs with no unicode mapping are skipped (they
  can't match a quote).
- The PDF's own whitespace glyphs are kept (normalized to one space) as word
  boundaries, so a font's intra-word letter-spacing isn't mistaken for a space.
- Never panics.

**Cost & threading** — cheaper than rendering, but it still parses and interprets
the whole page: run it off-thread and **cache the result** (`PdfView` does both).
Pure and thread-safe.

---

## `struct PageText`

*(`markup` feature)*

```rust
pub struct PageText { /* private */ }
```

A page's extracted text: the glyph runs in draw order, plus a lowercased,
whitespace-stripped index for robust quote matching. Built by
[`extract_page_text`](#extract_page_text). All coordinates in and out are
normalized (0..1, top-left origin).

### `PageText::is_empty`

```rust
pub fn is_empty(&self) -> bool
```

Whether the page has any extractable text — `true` for pure scans with no OCR
layer (the host should then fall back to area markup). **Parameters** — none
(`&self`).

### `PageText::text`

```rust
pub fn text(&self) -> String
```

A readable reconstruction of the page text: spaces inserted on horizontal gaps,
newlines on baseline changes. For search or display —
[`locate`](#pagetextlocate) uses the whitespace-insensitive index instead, so
don't feed this back into it expecting exact offsets. **Parameters** — none
(`&self`). Empty string for an empty page.

### `PageText::locate`

```rust
pub fn locate(&self, needle: &str, occurrence: usize) -> Vec<NormRect>
```

Locate a quote on the page.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `needle` | `&str` | The quote. Matched **case- and whitespace-insensitively** (both sides are lowercased with all whitespace removed), so a quote survives PDF spacing quirks. |
| `occurrence` | `usize` | Which match to return, 0-based. |

**Returns** — one [`NormRect`](#struct-normrect) **per line the match spans** (a
wrapped quote highlights as multiple line boxes), or empty if the quote (or that
occurrence of it) isn't on the page.

**Guarantees & edge cases**

- Empty or all-whitespace `needle` → empty vec.
- Occurrences are counted with overlapping starts (successive scans begin one byte
  after the previous hit).
- Runs on the same baseline merge into one rect per line; lines are returned
  top-to-bottom. Never panics (multi-byte characters are handled byte-aligned).

### `PageText::find_matches`

*(`search` feature)*

```rust
pub fn find_matches(&self, needle: &str) -> Vec<Vec<NormRect>>
```

Every **non-overlapping** case- and whitespace-insensitive match of `needle` on the
page, each as one rect per line it spans — the building block for find-in-PDF.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `needle` | `&str` | The query; same normalization as [`locate`](#pagetextlocate). |

**Returns** — matches in reading order; empty for an empty query or no hits.

### `PageText::select`

```rust
pub fn select(&self, from: NormPoint, to: NormPoint) -> Option<Selection>
```

Resolve a drag from `from` to `to` into a [`Selection`](#struct-selection): the
run range between the glyphs nearest each endpoint (draw order ≈ reading order),
its one-line quote, the occurrence index, and the rects to draw.

**Parameters**

| Name | Type | Description |
| --- | --- | --- |
| `from` | [`NormPoint`](#struct-normpoint) | Drag start, normalized page coords. |
| `to` | [`NormPoint`](#struct-normpoint) | Drag end. Order doesn't matter — endpoints are sorted. |

**Returns** — `Some(Selection)`, or `None` if the page has no text or the
selection resolves to only whitespace.

**Guarantees & edge cases**

- "Nearest" has no distance limit: a drag in the page margin still snaps to the
  closest glyphs. The caller decides what counts as a deliberate drag (`PdfView`
  requires ≥ 0.005 of the page in either axis).
- The quote is single-spaced and trimmed; `occurrence` counts earlier identical
  quotes on the page, so the stored highlight re-locates to the right match.
