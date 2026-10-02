# Timed UX and performance improvement pass

User-authorized window: 2026-10-02 20:53:48–22:53:48 UTC; stop on interruption.
Branch: `improvement/ux-performance-audit-2026-10-02`.
Baseline: upstream `d836fbe`, after PR6 merge. No new PR or push authorized/requested.

## Work log

- Initial read-only audits: backend efficiency, async dialog feedback, library/detail rendering.
- Each implemented set gets a separate commit, independent review, relevant tests and a verified
  post-commit build. No real profile/game files or account/helper operations used for testing.

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
- P267 confirmed account-safety finding: manual cloud sync/force confirmation may capture a new
  session after the originating Properties view becomes stale. Approved bounded session-binding
  fix before compatibility loader work; no real remote-save actions will be exercised.

## Deferred findings and verification limits

Material ambiguous behavior or risky changes will be recorded here rather than guessed while the
user is unavailable. Private GTK fixtures can exercise controls but not real game/portal behavior.

- Network-preparation timeout audit found no unbounded-default defect: locked reqwest0.12.28
  blocking Client::new uses30s default timeout, verified in local dependency source. Existing
  gog::client uses45s. No speculative network/deadline policy change made; aggregate multi-request
  duration remains distinct from per-request bounds.
- Targeted local refresh still inspects every game child before reconciling selected products.
  Proposed later root-only inspection can retain root/archive validation and full Storage
  diagnostics; pending separate ownership/review rather than weakening validation opportunistically.
- Game Properties Cloud Saves still reads its initial record synchronously and falls back on read
  failure; fixing safely requires separating its large record-dependent builder. Deferred pending
  a bounded design. The narrower Compatibility fixes read/loading/Retry change is approved next.
- Managed-file summary labels still read DB/file metadata on GTK after deletion. Async conversion
  needs proper per-view request generations and coordinated call-site changes; deferred rather
  than introduce stale-result races or a global cache.
