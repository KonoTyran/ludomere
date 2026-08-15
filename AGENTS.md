# Ludomere agent guide

## Project

Ludomere is a Rust GTK4/libadwaita desktop application for browsing a GOG library, managing
downloads, and installing and launching Linux and Windows games. Windows games use standalone
`/usr/bin/umu-run`; Bottles is not part of the architecture.

This repository is being prepared for its first release. Pre-release data from earlier private
iterations is disposable. Do not add migrations or compatibility paths for unreleased schemas,
names, directories, markers, or backends. When the persistent schema changes before the first
release, update the current schema and its tests directly.

## Invariants

- Filesystem installation markers and plausible payloads determine installedness. SQLite stores
  preferences, activity, downloads, metadata, and transient operations.
- Native Linux markers use schema 1. Windows/UMU markers use schema 2. Both are current formats.
- Missing UMU infrastructure or a missing prefix does not erase an installed Windows payload.
- Windows payloads live at `<library>/<slug>` and prefixes at
  `<library>/.ludomere/compatibility/<slug>` with a controlled `L:` mapping to the library.
- Preserve playtime, last-played activity, favorites, tags, and preferences across uninstall.
- Native Linux behavior must remain independent of UMU.
- Update UI state in place.
- Background work must never navigate, present windows, or steal focus. Only direct user actions
  may change the visible page or window.
- Do not run network, process, compatibility, or filesystem traversal work on the GTK thread.
- Never shell-concatenate commands or log credentials, tokens, signed URLs, or full environments.

## Code organization

- `src/compatibility/`: UMU backend, profiles, prefixes, process control, and fixes.
- `src/download/`: persistent queue, transfer, verification, layout, and managed files.
- `src/installation/`: planning, execution, markers, launch, patching, and recovery.
- `src/ui/`: GTK/libadwaita presentation; keep domain and backend modules GTK-free.
- `src/state.rs`: current SQLite schema and persistence APIs.
- `resources/`: desktop metadata, icons, and bundled compatibility data.

Prefer direct changes that reuse existing patterns. Preserve unrelated working-tree edits. Use
`apply_patch` for hand edits and `rg` for searching. Avoid speculative abstractions, compatibility
shims, and opportunistic refactors.

## Verification

Run these before handing off code changes:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Use `cargo build --release` when release behavior needs checking. Do not build or install an Arch
package unless the user requests it.
