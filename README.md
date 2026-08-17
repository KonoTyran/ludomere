# Ludomere

A native GOG library, download, and game manager for Linux.

Sign in to GOG to synchronize owned games, artwork, offline installers, and current Galaxy depot
build metadata. Ludomere can install native Linux offline builds, Windows offline installers, or
ready-to-run Windows Galaxy builds. Downloads and installations can be paused, resumed after an
interruption, cancelled, and repaired from the unified Downloads page.

> [!IMPORTANT]
> Ludomere was built entirely with AI assistance for the author's personal use. Its behavior and
> defaults reflect that environment and may make assumptions that do not apply to other systems,
> libraries, or workflows. Review the code and back up important data before relying on it.

## System requirements

Ludomere is currently developed and packaged for Linux. The Arch package requires
[`gtk4`](https://archlinux.org/packages/extra/x86_64/gtk4/),
[`libadwaita`](https://archlinux.org/packages/extra/x86_64/libadwaita/),
[`gdk-pixbuf2`](https://archlinux.org/packages/extra/x86_64/gdk-pixbuf2/),
[`webkit2gtk-4.1`](https://archlinux.org/packages/extra/x86_64/webkit2gtk-4.1/), and
[`libsecret`](https://archlinux.org/packages/core/x86_64/libsecret/). Install them with:

```bash
sudo pacman -S --needed gtk4 libadwaita gdk-pixbuf2 webkit2gtk-4.1 libsecret
```

Windows game installation and launching additionally require
[`umu-launcher`](https://github.com/Open-Wine-Components/umu-launcher), which supplies Proton and
the Steam Linux Runtime without requiring Steam. Ludomere currently invokes it specifically as
`/usr/bin/umu-run`, so install the system package rather than a user-local copy:

```bash
sudo pacman -S --needed umu-launcher
```

UMU downloads its runtime and a suitable UMU-Proton build when needed. Native Linux GOG installers
and uninstallers are shell scripts, so a POSIX-compatible `/bin/sh` must also be present (as it is
on a normal Arch installation).

Ludomere integrates with [Comet](https://github.com/imLinguin/comet), an open-source replacement
for the GOG Galaxy Communication Service used by some games for achievements, leaderboards, and
other Galaxy SDK features. Comet is not a system prerequisite: when an enabled compatibility fix
needs it, Ludomere downloads its pinned Comet binary and dummy service release, verifies their
SHA-256 checksums, and stores them under `$XDG_DATA_HOME/ludomere/tools/comet/`.

## Development

To build from source on Arch, add the Rust toolchain and standard packaging tools:

```bash
sudo pacman -S --needed base-devel rust gtk4 libadwaita gdk-pixbuf2 webkit2gtk-4.1 libsecret
```

[`rustup`](https://rustup.rs/) can provide the Rust toolchain instead of Arch's `rust` package.
The included `PKGBUILD` uses `makepkg`, supplied by `base-devel`.

Run directly from the repository:

```bash
cargo run
```

The first run creates `~/.config/ludomere/config.toml`. The default game library and managed
download location are `$XDG_DATA_HOME/ludomere/games`. The executable is `ludomere`.

Use isolated state during testing:

```bash
task_root="$(mktemp -d)"
XDG_CONFIG_HOME="$task_root/config" \
XDG_DATA_HOME="$task_root/data" \
XDG_CACHE_HOME="$task_root/cache" \
XDG_STATE_HOME="$task_root/state" \
cargo run
```

## Checks

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Build an optimized binary:

```bash
cargo build --release
```

## Current capabilities

- Native GOG authentication and owned-library synchronization
- Cached artwork grid and persistent game sidebar
- Search by title, slug, feature, or personal tag
- Search and filters for official tags, genres/themes, play modes, and store properties
- Metadata, offline installers, patches, extras, DLC, and changelogs
- Official GOG group/file identity with locally accumulated immutable download revisions
- Native Linux and Windows offline-installer installation
- Generation-two Windows Galaxy depot installation, updates, repair, DLC, and branch switching
- Chunk-level resumable depot downloads, including GOG small-files containers
- Parallel verification of existing depot files and destructive repair of managed files
- Windows installation and launch through UMU/Proton, with quiet registry and setup actions
- Comet-backed GOG Galaxy authentication for supported Windows games
- Unified download and installation queue with pause, resume, cancellation, speed history, and
  separate network and disk progress
- Configurable installation-source priority; the default is Linux offline, Windows Galaxy, then
  Windows offline
- Multiple game libraries with library-owned installation markers and operation recovery journals
- Persistent favorites and personal tags in SQLite
- Offline browsing after the first successful synchronization

Galaxy installs use the newest available generation-two build on the selected branch. Master is
the default branch. If alternate branches are advertised, they can be selected from the game's
settings; successful protected-branch passwords are saved automatically. Generation-one Galaxy
builds are not currently supported.

Changing between Galaxy and offline installation sources is a full reinstall, not an in-place
conversion. Ludomere attempts to preserve known save locations during that transition and warns
before continuing when no save locations are known. Normal cloud-save conflict handling remains
responsible for reconciling cloud and local saves.

## Data locations

- Configuration: `$XDG_CONFIG_HOME/ludomere/config.toml`
- Catalog metadata, preferences, activity, and the offline download queue:
  `$XDG_DATA_HOME/ludomere/library.sqlite3`
- Installation and runtime logs: `$XDG_DATA_HOME/ludomere/installation-logs/` and
  `$XDG_DATA_HOME/ludomere/runtime-logs/`
- Replaceable artwork and screenshots: `$XDG_CACHE_HOME/ludomere/`
- Offline installers, patches, and extras: the managed directory selected in Settings
- Installed games and their operation state: the selected game library

GOG credentials are stored through the desktop Secret Service. Access and refresh tokens are not
written to `config.toml` or the application database. Protected-branch passwords are encrypted in
SQLite with an account-bound key kept in the Secret Service. Comet receives tokens through a
private, per-session compatibility file that is removed when the game exits.

Each game library is self-describing. A completed installation stores its permanent marker beneath
the game directory; an active or interrupted operation stores its journal in the library control
directory:

```text
<library>/<game slug>/.ludomere/installation.json
<library>/.ludomere/staging/<game slug>.operation.json
<library>/.ludomere/staging/<game slug>.json
<library>/.ludomere/compatibility/<game slug>/
```

The operation files contain the information needed to resume or permanently cancel an interrupted
install without relying on SQLite. Depot payload is written directly into the final game directory;
temporary file parts remain beside the files they are building. A library scan can reconstruct
installed-game state from markers and plausible payloads if the application database is lost.

## Installation sources and storage

Settings controls the preferred order of Linux offline installers, Windows Galaxy builds, and
Windows offline installers. Rows can be reordered by dragging them or with the arrow buttons. A
choice made in the installation dialog applies only to that installation and does not change the
saved order. When more than one matching offline installer exists, the newest version is preferred.

Offline downloads use a plain, browsable layout. Base-game files live directly beneath their game
slug, while DLC is nested beneath its parent game:

```text
<managed download directory>/<game slug>/installer/...
<managed download directory>/<game slug>/patch/...
<managed download directory>/<game slug>/extra/...
<managed download directory>/<game slug>/dlc/<dlc slug>/installer/...
```

Galaxy depot installations are ready-made game trees rather than installer archives. Ludomere
downloads compressed chunks, verifies their GOG hashes, decompresses them into managed files, and
runs the repository's supported setup tasks afterward. Updates, repairs, DLC changes, and branch
switches reconcile the installed tree with the selected target manifests. Repair restores every
modified or corrupt managed file and leaves unknown files untouched.

Settings can change the managed directory and simultaneous-download limit, open the download
directory, clear replaceable image data, or request a complete online metadata refresh.

Developers can compare the compatibility manifest projection with Product API, Store API v2,
GamesDB, and content-system builds without writing application state:

```bash
cargo run -- --audit-gog-sources
```

The audit reads the existing keyring login and prints normalized counts only. It does not print
tokens or signed download links and does not write SQLite, cache, or download files.

## License

Ludomere is licensed under the [GNU General Public License version 3 or later](LICENSE).
Third-party components and artwork retain their own licenses; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
