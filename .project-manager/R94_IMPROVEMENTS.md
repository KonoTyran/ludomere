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
- P257 sign-in: loading indicator, generic safe failure with Retry, stopped-browser feedback,
  canceled-navigation/redirect teardown handling. Retry starts the public login URL; no raw OAuth
  URLs/errors displayed. Independent synthetic GTK1 PASS; no WebView/account/network execution.
- Wave2 shipping compilation, fmt/diff and all-target Clippy PASS. Each set gets its own commit
  and exact post-commit build. P259 request/DLC lookup reuse approved next, awaiting P256 commit.

## Deferred findings and verification limits

Material ambiguous behavior or risky changes will be recorded here rather than guessed while the
user is unavailable. Private GTK fixtures can exercise controls but not real game/portal behavior.
