# GOG-native feature roadmap for Ludomere

## Summary

Implement the selected Galaxy-equivalent features as phased, independently releasable work:

1. Validate undocumented GOG protocols and establish capability gates.
2. Add unified queue ordering and bandwidth limiting.
3. Add automatic Galaxy updates, offline-installer refresh, retention, and installed-language selection.
4. Add cloud-save export and deliberate remote deletion.
5. Add saved views, custom artwork, hide/unhide, and two-way GOG tag synchronization.
6. Add achievements, sessions, statistics, friends, invitations, and presence.
7. Enable validated writeback for gameplay and social operations.
8. Implement native text chat only if its complete protocol passes the feasibility gate.

Explicitly exclude overlays, store browsing/purchasing/redeeming, rollback support, Galaxy 2.0 cross-platform aggregation, activity feeds, crossplay, and multiplayer invites.

## Implementation status

- Phase 0 complete: typed friends, presence, and gameplay reads; capability manifest; sanitized audit and guarded live-write command.
- Phase 1 complete: durable mixed-work queue, drag and accessible reorder controls, and a live shared bandwidth limiter.
- Phase 2 complete: inherited update policies, post-sync and six-hour checks, manual checks, language reconciliation, and verified installer retention.
- Phase 3 complete: verified cloud-save exports, selective remote deletion with recovery snapshots and revision checks, and local tombstones.
- Phase 4 local organization complete: saved views, cover/background overrides, hidden games, and full local tag management/filtering. Live account tag and hidden reads pass validation; remote mutation and two-way synchronization remain gated pending safe write validation.
- Phases 5 through 7 remain in progress.

## Phase 0: GOG Protocol Validation

Create typed GOG service modules rather than exposing raw JSON outside the protocol layer:

- `src/gog/account.rs`: tags, hidden state, user search.
- `src/gog/friends.rs`: friends, invitations, block state, mutations.
- `src/gog/presence.rs`: status and current-game publication.
- `src/gog/gameplay.rs`: achievements, statistics, sessions, friend comparisons.
- `src/gog/chat.rs`: only after chat feasibility succeeds.
- `src/gog/capabilities.rs`: runtime feature/capability registry.

Extend the existing audit mechanism with:

- `--audit-gog-capabilities`: read-only endpoint/schema checks with no database or account mutations.
- `--validate-gog-write <capability> --account <expected-id> --confirm-live-write`: deliberate live-account validation.
- Chat-specific receive/send validation if a viable transport is found.

Every reverse-engineered write capability remains disabled until:

1. Sanitized response fixtures cover success, authorization failure, rate limiting, malformed payloads, and changed schemas.
2. The operation passes live validation against a designated test account.
3. Retry and duplicate-write behavior is understood.
4. Any test mutation can be reversed or cleaned up.
5. The capability is explicitly enabled in a checked-in capability manifest.

Diagnostics may record endpoint names, status codes, correlation IDs, cursor positions, counts, and sanitized protocol types. They must never record access tokens, signed URLs, message bodies, full environments, or private profile data.

## Phase 1: Unified Queue and Bandwidth Controls

### Global queue

Add a durable `work_queue` registry referencing existing download, depot, update, and installation operations:

- `work_id`
- `work_kind`
- `source_operation_id`
- `product_id`
- `queue_position`
- `created_at`

Existing operation tables remain authoritative for operation state. The registry only coordinates order.

Add a GTK-free `QueueCoordinator` that:

- Registers newly waiting work.
- Selects the next eligible item globally.
- Removes completed or cancelled work.
- Reorders waiting items transactionally.
- Leaves active operations in place.
- Skips blocked items without losing their relative order.
- Coordinates existing managers through the operation gate.

The unified Downloads page will support drag-and-drop ordering plus accessible “Move earlier” and “Move later” actions. The displayed waiting order must match execution order across all job types.

During migration, populate the global order from existing waiting jobs using creation time and existing per-manager positions as stable tie-breakers.

### Bandwidth limiting

Add a process-wide, thread-safe token-bucket limiter shared by:

- Offline installer downloads.
- Galaxy depot/chunk downloads.
- Cloud-save downloads and exports that fetch remote content.

Do not throttle metadata requests or uploads in this phase. Changes to the limit apply without restarting active transfers.

Add a Downloads setting with:

- Unlimited, the default.
- Common KiB/s and MiB/s presets.
- A custom numeric limit.

The limiter must wait only on worker threads and must respond promptly to pause, cancel, shutdown, and live configuration changes.

## Phase 2: Updates, Installer Refresh, and Language

### Serialized configuration

Extend `Config` with backward-compatible defaults:

- `auto_update_galaxy_installations: bool = true`
- `auto_download_offline_installers: bool = false`
- `prune_superseded_offline_installers: bool = false`
- `download_bandwidth_limit_bps: Option<u64> = None`

Add three independent switches to Downloads settings.

### Per-game overrides

Extend `GamePreferences` with nullable inherit/override values:

- `auto_update_galaxy: Option<bool>`
- `auto_download_offline_installer: Option<bool>`
- `prune_superseded_installers: Option<bool>`
- `galaxy_language: Option<String>`

`None` inherits the global setting. The game settings Updates section will show the effective value and its source.

### Automatic update scheduler

Run discovery:

- After startup, authentication, and initial library synchronization.
- Every six hours while online.
- From a manual “Check for updates” action.

Checks are single-flight and run off the GTK thread. Network loss, authentication expiry, or an already-running check defers work with bounded backoff.

For Galaxy depot installations:

- Follow the installed branch.
- Select the newest eligible build.
- Preserve DLC and architecture selections.
- Apply the effective installed-language preference.
- Queue an update only when the installed provenance differs.
- Never modify a running game; queued work waits for it to exit.
- Reuse the existing reconcile/verification pipeline.

For offline installers:

- Download the newest complete primary installer revision matching the effective platform and language preferences.
- Do not automatically execute offline installers.
- Do not automatically refresh extras or patches.

When pruning is enabled:

- Keep the newest complete, verified installer revision.
- Move superseded managed installer files to the desktop trash only after replacement verification succeeds.
- Preserve old files if download, checksum, indexing, or trash operations fail.
- Never delete unmanaged user files.

Add a trash implementation suitable for worker-thread use.

### Installed-language changes

The game settings language selector will list only languages available for the installed branch/build.

Changing language:

1. Saves the per-game preference.
2. Shows the affected download size and files.
3. Queues a reconcile operation after confirmation.
4. Updates installation provenance only after successful verification.
5. Leaves the existing installation and marker untouched on failure.

Installation markers must record the actual selected Galaxy language. Unsupported or removed languages produce an actionable error and retain the current language.

## Phase 3: Cloud-Save Export and Remote Deletion

Extend the cloud `Storage` trait with typed delete support, including conditional revision/ETag checks where provided.

### Export

Add “Export cloud saves” to the per-game cloud-save UI.

Export to a user-selected directory containing:

- The original relative save paths.
- A `manifest.json` with product ID, export time, remote identifiers, sizes, timestamps, revisions, and checksums.
- No credentials or signed URLs.

Reject traversal, absolute, duplicate, or platform-invalid remote paths.

### Remote deletion

Allow selection of individual remote files or all remote saves for a game.

Before deletion:

1. Refresh remote inventory.
2. Download a complete recovery snapshot.
3. Verify the snapshot and manifest.
4. Show an explicit confirmation containing the game and file count.

Abort deletion if the recovery snapshot cannot be completed. A remote revision change during deletion stops the operation and refreshes inventory.

Record a local tombstone containing the deleted remote identity and the fingerprint of any corresponding local file. Synchronization must not immediately recreate the remote object while the local file remains unchanged. A later local modification is treated as new content and may upload normally.

## Phase 4: Library Organization

### Saved views

Add local saved views using a versioned JSON query definition. Supported criteria:

- Text search.
- Installed, downloaded, or owned state.
- Favorites.
- Played or never played.
- Hidden inclusion.
- Any/all selected tags.
- Operating-system availability.
- Sort order.

Users can create, rename, update, reorder, and delete saved views. Views appear beneath Collections and remain local to Ludomere.

### Custom artwork

Support local PNG and JPEG cover and background overrides.

- Validate decoding and reasonable dimensions before accepting.
- Copy originals into application-owned data storage by product ID.
- Regenerate display cache entries.
- Provide separate reset actions for cover and background.
- Never upload custom artwork to GOG.

### Hidden games

Add `hidden` state to the library model.

- Hidden games are excluded from normal views by default.
- Add a Show Hidden filter and a dedicated Hidden collection.
- Provide Hide/Unhide actions in game menus and settings.
- Synchronize GOG hidden state only after its read and write endpoints pass validation.

First synchronization uses the union of local and GOG hidden state to avoid unexpectedly revealing games. Thereafter, pending local mutations win; otherwise confirmed remote changes apply locally.

### Tags

Upgrade the current local tags to support:

- Add and remove assignments.
- Create, rename, and delete tags.
- Case-insensitive duplicate prevention.
- Tag filtering and saved views.
- Stable GOG tag identifiers when synchronized.

On the first validated synchronization:

- Merge local and GOG tags by normalized name.
- Preserve assignments from both sides.
- Enqueue missing local tags and assignments for GOG writeback.

Afterward, local mutations use an outbox; confirmed external GOG changes apply when no conflicting local mutation is pending. Tag deletion requires confirmation because it removes all assignments.

Saved views and custom artwork remain local even if future undocumented endpoints appear.

## Phase 5: GOG Gameplay and Social Read Model

Add account-scoped cache tables for:

- Friends and relationship state.
- Incoming and outgoing invitations.
- Blocked users.
- Presence and current game.
- Achievements and unlock progress.
- Game statistics.
- Completed sessions.
- Synchronization cursors and timestamps.

All records are keyed by the authenticated GOG user ID so cached accounts cannot overlap.

### UI

Add a first-class Social destination with:

- Friends list with online/current-game state.
- Incoming and outgoing request sections.
- User search.
- Friend profile pages.
- Friend achievement and session comparisons.
- Blocked-user management.

Add per-game Achievements and Activity sections showing:

- Unlock status, date, rarity, and progress when available.
- Local and GOG sessions with their source identified.
- GOG statistics and leaderboards when a validated GOG-native endpoint exposes them.
- Friend comparison data.

Missing or unsupported endpoint fields produce partial UI rather than failing the whole page. Cached data remains viewable offline with a visible last-synchronized time.

## Phase 6: Presence and Validated Writeback

### Presence

Store account-scoped preferences:

- `presence_enabled = true`
- `game_activity_enabled = true`

When presence is disabled:

- Publish no online state.
- Clear previously published state on a best-effort basis.
- Grey out game activity.
- Remember the game-activity preference without applying it.

When enabled:

- Publish online after authenticated startup.
- Refresh presence as required by the validated protocol.
- Publish the GOG product and public game title when a game starts if game activity is enabled.
- Clear game activity when the game exits.
- Clear stale game activity during crash recovery and on the next authenticated startup.
- Attempt an offline transition on sign-out and normal shutdown.

No executable paths, launch arguments, compatibility details, or local library paths may be published.

### Friend mutations

After individual write capabilities pass validation, enable:

- Send invitation.
- Accept or decline invitation.
- Remove friend.
- Block and unblock user.

Every mutation uses optimistic UI only when it can be reconciled safely; otherwise the UI waits for confirmation. Rate limits and ambiguous responses trigger remote readback before retrying.

### Sessions and statistics

Replace aggregate-only local session recording with durable individual session events:

- Stable local UUID.
- Product ID.
- Start and end timestamps.
- Duration.
- Source.
- Remote synchronization state.

Continue deriving aggregate playtime from sessions while preserving existing durable totals during migration.

Implement an outbox for locally authoritative writes. It must support:

- Idempotency keys where accepted.
- Remote read-before-write where no idempotency contract exists.
- Retry with exponential backoff.
- Readback after ambiguous responses.
- Per-record acknowledgement.
- No duplicate session or counter increments after restart.

Write all statistics for which both conditions are true:

1. Ludomere has an authoritative local source for the value or event.
2. The corresponding GOG write contract has passed fixture and live validation.

Initial authoritative sources are launcher-observed sessions/playtime and any structured game-authored events that the validated Comet interface can expose. Monotonic counters must never be reduced. Arbitrary game counters must not be inferred from playtime or process state.

Achievements may be imported and compared, but Ludomere itself must not synthesize unlocks. Game-authored achievement/stat writes remain Comet’s responsibility unless Comet exposes a validated structured handoff that preserves the original game event.

The gameplay/statistics phase is not considered complete until import and safe deduplicated writeback both pass live validation. Unsupported statistics remain visibly read-only rather than being silently approximated.

## Phase 7: Conditional Native Chat

Perform the protocol investigation early, but ship chat only after the social foundation is stable.

Native chat is feasible only if testing establishes all of the following:

- Authentication without extracting or logging unsafe credentials.
- Conversation enumeration.
- Paginated history.
- One-to-one text sending.
- Reliable receiving through push or bounded polling.
- Reconnection and token renewal.
- Stable message identifiers and deduplication.
- Read-state handling.
- Rate-limit and offline behavior.

If feasible, add:

- Conversations section under Social.
- One-to-one text chat.
- Paginated history.
- Unread counts and read state.
- Reconnect with exponential backoff.
- A bounded account-scoped cache, with a clear-history control.
- Background receipt that updates state without navigating, presenting windows, or stealing focus.

Version one excludes attachments, voice, group conversations, rich embeds, and embedded web chat.

If any essential contract cannot be reproduced safely, document the explored transports and failure point, then remove experimental code, dependencies, and UI. There will be no web fallback.

## Schema and Migration Policy

Keep SQLite `user_version` at schema 25.

Advance internal development revisions from the current revision 5 as phases introduce schema changes:

- Revision 6: global work queue, update overrides, installed-language provenance, individual sessions.
- Revision 7: cloud deletion tombstones and export metadata.
- Revision 8: saved views, artwork overrides, hidden state, synchronized tag identity/outbox.
- Revision 9: friends, invitations, presence, achievements, statistics, and gameplay cursors.
- Revision 10: social/gameplay write outbox and account preferences.
- Revision 11 only if native chat ships.

Update the canonical schema-24-to-25 migration in place with the final schema. Retain development-revision transitions only for known schema-25 development databases, then squash them before release.

Migration must preserve existing preferences, tags, favorites, playtime, last-played timestamps, downloads, and installations.

## Public Interfaces and Types

Introduce these core interfaces:

- `QueueCoordinator`: global registration, eligibility, completion, cancellation, and reordering.
- `BandwidthLimiter`: live-configurable shared byte acquisition.
- `UpdatePolicy { Inherit, Enabled, Disabled }`.
- `UpdateScheduler`: startup, periodic, and manual discovery.
- `GogCapabilityRegistry`: validated read/write availability.
- `GogFriendsClient`, `GogPresenceClient`, and `GogGameplayClient`.
- `GameplayOutbox`: durable idempotent writeback.
- `CloudStorage::delete`.
- `SavedViewQuery`: versioned local filter definition.
- `ChatTransport`: compiled and exposed only if the chat feasibility gate succeeds.

Raw endpoint payloads must be converted to typed domain values inside `src/gog/`.

## Testing and Acceptance

### Database

Test:

- Fresh schema-25 creation.
- Canonical schema-24 migration.
- Advancement from every retained schema-25 development revision.
- Preservation of all durable user data.
- Rejection of future and unidentified schemas.
- Account isolation.
- Queue and outbox recovery after restart.

### Queue, updates, and downloads

Test:

- Global ordering across every work type.
- Reordering persistence and cancellation.
- Blocked/running-game behavior.
- Six-hour scheduling with a fake clock.
- Single-flight update checks.
- Global and per-game policy resolution.
- Galaxy branch/language preservation.
- Offline installer verification before trashing.
- Failed replacement never removes the old installer.
- Shared bandwidth enforcement, cancellation responsiveness, and live limit changes.

### Cloud saves

Test:

- Export manifests and path sanitization.
- Recovery snapshot verification.
- Conditional deletion conflicts.
- Partial remote deletion failures.
- Tombstones preventing immediate re-upload.
- Changed local files becoming uploadable again.

### Library

Test:

- Saved-view serialization and all filter combinations.
- Artwork validation, replacement, and reset.
- Hidden-state first-sync union and later reconciliation.
- Tag merging, rename/delete behavior, pending mutations, and remote conflicts.

### Social and gameplay

Use sanitized HTTP/protocol fixtures for:

- Pagination.
- Missing optional fields.
- Unauthorized and expired sessions.
- Rate limiting and backoff.
- Changed response schemas.
- Friend lifecycle transitions.
- Presence publication and clearing.
- Achievement/stat/session deduplication.
- Ambiguous write responses and readback reconciliation.
- Monotonic counters.
- Multi-account cache separation.

Live write validation is separate from the normal automated suite and must require explicit command-line confirmation.

### Chat

If feasible, test:

- History pagination.
- Duplicate and out-of-order messages.
- Reconnect and token renewal.
- Offline send failures.
- Unread/read transitions.
- Cache limits.
- Redaction of message bodies and credentials from diagnostics.

Before each phase is handed off, run:

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

## Assumptions and Defaults

- Ludomere continues to support one active GOG account at a time, while persisted online data remains keyed by account ID.
- Galaxy automatic updates default on.
- Offline-installer automatic downloads default off.
- Superseded-installer pruning defaults off.
- Pruning retains only the newest complete verified revision and uses trash.
- Update discovery runs at startup and every six hours while online.
- Queue ordering is global across all visible waiting work.
- Presence and game activity default on, with separate controls.
- Reverse-engineered writes are enabled capability by capability, never as one global switch.
- GOG tags and hidden state synchronize only after write validation.
- Saved views and artwork remain local.
- Two-way statistics include every proven, authoritative source but never inferred or fabricated values.
- Native chat is text-only and conditional; failure of its feasibility gate permanently drops it from this roadmap.
