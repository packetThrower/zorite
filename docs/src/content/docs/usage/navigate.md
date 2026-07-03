---
title: All pages & the graph
description: 'Browse every page, whiteboard, and PDF in one filterable index, and see your notes as an interactive Logseq-style graph.'
---

Two views for finding things when you don't remember where they live: a
filterable **All pages** index, and a **graph view** of every page and the
links between them.

## The All pages browser

The list icon in the sidebar toolbar opens **All pages**: every named page,
whiteboard, and stored PDF in one index.

- An **A–Z / 0–9 / #** strip filters by first character — letters with no
  matches dim, and clicking the active letter clears it.
- **Kind chips** (All types / Pages / Whiteboards / PDFs) compose with the
  letter filter.
- Each row shows a **type badge** and its **created / updated** dates (file
  dates for PDFs), shown in your local time zone.
- The filters and column headers stay **pinned** while the list scrolls.

Clicking a row opens the page, board, or PDF. Journal days are deliberately
excluded — the sidebar calendar (with a dot on every day that has an entry)
is their browser.

## The graph view

The **Graph** button in the All pages header opens a Logseq-style map: every
page and whiteboard is a node, every `[[wiki-link]]` / `#tag` connection an
edge, laid out by a force simulation — your most-linked hub pages pull into
the middle, small clusters settle around them, and unlinked pages ring the
outside.

- **Drag** the background (or scroll) to pan, **pinch** or `⌘`-scroll to
  zoom, **drag a node** to reposition it, **click** one to open it, and
  hover to highlight a node's neighborhood.
- The panel carries a **legend with live counts** (pages, whiteboards,
  links, orphans), a **search box** that lights up matching nodes and dims
  the rest, and filters: **journal days** (off by default — thousands of day
  nodes swamp the map), **orphan pages**, and **whiteboards**.
- **Reset graph** re-runs the layout with a fresh camera fit.

A page counts as an *orphan* only if nothing links to it anywhere — a page
referenced only from (hidden) journal days still shows; it just has no
visible edges until you switch journals on.
