---
title: Search
description: 'Full-text trigram search across titles and content, with type-aware results for pages, PDFs, images, and whiteboards, prefix and chip filters, and find-in-page.'
---

Search in Zorite is a trigram FTS5 index over the titles and content of your
notes. It stays fast as your notes grow — about a tenth of a millisecond per
keystroke at fifty thousand pages.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-search-light.png" />
  <img
    src="/zorite/screenshots/zorite-search-dark.png"
    alt="Zorite search results — type-aware results spanning pages, PDFs, images, and whiteboards with filter chips"
  />
</picture>

## Type-aware results

Results aren't just pages. They also surface the **PDF and image files** and the
**whiteboards** referenced in your notes — so a search turns up the diagram you
sketched and the PDF you annotated, not just pages.

## Filtering

Narrow the results two ways:

- A **prefix** in the query — `pdf:`, `img:`, `wb:`, or `page:` — restricts to
  that one type.
- A **chip** in the results pane — one per type, each showing a live count —
  toggles that type on or off.

| Prefix | Restricts to |
|---|---|
| `page:` | Pages |
| `pdf:` | PDF files |
| `img:` | Images |
| `wb:` | Whiteboards |

## Find in page

Separate from global search, **find in page** searches the rendered text of the
current note (or PDF), with a highlight and a running match count — for pinning
down a phrase inside the document you're already reading.
