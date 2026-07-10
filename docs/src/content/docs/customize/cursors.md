---
title: Mouse cursors
description: 'Pick a cursor theme — the bundled Bibata set, a theme-reactive variant that re-colors with your skin, or any XCursor cursor theme you drop in.'
---

Zorite can restyle the mouse cursor inside its windows — the arrow, the text
I-beam, the pointing hand, resize arrows, all of it. Outside Zorite your
system cursors are untouched.

## Picking a cursor theme

**Settings → Appearance → Mouse cursor** offers:

- **System (default)** — your OS cursors, unchanged.
- **Bibata-Catppuccin-Mocha** — the bundled
  [Bibata](https://github.com/ful1e5/Bibata_Cursor) set in Catppuccin Mocha.
- **Bibata (match theme)** — the same cursor shapes, **re-colored live from
  your active theme**: the body takes the accent color, the outline the text
  color. Switch skins or light/dark — including your own custom `.json`
  themes — and the cursors follow instantly.
- Any theme you've added (see below), with an extra **"(match theme)"** entry
  when the pack carries SVG sources.

Changes apply immediately on macOS and Windows; on Linux a change takes
effect the next time Zorite starts (cursor themes there are read by the
display connection at launch).

## Adding your own

Cursor packs work like fonts: click **Add cursor theme…** and pick a theme
folder, or use **Reveal cursors folder** and drop folders in yourself. The
selection is **per-notebook** and travels with your data folder.

Two pack formats are accepted (a folder can carry both):

### XCursor themes (ready-made)

The standard Linux cursor-theme format — a folder with a `cursors/`
directory inside (usually alongside an `index.theme`). Thousands of finished
themes exist; pick the theme's folder and it imports as a fixed-color pack.
Hotspots come from the files themselves.

### SVG packs (theme-reactive)

A pack that also contains an **`svg/` folder** renders theme-reactively, like
the bundled Bibata. One SVG per cursor, named by the standard cursor names —
`default.svg`, `text.svg`, `pointer.svg`, `grabbing.svg`, `ew-resize.svg`,
and so on — using Bibata's color-slot convention:

| Slot color | Becomes |
| --- | --- |
| `#00FF00` | the cursor body → your theme's **accent** |
| `#0000FF` | the outline → your theme's **text color** |
| `#FF0000` | accents (watch hands, etc.) → the accent |

Since upstream [Bibata](https://github.com/ful1e5/Bibata_Cursor) publishes
its SVG sources in exactly this convention, they're a ready starting point
for your own shapes.

Hotspots (the pixel that actually clicks) resolve from the pack's raster
`cursors/` files when present; an SVG-only pack instead includes a
`hotspots.json` with coordinates on a 64px grid:

```json
{ "default": [13, 5], "text": [30, 32], "pointer": [22, 8] }
```

## Platform notes

- **macOS** — all cursor shapes are themed, at Retina sharpness.
- **Windows** — Windows shares one cursor across several roles (all
  horizontal resizes are one cursor, for example), so themes apply at that
  granularity.
- **Linux** — works on X11 and Wayland through the system's own cursor-theme
  mechanism; changes take effect on the next launch.
