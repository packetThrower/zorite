<p align="center">
  <img src="resources/icons/icon.png" alt="Zorite" width="128">
</p>

# Zorite

[![CI](https://img.shields.io/github/actions/workflow/status/packetThrower/zorite/ci.yml?branch=main&style=flat-square&logo=github&label=CI)](https://github.com/packetThrower/zorite/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/packetThrower/zorite?style=flat-square&logo=github&label=release&include_prereleases)](https://github.com/packetThrower/zorite/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/packetThrower/zorite/total?style=flat-square&logo=github&label=downloads)](https://github.com/packetThrower/zorite/releases)
[![winget](https://img.shields.io/badge/winget-packetThrower.Zorite-0078D4?style=flat-square&logo=windows&logoColor=white)](https://github.com/microsoft/winget-pkgs/tree/master/manifests/p/packetThrower/Zorite)
[![Rust](https://img.shields.io/badge/Rust-stable-CE422B?style=flat-square&logo=rust&logoColor=white)](Cargo.toml)
[![gpui](https://img.shields.io/badge/gpui-from%20Zed-1F1F28?style=flat-square)](https://www.gpui.rs/)
[![License: GPL v3+](https://img.shields.io/badge/license-GPLv3%2B-blue?style=flat-square)](LICENSE)

## Minimum OS versions

**macOS** (Apple Silicon and Intel)

[![macOS 11+](https://img.shields.io/badge/macOS-11%2B-333?style=flat-square&logo=apple&logoColor=white)](#install)
[![Apple Silicon](https://img.shields.io/badge/Apple%20Silicon-arm64-333?style=flat-square&logo=apple&logoColor=white)](#install)
[![Intel](https://img.shields.io/badge/Intel-x86__64-333?style=flat-square&logo=apple&logoColor=white)](#install)

**Windows** (x64 and ARM64)

[![Windows 10 21H2+](https://img.shields.io/badge/Windows%2010%2021H2%2B-x64%20%2F%20arm64-0078D4?style=flat-square&logo=windows&logoColor=white)](#install)

**Linux** (amd64 and arm64)

[![Ubuntu 22.04+](https://img.shields.io/badge/Ubuntu-22.04%2B-E95420?style=flat-square&logo=ubuntu&logoColor=white)](#install)
[![Debian 12+](https://img.shields.io/badge/Debian-12%2B-A81D33?style=flat-square&logo=debian&logoColor=white)](#install)
[![Fedora 38+](https://img.shields.io/badge/Fedora-38%2B-294172?style=flat-square&logo=fedora&logoColor=white)](#install)
[![Arch](https://img.shields.io/badge/Arch-1793D1?style=flat-square&logo=archlinux&logoColor=white)](#install)

Linux additionally needs a Vulkan-capable GPU with current Mesa drivers.

A local-first, Logseq-style **daily journal** for the desktop — but with a
**Word-like** typing experience rather than an outliner. Notes are Markdown in a
local SQLite database; link them with `[[wiki-links]]`, embed and annotate PDFs,
drop in images, sketch on freeform **whiteboards**, and search across
everything. No cloud, no account — your notes stay on your machine. Built in Rust
with [gpui](https://www.gpui.rs/) (Zed's GPU-accelerated UI framework) +
[gpui-component](https://github.com/longbridge/gpui-component).

Developed in close collaboration with Claude (Anthropic).

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/public/screenshots/zorite-macos-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="docs/public/screenshots/zorite-macos-light.png">
    <img src="docs/public/screenshots/zorite-macos-dark.png" alt="Zorite — a linked note with sub-pages and backlinks, the Favorites and Whiteboards sidebar, and multiple open tabs" width="820">
  </picture>
</p>

📝 [**Changelog**](CHANGELOG.md) · 🗺️ [**Roadmap**](TODO.md)

## Highlights

**At a glance** — an infinite daily-journal feed with Word-like, live-preview
Markdown editing · `[[wiki-links]]`, backlinks, and a graph view · Obsidian-style
properties, block references, and embeds · freeform whiteboards · a PDF viewer
with highlights and **fillable forms** · typeset LaTeX math with a visual editor ·
instant full-text search · optional whole-database encryption · everything in one
local SQLite file, no cloud, no account.

### Write

- **A journal that edits like a document** — days stream newest-first, every line
  is just text, and clicking anywhere drops the caret there. Live-preview
  Markdown: markers hide until the caret lands on them.
- **CommonMark + GFM and then some** — tables edited cell-by-cell like a small
  spreadsheet, GitHub alerts (including Obsidian's foldable `> [!NOTE]-`),
  **collapsible headings**, syntax-highlighted code, and Mermaid diagrams
  rendered pure-Rust and themed to your skin.
- **`/` palette & templates** — insert any construct or your own `{{date}}`-style
  templates; `/date`, `/time`, and a table size picker included.
- **Completion that stays out of the way** — `[[` suggests pages, `#` tags,
  `{{` placeholders; brackets auto-pair prose-safely; an opt-in auto-linker wraps
  existing page titles as you type.
- **Math for real** — inline `$…$` and display `$$…$$` typeset crisply; click in
  to edit fractions, roots, and matrices **visually** (MathQuill-style), with
  copy-LaTeX and PNG/SVG export.

*Details: [Journal & pages](https://packetthrower.github.io/zorite/usage/journal/)
· [Math](https://packetthrower.github.io/zorite/usage/math/)*

### Link & organize

- **`[[wiki-links]]`, `#tags`, backlinks** — links auto-create pages, every page
  lists what references it (plus unlinked mentions, one click to link), and
  `Projects::Tasks` naming builds a namespace tree with cascade renames.
- **Block references & embeds** — address any line with ` ^id`, link to it (or to
  a heading) from anywhere, and transclude whole notes, sections, or blocks with
  `![[Note]]` — live-updating, in both views.
- **Properties** — `key:: value` lines render as tidy panels with icons and
  clickable pills, edit in place, and roll up into a vault-wide Properties index.
- **See everything** — an All-pages browser, a force-directed **graph view**, a
  calendar dotted with entries, favorites, and tabs you can tear into
  independent, live-syncing windows.

*Details: [Journal & pages](https://packetthrower.github.io/zorite/usage/journal/)
· [All pages & the graph](https://packetthrower.github.io/zorite/usage/navigate/)*

### Files, PDFs & whiteboards

- **Images just work** — paste or drop them in (HEIC/AVIF included), drag a corner
  to resize, mid-sentence images flow as thumbnails, duplicates dedupe, and a
  sweeper clears unused files.
- **A real PDF viewer** — page-virtualized (800 pages stay light), with a TOC,
  drag-to-highlight markup that links back to your notes, find-in-PDF, password
  support — and **AcroForm filling**: click checkboxes, type into fields, Tab
  through the form; saved files render correctly everywhere.
- **Export any note to PDF** — `⌘P` produces a real, styled PDF (tables, images,
  math, diagrams), pure-Rust, no browser.
- **Whiteboards** — infinite pan/zoom canvases with shapes, arrows, freehand,
  images, rich text, templates, and page-cards linking back to notes.

*Details: [PDF & images](https://packetthrower.github.io/zorite/usage/pdf/)
· [Whiteboards](https://packetthrower.github.io/zorite/usage/whiteboards/)*

### Find

- **Search everything, instantly** — a trigram full-text index answers per
  keystroke at 50,000 pages; results are **type-aware** (pages, PDFs, images,
  whiteboards, with filter chips), and `⌘F` finds within the rendered page.

*Details: [Search](https://packetthrower.github.io/zorite/usage/search/)*

### Yours

- **Private by construction** — notes live in one local SQLite file (images and
  PDFs beside it); optionally encrypt the whole database (SQLCipher, AES-256)
  with an unlock screen, keychain remember, and idle auto-lock.
- **Bring your notes** — importers for **Obsidian** (vault, links, properties,
  callouts, canvas boards) and **Logseq** (graph, whiteboards, PDF highlights)
  land everything as working notes, non-destructively.
- **Make it look right** — built-in and JSON-file themes, light/dark/auto, any
  font (installed or imported), and a 14–20 px text-size dial.

*Details: [Password & encryption](https://packetthrower.github.io/zorite/usage/security/)
· [Import your notes](https://packetthrower.github.io/zorite/usage/import/)
· [Themes](https://packetthrower.github.io/zorite/customize/themes/)*

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

Then type `/meeting` in any day or page to insert it. Placeholders expand on
insert: `{{date}}`, `{{time}}`, `{{title}}` (the current page/day), and
`{{cursor}}` (where the caret lands). Built-in Markdown commands live in
[`gpui-markdown`](crates/gpui-markdown/README.md) as `SNIPPETS`.

## Themes

Zorite ships several built-in themes (Zorite, Nord, Solarized, Dracula, Tokyo
Night, Foundry, Cyberpunk, CRT, E-Ink), each with a light and dark variant
(Cyberpunk and CRT are dark-only). Open **Settings** (the ⚙ in the title bar) to
pick a theme and choose **Light / Dark / Auto** (Auto follows your system
appearance). A quick light/dark toggle also lives in the title bar.

Settings → Appearance also picks the app **font** (any installed family, or
import a `.ttf`/`.otf`) and the note **text size**; a custom theme can name its
own font and override every palette token — see the
[themes docs](https://packetthrower.github.io/zorite/customize/themes/).

Drop a `.json` file in your themes folder (Settings → **Reveal themes folder**)
and click **Reload**. Any color you omit falls back to the base palette, so a
theme can be just a few colors:

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
`"dark_only": true` for an always-dark theme.

## Install

> Installer packages land with the first **stable** release. Until then, grab a
> build from the [Releases](https://github.com/packetThrower/zorite/releases)
> page — every beta attaches `.dmg`, `.exe`/`.msi`, `.deb`, `.AppImage`, `.rpm`,
> and `.pkg.tar.zst` artifacts plus `SHA256SUMS`.

On macOS and Windows, the package managers track the latest stable tag and get
you past the first-launch Gatekeeper / SmartScreen warnings.

```sh
# macOS — Homebrew (6.0.0+ requires trusting the tap before installing)
brew tap packetThrower/tap
brew trust packetThrower/tap
brew install --cask zorite                          # stable
brew install --cask zorite@alpha                    # pre-release

# Windows — winget (Microsoft's package manager, preinstalled)
winget install packetThrower.Zorite                 # or: winget install zorite

# Windows — Scoop
scoop bucket add packetThrower https://github.com/packetThrower/scoop-bucket
scoop install zorite                                # stable
scoop install zorite-prerelease                     # pre-release
```

winget carries **stable only**; for pre-release builds on Windows use Scoop or
the [Releases](https://github.com/packetThrower/zorite/releases) page directly.
Linux users grab the matching `.deb` / `.rpm` / `.AppImage` / `.pkg.tar.zst`
from Releases (`pacman -U` for the Arch package).

To install by hand, download from Releases and drag `Zorite.app` to
`/Applications` on macOS, or run the installer on Windows. The macOS builds are
ad-hoc signed, so the first launch needs a right-click → **Open** (or
`xattr -cr Zorite.app`); the Windows installer is unsigned, so SmartScreen needs
**More info → Run anyway**. Notarized macOS and signed Windows builds are planned
— see [TODO.md](TODO.md).

## Building from source

A small Rust workspace (the app plus three reusable crates).

```sh
git clone git@github.com:packetThrower/zorite.git
cd zorite
cargo run                       # debug build + launch
cargo build --release           # optimized binary at target/release/zorite
cargo test --workspace          # run the tests
```

The first `cargo build` compiles gpui's full dependency graph and takes a few
minutes; incremental builds are fast. Toolchain: a recent stable Rust (via
[rustup](https://rustup.rs/)). Platform libraries:

- **macOS**: Xcode command-line tools (`xcode-select --install`).
- **Debian / Ubuntu**: `sudo apt install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libxcb1-dev libxcb-randr0-dev libxcb-xkb-dev libxcb-cursor-dev libxcb-shape0-dev libxcb-xfixes0-dev libxcb-render0-dev libfontconfig1-dev libfreetype-dev pkg-config`
- **Windows**: nothing extra; the gpui DirectX backend ships with Windows 10+.

Your data lives at:

| OS      | Path                                                   |
| ------- | ------------------------------------------------------ |
| macOS   | `~/Library/Application Support/zorite/zorite.db`       |
| Linux   | `$XDG_DATA_HOME/zorite/` (or `~/.local/share/zorite/`) |
| Windows | `%APPDATA%\zorite\`                                     |

`ZORITE_DATA` overrides the whole data directory (and `ZORITE_DB` just the
database file), so you can run against a throwaway data set without touching your
real notes.

## Workspace layout

```
zorite/
├── src/                       the app — journal feed, pages, search, slash menu, import, SQLite
└── crates/
    ├── gpui-editor/           a from-scratch WYSIWYG Markdown editor (live preview, tables, math hosting)
    ├── gpui-markdown/         a reusable Markdown renderer for gpui (clickable links, Mermaid, math)
    ├── gpui-pdf/              a page-virtualized PDF viewer (pure-Rust hayro) with highlight markup
    ├── gpui-whiteboard/       a host-agnostic infinite-canvas whiteboard
    ├── os-spellcheck/         native OS spell-checking (NSSpellChecker / ISpellChecker)
    └── ratex-gpui/            a structural (MathQuill-style) math editor + LaTeX renderer (RaTeX)
```

Each crate is host-agnostic and carries its own README.

## Performance

Zorite stays responsive with large note collections. The numbers below come from
synthetic databases built by [`scripts/gen_perf_db.py`](scripts/gen_perf_db.py)
— a 3-level `Area::Topic::Note` namespace tree with `[[wiki-links]]`, inline
images, and a couple weeks of journal days. `ZORITE_DB` points the app at a
throwaway database, so your real notes are never touched:

```sh
python3 scripts/gen_perf_db.py 10000 /tmp/zorite-perf.db
ZORITE_DB=/tmp/zorite-perf.db cargo run
```

**Hot-path query timings** (SQLite, best of several runs on a development Mac):

| Operation                                | 1,000    | 10,000   | 50,000   |
| ---------------------------------------- | -------- | -------- | -------- |
| Load the page list (`list_pages`)        | 0.3 ms   | 4.4 ms   | 26 ms    |
| Search per keystroke (FTS5 trigram)      | 0.07 ms  | 0.11 ms  | 0.09 ms  |
| Backlinks for a page (indexed)           | 0.005 ms | 0.005 ms | 0.005 ms |
| Seed the recent list (first launch only) | 0.1 ms   | 2.3 ms   | 12 ms    |

`list_pages` loads only `id`/`title`, not page content — keeping it fast and
memory flat. Search is a **trigram FTS5 index**, so it stays ~0.1 ms whether
you have a thousand pages or fifty. **Memory** (resident set size):

| Metric                       | Empty DB | 10,000 pages | 50,000 pages |
| ---------------------------- | -------- | ------------ | ------------ |
| RAM (RSS)                    | ~86 MB   | ~135 MB      | ~138 MB      |
| Database file (incl. index)  | 60 KB    | 48 MB        | 229 MB       |

RAM barely moves from 10k to 50k: the page list holds only `id`/`title` (~2 MB at
50k), not the note bodies — those load one page at a time as you open them. The
database file grows mostly with the FTS trigram index. At 50,000 pages, launch,
navigation, search, and scrolling all stay immediate, and the sidebar's cost is
independent of the total (it's capped to recently-viewed pages).

## License

[GNU General Public License v3.0 or later](LICENSE). Forks are welcome;
derivative works must stay open under the same license. Commercial use is
permitted but can't close the source.

Third-party dependencies keep their own licenses (see
[`THIRD-PARTY-LICENSES.html`](THIRD-PARTY-LICENSES.html)); HEIC/AVIF decoding
bundles an AGPL-3.0 component (`heic_decoder`).
