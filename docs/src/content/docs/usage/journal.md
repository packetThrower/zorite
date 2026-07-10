---
title: Journal & pages
description: 'The daily-journal feed, click-to-edit Markdown, wiki-links and backlinks, block references and embeds, properties, tags, namespaces and sub-pages, aliases, the slash command palette, templates, and auto-pairing.'
---

The journal is an infinite, reverse-chronological stream of days — today on top,
older days lazy-loaded as you scroll. Each day is a single
Markdown document — there's no "filling out a form" feel, just editable text.

## The daily feed

Open Zorite and you land on the journal. Scroll down and previous days load on
demand; the feed never ends. The journal is the pinned first tab, so it's always
one click away.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-journal-light.png" />
  <img
    src="/zorite/screenshots/zorite-journal-dark.png"
    alt="The Zorite journal — a reverse-chronological feed of daily Markdown pages"
  />
</picture>

Each day is one Markdown document. To edit it, click in (or right-click →
**Edit**) to switch that day to its raw text; click away and it re-renders.
Clicking the rendered page drops the caret where you clicked — it edits like a
word processor, not an outliner.

## Pages, wiki-links, and backlinks

Any note can link to any other with `[[wiki-links]]`. Typing `[[` completes
against your page titles **and their aliases** (an alias shows as
`alias → Title` and links to its page). In the rendered view a wiki-link is
clickable; following one navigates to that page, **creating it** if it doesn't
exist yet. The reverse direction is automatic: every page shows a
**Linked References** panel listing the places that link to it.

`#tags` work the same way — clickable in the rendered view, each one its own
page with its own backlinks.

## Block references and heading links

A link can point *inside* a page, not just at it. End any line with a
` ^some-id` marker to give it an address, then link to it from anywhere with
`[[Note#^some-id]]`; or skip the marker and link straight to a heading with
`[[Note#My Heading]]` (case-insensitive). Clicking either opens the note and
jumps to that line.

Both links read cleanly — `[[Note#^some-id]]` displays as **Note → some-id**
(an `|alias` still overrides the label) — and the ` ^id` marker itself stays
out of the way: hidden in the reading view, dimmed in the editor until your
caret is on its line. `file.pdf#p3` keeps its page-jump meaning, and a page
whose title literally contains `#` still opens by its full name.

## Embeds (transclusion)

A line holding just `![[Note]]` renders that note's content right there, in a
quoted box with a clickable source label. `![[Note#My Heading]]` embeds one
section, `![[Note#^some-id]]` a single block, and an `|alias` renames the
label. The box shows the real rendered content — images, math, diagrams,
code, even embeds inside embeds — scrolls when it overflows (the wheel hands
back to the page at the edges), and **live-updates** as you edit the source
page. In the editor, put the caret on the line to edit the raw `![[…]]` text.

## Properties

A `key:: value` line anywhere in a note is a **property** — Obsidian/Logseq
style. Consecutive property lines render as a clean two-column panel: a
per-key icon, the muted key, and the value with `#tags` and `[[wiki-links]]`
shown as clickable pills. Any key works — `attendees::`, `status::`,
`time::`, whatever your templates use.

Click the panel (or arrow the caret into it) and it opens an **in-place
editor** that mirrors the rendered panel: edit keys and values directly, pick
from a dropdown of every key already used across your notes, add or remove
rows, and move between fields entirely from the keyboard. Clicking away
writes the `key:: value` lines back.

**All pages → Properties** opens the property index: every key with its icon,
page count, and values — expand a key to see each value and the pages
carrying it, override any key's **icon** from a picker (or map one before the
key's first use), and **rename a key** across every page at once.

## Namespaces and sub-pages

Name a page with `::` to nest it. A page called `Projects::Tasks` lives under
`Projects`: the sidebar shows the namespace tree, each page lists its
sub-pages, and a child page shows a clickable **breadcrumb** back to its
ancestors above the title. Namespaces are just naming — there are no folders
to move things between.

Renaming a namespace **cascades**: rename `Projects` and every `Projects::*`
page is retitled with it, `[[links]]` included (if a child would collide with
an existing title, nothing moves). Right-click a page in the sidebar for
**New sub-page**, which starts the New-page dialog pre-filled with
`Parent::`.

## Aliases

A subdued `alias::` field on a page takes alternate names for it. Give your
`chicken` page `alias:: hen` and a `[[hen]]` link anywhere resolves to it. Handy
for abbreviations, plurals, and renamings you don't want to chase down.

## The `/` command palette

Type `/` anywhere in an editable note to open a compact menu. Pick a **Markdown**
construct — headings, lists, to-dos, quotes, code blocks, **tables**, **math**,
**alerts**, footnotes, dividers, or inline formatting — or a **Template**, or
insert the current date or time with `/date` and `/time`. Typing filters across
everything; click an item or press Enter to insert it.

## Editing tables, lists, and text

Everything you insert stays editable in place:

- **Tables** behave like a small spreadsheet. The arrow keys move from cell to
  cell and keep your column; `Tab` / `Shift+Tab` step through cells, and `Enter`
  drops to the row below. Hover a table for `+` / `−` handles to add or delete a
  row or column, or right-click it for the same plus per-column alignment and
  **Delete table**.
- **Lists and to-dos continue themselves** — press `Enter` and the `-`, `1.`, or
  `- [ ]` marker carries to the next line; press it again on an empty item to end
  the list. Click a to-do's checkbox to toggle it. Numbered lists display
  **Word-style markers by depth** (`1.` → `a.` → `i.`), always counted from 1 —
  indenting with `Tab` starts the nested list over, whatever the raw digits say.
- **Inline formatting** — select text and press `⌘B` / `⌘I` / `⌘E` (`Ctrl` on
  Windows and Linux) to wrap it in bold, italic, or inline code.

## Templates

Create a page named `Templates` and define snippets with `!name` headers. Every
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

Type `/meeting` in any day or page to insert it. Placeholders expand on insert:

| Placeholder | Expands to |
|---|---|
| `{{date}}` | Today's date |
| `{{time}}` | The current time |
| `{{title}}` | The current page or day's title |
| `{{cursor}}` | Where the caret lands after insertion |

## As-you-type completion

Completion menus appear as you type:

- `[[` suggests pages **and whiteboards** — and offers to **create** one if no
  match exists.
- `#` suggests tags.
- `{{` suggests template placeholders.

Brackets and quotes **auto-pair**, and the pairing is prose-aware, so an
apostrophe in `don't` is left alone rather than turned into a pair.

There's also an opt-in **auto-link** (Settings → Markdown): type an existing
page's title and it wraps itself as `[[Title]]` on the next boundary keystroke.
One undo step reverts a wrap you didn't want.

## Markdown & diagrams

The rendered view is CommonMark + GFM: headings, **bold** / *italic* / `code`,
lists, quotes, tables, ~~strikethrough~~, and `<mark>` highlights. **GitHub
alerts** (`> [!NOTE]` through `[!CAUTION]`) render with icons and themeable
colors, and fenced code blocks get **syntax highlighting** for the common
languages. **Mermaid diagrams** (flowchart, sequence, class) render pure-Rust,
themed to your skin; click one to expand it.

Long notes fold: an Obsidian-style fold char makes a **callout collapsible**
(`> [!NOTE]-` starts folded, `+` starts open — click the chevron by the title,
and the state persists in the note), and every **heading** folds too — hover
one and a chevron appears past the text; click it to collapse everything
under that heading up to the next one at its level. Heading folds are view
state, not written into the note, and in the editor arrowing the caret into a
folded section reveals it while you edit.

<picture>
  <source media="(prefers-color-scheme: light)" srcset="/zorite/screenshots/zorite-mermaid-light.png" />
  <img
    src="/zorite/screenshots/zorite-mermaid-dark.png"
    alt="A note with a rendered Mermaid diagram"
  />
</picture>

**Find in page** searches the rendered text of the current note, with a
highlight and a match count — see [Search](/zorite/usage/search/).

## Tabs & multiple windows

Open pages, PDFs, and boards in tabs; the journal is the pinned first tab.
**Drag a tab** to reorder it, drop it on another window to move it there, or
release it on empty space to **tear it off into a new window**. Every window is
independent, and edits **sync live** across all of them.

## Sidebar

The sidebar collapses to a slim icon rail. It carries a **Favorites** group
(right-click a page → **Add to favorites**), a calendar that jumps to any date
(days with entries are **dotted**), an [All pages browser](/zorite/usage/navigate/)
with a graph view, collapsible sections, and a recently-viewed page tree with
namespace nesting.
