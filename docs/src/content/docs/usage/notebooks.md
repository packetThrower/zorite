---
title: Notebooks
description: 'Keep more than one set of notes — self-contained data folders you switch between, each with its own database, attachments, settings, and theme.'
---

A **notebook** is one complete set of notes: a folder holding its own
database, images, PDFs, themes, and fonts. Most people need exactly one — and
until you add a second, Zorite behaves as if the feature weren't there. But if
you want work and personal notes apart, or a notebook that lives in a synced
folder like Dropbox, you can keep several and switch between them.

Everything is per-notebook: pages, journals, whiteboards, favorites, settings,
theme, even the password. Switching notebooks is switching worlds.

## Switching

The chip at the **bottom of the sidebar** names the notebook you're in. Click
it for the switcher: every registered notebook (a ✓ marks the current one),
and **Add notebook…** at the bottom. Picking another notebook relaunches
Zorite into it — an encrypted notebook lands on its unlock screen, exactly
like a normal launch.

The window title shows which notebook you're in once more than one is
registered.

## Adding a notebook

**Add notebook…** (in the chip's switcher, or Settings → Notebooks) opens a
folder picker:

- Pick an **empty folder** and Zorite starts a fresh, empty notebook there.
- Pick a folder that **already contains a Zorite database** — a notebook from
  another machine, a synced folder, a backup — and it opens as-is.

Either way you confirm before the relaunch.

## Managing

Each notebook row (in the switcher, or as full rows under **Settings →
Notebooks**) offers:

- **Rename** — a display name; the folder on disk is never renamed. The name
  travels with the folder (it's saved inside it), so a renamed notebook keeps
  its name if you remove and re-add it, or open it on another machine.
- **Reveal** — show the folder in your file manager.
- **Remove from list** — forgets the entry. The folder and everything in it
  stay on disk; add it back any time.

Settings → Notebooks also carries the **Data location** card: it shows the
current notebook's folder and can **move** its data to a different folder
(the move runs on the next launch). A folder that already holds a database
can't be a move target — that's a notebook; add it from the switcher instead.

## Where the data lives

Each notebook is fully self-contained — the folder is the backup unit. Copy
it, sync it, or zip it and the whole notebook goes along: database,
attachments, themes, and settings. The list of registered notebooks itself is
the only thing stored outside, in a small pointer file in the platform's
default data directory.
