---
title: Bundled icons
description: The Lucide icons bundled into Zorite's UI as gpui-component IconName variants — a developer reference.
---

:::note
A developer reference for building on Zorite's UI — the Lucide icon set bundled
through gpui-component.
:::

gpui-component bundles the icon set below and exposes each one as an
`IconName` variant. The icons are [**Lucide**](https://lucide.dev/icons/)
(ISC-licensed) — every bundled SVG carries the `class="lucide lucide-…"`
marker. Browse the full ~1,600-icon catalog at <https://lucide.dev/icons/>.

## Using a bundled icon

All 99 icons here are type-safe `IconName` variants — no file work needed:

```rust
use gpui_component::{Icon, IconName};

Icon::new(IconName::ArrowDown).size_4()
```

## Using any *other* Lucide icon

The other ~1,500 Lucide icons aren't bundled, but `Icon` takes an arbitrary
path, so it's a one-file drop:

1. Grab the SVG from <https://lucide.dev/icons/> (or `npm i lucide-static`).
2. Drop it under this repo's assets (e.g. `resources/icons/my-icon.svg`).
3. Reference it by path:

```rust
Icon::default().path("icons/my-icon.svg").size_4()
```

Lucide SVGs use `stroke="currentColor"`, so they inherit theme color and
sizing just like the bundled ones.

## The 99 bundled icons

`IconName` variants are pascal-cased from the filename. A few gpui filenames
are **aliases** — the file is named differently from the Lucide glyph it draws
(flagged below); the link points at that underlying Lucide icon.

| `IconName` variant | gpui file | Lucide glyph |
| --- | --- | --- |
| `IconName::ALargeSmall` | `a-large-small.svg` | [`a-large-small`](https://lucide.dev/icons/a-large-small) |
| `IconName::ArrowDown` | `arrow-down.svg` | [`arrow-down`](https://lucide.dev/icons/arrow-down) |
| `IconName::ArrowLeft` | `arrow-left.svg` | [`arrow-left`](https://lucide.dev/icons/arrow-left) |
| `IconName::ArrowRight` | `arrow-right.svg` | [`arrow-right`](https://lucide.dev/icons/arrow-right) |
| `IconName::ArrowUp` | `arrow-up.svg` | [`arrow-up`](https://lucide.dev/icons/arrow-up) |
| `IconName::Asterisk` | `asterisk.svg` | [`asterisk`](https://lucide.dev/icons/asterisk) |
| `IconName::BatteryCharging` | `battery-charging.svg` | [`battery-charging`](https://lucide.dev/icons/battery-charging) |
| `IconName::BatteryFull` | `battery-full.svg` | [`battery-full`](https://lucide.dev/icons/battery-full) |
| `IconName::BatteryLow` | `battery-low.svg` | [`battery-low`](https://lucide.dev/icons/battery-low) |
| `IconName::BatteryMedium` | `battery-medium.svg` | [`battery-medium`](https://lucide.dev/icons/battery-medium) |
| `IconName::BatteryWarning` | `battery-warning.svg` | [`battery-warning`](https://lucide.dev/icons/battery-warning) |
| `IconName::Battery` | `battery.svg` | [`battery`](https://lucide.dev/icons/battery) |
| `IconName::Bell` | `bell.svg` | [`bell`](https://lucide.dev/icons/bell) |
| `IconName::BookOpen` | `book-open.svg` | [`book-open`](https://lucide.dev/icons/book-open) |
| `IconName::Bot` | `bot.svg` | [`bot`](https://lucide.dev/icons/bot) |
| `IconName::Building2` | `building-2.svg` | [`building-2`](https://lucide.dev/icons/building-2) |
| `IconName::Calendar` | `calendar.svg` | [`calendar`](https://lucide.dev/icons/calendar) |
| `IconName::CaseSensitive` | `case-sensitive.svg` | [`case-sensitive`](https://lucide.dev/icons/case-sensitive) |
| `IconName::ChartPie` | `chart-pie.svg` | [`chart-pie`](https://lucide.dev/icons/chart-pie) |
| `IconName::Check` | `check.svg` | [`check`](https://lucide.dev/icons/check) |
| `IconName::ChevronDown` | `chevron-down.svg` | [`chevron-down`](https://lucide.dev/icons/chevron-down) |
| `IconName::ChevronLeft` | `chevron-left.svg` | [`chevron-left`](https://lucide.dev/icons/chevron-left) |
| `IconName::ChevronRight` | `chevron-right.svg` | [`chevron-right`](https://lucide.dev/icons/chevron-right) |
| `IconName::ChevronUp` | `chevron-up.svg` | [`chevron-up`](https://lucide.dev/icons/chevron-up) |
| `IconName::ChevronsUpDown` | `chevrons-up-down.svg` | [`chevrons-up-down`](https://lucide.dev/icons/chevrons-up-down) |
| `IconName::CircleCheck` | `circle-check.svg` | [`circle-check`](https://lucide.dev/icons/circle-check) |
| `IconName::CircleUser` | `circle-user.svg` | [`circle-user`](https://lucide.dev/icons/circle-user) |
| `IconName::CircleX` | `circle-x.svg` | [`circle-x`](https://lucide.dev/icons/circle-x) |
| `IconName::Close` | `close.svg` | [`x`](https://lucide.dev/icons/x) *(alias)* |
| `IconName::Copy` | `copy.svg` | [`copy`](https://lucide.dev/icons/copy) |
| `IconName::Cpu` | `cpu.svg` | [`cpu`](https://lucide.dev/icons/cpu) |
| `IconName::Dash` | `dash.svg` | [`minus`](https://lucide.dev/icons/minus) *(alias)* |
| `IconName::Delete` | `delete.svg` | [`delete`](https://lucide.dev/icons/delete) |
| `IconName::EllipsisVertical` | `ellipsis-vertical.svg` | [`ellipsis-vertical`](https://lucide.dev/icons/ellipsis-vertical) |
| `IconName::Ellipsis` | `ellipsis.svg` | [`ellipsis`](https://lucide.dev/icons/ellipsis) |
| `IconName::ExternalLink` | `external-link.svg` | [`external-link`](https://lucide.dev/icons/external-link) |
| `IconName::EyeOff` | `eye-off.svg` | [`eye-off`](https://lucide.dev/icons/eye-off) |
| `IconName::Eye` | `eye.svg` | [`eye`](https://lucide.dev/icons/eye) |
| `IconName::File` | `file.svg` | [`file`](https://lucide.dev/icons/file) |
| `IconName::FolderClosed` | `folder-closed.svg` | [`folder-closed`](https://lucide.dev/icons/folder-closed) |
| `IconName::FolderOpen` | `folder-open.svg` | [`folder-open`](https://lucide.dev/icons/folder-open) |
| `IconName::Folder` | `folder.svg` | [`folder`](https://lucide.dev/icons/folder) |
| `IconName::Frame` | `frame.svg` | [`frame`](https://lucide.dev/icons/frame) |
| `IconName::GalleryVerticalEnd` | `gallery-vertical-end.svg` | [`gallery-vertical-end`](https://lucide.dev/icons/gallery-vertical-end) |
| `IconName::Github` | `github.svg` | [`github`](https://lucide.dev/icons/github) |
| `IconName::Globe` | `globe.svg` | [`globe`](https://lucide.dev/icons/globe) |
| `IconName::HardDrive` | `hard-drive.svg` | [`hard-drive`](https://lucide.dev/icons/hard-drive) |
| `IconName::HeartOff` | `heart-off.svg` | [`heart-off`](https://lucide.dev/icons/heart-off) |
| `IconName::Heart` | `heart.svg` | [`heart`](https://lucide.dev/icons/heart) |
| `IconName::Inbox` | `inbox.svg` | [`inbox`](https://lucide.dev/icons/inbox) |
| `IconName::Info` | `info.svg` | [`info`](https://lucide.dev/icons/info) |
| `IconName::Inspector` | `inspector.svg` | [`square-dashed-mouse-pointer`](https://lucide.dev/icons/square-dashed-mouse-pointer) *(alias)* |
| `IconName::LayoutDashboard` | `layout-dashboard.svg` | [`layout-dashboard`](https://lucide.dev/icons/layout-dashboard) |
| `IconName::LoaderCircle` | `loader-circle.svg` | [`loader-circle`](https://lucide.dev/icons/loader-circle) |
| `IconName::Loader` | `loader.svg` | [`loader`](https://lucide.dev/icons/loader) |
| `IconName::Map` | `map.svg` | [`map`](https://lucide.dev/icons/map) |
| `IconName::Maximize` | `maximize.svg` | [`maximize-2`](https://lucide.dev/icons/maximize-2) *(alias)* |
| `IconName::MemoryStick` | `memory-stick.svg` | [`memory-stick`](https://lucide.dev/icons/memory-stick) |
| `IconName::Menu` | `menu.svg` | [`menu`](https://lucide.dev/icons/menu) |
| `IconName::Minimize` | `minimize.svg` | [`minimize-2`](https://lucide.dev/icons/minimize-2) *(alias)* |
| `IconName::Minus` | `minus.svg` | [`minus`](https://lucide.dev/icons/minus) |
| `IconName::Moon` | `moon.svg` | [`moon`](https://lucide.dev/icons/moon) |
| `IconName::Network` | `network.svg` | [`network`](https://lucide.dev/icons/network) |
| `IconName::Palette` | `palette.svg` | [`palette`](https://lucide.dev/icons/palette) |
| `IconName::PanelBottomOpen` | `panel-bottom-open.svg` | [`panel-bottom-open`](https://lucide.dev/icons/panel-bottom-open) |
| `IconName::PanelBottom` | `panel-bottom.svg` | [`panel-bottom`](https://lucide.dev/icons/panel-bottom) |
| `IconName::PanelLeftClose` | `panel-left-close.svg` | [`panel-left-close`](https://lucide.dev/icons/panel-left-close) |
| `IconName::PanelLeftOpen` | `panel-left-open.svg` | [`panel-left-open`](https://lucide.dev/icons/panel-left-open) |
| `IconName::PanelLeft` | `panel-left.svg` | [`panel-left`](https://lucide.dev/icons/panel-left) |
| `IconName::PanelRightClose` | `panel-right-close.svg` | [`panel-right-close`](https://lucide.dev/icons/panel-right-close) |
| `IconName::PanelRightOpen` | `panel-right-open.svg` | [`panel-right-open`](https://lucide.dev/icons/panel-right-open) |
| `IconName::PanelRight` | `panel-right.svg` | [`panel-right`](https://lucide.dev/icons/panel-right) |
| `IconName::Pause` | `pause.svg` | [`pause`](https://lucide.dev/icons/pause) |
| `IconName::Play` | `play.svg` | [`play`](https://lucide.dev/icons/play) |
| `IconName::Plus` | `plus.svg` | [`plus`](https://lucide.dev/icons/plus) |
| `IconName::Redo2` | `redo-2.svg` | [`redo-2`](https://lucide.dev/icons/redo-2) |
| `IconName::Redo` | `redo.svg` | [`redo`](https://lucide.dev/icons/redo) |
| `IconName::Replace` | `replace.svg` | [`replace`](https://lucide.dev/icons/replace) |
| `IconName::ResizeCorner` | `resize-corner.svg` | [`resize-corner`](https://lucide.dev/icons/resize-corner) |
| `IconName::Search` | `search.svg` | [`search`](https://lucide.dev/icons/search) |
| `IconName::Settings2` | `settings-2.svg` | [`settings-2`](https://lucide.dev/icons/settings-2) |
| `IconName::Settings` | `settings.svg` | [`settings`](https://lucide.dev/icons/settings) |
| `IconName::SortAscending` | `sort-ascending.svg` | [`chevrons-up-down`](https://lucide.dev/icons/chevrons-up-down) *(alias)* |
| `IconName::SortDescending` | `sort-descending.svg` | [`chevrons-up-down`](https://lucide.dev/icons/chevrons-up-down) *(alias)* |
| `IconName::SquareTerminal` | `square-terminal.svg` | [`square-terminal`](https://lucide.dev/icons/square-terminal) |
| `IconName::StarFill` | `star-fill.svg` | [`star`](https://lucide.dev/icons/star) *(alias)* |
| `IconName::StarOff` | `star-off.svg` | [`star-off`](https://lucide.dev/icons/star-off) |
| `IconName::Star` | `star.svg` | [`star`](https://lucide.dev/icons/star) |
| `IconName::Sun` | `sun.svg` | [`sun`](https://lucide.dev/icons/sun) |
| `IconName::ThumbsDown` | `thumbs-down.svg` | [`thumbs-down`](https://lucide.dev/icons/thumbs-down) |
| `IconName::ThumbsUp` | `thumbs-up.svg` | [`thumbs-up`](https://lucide.dev/icons/thumbs-up) |
| `IconName::TriangleAlert` | `triangle-alert.svg` | [`triangle-alert`](https://lucide.dev/icons/triangle-alert) |
| `IconName::Undo2` | `undo-2.svg` | [`undo-2`](https://lucide.dev/icons/undo-2) |
| `IconName::Undo` | `undo.svg` | [`undo`](https://lucide.dev/icons/undo) |
| `IconName::User` | `user.svg` | [`user`](https://lucide.dev/icons/user) |
| `IconName::WindowClose` | `window-close.svg` | [`window-close`](https://lucide.dev/icons/window-close) |
| `IconName::WindowMaximize` | `window-maximize.svg` | [`window-maximize`](https://lucide.dev/icons/window-maximize) |
| `IconName::WindowMinimize` | `window-minimize.svg` | [`window-minimize`](https://lucide.dev/icons/window-minimize) |
| `IconName::WindowRestore` | `window-restore.svg` | [`window-restore`](https://lucide.dev/icons/window-restore) |

---

_99 icons, from gpui-component's `crates/assets/assets/icons/`. If you bump the
gpui-component pin and the set changes, regenerate this list._
