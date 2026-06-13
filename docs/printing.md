# Printing & export

> Status: **design** — not yet implemented. Pairs with the roadmap line in
> [TODO.md](../TODO.md) ("Print / PDF export"). Captures the research and the two
> open decisions (HTML-first vs PDF-first; whether to extract a crate).

Printing a page or journal entry: turn the note's markdown into a paginated
document and let the OS print it.

---

## The constraint: GPUI can't print, and shouldn't

GPUI renders straight to the GPU (wgpu/Metal). It exposes **no print path** — a
whole-Zed grep for `NSPrintOperation` returns zero hits; if Zed never wired up
printing, there's nothing in the framework to build on. The only window→image
API (`render_to_image`) is `#[cfg(test)]`-gated (compiled out of our build) and
single-viewport.

That's fine, because **the on-screen view is the wrong thing to print anyway** —
it's a scrolling editor surface, not pages. The thing worth printing is the
*note*. So printing generates a document from the markdown **outside the render
tree** and hands it to the OS. Not a workaround — the correct design.

---

## How desktop printing actually works (the short version)

A "print job" is a page-description document sent to a print queue, which a
driver converts to the printer's own language. Two facts drive our design:

1. **The currency is PDF, not PostScript.** PostScript is PDF's ancestor; the
   industry moved on two decades ago. macOS is PDF-native (Quartz), and both
   macOS and Linux print via CUPS, which takes **PDF** as the standard job
   format (modern CUPS deprecated the PostScript filter chain). Windows never
   used PostScript natively (GDI/XPS). There's also no good pure-Rust PostScript
   generator. **→ Target PDF; never PostScript.**

2. **You don't build the print dialog — you delegate it.** Choosing a printer,
   copies, margins, duplex, "Save to PDF" is a pile of platform-specific code.
   Hand the document to an app that already has a print dialog:
   - **HTML → the browser** — it lays out *and* prints (print-to-PDF built in).
   - **PDF → the default PDF viewer** (Preview, etc.) — you control layout, it
     owns the dialog.

   Both end by `open`-ing the file and letting that app print. (You *can* shell
   to `lp`/`lpr` to hit the default printer with no dialog — rarely what a user
   wants.)

---

## What we already have

- **The content is in hand** — raw markdown in `pages.content`, via
  `get_page(id)` / `get_page_by_title` ([src/db.rs:285](../src/db.rs:285)), or
  straight off the live editor. A journal entry is a `Page` titled by date.
- **`markdown::to_html` is already compiled in** — the `markdown` v1 crate used
  for `to_mdast` ([gpui-markdown/src/lib.rs:335](../crates/gpui-markdown/src/lib.rs:335))
  also emits HTML. **Markdown → HTML costs zero new dependencies.**
- **A cross-platform "open this file" idiom exists** — `open` / `explorer` /
  `xdg-open` via `std::process::Command` ([src/app.rs:2582](../src/app.rs:2582)).
- No PDF *writer* is vendored — `hayro` is read-only. A true PDF generator is a
  new dependency.

---

## Document target — the recommendation

| Target | New deps | Print quality | Effort | Verdict |
|---|---|---|---|---|
| **HTML → browser** | none (`to_html` is compiled in) | excellent (browser layout + print-to-PDF) | low | **v1 — start here** |
| **PDF → viewer** | a Rust PDF generator (`typst` / `printpdf` / `genpdf`) | full control, self-contained, in-app "Export PDF" | high (hand-built layout) | later, if the browser hop is unacceptable |
| PostScript | (none good in Rust) | — | — | **rejected** (legacy; PDF replaced it) |

**Start with HTML → browser.** Browsers lay out markdown-derived HTML
beautifully, and their print dialog already gives page setup, real printers, and
print-to-PDF — so "Export to PDF" falls out for free, cross-platform, with no new
dependency. Escalate to in-app PDF only if a browser window popping up doesn't
fit the product feel. There is **no good pure-Rust HTML→PDF engine** (weasyprint
is Python, wkhtmltopdf is deprecated C++/Qt, headless Chrome is heavy), so the
PDF path means rebuilding markdown layout from scratch on a PDF library — real
work, deferred.

---

## The real work is fidelity, not plumbing

`to_html` handles standard markdown (headings, lists, tables, code, blockquotes,
links) on day one. The effort is in Zorite's **non-standard extensions**, applied
*after* parsing in the GPUI renderer, which a faithful print must reproduce:

- **`[[wiki-links]]` and `#tags`** — string-scanned post-parse
  ([gpui-markdown/src/lib.rs:939](../crates/gpui-markdown/src/lib.rs:939)); emit
  styled spans (or plain text).
- **`{width=N}` image attributes** — parsed separately
  ([lib.rs:613](../crates/gpui-markdown/src/lib.rs:613)).
- **Images** — rewrite local paths to absolute `file://` URLs or inline as
  data-URIs so the browser/PDF can load them.
- **Mermaid diagrams** — already rasterized via GPUI's `SvgRenderer`
  ([src/mermaid.rs:159](../src/mermaid.rs:159)); emit the SVG inline.
- **Embedded PDFs** (`![](file.pdf)`) — hayro-render page 1 to an image and
  embed it; a browser won't rasterize a linked PDF.

Rough estimate: ~a day for the plain-text/table/code path; another day or two
for images + mermaid + the wiki-link/tag extensions.

---

## Should this be a crate?

**Split decision:**

- **The HTML path: no.** It's one compiled-in function call (`to_html`) + a CSS
  template + *app-specific* extension handling. The reusable core is trivial; the
  rest is Zorite-only. Keep it inline (a `src/print.rs` module).

- **The PDF path: yes, when built.** A pure markdown→PDF crate fills a genuine
  ecosystem gap (today everyone shells out to Chrome/weasyprint), and — unlike
  `gpui-markdown`/`gpui-pdf`, which render *to* gpui — it would have **zero gpui
  dependency**, making it the most reusable crate in the tree (any Rust project
  could use it).

  Design it to **mirror `gpui-markdown`'s contract**: the crate owns the generic
  pipeline (parse mdast → lay out pages → style), and the host injects closures
  for non-standard nodes — `on_wiki_link`, `on_image`, `on_mermaid`, just like
  the screen renderer's builder methods
  ([gpui-markdown/src/lib.rs:258](../crates/gpui-markdown/src/lib.rs:258)). Same
  input (markdown + extension hooks), different output target (paper vs. screen).
  Zorite's quirks stay in the app; the pipeline stays publishable.

  Candidate generators (all GPL-compatible): `typst` (Apache-2.0, gorgeous
  output, large dep), `printpdf` / `genpdf` (MIT, lighter, manual layout).

**Net:** ship the HTML path in-app first; extract a `markdown-print`-style crate
only when you build the PDF path, where reuse actually pays off. Crating the thin
HTML path now would be premature.

---

## Build outline

| Phase | Scope | Verify |
|---|---|---|
| **1 — HTML print (plain)** | `Print…` action in the File menu ([src/actions.rs:124](../src/actions.rs:124)) + `AppView` handler; grab active page markdown; `to_html` → styled HTML shell with print CSS (`@page` margins, fonts, code/table styling); temp file → `open`. | Print a text-only page from the File menu; browser renders it correctly; ⌘P → print-to-PDF works. |
| **2 — Fidelity** | Pre/post-process `[[wiki-links]]`, `#tags`, `{width=N}`; images → data-URIs; mermaid → inline SVG; embedded PDFs → rasterized image. | A page with an image + a mermaid diagram + wiki-links prints faithfully. |
| **3 — Journal** | Print a journal entry (title = date); optionally a date range / "print this week". | A journal day prints with its date as the title. |
| **4 — (optional) in-app PDF** | Extract the `markdown-print` crate (hook-injection contract); add a generator; `Export to PDF…` with a save dialog. | Export a page to a PDF file with no browser hop. |

Phases 1–3 are the shippable feature (and already give print-to-PDF via the
browser). Phase 4 is the heavier, crate-worthy follow-up.

---

## References

- Markdown pipeline & extensions:
  [gpui-markdown/src/lib.rs](../crates/gpui-markdown/src/lib.rs)
  (`to_mdast` `:335`, wiki-link/tag scan `:939`, `{width}` `:613`, builder hooks
  `:258`).
- Content model & fetch: [src/models.rs:7](../src/models.rs:7),
  [src/db.rs:285](../src/db.rs:285).
- Menu/actions: [src/actions.rs:124](../src/actions.rs:124).
- OS-open idiom: [src/app.rs:2582](../src/app.rs:2582).
- SVG-raster precedent (mermaid): [src/mermaid.rs:159](../src/mermaid.rs:159).
- Roadmap line: [TODO.md](../TODO.md) — "Print / PDF export".
