---
title: Themes
description: 'Zorite ships several built-in themes with light and dark variants, a Light / Dark / Auto mode, and a custom .json theme format you can drop into your themes folder.'
---

Zorite ships several built-in themes, each with a light and a dark variant:
**Zorite**, **Nord**, **Solarized**, **Dracula**, **Tokyo Night**, **Foundry**,
**Cyberpunk** (dark-only), and **E-Ink**.

## Picking a theme

Open **Settings** (the ⚙ in the title bar) to choose a theme and set the mode:

- **Light** — always the light variant.
- **Dark** — always the dark variant.
- **Auto** — follows your system appearance.

A quick light/dark toggle also lives in the title bar for switching on the fly.

## Custom themes

Drop a `.json` file in your themes folder (**Settings → Reveal themes folder**)
and click **Reload**. Any colour you omit falls back to the base palette, so a
theme can be just a few colours:

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

### Tokens

Each value is a `#RRGGBB` hex colour:

| Token | Controls |
|---|---|
| `bg_window` | The window background |
| `bg_sidebar` | The sidebar background |
| `bg_content` | The note / content background |
| `fg` | Text |
| `accent` | Accent colour (links, highlights, selection) |
| `tag` | `#tag` colour |
| `code` | Inline code and code blocks |

Provide a `dark` and/or a `light` block. Add `"dark_only": true` for a theme
that should always render dark, regardless of the chosen mode.
