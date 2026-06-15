---
title: Import from Logseq
description: 'Bring a Logseq graph into Zorite — pages, journals, assets, namespaces, tasks, properties, aliases, embeds, block-refs, PDF-highlight pages, whiteboards, and favorites — with a non-destructive collision policy.'
---

Already keep a Logseq graph? **File → Import from Logseq…** brings it in. Point
it at your graph directory and Zorite walks the pages, journals, and assets and
converts them to native notes.

## What converts

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

Import is **non-destructive**. Existing content is kept — if a page with the
same name already exists, the import **appends** to it rather than overwriting.
Run it as often as you like — it won't clobber notes you've already written.
