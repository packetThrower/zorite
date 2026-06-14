---
title: Requirements
description: 'Minimum OS versions for Zorite on macOS, Windows, and Linux, the per-OS system libraries needed to build from source, and where your data lives.'
---

## Minimum OS versions

**macOS** (Apple Silicon and Intel) — macOS **11+**.

**Windows** (x64 and ARM64) — Windows **10 21H2+**.

**Linux** (amd64 and arm64):

| Distro | Floor |
|---|---|
| Ubuntu | 22.04+ |
| Debian | 12+ |
| Fedora | 38+ |
| Arch | rolling |

Linux additionally needs a **Vulkan-capable GPU with current Mesa drivers**.

## Building from source

Zorite is a small Rust workspace — the app plus three reusable crates
(`gpui-markdown`, `gpui-pdf`, `gpui-whiteboard`).

```sh
git clone git@github.com:packetThrower/zorite.git
cd zorite
cargo run                       # debug build + launch
cargo build --release           # optimized binary at target/release/zorite
cargo test --workspace          # run the tests
```

The first `cargo build` compiles gpui's full dependency graph and takes a few
minutes; incremental builds are fast. You'll need a recent **stable Rust**
toolchain (via [rustup](https://rustup.rs/)) plus the platform libraries below.

### Platform libraries

- **macOS** — Xcode command-line tools: `xcode-select --install`.
- **Debian / Ubuntu**:

  ```sh
  sudo apt install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
    libx11-dev libxcb1-dev libxcb-randr0-dev libxcb-xkb-dev \
    libxcb-cursor-dev libxcb-shape0-dev libxcb-xfixes0-dev \
    libxcb-render0-dev libfontconfig1-dev libfreetype-dev pkg-config
  ```

- **Windows** — nothing extra; the gpui DirectX backend ships with Windows 10+.

## Where your data lives

| OS | Path |
|---|---|
| macOS | `~/Library/Application Support/zorite/zorite.db` |
| Linux | `$XDG_DATA_HOME/zorite/` (or `~/.local/share/zorite/`) |
| Windows | `%APPDATA%\zorite\` |

Notes live in a local SQLite database; images and PDFs sit beside it as files.

Two environment variables let you run against a throwaway data set without
touching your real notes:

- `ZORITE_DATA` overrides the whole data directory.
- `ZORITE_DB` overrides just the database file.
