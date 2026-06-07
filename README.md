# zorite

A local-first, Logseq-style **daily journal** for the desktop — but with a
**Word-like** typing experience rather than an outliner. Built in Rust with
[GPUI](https://www.gpui.rs/) + [gpui-component](https://github.com/longbridge/gpui-component)
and a SQLite backend.

> Status: early, but usable. A personal project.

## Features

- **Daily journal feed** — an infinite, reverse-chronological stream of days
  (today on top, older days lazy-loaded as you scroll). Each day is a single
  markdown document.
- **Read / edit per day** — a day renders as formatted markdown until you click
  in to edit it (raw text); click away and it re-renders. No "filling out a form"
  feel — every line is just editable text.
- **`[[wiki-links]]` and `#tags`** — clickable in the rendered view, navigate to
  (and auto-create) pages, and power **backlinks** ("Linked References").
- **Page hierarchy** — name a page `Projects::Tasks` to nest it with `::`; the
  sidebar shows the namespace tree and each page lists its **sub-pages** as an
  index (Logseq-style — the path *is* the title).
- **Page aliases** — a subdued `alias::` field under the page title takes a comma
  list of alternate names, so `[[hen]]` can resolve to your `chicken` page.
- **As-you-type completion** — typing `[[` suggests pages (and offers to create a
  new one), `#` suggests tags, and `{{` suggests template placeholders. Lists
  filter as you type and are capped so they stay manageable with many pages.
- **`/` command palette** — a compact menu: pick **Markdown** (headings, lists,
  to-dos, quotes, code blocks, **tables**, dividers, inline formatting) or
  **Templates**, or insert the current date/time with **`/date`** / **`/time`**.
  Typing filters across everything, so `/table` or `/h1` jumps straight to it.
- **Auto-paired brackets & quotes** — typing `(`, `[`, `{`, `<`, `"`, or `'`
  inserts the matching closer and leaves the caret between; typing the closer
  steps over it. Prose-aware, so it won't mangle contractions like `don't`.
- **Templates** — reusable snippets defined on a `Templates` page, inserted from
  `/` with `{{date}}` / `{{time}}` / `{{title}}` / `{{cursor}}` placeholders.
- **Full-text search** across all notes.
- **Sidebar** — collapses to a slim icon rail; a calendar icon jumps to any date
  (creating the day if needed); the page list shows your recently-viewed pages
  (the rest are a search away).
- **Inline images** — `![](path-or-url)` images render for real; **paste** (Cmd+V)
  or **drag-and-drop** a file to add one (copied into the data dir's `images/`
  folder), and **drag the corner handle to resize** (saved as `{width=N}` in the
  markdown).
- **PDF viewer** — link a PDF with `[[file.pdf]]` or `![](file.pdf)`, or **drop a
  PDF onto a note**, to get a chip that opens it in a dedicated viewer tab.
  Rendering uses the pure-Rust [`hayro`](https://crates.io/crates/hayro) crate (no
  native dependencies). The viewer is **page-virtualized**: only the pages near the
  viewport are rasterized, and pages scrolled away are freed (memory *and* GPU
  texture), so an 800-page document stays as light as a one-pager. Dropped PDFs are
  copied into the data dir's `pdf/` folder.
- **Markdown rendering** — headings, bold/italic/code, lists, quotes, GFM tables,
  and strikethrough — via a custom renderer crate,
  [`gpui-markdown`](crates/gpui-markdown/README.md).
- **Local SQLite** storage for notes; images and PDFs live beside it as files.
  Everything stays on your machine.

## Templates

Create a page named `Templates` and define snippets with `!name` headers — every
line under a `!name` (until the next `!name`) is that template's body:

```text
!meeting
## Meeting {{date}}
- Attendees:
- Notes: {{cursor}}

!standup
- Yesterday:
- Today:
- Blockers:
```

Then type `/meeting` in any day or page to insert it. Placeholders expand on insert:
`{{date}}`, `{{time}}`, `{{title}}` (the current page/day), and `{{cursor}}` (where
the caret lands). Built-in markdown commands live in
[`gpui-markdown`](crates/gpui-markdown/README.md) as `SNIPPETS`.

## Themes

zorite ships several built-in themes (Zorite, Nord, Solarized, Dracula, Tokyo
Night, Foundry, Cyberpunk, E-Ink), each with a light and dark variant (Cyberpunk
is dark-only). Open **Settings** (the ⚙ in the
title bar) to pick a theme and choose **Light / Dark / Auto** (Auto follows your
system appearance). A quick light/dark toggle also lives in the title bar.

### Your own themes

Drop a `.json` file in your themes folder (Settings → **Reveal themes folder**,
i.e. `~/Library/Application Support/zorite/themes/`) and click **Reload**. Any
color you omit falls back to the base palette, so a theme can be just a few
colors:

```json
{
  "id": "midnight",
  "name": "Midnight",
  "dark": {
    "bg_window": "#0d1117",
    "bg_sidebar": "#161b22",
    "bg_content": "#0d1117",
    "fg": "#e6edf3",
    "accent": "#ff7b72",
    "tag": "#d2a8ff",
    "code": "#79c0ff"
  },
  "light": { "accent": "#0969da" }
}
```

Tokens (each `#RRGGBB`): `bg_window`, `bg_sidebar`, `bg_content`, `fg` (text),
`accent`, `tag`, `code`. Provide a `dark` and/or `light` block. Add
`"dark_only": true` for an always-dark theme — the `light` block is ignored and
the window chrome (titlebar) stays dark regardless of the Light/Dark/Auto setting.

## Build & run

Requires a recent Rust toolchain.

```sh
cargo run
```

The first build compiles GPUI from source and takes a while; later builds are fast.
Run the tests with `cargo test --workspace`.

Your data lives at:

| OS      | Path                                          |
| ------- | --------------------------------------------- |
| macOS   | `~/Library/Application Support/zorite/zorite.db` |
| Linux   | `$XDG_DATA_HOME/zorite/` (or `~/.local/share/zorite/`) |
| Windows | `%APPDATA%\zorite\`                             |

## Workspace layout

```
zorite/
├── src/                     the app — journal feed, pages, search, slash menu, SQLite
└── crates/gpui-markdown/    a reusable Markdown renderer for GPUI (clickable links)
```

`gpui-markdown` is host-agnostic; see its [README](crates/gpui-markdown/README.md).

## Performance

zorite stays responsive with large note collections. The numbers below come from
synthetic databases built by [`scripts/gen_perf_db.py`](scripts/gen_perf_db.py)
— a 3-level `Area::Topic::Note` namespace tree with `[[wiki-links]]`, inline
images, and a couple weeks of journal days. The `ZORITE_DB` environment variable
points the app at a throwaway database, so your real notes are never touched:

```sh
python3 scripts/gen_perf_db.py 10000 /tmp/zorite-perf.db
ZORITE_DB=/tmp/zorite-perf.db cargo run
```

**Hot-path query timings** (SQLite, best of several runs on a development Mac):

| Operation                                  | 1,000   | 10,000  | 50,000  |
| ------------------------------------------ | ------- | ------- | ------- |
| Load the page list (`list_pages`)          | 0.3 ms  | 4.6 ms  | 28 ms   |
| Search (substring `LIKE`, per keystroke)   | 0.4 ms  | 4.4 ms  | 23 ms   |
| Backlinks for a page (indexed)             | 0.01 ms | 0.01 ms | 0.01 ms |
| Seed the recent list (first launch only)   | 0.1 ms  | 2.3 ms  | 12 ms   |

`list_pages` loads only `id`/`title`, not page content — that keeps it ~4× faster
(at 50k pages, 28 ms versus ~103 ms with content) and, just as importantly, keeps
memory flat (below).

**Memory** (resident set size):

| Metric        | Empty DB | 10,000 pages | 50,000 pages |
| ------------- | -------- | ------------ | ------------ |
| RAM (RSS)     | ~86 MB   | ~135 MB      | ~138 MB      |
| Database file | 36 KB    | 14.8 MB      | 74.9 MB      |

RAM barely moves from 10k to 50k: the page list holds only `id`/`title` (~2 MB at
50k), not the 75 MB of note text — bodies load one page at a time as you open
them.

At 50,000 pages, launch, navigation, and scrolling stay immediate, and the
sidebar's cost is independent of the total (it's capped to recently-viewed
pages). The remaining linear cost is search — a `LIKE` scan, where a full-text
index would help at this scale — and the journal feed (all loaded days stay
mounted). Both are on the [roadmap](TODO.md).

## Roadmap

See [TODO.md](TODO.md).

## License

GPL-3.0-or-later.
