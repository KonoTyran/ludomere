# Ludomere

A native GOG library, download, and game manager for Linux.

Sign in to GOG to synchronize owned game names in batches of 50, followed by cover images and
sidebar icons acquired together. Icons do not require opening game details.
The bottom-left status identifies game-list, grid-image and requested metadata work. Cards show
queued/loading indicators, then an image, an unavailable message, or a visible failure. Partial
failures leave cached games usable and offer Retry; opening details shows a small indicator for
each requested section without waiting to enable Play or Download.
Footer Retry retries only failed covers and icons, keeping your current page. Dismiss hides the
notice without clearing failures; new failures appear again. Use Settings → Maintenance → Refresh
all online metadata for full synchronization. Downloads is centered in the footer, with account
status on the right. In the
screenshot viewer, Left and Right navigate with the same wraparound behavior as its arrow buttons.
Descriptions, screenshots, artwork and game information load when their detail view is opened.
Offline installer choices and Galaxy build information load independently when needed.
Empty screenshot sections show an unavailable message. Individual screenshots show loading or
failure states and can be retried without refreshing the whole game; failed images do not leave
navigation arrows over the next section.
Ludomere can install native Linux offline builds, Windows offline installers, or
ready-to-run Windows Galaxy builds. Downloads and installations can be paused, resumed after an
interruption, cancelled, and repaired from the unified Downloads page.

The one-time setup guide opens with your existing folder and Proton preferences. Both folder fields
have a directory picker and remain editable. Changing the game folder suggests its `downloads`
subfolder; editing the download folder independently never changes the game folder. Proton choices
show their full paths, including a selectable wrapping path below the version selector. Check Windows
requirements, then save the settings before optional GOG sign-in opens in its own modal. Close the
sign-in modal to skip; already signed-in users do not need to sign in again. Failed saves keep your
edits in the setup form for retry. You can defer Windows
setup or close the guide; Finish setup remains available without repeatedly opening the guide.
Ready Windows actions use saved defaults and per-game overrides after a background check. Missing
components show an actionable status; downloads still require an explicit choice in setup.

Install after downloading is selected by default. It installs a selected base-game installer and
compatible selected DLC using saved folder, language and compatibility choices after all required
downloads finish. Extras, patches and DLC-only selections only download. Uncheck it for downloads
only. Missing prerequisites leave a visible blocked installation in Downloads; resolve them in
Finish setup, then use Retry installation. Existing installations are preserved.
Extras starts unchecked for new profiles. Existing “Include extras by default” preferences remain
in effect and can still be changed in Settings.

Manage → Delete Downloaded Files removes confirmed, recorded installer/patch/extra files and related
DLC downloads while preserving installed payloads, saves and preferences. It appears when managed
downloads are present. Uninstall also offers an unchecked cleanup checkbox; cleanup runs only after
successful uninstall. Active work and unsafe/replaced files are protected, and partial failures stay
visible so you can inspect or retry them.

> [!IMPORTANT]
> Ludomere was built entirely with AI assistance for the author's personal use. Its behavior and
> defaults reflect that environment and may make assumptions that do not apply to other systems,
> libraries, or workflows. Review the code and back up important data before relying on it.

## System requirements

**Arch Linux x86-64 is the supported platform.** The pacman package declares GTK4 4.14+,
libadwaita 1.5+, WebKitGTK API 6.0 at version 2.50+, GDK-Pixbuf, D-Bus, libsecret, xz,
Python, python-xlib, and python-urllib3. Arch supplies development headers with its library packages.
A graphical session and session D-Bus are needed to run the application. Login additionally needs
an unlocked Secret Service provider, such as GNOME Keyring or a compatible desktop keyring;
installing libsecret alone does not provide that service.

The package includes a private [UMU Launcher](https://github.com/Open-Wine-Components/umu-launcher)
1.4.4 and [Comet](https://github.com/imLinguin/comet) v0.3.2 with its Windows service helper.
Comet provides supported games' GOG Galaxy authentication, achievements, and related online
features. These helper binaries do not require a system `umu-launcher` installation.
They live under `/usr/lib/ludomere/`; source archives, checksums,
and licenses accompany the package.

Comet's proprietary GOG peer libraries are **not bundled**. The official, unmodified Comet helper
downloads and updates them automatically when online services start, following upstream behavior.
There is no separate Ludomere confirmation for those dependency downloads. Comet stores its
state and peer files under `$XDG_DATA_HOME/ludomere/comet/state/comet/`.

Startup and manual checks fetch version metadata only. Installing a newer Comet binary requires
confirmation. Ludomere verifies the official stable release's Linux binary and Windows service
against GitHub's published SHA-256 digests before selecting them together. Updates live under
`$XDG_DATA_HOME/ludomere/comet/helpers`; pacman-owned files and the previous helper remain intact.
A newer packaged helper takes precedence over an older application-managed version. Real-game
Comet verification still requires the user's help.

Proton and the Steam Linux Runtime are **not bundled**. Choose an existing Proton installation or
accept an offered download. Windows games also need working graphics drivers and any required
32-bit Vulkan/OpenGL driver libraries. Native Linux installation and launch remain independent of
Proton and UMU, and use the normal Arch `/bin/sh`. The package is not an AppImage; Ubuntu and other
distributions are outside the current support commitment.

## Proton selection

Settings → Proton discovers native and Flatpak Steam installations, additional Steam libraries,
Heroic and Lutris Proton folders, and versions downloaded by Ludomere. This includes compatible
versions placed there by ProtonPlus. Choose folder supports other locations. External versions
are used in place and are never installed, updated, or deleted by Ludomere's download manager.

Automatic selection prefers the newest stable GE-Proton, then UMU-Proton, then Valve Proton.
The first automatically or manually chosen version becomes the saved application default;
installing a newer version elsewhere does not change it. Game settings can override that default
or return to inheriting it. These preferences survive uninstall. If a saved directory disappears,
choose a replacement: Ludomere does not silently switch versions.

The download controls offer stable current and historical GE-Proton and UMU-Proton releases;
Valve Proton is discovery-only. Missing Proton or a required Steam Linux Runtime produces an offer
from a direct user action. Downloads show progress, support cancellation, verify publisher
checksums, and publish only completed installations. Retry the requested action after preparation.
Recovered/background operations fail with an actionable error instead of acquiring components or
opening dialogs. A private UMU adapter prevents its usual automatic component acquisition.

## Development

Rust **1.92 or later** is required by the locked dependencies. Install the local build requirements
on an up-to-date Arch system (this command changes your system; the build scripts do not):

```bash
sudo pacman -S --needed base-devel rust pkgconf gtk4 libadwaita gdk-pixbuf2 webkitgtk-6.0 libsecret dbus xz python python-xlib python-urllib3
```

[`rustup`](https://rustup.rs/) can provide the Rust toolchain instead of Arch's `rust` package.
With rustup, also install `rustfmt` and `clippy`. `base-devel` provides the C compiler, linker,
makepkg, and fakeroot; pkgconf locates the native development libraries. SQLite is bundled and
the HTTP client uses Rustls, so separate SQLite/OpenSSL development packages are not required.

Run directly from the repository:

```bash
cargo run
```

To exercise Windows support and Comet from a source checkout, first stage the pinned helpers into
an empty directory. This downloads only build-time helper assets, not Proton or a runtime:

```bash
python3 tools/prepare-helpers.py --destination target/helpers
LUDOMERE_UMU_RUN="$PWD/target/helpers/umu/umu-run" \
LUDOMERE_COMET_DIR="$PWD/target/helpers/comet" cargo run
```

The environment overrides must be absolute paths. Use the staged UMU adapter, not a host UMU
executable, to retain the explicit-download policy. To stage again, choose another empty destination;
verified downloads are cached in `target/helper-downloads/`. Comet and its Windows service are
the unchanged official release binaries; helper preparation neither patches nor compiles Comet.
Debug builds also find the staged UMU adapter under `target/helpers` relative to their executable,
after explicit and installed helper paths. Plain `cargo run` therefore recognizes staged UMU;
the Comet override above remains necessary for source runs. Release builds use the packaged helper
or the explicit override.

The first run creates `~/.config/ludomere/config.toml`. The default game library and managed
download location are `$XDG_DATA_HOME/ludomere/games`. The executable is `ludomere`.

Settings → Account → **Clear full profile when signing out** is off by default. Enabling it only
saves the preference. On your next sign-out, Ludomere closes and clears login data, settings,
favorites, tags, playtime, queue records, cached metadata, images and logs. Games, downloaded
installers, Proton versions and runtimes stay intact. Finish active work and finish or cancel
unfinished installations before resetting. If cleanup fails, Ludomere offers Retry or Close
without starting library workers. The
next launch starts signed out with default settings, including this toggle off; add any custom
game-library paths again to use their preserved installations.

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

`bash tools/check.sh` runs `cargo fmt --check`, Clippy with warnings denied, the complete Cargo test
suite, and the private UMU adapter tests. It gives tests disposable XDG configuration, data, cache,
state, and runtime directories and removes them afterward. It does not change `HOME` or use GOG
credentials. Tests need loopback TCP, subprocesses, pseudo-terminals, writable temporary storage,
and working GdkPixbuf loader IPC. Restricting these can cause environment failures even when the
dependencies are installed. Automated tests do not require a graphical display, a game, Proton,
UMU, or GOG credentials. Actual login and game testing require your normal desktop session.

Build an optimized binary:

```bash
cargo build --release
```

## Arch package and build container

```bash
python3 tools/build-package.py
```

This snapshots current product files (including uncommitted/new source files), verifies the local
source archive checksum, fetches locked Rust dependencies and checksum-pinned helpers, runs checks,
builds the release binary, and produces `dist/ludomere-*.pkg.tar.zst`. It also leaves the application
source archive and its SHA-256 file in `dist/`. It does not install the package. To create only the
source archive, add `--source-only`. `PKGBUILD` is a template: this script replaces its source checksum
placeholder before invoking makepkg. Do not run the template directly or use `git archive HEAD`
to package an edited checkout. Build caches stay beneath ignored `target/` and `dist/` directories.

For a clean Arch build environment using Docker:

```bash
docker build --build-arg BUILD_UID="$(id -u)" --tag ludomere-arch --file tools/Containerfile tools
docker run --rm --init --volume "$PWD:/workspace" ludomere-arch bash -c 'bash tools/check.sh && python3 tools/build-package.py'
```

Rootless Podman can use the same commands with `podman` and `--userns=keep-id` on `run`.
Keep `--init`: process-control tests intentionally orphan child processes and need an init process
to reap them, just as a normal desktop does.
The image digest and dated Arch repository snapshot constrain the native environment; Cargo.lock
and helper checksums constrain downloaded sources. Update these pins deliberately for security
updates. This is a repeatable build recipe, not a claim of independently verified byte-identical
packages. Container runs need enough memory/disk for GTK/WebKit dependencies and Rust builds.
The GitHub Actions workflow runs these checks and uploads pacman/source artifacts; it does not
publish releases or install packages on the host. Its Ubuntu runner only hosts the Arch container.

## Guided game verification

Use a disposable test library and your own GOG login on Arch; do not share credentials or token
files. Automated tests do not establish these desktop/game checks:

1. Start the application, log in through WebKit, refresh the library, and restart to confirm keyring
   access. Verify a native Linux game can install and launch without selecting Proton.
2. Refresh Proton settings with versions installed by Steam/ProtonPlus and Heroic/Lutris if present.
   Confirm the saved default remains fixed, folder selection works, and a game override differs
   from the global default. Returning to “Use application default” must restore inheritance.
3. With no selection/discoverable version in a disposable profile, start a Windows action and open
   Finish setup from the missing-component status. Cancel one explicit download, retry, and select its completed version. If its Steam
   Linux Runtime is absent, accept the separate offer and retry the action.
4. Install and launch an owned Windows game, preferably once through an offline installer and once
   through Galaxy depots. Exercise update/repair, DLC, patch and uninstall controls where applicable.
   Verify the configured Proton is used and its preference survives uninstall. Temporarily move a
   disposable selected Proton directory to verify the replacement request without automatic fallback.
5. For a game known to support Comet, enable GOG online services and verify the expected online
   feature after upstream Comet prepares its peers. Confirm update checks fetch only metadata and
   display results in place; a Comet binary update needs confirmation, while Comet manages peer
   downloads and updates automatically. Cancel a binary update and confirm the old helper remains
   selected. Verify a completed update leaves the package-owned helper untouched.
   Confirm queued/recovered work never opens a dialog, changes page, or steals focus.
6. With a large library (500 or more cached products), reopen the app and search or select games
   while covers load. Open several detail pages quickly, then switch tabs: each section must keep
   its own loading/error state without replacing the selected page, scroll position or focus.
   Play should remain usable from a local installation; Download should open its chooser immediately.
   Open metadata filters and verify their incomplete/error indicator until missing metadata loads.
   Local filters remain available; unknown metadata stays visible until it can be evaluated. Retry
   a failed section and reopen a cached section to check reuse. Use a disposable profile for offline
   and sign-out-during-load checks; never reuse a private database for automated GUI fixtures.

Record the Arch package version, graphics driver, selected Proton, game/build, steps, and observed
result. Treat unavailable controls or unsupported game features as untested, not passing evidence.

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
- Global Proton default and per-game overrides: `$XDG_CONFIG_HOME/ludomere/proton.json`
- Downloaded Proton versions: `$XDG_DATA_HOME/ludomere/proton/`
- Steam Linux Runtimes: `$XDG_DATA_HOME/ludomere/umu/steamrt*/`
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
