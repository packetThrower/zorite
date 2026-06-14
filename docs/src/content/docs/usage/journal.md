---
title: Journal & pages
description: 'The daily-journal feed, click-to-edit Markdown, wiki-links and backlinks, tags, namespaces and sub-pages, aliases, the slash command palette, templates, and auto-pairing.'
---

The journal is the heart of Zorite: an infinite, reverse-chronological stream of
days, today on top, older days lazy-loaded as you scroll. Each day is a single
Markdown document — there's no "filling out a form" feel, just editable text.

## The daily feed

Open Zorite and you land on the journal. Scroll down and previous days load on
demand; the feed never ends. The journal is the pinned first tab, so it's always
one click away no matter what else you have open.

Each day is one Markdown document. To edit it, click in (or right-click →
**Edit**) to switch that day to its raw text; click away and it re-renders.
Clicking the rendered page drops the caret exactly where you clicked, so editing
feels like a word processor, not a sequence of bullet points.

![Editing a journal day — raw Markdown on click, rendered on click-away](/zorite/screenshots/zorite-edit-dark.png)

## Pages, wiki-links, and backlinks

Any note can link to any other with `[[wiki-links]]`. In the rendered view a
wiki-link is clickable; following one navigates to that page, **creating it** if
it doesn't exist yet. The reverse direction is automatic: every page shows a
**Linked References** panel listing the places that link to it, so structure
emerges from your writing rather than from manual filing.

`#tags` work the same way — clickable in the rendered view, each one its own
page with its own backlinks.

## Namespaces and sub-pages

Name a page with `::` to nest it. A page called `Projects::Tasks` lives under
`Projects`: the sidebar shows the namespace tree, and each page lists its
sub-pages. Namespaces are just naming — you don't move files around, you name
the page and the hierarchy follows.

## Aliases

A subdued `alias::` field on a page takes alternate names for it. Give your
`chicken` page `alias:: hen` and a `[[hen]]` link anywhere resolves to it. Handy
for abbreviations, plurals, and renamings you don't want to chase down.

## The `/` command palette

Type `/` anywhere in an editable note to open a compact menu. Pick a **Markdown**
construct — headings, lists, to-dos, quotes, code blocks, **tables**, dividers,
or inline formatting — or a **Template**, or insert the current date or time with
`/date` and `/time`. Typing filters across everything; click an item or press
Enter to insert it.

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

- `[[` suggests pages — and offers to **create** one if no match exists.
- `#` suggests tags.
- `{{` suggests template placeholders.

Brackets and quotes **auto-pair**, and the pairing is prose-aware, so an
apostrophe in `don't` is left alone rather than turned into a pair.

## Markdown & diagrams

The rendered view is CommonMark + GFM: headings, **bold** / *italic* / `code`,
lists, quotes, tables, ~~strikethrough~~, and `<mark>` highlights. **Mermaid
diagrams** (flowchart, sequence, class) render pure-Rust, themed to your skin;
click one to expand it.

![A note with a rendered Mermaid diagram](/zorite/screenshots/zorite-mermaid-dark.png)

**Find in page** searches the rendered text of the current note, with a
highlight and a match count — see [Search](/zorite/usage/search/).

## Tabs & multiple windows

Open pages, PDFs, and boards in tabs; the journal is the pinned first tab.
**Drag a tab** to reorder it, drop it on another window to move it there, or
release it on empty space to **tear it off into a new window**. Every window is
independent, and edits **sync live** across all of them.

## Sidebar

The sidebar collapses to a slim icon rail. It carries a **Favorites** group
(right-click a page → **Add to favorites**), a calendar that jumps to any date,
collapsible sections, and a recently-viewed page tree with namespace nesting.
