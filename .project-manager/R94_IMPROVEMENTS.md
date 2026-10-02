# Timed UX and performance improvement pass

User-authorized window: 2026-10-02 20:53:48–22:53:48 UTC; stop on interruption.
Branch: `improvement/ux-performance-audit-2026-10-02`.
Baseline: upstream `d836fbe`, after PR6 merge. No new PR or push authorized/requested.

## Work log

- Initial read-only audits: backend efficiency, async dialog feedback, library/detail rendering.
- Each implemented set gets a separate commit, independent review, relevant tests and a verified
  post-commit build. Tests use synthetic fixtures and private profiles; the P263 isolation mistake
  and its uncertain impact are documented below. No real account/helper operations were requested.

## Current completion index

| Commit | Task | Change |
| --- | --- | --- |
| 0dd8af8 | P253 | Reuse installed-game lookups and avoid unchanged sidebar widget writes. |
| 0e69cd7 | P251 | Batch cached catalog reads and index DLC parents in a coherent snapshot. |
| 30fdce4 | P252 | Move source migration preparation off GTK and show meaningful stages/results. |
| b7a45fd | P256 | Use a FIFO image queue without repeated vector shifting. |
| b2a451f | P255 | Show operation-log loading, partial failures, Retry and fileless errors. |
| 689c786 | P257 | Show embedded sign-in page loading, safe failures and Retry. |
| 6b03690 | P259 | Reuse resolved download identities and DLC membership during auto-install planning. |
| 277888a | P261 | Inspect preferred patches asynchronously with consent and in-place progress/results. |
| a6fe580 | P260 | Reuse one filter predicate pass for counts and sidebar headers. |
| b383ad9 | P266 | Preserve sanitized update-check and policy-loading error details. |
| 656d145 | P263 | Skip unrelated game-folder traversal during targeted local refresh. |
| 7fd0c7f | P268 | Show explicit archive patch progress and terminal details in the existing row. |
| e326e21 | P267 | Bind manual cloud actions and confirmations to the originating account. |
| 4ed3ee3 | P269 | Batch catalog evidence used to classify stored files without changing validation. |
| b2a7c99 | P264 | Load Compatibility preferences asynchronously with error details and Retry. |
| 1511b83 | P270 | Guard hidden-game and tag saves across sign-out/reset and report save failures. |
| 95b2f0f | P272 | Keep verification results inline, distinguish unavailable checksums and reject stale repairs. |
| a5752fb | P274 | Report achievement cache failures accurately and guard loading sessions. |
| 7a305df | P275 | Read installation-status logs incrementally with bounded memory. |
| e431fae | P276 | Reject unsafe authoritative checksum filenames and malformed MD5 values. |

Every indexed commit passed its exact post-commit `cargo build --locked` in an isolated checkout.
The numbered `/tmp/ludomere-r94-setN-build.log` files correspond to table order. Notes below are
chronological evidence, including intermediate states; this index is the current disposition.

P269 batched storage classification, P264 Compatibility preference loading and P270 account-bound
hidden/tag writes are source-ready under independent review and coordinated private testing.

- P269 independent source and private 500-product regression PASS, with original category/root/
  retired/error semantics retained. Catalog SELECTs 1000→4; independent synthetic timing 28.503ms
  →4.960ms, not end-to-end refresh timing. Wave5 shipping compilation/fmt/diff/all-target Clippy
  PASS. Report `/tmp/ludomere-p270-storage-review.md`; committing as set14.
  Commit4ed3ee3 exact post-commit build PASS; owner storage11 and independent regression1 PASS.
- P264 owner and independent private GTK1 each PASS, including error/Retry/stored overrides,
  no initial autosaves, uninstalled/stale controls and group release. Review
  `/tmp/ludomere-p264-independent-review.md`; committing as set15.
  Commitb2a7c99 exact post-commit build PASS.
- P270 hidden/tag writes now register profile activity and the original account commit guard,
  show immediate visibility-save feedback, preserve pending behavior and retain sanitized errors.
  Owner and independent private GTK1+pure1 each PASS; independent security review PASS
  `/tmp/ludomere-p271-organization-review.md`. Committing as set16.
  Commit1511b83 exact post-commit build PASS.
- P272 verification feedback/account guards, P273 rebuild title lookup reuse and P274 achievement
  loading/account guards approved next. Product edits remain bounded; no new edits after22:25 UTC.
- P272 verified owner+independent private GTK1 each PASS, source/session/lock review PASS. Full
  sanitized verification outcomes stay inline; unavailable checksums and disconnected workers are
  explicit, progress remains determinate where known, stale account/reset work is rejected.
  Existing repair choices retained. `/tmp/ludomere-p272-report.md` and
  `/tmp/ludomere-p275-verification-review.md`. Wave6c shipping compile/all-target Clippy PASS after
  a style-only collapsible-if correction. Committing set17; no real verification/repair executed.
  Commit95b2f0f exact post-commit build PASS.
- P274 owner and independent offline private GTK1 each PASS: cache error/Retry/empty/cached/reset
  reservation/account-change behavior. Worker activity and original-session guard, sanitized useful
  diagnostics and truthful offline cache status preserved. Review
  `/tmp/ludomere-p274-independent-review.md`; committing set18.
  Commita5752fb exact post-commit build PASS.
- Final implementation wave is P273 title lookup, P275 incremental status-log reads, P276 strict
  authoritative checksum name/hash validation, P277 migration callback/preflight lifecycle.
  Cumulative independent backend review found no added critical issue; UI review identified P277.
  No real integration/delete/repair/helper tests are authorized by these changes.
- P275 owner and independent synthetic log-reader2 each PASS; source review PASS. New reader
  reads appended64KiB chunks, bounds unfinished status lines16KiB, handles observed truncation/
  inode replacement and split UTF-8, preserves normal statuses and stops between chunks. Raw log
  untouched. Twenty unchanged polls read0 payload bytes versus over20MiB previously in fixture.
  `/tmp/ludomere-p275-report.md`, `/tmp/ludomere-p276-log-review.md`. Wave7 shipping compile,
  fmt/diff/all-target Clippy PASS. Committing set19.
  Commit7a305df exact post-commit build PASS; existing status-mapper regression also PASS.
- P276 rejects unsafe authoritative checksum filenames and malformed MD5 before they can drive
  repair decisions; valid Unicode/spacing/multipart names and upper/lower hashes preserved exactly.
  Owner and independent parser3 each PASS, source/security review PASS
  `/tmp/ludomere-p278-checksum-review.md`. No network or deletion tests. Committing set20.
  Commite431fae exact post-commit build PASS.
- P277 removes migration-dialog Close/Continue reference cycles and guards initial inspection
  with original account/profile activity. Owner and independent extended private GTK1 each PASS,
  including weak destruction and stale/reset rejection before database creation. Review
  `/tmp/ludomere-p276-migration-review.md`; committing set21.
- P273 title-map optimization deferred: repeated synthetic GTK fixture selection assertions failed,
  including when exercising the unchanged sidebar finalizer and registering the production filter.
  This is insufficient evidence of a production defect or safe full-control regression pass.
  Owner removes only the uncommitted P273 delta; previously verified P253/P260 remain. Do not claim
  the proposed title optimization or a sidebar-selection fix as delivered.

## Completed sets

- P253 sidebar refresh (reviewed, committing): use the existing installed-game map instead of
  cloning and linearly searching; skip unchanged CSS/title/tooltip/opacity writes. State/color
  policies and row/focus/selection identity preserved. Two focused unit tests and one private
  1000-row GTK regression pass; unchanged20 refreshes emit zero CSS notifications. Synthetic
  installed lookup compares500,500 entries before versus1000 map lookups afterward (1.60ms versus
  0.125ms locally; not whole-app timing). Independent P254 sidebar review PASS. Exact post-commit
  build PASS at0dd8af8 (/tmp/ludomere-r94-set1-build.log). Evidence /tmp/ludomere-p253-gtk.log and
  /tmp/ludomere-p254-sidebar-review.md.

## Further completed sets

- P251 cache loading (reviewed, committing): batch selected product catalogs in400-ID groups and
  index DLC parents; one deferred read snapshot, narrow lookup/order/error semantics retained.
  Seven focused tests pass, including500 bases+500 DLC/2000parts/builds and existing caller
  transactions. Synthetic10 reads:1.012s baseline→0.421s patched (independent0.402s); source-derived
  SELECTs3001→10. Independent P254 state review PASS. Reports /tmp/ludomere-p251-report.md and
  /tmp/ludomere-p254-state-review.md. Commit0e69cd7 exact post-commit build PASS
  (/tmp/ludomere-r94-set2-build.log).
- P252 source-change feedback (reviewed, committing): worker preflight/target preparation replaces
  GTK filesystem/DB work; immediate activity, truthful backup/uninstall/install/restore stages,
  duplicate prevention, Close explanation, terminal results, stopped-worker/branch errors and
  account-change feedback. Existing migration/saves policy retained. Private actual GTK test and
  independent P254 review PASS; final account callback correction source-reviewed. Reports
  /tmp/ludomere-p252-report.md and /tmp/ludomere-p254-migration-review.md. Commit30fdce4 exact
  build PASS (/tmp/ludomere-r94-set3-build.log).
## Approved sets in progress

- P255 operation logs: named loading state, explicit partial-read failures and Retry, retained
  successful log sources, installation failures without a log file, disconnected-worker errors,
  stale request/account guards and profile-reset activity guard. Shipping private GTK1+pure3
  PASS; independent source review PASS, independent execution pending. Report p255-report.md.
- P256 image scheduling: FIFO VecDeque removes repeated shifting of all pending jobs under the
  queue mutex; explicit priority ordering, four workers and cancellation behavior unchanged.
  Three focused tests PASS, independent P258 review and2 reruns PASS. Commitb7a45fd exact
  post-commit build PASS (/tmp/ludomere-r94-set4-build.log).
- P257 sign-in: loading indicator, generic safe failure with Retry, stopped-browser feedback,
  canceled-navigation/redirect teardown handling. Retry starts the public login URL; no raw OAuth
  URLs/errors displayed. Independent synthetic GTK1 PASS; no WebView/account/network execution.
- Wave2 shipping compilation, fmt/diff and all-target Clippy PASS. Each set gets its own commit
  and exact post-commit build. P259 request/DLC lookup reuse approved next, awaiting P256 commit.
- P255 final independent source/GTK review PASS, /tmp/ludomere-p258-logs-review.md; committing.
  Commitb2a451f exact post-commit build PASS (/tmp/ludomere-r94-set5-build.log).
- P257 owner GTK1+account1 and independent GTK1 PASS, /tmp/ludomere-p258-login-review.md.
  Commit689c786 exact post-commit build PASS (/tmp/ludomere-r94-set6-build.log).
- P259 automatic-install request/DLC lookup reuse source ready; P260 filter count reuse and
  P261 asynchronous preferred-patch inspection/in-place activity feedback approved and implementing.
- P259 focused automatic-install4 PASS: selection/legacy identity, parts+DLC/reopen dispatch once,
  blocked prerequisites/consent and existing/untracked payload protection. For the retained5-request,
  3-DLC fixture, source-derived expensive identity calls19→5 and base catalog reads3→1. No selection,
  installation or consent policy changed. Wave3 shipping compile/fmt/diff/all-target Clippy PASS.
  Independent P262 source+4 rerun PASS; commit6b03690 exact post-commit build PASS
  (/tmp/ludomere-r94-set7-build.log).
- P261 preferred-patch dialog: worker inspection with explicit no-patch/error/full-update choices,
  Apply consent retained, pulsing active stages, in-place terminal/disconnect feedback and no
  completion focus-stealing modal. Close-before-start prevents launch; started work continues.
  Profile/session guards and weak callbacks retained. Private GTK1 PASS, independent P262 PASS
  /tmp/ludomere-p262-patch-review.md. No real patch/helper executed. Committing next.
  Commit277888a exact post-commit build PASS (/tmp/ludomere-r94-set8-build.log).
- P260 filter fixture corrections: numeric row names and expected incomplete-metadata search
  notice were missing from the test. Production code unchanged; final GTK rerun required before
  commit. Intermediate run passed count/header/collapse/selection/focus checks before final text.
  Final owner+independent GTK1 each PASS; pure21-case matrix PASS. Count/header work now shares
  one direct predicate pass and matching-ID set; for1000 games source-derived ID comparisons
  1,001,000→1000 direct predicates+1000 memberships, excluding unchanged GTK row callbacks.
  Final review /tmp/ludomere-p262-filters-review.md; wave3c fmt/diff/Clippy PASS.
  Commita6fe580 exact post-commit build PASS (/tmp/ludomere-r94-set9-build.log).
- P266 update-check/policy-load errors now retain sanitized diagnostic chains and distinguish
  stopped workers; existing recovery/control rules unchanged. Focused3 PASS, independent source
  review+sanitizer PASS /tmp/ludomere-p265-update-diagnostics-review.md. Await separate commit.
  Commitb383ad9 exact post-commit build PASS (/tmp/ludomere-r94-set10-build.log).
- P267 confirmed account-safety finding: manual cloud sync/force confirmation may capture a new
  session after the originating Properties view becomes stale. Approved bounded session-binding
  fix before compatibility loader work; no real remote-save actions will be exercised.
- P263 targeted local refresh: root-only Game Files inspection skips sibling traversal while
  preserving root/archive validation and full Storage diagnostics. Owner isolated storage10 PASS;
  independent500-sibling regression PASS, full12.9ms→root0.211ms locally (not whole-app timing).
  Review /tmp/ludomere-p268-storage-review.md. Initial owner non-isolated invocation had an
  environment-dependent existing test failure; isolated full focused run passes, source unchanged.
  Isolation incident: the first invocation inherited HOME=/home/chris; validation can call
  StateStore::open at the normal profile database (create/open/chmod/schema read). The swallowed
  error prevents determining how far it got; do not claim no access or mutation. No follow-up
  real-profile inspection was performed. All subsequent owner/reviewer runs explicitly isolate
  HOME/allXDG/TMP; this limitation is retained rather than treating the first run as valid evidence.
- Wave4 compilation/fmt PASS. Clippy identified obsolete patch_with_components after both callers
  migrated; P268 owner removed that unused wrapper instead of suppressing the warning. P268 GTK1
  PASS; P267 GTK fixture waiting on synthetic dialog teardown requires correction before commit.
  Corrected fixture uses actual Cancel/Force buttons, not a raw response signal that does not
  close the widget. Final P267 owner+independent GTK1 each PASS, cloud pure2 PASS. No real cloud
  calls. Wave4b all-target Clippy PASS after obsolete wrapper removal.
- P268 explicit Run Patch rows now pulse during requirements/applying, retain sanitized details
  and terminal/disconnect feedback in place, suppress stale account/view actions, and no longer
  open background completion/error modals. Unused patch_with_components removed. GTK1 PASS and
  independent review PASS /tmp/ludomere-p268-patch-review.md. Separate commit next.
  Commit7fd0c7f exact post-commit build PASS (/tmp/ludomere-r94-set12-build.log).
- P263 commit656d145 exact post-commit build PASS (/tmp/ludomere-r94-set11-build.log).
- P267 manual cloud Normal/Force/Check now actions carry original auth+online+UI account identity
  through confirmation, worker and result; stale actions/results disable with reopen guidance.
  Inventory joins profile activity; diagnostics sanitized and unused fresh-session sync wrapper
  removed. Owner GTK1+pure2 and independent GTK1/security review PASS
  (/tmp/ludomere-p268-cloud-review.md). No real cloud/keyring/save operations. Committing next.
  Commite326e21 exact post-commit build PASS (/tmp/ludomere-r94-set13-build.log).

## Deferred findings and verification limits

Material ambiguous behavior or risky changes will be recorded here rather than guessed while the
user is unavailable. Private GTK fixtures can exercise controls but not real game/portal behavior.

- Network-preparation timeout audit found no unbounded-default defect: locked reqwest0.12.28
  blocking Client::new uses30s default timeout, verified in local dependency source. Existing
  gog::client uses45s. No speculative network/deadline policy change made; aggregate multi-request
  duration remains distinct from per-request bounds.
- Grid scrolling still checks all card bounds on each adjustment signal. Coalescing that work may
  help, but introduces timing/lifecycle behavior; deferred while the smaller title lookup fix proceeds.
- Game Properties Cloud Saves still reads its initial record synchronously and falls back on read
  failure; fixing safely requires separating its large record-dependent builder. Deferred pending
  a bounded design. The narrower Compatibility fixes read/loading/Retry change is approved next.
- Managed-file summary labels still read DB/file metadata on GTK after deletion. Async conversion
  needs proper per-view request generations and coordinated call-site changes; deferred rather
  than introduce stale-result races or a global cache.
