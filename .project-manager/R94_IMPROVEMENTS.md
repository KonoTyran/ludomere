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
  build result will be recorded after commit. Evidence /tmp/ludomere-p253-gtk.log and
  /tmp/ludomere-p254-sidebar-review.md.

## Approved sets in progress

- P251: batch repeated product revision/build queries during cached library loading; preserve
  single-product scope and DLC ordering, benchmark synthetic500-game data.
- P252: move source-change inspection/preparation off GTK, display actual migration phases and
  stopped-worker errors, prevent duplicate submissions; keep existing source/saves policy.
- Follow-ons queued separately: efficient image FIFO; GOG login page loading/error/retry feedback.

## Deferred findings and verification limits

Material ambiguous behavior or risky changes will be recorded here rather than guessed while the
user is unavailable. Private GTK fixtures can exercise controls but not real game/portal behavior.
