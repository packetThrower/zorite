---
title: Themes
description: 'Zorite ships several built-in themes with light and dark variants, a Light / Dark / Auto mode, and a custom .json theme format you can drop into your themes folder.'
---

Zorite ships several built-in themes, each with a light and a dark variant:
**Zorite**, **Nord**, **Solarized**, **Dracula**, **Tokyo Night**, **Foundry**,
**Cyberpunk** (dark-only), **CRT (Green Phosphor)** (dark-only), and **E-Ink**.

## Picking a theme

Open **Settings** (the ⚙ in the title bar) to choose a theme and set the mode:

- **Light** — always the light variant.
- **Dark** — always the dark variant.
- **Auto** — follows your system appearance.

A quick light/dark toggle also lives in the title bar for switching on the fly.

## Custom themes

Drop a `.json` file in your themes folder (**Settings → Reveal themes folder**)
and click **Reload**. This example shows every option — only `id` and `name`
are required. Any colour you omit falls back to the base palette (so a theme
can be just a few colours), and `font` / `dark_only` can be left out entirely:

```json
{
  "id": "midnight",
  "name": "Midnight",
  "font": "JetBrains Mono",
  "dark_only": false,
  "dark": {
    "bg_window": "#0d1117",
    "bg_sidebar": "#161b22",
    "bg_content": "#0d1117",
    "fg": "#e6edf3",
    "accent": "#ff7b72",
    "tag": "#d2a8ff",
    "code": "#79c0ff"
  },
  "light": {
    "bg_window": "#f6f8fa",
    "bg_sidebar": "#eaeef2",
    "bg_content": "#ffffff",
    "fg": "#1f2328",
    "accent": "#0969da",
    "tag": "#8250df",
    "code": "#953800"
  }
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

### Advanced tokens

Everything else in the UI derives from the base tokens above — text tiers
come from `fg` at reduced opacity, borders and hover washes from a white or
black overlay, and the accent's hover/active/tint variants from `accent`.
Any of them can also be pinned directly in a `dark` / `light` block. These
accept `#RRGGBBAA` too (the derived defaults are mostly translucent):

| Token | Controls |
|---|---|
| `text_primary` | Main text (default: `fg` at 92% opacity) |
| `text_secondary` | Muted text, e.g. sidebar sections (default: `fg` at 60%) |
| `text_tertiary` | Faint text, e.g. placeholders, markers (default: `fg` at 40%) |
| `elevated` | Raised cards / panels (settings cards) |
| `glass` | Translucent panel wash (code blocks, chips) |
| `glass_strong` | Stronger panel wash |
| `hover` | Row / button hover wash |
| `border_subtle` | Hairline borders |
| `divider` | Visible rules, e.g. between journal days |
| `accent_hover` | Accent when hovered |
| `accent_active` | Accent when pressed |
| `accent_tint` | Translucent accent wash (selection highlights) |
| `alert_note` | `> [!NOTE]` alert border + title (default: GitHub blue) |
| `alert_tip` | `> [!TIP]` (default: GitHub green) |
| `alert_important` | `> [!IMPORTANT]` (default: GitHub purple) |
| `alert_warning` | `> [!WARNING]` (default: GitHub yellow) |
| `alert_caution` | `> [!CAUTION]` (default: GitHub red) |

### Fonts

A theme can name a typeface with a top-level `"font": "Family Name"`. It's a
reference, not a bundled file — the family must be installed on your system
or added via **Settings → Appearance → Font → Add font file…**; an unknown
name falls back to the default font. Your own Font setting, when not
Default, always wins over the theme's.
