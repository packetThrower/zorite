---
title: Import from Obsidian or Logseq
description: 'Bring an Obsidian vault or a Logseq graph into Zorite — pages, journals, assets, namespaces, properties, aliases, embeds, block references, canvas boards / whiteboards, and favorites — with a non-destructive collision policy.'
---

Already keep your notes in Obsidian or Logseq? **File → Import from Obsidian…**
/ **Import from Logseq…** brings them in. Point the dialog at your vault or
graph directory and Zorite walks the pages, journals, and assets and converts
them to native notes.

## From Obsidian

| From Obsidian | Becomes in Zorite |
|---|---|
| Notes (folders) | Pages — folders become `Foo::Bar` namespaces, or flatten (your choice in the dialog) |
| `[[Links]]` and `[[Links\|aliases]]` | Wiki-links, resolved through a name→title map |
| `[[Note#Heading]]` / `[[Note#^id]]` | Kept as-is — Zorite jumps to headings and blocks |
| ` ^block-id` markers | Kept as-is — they're link targets in Zorite too |
| `![[Note]]` transclusions | Real embeds (moved onto their own line where needed) |
| `![[image.png]]` embeds | Inline images (assets copied into the managed store) |
| Callouts (~13 types, incl. foldable) | Zorite's five alerts |
| `==highlights==` / `%%comments%%` | `<mark>` highlights / dropped |
| YAML frontmatter | `aliases:` → page aliases, `tags:` → `#tags`, other keys → `key:: value` properties |
| Daily notes (`YYYY-MM-DD`) | Journal days |
| **`.canvas` boards** | Native whiteboards — text cards as labeled boxes, note cards as clickable page cards, images placed, groups as outlines, edges as arrows with their labels |

Anything that can't map one-to-one is downgraded gracefully and listed in the
import summary.

## From Logseq

| From Logseq | Becomes in Zorite |
|---|---|
| Pages and journals | Pages and journal days |
| Assets (images, PDFs) | Files in the data directory, linked from notes |
| `Foo/Bar` namespaces | `Foo::Bar` namespaces and sub-pages |
| Tasks (`TODO` / `DOING` / `DONE`) | Markdown to-dos |
| Page and block properties | Note properties (including `alias::`) |
| Aliases | Page aliases |
| Embeds and block-refs | Resolved into the imported notes |
| `hls__*` PDF-highlight pages | PDF highlights with note↔PDF jump links |
| tldraw whiteboards | Native Zorite whiteboards — images and all |
| Favorites | Your Favorites group |

Namespaces, tasks, properties, aliases, embeds, block-refs, and the special
`hls__*` highlight pages are all handled, so the graph lands as working notes,
not raw Markdown.

## Collision policy

Both importers are **non-destructive**. Existing content is kept — if a page
with the same name already exists, the import **appends** to it rather than
overwriting. Run one as often as you like — it won't clobber notes you've
already written.
