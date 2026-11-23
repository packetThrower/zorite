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

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-allpages-light.png" />
  <img
    src="/zorite/screenshots/zorite-allpages-dark.png"
    alt="The All pages browser with the Journal chip selected — year and month chips above a newest-first list of journal days, each with its first line beside the date"
  />
</picture>

- An **A–Z / 0–9 / #** strip filters by first character — letters with no
  matches dim, and clicking the active letter clears it.
- **Kind chips** (All types / Pages / Whiteboards / PDFs) compose with the
  letter filter.
- The **Journal** chip lists every day you wrote something, newest first, with
  the first line of each day beside its date. A strip of **years** replaces
  the letters; pick a year and a strip of **months** narrows it further.
  Journal days stay out of **All types** so they don't bury your pages.
- Right-click a day for the same menu as a page. **Delete page** removes the
  whole day after a confirmation; opening that date again starts it empty.
- Each row shows a **type badge** and its **created / updated** dates (file
  dates for PDFs), shown in your local time zone.
- The filters and column headers stay **pinned** while the list scrolls.

Clicking a row opens the page, board, PDF, or journal day. The sidebar
calendar (with a dot on every day that has an entry) is another way to browse
your days.

## The graph view

The **Graph** button in the All pages header opens a Logseq-style map: every
page and whiteboard is a node, every `[[wiki-link]]` / `#tag` connection an
edge, laid out by a force simulation — your most-linked hub pages pull into
the middle, small clusters settle around them, and unlinked pages ring the
outside.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-graph-light.png" />
  <img
    src="/zorite/screenshots/zorite-graph-dark.png"
    alt="The graph view in the Nebula style — linked pages glowing in their topic colors, unlinked pages as faint dots around them, and the style picker in the panel"
  />
</picture>

- **Drag** the background (or scroll) to pan, **pinch** or `⌘`-scroll to
  zoom, **drag a node** to reposition it, **click** one to open it, and
  hover to highlight a node's neighborhood. **Right-click** a node for the
  same page menu as everywhere else.
- The panel carries a **legend with live counts** (pages, whiteboards,
  links, orphans), a **search box** that lights up matching nodes and dims
  the rest, and filters: **journal days** (off by default — thousands of day
  nodes swamp the map), **orphan pages**, and **whiteboards**.
- **Reset graph** re-runs the layout with a fresh camera fit.
- Click the panel's title bar to retract it to just that bar (and again to
  bring it back); Zorite remembers which way you left it.

### Graph styles

The row of names at the top of the panel switches how the graph looks; the
nodes glide to their new places, and Zorite remembers your pick. With your
system's reduce-motion setting on, the graph jumps straight to the new layout
instead.

- **Classic**: the map described above.
- **Nebula**: linked pages glow in their topic's color, sized by how linked
  they are; unlinked pages drift as faint dust around them.
- **Islands**: each topic is an outlined island with its name; links between
  topics are dashed, and unlinked pages wait in a tray underneath.
- **Focus**: one page in the middle, the pages it links with on an inner ring
  and their links on an outer one. Click a page to bring it to the center;
  click the center to open it.
- **Timeline**: pages sit at the day they first appeared, one lane per topic,
  with links arcing between them. That's the day a page was created, or the
  first journal day that links to it if that's earlier, so imported pages
  keep their history. Unlinked pages run along the bottom.

Topics come from the links alone: pages that link to each other more than to
the rest form one, named after its most-linked page. No tags or folders
needed.

A page counts as an *orphan* only if nothing links to it anywhere — a page
referenced only from (hidden) journal days still shows; it just has no
visible edges until you switch journals on.
