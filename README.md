# Ludomere

A native GOG library, download, and game manager for Linux.

Sign in to GOG to synchronize owned games, artwork, structured offline-installer manifests,
official store metadata, GamesDB enrichment, and locally observed Galaxy build history. Catalog
metadata is cached for offline browsing, while installers, patches, and extras are stored in a
plain, browsable managed download directory. Downloads use a persistent, concurrency-limited
queue, can be resumed after interruption, and can be explicitly verified. Future releases will
expand the installation and compatibility features already included.

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

The first run creates `~/.config/ludomere/config.toml`. Managed downloads default to
`$XDG_DATA_HOME/ludomere/downloads`. The executable is `ludomere`.

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
- Metadata, installer, patch, extra, DLC, changelog, and Galaxy-build details
- Official GOG group/file identity with locally accumulated immutable download revisions
- Resumable offline-installer downloads, verification, and managed storage
- Windows installation and launch through UMU/Proton, with Comet-backed GOG Galaxy authentication
- Persistent ordered queue with configurable concurrency from one through four groups
- Collision-free partial staging under the selected download directory
- Persistent favorites and personal tags in SQLite
- Offline browsing after the first successful synchronization

## Data locations

- Configuration: `$XDG_CONFIG_HOME/ludomere/config.toml`
- Persistent library and queue state: `$XDG_DATA_HOME/ludomere/library.sqlite3`
- Replaceable artwork and screenshots: `$XDG_CACHE_HOME/ludomere/`
- Installers, patches, extras, and partial staging: the directory selected in Settings

GOG credentials are stored through the desktop Secret Service. Access and refresh tokens are not
written to `config.toml` or the application database. Comet receives them through a private,
per-session compatibility file that is removed when the game exits.

## License

Ludomere is licensed under the [GNU General Public License version 3 or later](LICENSE).
Third-party components and artwork retain their own licenses; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Settings can change the managed directory and simultaneous-download limit, open the download
directory, clear replaceable image data, or request a complete online metadata refresh.

Developers can compare the compatibility manifest projection with Product API, Store API v2,
GamesDB, and content-system builds without writing application state:

```bash
cargo run -- --audit-gog-sources
```

The audit reads the existing keyring login and prints normalized counts only. It does not print
tokens or signed download links and does not write SQLite, cache, or download files.

Base-game files live directly beneath their game slug. DLC is nested under its parent game:

```text
<download directory>/<game slug>/installer/...
<download directory>/<game slug>/patch/...
<download directory>/<game slug>/extra/...
<download directory>/<game slug>/dlc/<dlc slug>/installer/...
```
