# rumin

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
- **`/` command palette** — a compact two-level menu: pick **Markdown** (headings,
  lists, to-dos, quotes, code blocks, **tables**, dividers, inline formatting) or
  **Templates**. Typing filters across everything, so `/table` or `/h1` jumps
  straight to it.
- **Templates** — reusable snippets defined on a `Templates` page, inserted from
  `/` with `{{date}}` / `{{time}}` / `{{title}}` / `{{cursor}}` placeholders.
- **Full-text search** across all notes.
- **Markdown rendering** — headings, bold/italic/code, lists, quotes, GFM tables,
  strikethrough, and images-as-links — via a custom renderer crate,
  [`gpui-markdown`](crates/gpui-markdown/README.md).
- **Local SQLite** storage; everything stays on your machine.

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
| macOS   | `~/Library/Application Support/rumin/rumin.db` |
| Linux   | `$XDG_DATA_HOME/rumin/` (or `~/.local/share/rumin/`) |
| Windows | `%APPDATA%\rumin\`                             |

## Workspace layout

```
rumin/
├── src/                     the app — journal feed, pages, search, slash menu, SQLite
└── crates/gpui-markdown/    a reusable Markdown renderer for GPUI (clickable links)
```

`gpui-markdown` is host-agnostic; see its [README](crates/gpui-markdown/README.md).

## Roadmap

See [TODO.md](TODO.md).

## License

GPL-3.0-or-later.
