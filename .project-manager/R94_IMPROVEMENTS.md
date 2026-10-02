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

## Approved sets in progress

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
  /tmp/ludomere-p252-report.md and /tmp/ludomere-p254-migration-review.md; exact build pending.
- Follow-ons queued separately: efficient image FIFO; GOG login page loading/error/retry feedback.

## Deferred findings and verification limits

Material ambiguous behavior or risky changes will be recorded here rather than guessed while the
user is unavailable. Private GTK fixtures can exercise controls but not real game/portal behavior.
