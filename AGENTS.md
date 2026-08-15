# Ludomere agent guide

## Project

Ludomere is a Rust GTK4/libadwaita desktop application for browsing a GOG library, managing
downloads, and installing and launching Linux and Windows games. Windows games use standalone
`/usr/bin/umu-run`; Bottles is not part of the architecture.

This repository is being prepared for its first release. Schema 24 is the first-release database
baseline.

## Database schema policy

SQLite `user_version` represents released schema compatibility boundaries. Do not increment it for
each schema change made during development. When an upcoming release requires schema changes,
allocate one target schema version and keep that target unchanged throughout the release cycle.
The next release requiring database changes should target schema 25, regardless of how many
development iterations are needed before that release.

Track intermediate changes to an unreleased target schema using a small internal development
revision. Development revisions exist only to advance local databases created by earlier builds of
the same unreleased target. They must not consume additional public schema-version numbers.

Maintain one canonical migration from the most recently released or designated baseline schema to
the current target schema. Update that canonical migration in place as development continues.
Before release, squash all development changes into this single clean migration so normal users
execute only one transition between releases.

A database matching the target `user_version` must also have its development revision checked until
the target schema is finalized. Apply any retained development-revision steps needed to bring it to
the current target shape.

Development revision records should be short, ordered, and documented with their schema effect.
They are temporary implementation history, not permanent public migration history. Remove or
consolidate them once the release migration is finalized, except where a retained step is still
needed to open known development databases safely.

Do not add compatibility paths for arbitrary private schema variants. If a development database
cannot be identified by a supported development revision, reject it with an actionable error or
offer an explicit recoverable reset.

Schema verification must cover:

- Creation of a fresh current database.
- The canonical migration from the previous released or designated baseline schema.
- Advancement from every retained development revision of the current target.
- Preservation of durable user data during those transitions.
- Rejection of future or unidentified schema versions.

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
