# R94 UX and performance audit

Authorized window: **2026-10-02 20:53:48–22:53:48 UTC**, stopping earlier on interruption.
Branch: `improvement/ux-performance-audit-2026-10-02`.
Baseline: upstream `d836fbe`, after PR6 merged.
No PR, push, package build/install, version bump, dependency or schema change.

## Delivered changes

Each row is a separate local commit. Every commit passed an exact post-commit
`cargo build --locked`; sets 1–21 used a detached verification checkout, and set 22 used the
frozen main checkout. Work concluded within the authorized two-hour window.

| Set | Commit | Task | Features and fixes |
| --- | --- | --- | --- |
| 1 | `0dd8af8` | P253 | Reuse installed-game map lookups; skip unchanged sidebar CSS, title, tooltip and opacity writes. Preserve colors, row identity, focus and selection. |
| 2 | `0e69cd7` | P251 | Batch cached catalog reads in groups of 400; index DLC parents; use a coherent read snapshot while preserving order, narrow lookup and error behavior. |
| 3 | `30fdce4` | P252 | Move installation-source inspection/preparation off GTK. Show backup, uninstall, install and restore stages, errors and stopped-worker feedback; preserve explicit consent and explain closing active progress. |
| 4 | `b7a45fd` | P256 | Replace shifting FIFO image-job vectors with a deque; preserve priorities, cancellation and four-worker scheduling. |
| 5 | `b2a451f` | P255 | Show operation-log loading and Retry; retain successful sources alongside partial failures; display installation errors without log files; reject stale results and guard profile reads. |
| 6 | `689c786` | P257 | Show embedded GOG sign-in loading, safe failures and Retry. Handle canceled navigation and closed/successful redirects without exposing OAuth URLs or raw browser errors. |
| 7 | `6b03690` | P259 | Reuse resolved download identities and read DLC membership once during automatic-install planning; replace an impossible-selection panic with an error. Preserve choices, legacy identities and consent. |
| 8 | `277888a` | P261 | Inspect preferred patches asynchronously. Show no-patch/error/full-update choices and patch stages/results in place; keep explicit Apply consent and stale/closed-view guards. |
| 9 | `a6fe580` | P260 | Share one filter predicate pass and matching-ID set for library counts and activity headers; preserve existing filtering, collapsed sections and incomplete-metadata notices. |
| 10 | `b383ad9` | P266 | Preserve sanitized diagnostic chains for update checks and update-policy loading; distinguish disconnected workers. |
| 11 | `656d145` | P263 | Skip unrelated Game Files child traversal during targeted local refresh. Retain root safety, archive checks, per-target validation and full Storage diagnostics. |
| 12 | `7fd0c7f` | P268 | Show explicit archive-patch requirements, progress and full terminal errors inline instead of opening background completion dialogs. Remove the now-unused patch wrapper. |
| 13 | `e326e21` | P267 | Bind manual cloud sync, Force confirmations and inventory to the originating auth/online/UI session. Reject stale actions/results, guard inventory activity and remove an unused fresh-session wrapper. |
| 14 | `4ed3ee3` | P269 | Batch catalog evidence used to classify library files; preserve represented-product scope, retired revisions, category fallback and malformed-row errors. |
| 15 | `b2a7c99` | P264 | Load Compatibility fixes asynchronously. Keep controls disabled while loading/failed, show error details and Retry, and populate saved overrides without autosaving defaults. |
| 16 | `1511b83` | P270 | Guard hidden-game and personal-tag writes across sign-out/reset. Show immediate visibility-save activity, preserve state on failure and restore controls with sanitized diagnostics. |
| 17 | `95b2f0f` | P272 | Keep complete, selectable verification results inline and across revisiting Files. Distinguish unavailable checksums from no downloads, correct stopped/determinate progress, and reject stale repair work. |
| 18 | `a5752fb` | P274 | Report achievement cache errors and offline empty/cached states accurately; preserve cache-first rendering, guard worker activity and clear obsolete account results. |
| 19 | `7a305df` | P275 | Read only appended installation-log data in bounded chunks. Handle observed truncation/replacement and split UTF-8, bound unfinished status lines, and check shutdown between chunks. Full saved logs remain untouched. |
| 20 | `e431fae` | P276 | Reject unsafe authoritative checksum filenames without normalization or fallback substitution; require 32 hexadecimal MD5 digits before metadata can drive repair decisions. |
| 21 | `8d02889` | P277 | Remove migration-dialog Close/Continue reference cycles and guard initial inspection with the originating account and profile activity. |
| 22 | `a456d48` | P279 | Remove an ignored GTK CSS overflow declaration that printed a warning at startup; preserve supported styling and layout. |

## Verification

- All 22 exact post-commit product builds passed. Logs: `/tmp/ludomere-r94-set1-build.log`
  through `/tmp/ludomere-r94-set22-build.log`.
- Final `tools/check.sh` passed on the complete product source at `8d02889`:
  formatting, all-target Clippy with warnings denied, **539 library tests, six integration tests,
  and five Python helper tests**. Log: `/tmp/ludomere-r94-final-check.log`.
- The default suite reported 35 ignored entries. Changed GTK controls were exercised separately
  with private Broadway displays and D-Bus sessions; some ignored child-process helpers are
  invoked by their integration tests. This is not a claim that every ignored test ran.
- Final build-process execution used a private HOME, all XDG roots and TMPDIR, offline Cargo,
  and removed inherited desktop/helper overrides. No package was built or installed.
- After the CSS-only set 22, formatting and all-target Clippy passed again. The existing ignored
  empty-profile startup test was explicitly run against the rebuilt source: **one passed**, and
  the unsupported CSS warning was absent. Its private D-Bus disabled service activation; private
  Broadway, HOME/all XDG/TMP and a dead-loopback proxy isolated desktop, credentials and outbound
  probes. No product/test source was changed for the harness. Logs/review:
  `/tmp/ludomere-empty-startup.log`, `/tmp/ludomere-p279-css-review.md` and
  `/tmp/ludomere-r94-css-clippy.log`. Nonfatal isolated GSettings-schema warnings remained.
- Focused tests and independent reviews were required before each implemented set was accepted.
  Shared test binaries were copied before parallel tests; exact-commit builds used a separate
  checkout until the final source freeze so another worker's uncommitted source could not
  accidentally satisfy a build. Final records-only closeout also receives a post-commit build.
- Final independent backend, UI and security/lifecycle reviews found no introduced blocker
  within the reviewed scope. Reports:
  `/tmp/ludomere-p278-backend-assurance.md`,
  `/tmp/ludomere-p278-final-ui-assurance.md`,
  `/tmp/ludomere-p278-security-review.md`.

### Focused evidence

| Area | Evidence |
| --- | --- |
| Sidebar/filter work | Private 1000-row GTK tests, unchanged-notification checks, state colors/identity/focus/selection, and a 21-scenario filter matrix. |
| Catalog/storage/planning | Seven focused cache tests; storage scope/category/corruption fixtures; four automatic-install planning tests; owner and independent 500-product evidence equivalence. |
| Login and logs | Synthetic loading/error/Retry/closed-state GTK tests; partial log sources, corrected private DB failures, error-only logs and stale results. Login tests did not instantiate a WebView or authenticate. |
| Migration and patches | Synthetic actual-control tests for inspection, stages, close/session behavior, errors/disconnection and weak dialog destruction; consent/preflight boundaries source-reviewed. No real patch/migration/helper execution. |
| Cloud, Compatibility, tags | Actual-control tests for stale confirmations, guarded worker entry, cache load failures/Retry, initialization without autosave, busy/error restoration and synthetic persistence. No real cloud/account/reset cleanup. |
| Verification/achievements | Owner and independent GTK tests for terminal outcomes, redaction, retained text, no background modal, offline cache states and stale/reset rejection. No real checksum request or repair deletion. |
| Log reader/checksum parser | Three focused log/status tests; three parser tests covering unsafe XML/fallback names, entity decoding, Unicode/spacing and valid/invalid hash formats. Independent reruns passed. |

### Measured or source-derived improvements

These are local synthetic fixtures or source/control-flow counts, not real-account or whole-app
performance promises.

- A 500-base-game/500-DLC cache fixture reduced product/catalog SELECTs from 3,001 to 10.
  Ten cached reads measured approximately 1.012 s before and 0.421 s after.
- A 1000-game sidebar fixture replaces 500,500 installed-ID comparisons with 1000 map lookups;
  20 unchanged refreshes emitted zero CSS notifications.
- Filter count/header work replaces roughly 1,001,000 ID comparisons with 1000 direct predicates
  and 1000 set memberships, excluding unchanged GTK row-filter callbacks.
- Five-request/three-DLC installation planning reduces repeated identity resolutions from 19 to
  five and base catalog reads from three to one.
- A 500-product storage classification fixture reduces catalog SELECTs from 1000 to four;
  independent local timing was approximately 28.5 ms versus 5.0 ms.
- A 500-sibling targeted refresh fixture measured approximately 12.9 ms for full inspection
  versus 0.21 ms for root-only inspection; full Storage diagnostics still perform full inspection.
- Twenty unchanged status-log polls read zero payload bytes after the initial read, instead of
  rereading more than 20 MiB in the synthetic fixture. Pending status text is bounded to 16 KiB;
  the complete saved log is unchanged.

## Deferred findings and limits

- **P273 title lookup optimization was not delivered.** Source review was favorable, but repeated
  synthetic GTK selection assertions remained inconclusive after adding the production filter
  and an unchanged-finalizer baseline. Its entire uncommitted delta was removed. This does not
  establish a production selection bug; earlier P253/P260 changes remain verified and committed.
- Grid scrolling still checks every card's bounds on adjustment signals. Coalescing needs
  separate timing/lifetime validation; it and the P273 title optimization are deferred.
- Initial Cloud Saves settings still read their record synchronously; moving the large
  record-dependent builder safely needs a separate design.
- Managed-file summary refresh still performs DB/file metadata work on GTK after deletion;
  asynchronous conversion needs per-view request generations and coordinated call sites.
- Existing runtime-log viewer reads are not registered as profile activity. UI epoch/logout guards
  reject obsolete display; its path/list/read helpers do **not** recreate profile directories.
  This is a pre-existing read/reset overlap, not a demonstrated profile-recreation bug.
- The HTTP audit did not establish an unbounded default timeout: the locked blocking reqwest
  client has a 30-second default, and the GOG client uses 45 seconds. No speculative timeout
  policy change was made; aggregate multi-request duration remains a separate concern.
- The incremental status reader detects observed shortening and inode replacement, not
  same-inode truncation/regrowth wholly between polls. Arbitrary filesystem I/O can still block.
- No live GOG login, real games/controllers, Windows helpers, cloud saves, destructive repair/
  migration, file-manager portals or installed-package integration was exercised. Synthetic GTK
  evidence is finite; no exhaustive application QA or global “Prototype ready” claim is made.

## Test-isolation incident

The first P263 owner storage-test invocation mistakenly inherited the normal HOME instead of
explicitly setting a private profile. An existing validation test failed. Its code can call
`StateStore::open` on the normal profile database, including directory/open/permission/schema
operations; the swallowed error means it is **not known how far that attempt proceeded**.
No claim of zero access or mutation is made, and no follow-up inspection of private user data was
performed. The corrected private HOME/all-XDG/TMP run passed all ten then-relevant storage tests;
an independent private regression also passed. All subsequent launches explicitly isolated those
roots. This incident was disclosed during the work and is retained here rather than hidden by the
successful rerun.
