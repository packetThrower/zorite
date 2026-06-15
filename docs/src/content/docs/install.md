---
title: Install
description: 'Install Zorite via Homebrew, winget, Scoop, or a direct download. Stable and pre-release channels available.'
---

:::note
Installer packages and package-manager entries land with the first **stable**
release. Until then, grab a build from the
[Releases](https://github.com/packetThrower/zorite/releases) page — every beta
attaches `.dmg`, `.exe` / `.msi`, `.deb`, `.AppImage`, `.rpm`, and
`.pkg.tar.zst` artifacts plus `SHA256SUMS`.
:::

On macOS and Windows the recommended path is a package manager. They track the
latest stable tag and get you past the first-launch Gatekeeper / SmartScreen
warnings. System requirements (OS floors, runtime dependencies) and building
from source live in [Requirements](/zorite/reference/requirements/).

## macOS (Homebrew)

The tap [`packetThrower/tap`](https://github.com/packetThrower/homebrew-tap)
ships a `zorite` cask (stable) and a `zorite@alpha` cask (pre-release):

```sh
brew install --cask packetThrower/tap/zorite        # stable
brew install --cask packetThrower/tap/zorite@alpha  # pre-release
```

The cask handles the macOS quarantine attribute for you, so the app launches
without the right-click → **Open** prompt that a direct download needs. Zorite's
macOS builds are ad-hoc signed but not notarized; the cask is the path that
sidesteps Gatekeeper. Update with `brew upgrade --cask zorite`.

## Windows (winget)

<!-- WINGET-PENDING: remove this aside once microsoft/winget-pkgs#387921 merges -->
:::caution[Pending review]
Zorite's winget manifest is in Microsoft's [review queue](https://github.com/microsoft/winget-pkgs/pull/387921)
and isn't live yet — `winget install` will start working once it's merged. Until then,
use Scoop (below) or a [direct download](https://github.com/packetThrower/zorite/releases)
on Windows.
:::

[winget](https://learn.microsoft.com/en-us/windows/package-manager/winget/) is
Microsoft's own package manager, preinstalled on Windows 10 1809+ and
Windows 11. It resolves either by the full identifier or the short moniker:

```powershell
winget install packetThrower.Zorite    # full identifier
winget install zorite                   # short moniker (same result)
```

winget picks the right architecture automatically (x64 or arm64) based on the
host. It carries **stable only** — for pre-release builds on Windows use Scoop
below or grab the artifact directly from the
[Releases](https://github.com/packetThrower/zorite/releases) page. Update with
`winget upgrade packetThrower.Zorite`.

## Windows (Scoop)

The bucket [`packetThrower/scoop-bucket`](https://github.com/packetThrower/scoop-bucket)
ships two manifests: `zorite` (stable) and `zorite-prerelease`.

```powershell
# Scoop needs git to fetch + update buckets. If `git --version`
# already prints something, skip this line.
scoop install git

scoop bucket add packetThrower https://github.com/packetThrower/scoop-bucket
scoop install zorite                # stable
scoop install zorite-prerelease     # pre-release
```

Update with `scoop update zorite` (or `zorite-prerelease`).

## Linux

There's no package-manager bucket for Linux — grab the matching artifact for
your distro from the
[Releases](https://github.com/packetThrower/zorite/releases) page:

| Distro | Artifact | Install |
|---|---|---|
| Debian / Ubuntu | `.deb` | `sudo apt install ./zorite_<version>_amd64.deb` |
| Fedora / RHEL | `.rpm` | `sudo dnf install ./zorite-<version>.x86_64.rpm` |
| Arch | `.pkg.tar.zst` | `sudo pacman -U zorite-<version>-x86_64.pkg.tar.zst` |
| Any glibc distro | `.AppImage` | `chmod +x` then run it |

For ARM64 hosts use the matching `arm64` / `aarch64` artifact. Linux also needs
a Vulkan-capable GPU with current Mesa drivers — see
[Requirements](/zorite/reference/requirements/).

## Direct download (any OS)

If you'd rather not go through a package manager, every release on
[GitHub Releases](https://github.com/packetThrower/zorite/releases) ships the
same artifacts the package managers consume: `.dmg` and `.pkg` (macOS),
`.exe` / `.msi` (Windows), and `.deb` / `.rpm` / `.AppImage` / `.pkg.tar.zst`
(Linux), each per architecture, plus `SHA256SUMS`.

To install by hand, download from Releases and drag `Zorite.app` to
`/Applications` on macOS, or run the installer on Windows. Two bits of
first-launch friction live on this path; the brew / winget / scoop installs
sidestep both:

- **macOS Gatekeeper.** Direct builds are ad-hoc signed but not notarized.
  Right-click → **Open** on first launch, or run `xattr -cr Zorite.app` to strip
  the quarantine attribute.
- **Windows SmartScreen.** The installer is unsigned. Click **More info → Run
  anyway**.

Notarized macOS and signed Windows builds are planned — see
[TODO.md](https://github.com/packetThrower/zorite/blob/main/TODO.md).

## Pre-release channel

Pre-release tags (`vX.Y.Z-alpha.N`, `-beta.N`, `-rc.N`) publish under GitHub's
"Pre-release" badge and don't displace the "Latest release" pointer. Homebrew
(`zorite@alpha`) and Scoop (`zorite-prerelease`) each expose a separate manifest
for that channel; winget is stable-only. Linux users grab a pre-release tag's
artifact directly from the
[Releases](https://github.com/packetThrower/zorite/releases) page.

## Building from source

A small Rust workspace — the app plus three reusable crates. See
[Requirements](/zorite/reference/requirements/) for the per-OS system libraries.

```sh
git clone git@github.com:packetThrower/zorite.git
cd zorite
cargo run                       # debug build + launch
cargo build --release           # optimized binary at target/release/zorite
cargo test --workspace          # run the tests
```

The first `cargo build` compiles gpui's full dependency graph and takes a few
minutes; incremental builds are fast.
