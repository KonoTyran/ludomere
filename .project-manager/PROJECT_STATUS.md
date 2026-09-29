# Project status

Last meaningful update: 2026-09-29.

User explicitly authorized committing the accumulated session changes locally with a comprehensive
feature inventory in the commit message. Independent commit-content review cleared the70-file
source/docs/tests/tooling/records inventory; generated packages, caches and test profiles are excluded.
No push or publication requested. Prior no-commit restriction below is superseded for this commit.

Phase: P91 download cleanup/defaults and P92 detail image/layout fixes complete and independently verified.
P90 setup picker/path/sign-in adjustments complete and independently verified.
P88 targeted retry/footer/download defaults and P89 guided setup complete and independently verified.
P87 screenshot navigation, sidebar icons and footer corrections complete and independently verified.
P86 synchronization/loading feedback implemented and independently verified.
P84 startup crash and P85/R18 Settings reset toggle are implemented and independently verified.
Real-account, real-game and desktop acceptance remains outstanding.
**Prototype readiness has not been declared.**

## Current downloads and detail media

- R24/R25 user requests Extras unchecked, Manage delete downloads, optional uninstall cleanup,
  failed screenshot arrow layout and frequent detail-image failure investigation. Scope clear;
  unchecked destructive checkbox and existing managed-file boundaries are conservative defaults.
- Acquisition owns backend download cleanup and image investigation; compatibility UI/config/docs;
  security independent review with early destructive-path assessment. No actual user files deleted.
- Preserve all earlier behavior, no package/dependency/schema expansion; tests private HOME/allXDG/
  runtime/bus. Required checks and actual changed-control GUI evidence before completion.
- R25 anonymous official response for shown slug/product1587851209 uses dlcs:[], with nine
  screenshots. Initial inference that object deserialization rejects this was disproved by an actual
  serde probe: defaulted products accepts the empty sequence. Owner removes unnecessary parser
  change; manager corrected the earlier user-facing diagnosis. Exact live Product failure remains
  under investigation; evidence /tmp/ludomere-p92-public-product.json. Confirmed findings: empty strip
  always creates overlay arrows, screenshots trust nonempty cache and race shared .part, optional
  logo/icon failure discards successful hero. Scoped corrections under implementation/review.
- Asked one Extras preference question: always start unchecked/remove conflicting preference versus
  keep existing preference with new-profile default off. Independent cleanup/media work continues.
- User selected keeping Extras preference with default off for new profiles. Existing explicit
  saved values remain; UI owner notified, no Settings control removal.
- P91 agreed contract: async opaque managed-file snapshot/count/bytes, revalidated exact deletion
  with per-file failure result, and optional snapshot in existing uninstall journal (no schema change).
  Manager-owned success hook survives page closure; failed/cancelled uninstall never cleans downloads.
  Existing operation gate and serialized download-manager intent revocation protect active work.
  Independent review requires anchored nofollow paths and stable file identity, disposable sentinels.
- Early reviewer finding: post-uninstall blocking permit acquisition inside download dispatcher
  could freeze Pause/Remove/Shutdown behind a transfer. Owner uses nonblocking acquisition and
  releases uninstall permit before Complete; contention reports cleanup retry rather than waits.
  Partial snapshot retry/index reconciliation also under correction and focused verification.
- Reviewer identified write-on-load config use in asynchronous cleanup preview; owner must use
  read-only configuration access so preview/cancel cannot overwrite newer Settings. No global
  config refactor authorized. UI mirrors cleanup results into existing footer for sidebar/overview
  visibility, with bounded notice lifetime and no background modal/navigation.
- Actual partial/corrupt/valid screenshot GUI settles with Retry and no Library information overlap;
  physical modal Retry issues only failed screenshot request and repairs image. Post-retry keyboard
  navigation observation is under investigation before classification. Cleanup GUI remains pending.
- Focused backend passes5 cleanup,22 retained online and1 uninstall-consent tests (checkpoint had
  an extra parser-only test subsequently removed). Full prior Product also parses official fixture;
  no parser change remains. Report /tmp/ludomere-p91-backend-report.md explicitly withdraws diagnosis.
- Reviewer confirmed Retry hiding its focused button broke modal Left/Right; owner moved handler
  to window with same-gallery lifetime guard and removal on close, without focus grab. Two isolated
  GTK gallery tests pass; independent physical retry→key recheck pending.
- Initial final suite passed332 unit tests then restart integration's fixture server hit BrokenPipe
  writing to intentionally closed socket before accepting recovery. Isolated exact test passes0.02s;
  no fixture or production recovery edits. Failed log preserved. Final exact checks follow tiny
  cleanup count fix so already-absent retry entries are not reported as newly deleted files.
- Final exact fmt/Clippy/full332 unit/six integration/debug/diff checks pass after count fix;
  two affected GTK gallery tests pass separately. Debug timestamp2026-09-29 15:14:02 -0400.
  Reports /tmp/ludomere-p91-{backend,ui}-report.md; logs /tmp/ludomere-p91-{fmt,clippy,test,build}.log.
- Independent GUI confirms uninstall checkbox default off/toggle/Cancel and Manage Cancel preserve
  six sentinels. Confirmed deletion with one previewed file replaced by symlink removes only safe
  installer, preserves target/index/payload/save/preferences and shows partial error. Remaining
  restored-file retry, final keyboard/empty/Extras controls pending; no new product blocker reported.
- Final physical keyboard check disproved window-only controller fix: actual Left/Right did not
  dispatch even before Retry, though emitted-controller GTK regression passed. P92 reopened to UI
  owner; restore proven modal routing and handle focus on direct user Retry action. Actual pointer/
  keyboard acceptance required before refreshed checks; emitted signals alone do not close finding.
- Revised final fix restores overlay capture and moves focus synchronously on direct Retry click
  to stable gallery container; completion never grabs focus. Independent physical Left3→2, Retry,
  Right3→wrap1→Left3 and Escape pass without Tab. Empty section and final cleanup retry pass;
  all sentinels/preferences survive except explicitly deleted downloads. No scoped blocker remains.
- Exact-final rerun after revised fix passes fmt/Clippy/332 unit/six integration/debug/diff plus two
  isolated GTK regressions; binary2026-09-29 15:24:35 -0400. Reviewer preparing final report with
  original live Product failure explicitly unverified; no parser fix or real-uninstaller claim.
- Final independent report /tmp/ludomere-p91-review.md PASS, no scoped blocker. Manager reviewed
  reports, exact final logs and screenshots of partial media/cleanup. Actual cleanup Cancel/partial/
  retry and marker/payload/save/favorite/tag/playtime preservation pass; new-profile Extras visually
  off, existing values regression-tested. Final physical Retry/keys/wrap/Escape and empty screenshot
  state pass (/tmp/ludomere-p91-gui-keys-final.log). Only explicit disposable files deleted; private
  apps exited. No package built, no real uninstall/helper/account used. P91/P92 complete; live gates
  remain P70 and exact original service failure is not claimed reproduced.

## Current setup refinements

- R23/P90: directory pickers for both setup paths, one-way game→downloads suggestion, full visible
  Proton paths during selection, and separate optional GOG modal after other settings save.
- User scope is clear; no additional permission/interview round. UI compatibility implements;
  security reviews independently. Preserve prefilled preferences, deferral and explicit downloads.
- All verification isolated; no actual profile/credentials/helper or game execution. Prior P88/P89
  gates remain passed; real-account/game/desktop acceptance remains outstanding.
- Implementation checkpoint: both asynchronous folder pickers landed with cancellation/account guards;
  initial values precede the one-way game-folder change handler. Proton popup strings wrap, and a
  selectable full-path preview follows selection before Apply. Successful config persistence precedes
  model commit and the final existing GOG modal; save failure retains edits and does not advance.
  All-target compile passed; required checks and independent actual GUI review remain pending.
- Frozen candidate passes fmt, warnings-denied Clippy,323 unit/six integration tests and debug
  build, with private HOME/allXDG/session bus. Manager inspected report and final logs:
  /tmp/ludomere-p90-ui-report.md and /tmp/ludomere-p90-{fmt,clippy,test,build}.log.
  Debug binary timestamp2026-09-29 13:45:54 -0400. Independent GUI/save-failure gate pending.
- Independent GUI exposed a required persistence defect: Config::load normalization overwrites
  download_directory with installer_library.path, losing independent prefill/restart choices. Narrow
  config.rs correction and load/save regression delegated to compatibility; reviewer continues
  remaining controls and will recheck final persistence. This is required R23 support, not new scope.
- Removed only the normalization overwrite. Actual save/load regression fails before/passes after
  and preserves other library normalization. Corrected candidate passes fmt/Clippy/debug and serial
  full suite324 unit/six integration tests; timestamp2026-09-29 13:52:14 -0400. Initial parallel run
  hit existing Comet lock EWOULDBLOCK; subprocess inheritance is suspected, not proven. Preserved
  /tmp/ludomere-p90-parallel-test.log; serial run passes without Comet changes. GUI review pending.
- Final independent PASS: actual GTK pickers select/cancel spaces/long paths; separate download
  override persists across restart. Physical popup mouse/keyboard and unchanged-index Refresh
  show full Proton paths before Apply. Invalid path and injected save failure retain form/no auth;
  retry saves before separate optional login, close preserves settings, authenticated completion
  skips redundant login. Cancelled drafts do not save or sign in. Manager inspected final reports,
  PASS logs and full-path popup screenshot. Evidence /tmp/ludomere-p90-review.md,
  /tmp/ludomere-p90-evidence-final.log, /tmp/ludomere-p90-proton-evidence.log and
  /tmp/ludomere-p90-restart-final.log. Exact setup/proton/config match disposable source copy;
  only loopback/auth-storage fixture substitutions. No product blocker remains. No package rebuilt;
  live account/game/desktop portal gates remain explicit. All private app processes exited.

## Current retry and defaults work

- R21/P88: failed-images-only Retry, dismissible error, full re-sync in Settings, centered Downloads
  and right status; default-on functional install-after-download. R22/P89: defaults-first Windows
  actions and first-launch guide. Asked setup breadth, deferral and existing-profile introduction.
- Source confirms footer Retry invokes entire sync, and Install after downloading is currently a
  disabled placeholder with future-support tooltip. UMU source runs require staged helpers and an
  explicit environment path; investigation must distinguish missing staging from faulty detection.
- acquisition backend; compatibility UI/config/docs; security independent review and initial UMU
  diagnosis. P88 clear behavior may proceed while setup-dependent decisions wait for answers.
- All tests private HOME/XDG/runtime/bus, no real profile/credential or game/helper execution.
- Interview complete: folders/Proton/runtime/optional GOG; Windows may be deferred with Finish
  setup action; show existing profiles a one-time guide prefilled with their current settings.
  Both P88 and P89 implementation authorized. No new permission question needed.
- Independent source/build-artifact inspection confirms prepared target/helpers/umu/umu-run exists,
  but executable lookup checks only explicit absolute environment override or package path. Narrow
  development-only lookup anchored to executable/build location is authorized to UI owner in umu.rs;
  preserve override/package precedence and reject arbitrary working-directory discovery. No helper
  download/execution or new dependency required. Missing-helper guidance must distinguish source run.
- Functional automatic installation needs durable intent; existing checkbox has neither persistence
  nor a completion listener. Authorized narrow SQLite operation support (existing plan reuse if safe,
  otherwise target25 internal devrevision only with mandated migration tests), rather than separate
  files. User-confirmed queue choice must survive restart and avoid duplicate installs; removal
  cancels pending intent, prerequisites fail visibly without background prompts or acquisitions.
- Existing installation_operations is legacy input immediately migrated to runnable journals, so
  waiting-download intent cannot safely reuse it. Approved dedicated download_install_intents table
  in target25/devrevision6; handoff uses tagged existing installation journal/queue for crash-safe
  deduplication. Select one base by saved preferences plus matching selected DLC; extras/patch-only
  do not trigger install, and existing installed payloads are preserved rather than reinstalled.
  Independent reviewer is reviewing this boundary before final implementation.
- Review found automatic-install draft rejected any existing target directory, but default downloads
  already create installer-only children under the game folder. Fix must distinguish managed
  installers from installed payload/collisions and preserve both. Independent reviewer additionally
  requires checking existing installs across configured libraries before choosing a new default.
  These are required correctness fixes with shared-root/sentinel regressions, not scope expansion.
- Focused backend evidence now passes4 automatic-install tests,2 journal handoff/revocation tests,
  30 state tests including revision1–5→6 and1 exact failed-image retry/session test. Logs:
  /tmp/ludomere-p88-auto-test.log and /tmp/ludomere-p88-backend-tests.log. Reviewer found orphan
  tagged journal could bypass removed consent; owner added matching-consent checks in recovery
  and queued start. Final independent disposition and required full checks remain pending.
- Setup detached whole-config saves were replaced by current-config commit using existing small
  UI persistence pattern; folder validation/creation/preflight remain off GTK. Guide prefill preserves
  installer_library_id rather than replacing it with another library default. GUI confirms deferral,
  optional-login cancellation, persisted settings and one-time reopening behavior. Remaining controls
  use physical input where AT-SPI exposes no action. No real account or installer exercised.
- Final UI review identified false image-Retry eligibility after core failure before image jobs exist;
  narrow correction authorized so the notification points to Settings/full sync instead of offering
  an unusable image retry. Dismiss remains available; no full-sync fallback for image Retry.
- Independent physical GUI verified failed-image Retry emits exactly cover55/56 and icon57 requests,
  with zero ownership/core/detail/success-image requests and no navigation. Dismiss hides notice
  without requests, full synchronization in Settings generates core requests and resurfaces errors,
  Downloads is centered and opens queue, right status remains. Manager viewed footer screenshot.
  Evidence: /tmp/ludomere-p88-gui-final.log, /tmp/ludomere-p88-final-http-requests.json and
  /tmp/ludomere-p86-qa-zthovg6u/. Checkbox/fresh-profile/ready-Windows checks still pending.
- New batch enqueue now carries captured account generation; registration/auth availability changes
  hold existing account lock through validation and writes, released before installation preparation.
  This closes stale queued commands after sign-out without broader auth redesign. Backend frozen;
  owner report /tmp/ludomere-p88-backend-report.md. Exact-final required checks underway.
- Exact-final source passes fmt, warnings-denied Clippy,323 unit/six integration tests and debug
  build. Separate inert-file real-GTK Windows test passes: ready callback once/no modal and missing
  saved selection actionable. Logs /tmp/ludomere-p88-{fmt,clippy,test,build}.log and
  /tmp/ludomere-p89-ready-gtk.log. Final debug timestamp2026-09-29 12:10:37 -0400.
- Actual download chooser shows install-after checked by default; physical uncheck and Cancel pass,
  with no queued payload or installer executed (/tmp/ludomere-p88-download-final.log). Cold GUI
  scripts required pointer-coordinate intervention for controls lacking AT-SPI actions and a cached
  account menu labelled Sign in again; these harness issues were not treated as product failures.
  Final fresh-profile setup validation and independent report reconciliation remain pending.
- Final P88/P89 disposition: complete, independent PASS with no scoped blocker. Fresh profile
  rejects relative folders, persists chosen absolute folders, permits Windows deferral and retains
  one-time behavior/Finish setup after restart. Manager reviewed final reports, logs and footer
  screenshot. Evidence /tmp/ludomere-p89-fresh-evidence.log; independent report
  /tmp/ludomere-p88-review.md; backend/UI reports /tmp/ludomere-p88-{backend,ui}-report.md.
  Exact-final checks and binary timestamp above remain valid; no further product changes or package
  rebuild. Automatic installation uses inert dispatch/journal evidence here, not actual installer
  execution; real account/game/component-download/desktop gates remain explicit.

## Current interaction corrections

- R20/P87 captures new screenshot Left/Right navigation, sidebar icons alongside covers, footer
  Retry interception and persistent sign-in status. Backend acquisition, UI compatibility,
  independent QA/security security. Source fixes and private fixture testing are authorized.
- Previous P86 Retry evidence used accessibility actions; new review must use physical pointer
  events to catch parent-button interception, plus actual modal keys and sign-in completion.
- Explicit private HOME/XDG/runtime and session buses required for every test. No user profile or
  credentials accessed. Broader real-account/game gates remain outstanding.
- Source confirms footer Retry is nested inside a Downloads-action Button, successful token
  exchange never replaces its initial status, and core assets contain icon URLs but only covers
  are downloaded. Screenshot buttons already wrap; keys will share those semantics.
- Independent reviewer will reproduce pointer interception on the prior binary and exercise actual
  local WebView callback/token exchange using a documented disposable origin/auth fixture. Real
  credentials/keyring remain excluded; production status handling is retained in that fixture.
- P87 before-fix pointer reproduction confirmed Retry navigates to Downloads with HTTP requests
  unchanged3→3 (/tmp/ludomere-p87-before-evidence.log). Prior accessibility activation bypassed
  the problematic mouse route. Backend contract uses interleaved cover/icon jobs in the same four
  workers, independent events/totals and targeted icon persistence; no rich metadata acquisition.
- Reviewer found the touched sign-in failure branch could display a raw token-exchange request URL
  containing authorization query values. Narrow reuse of safe error classification is assigned to
  UI owner with synthetic HTTP503/no-leak verification; no wider auth redesign is authorized.
- Backend handed off frozen:20 online and30 state tests pass.125 products produce250 bounded
  interleaved outcomes; cover/icon failures remain independent,123 successful icons survive database
  reopen, and warm cache avoids HTTP. Manager inspected report/logs; integrated GUI review pending.
  Evidence: /tmp/ludomere-p87-backend-report.md and /tmp/ludomere-p87-{online,state}-tests.log.
- Integrated private GUI passed actual WebView callback cancellation/failure/success with no stale
  Signing-in text or synthetic credential disclosure.120 icon URLs and119 cover URLs were attempted
  before any detail request;118 successful icons persisted while failures stayed independent.
- Actual xdotool pointer Retry at screenshot-confirmed229,772 started requests and repaired failures;
  neutral footer was inert and distinct Downloads navigated. AT-SPI reported0,0 origins, so reviewer
  used screenshot-confirmed coordinates and corrected the harness geometry guard. Actual Left/Right
  keys wrapped/navigated after focus changes; Escape closed and preserved details. Manager viewed
  before/after screenshots. Evidence: /tmp/ludomere-p87-after-evidence.log and private fixture
  /tmp/ludomere-p86-qa-hw0vyv3q/. Final exact-source checks/report reconciliation pending.
- P87 final disposition: complete, no unresolved scoped QA/security blocker. Manager reviewed all
  reports, final logs, fixture substitutions and cold/retry screenshots. Exact-final fmt, Clippy,
  313 unit/six integration tests/debug/diff pass; separate gallery GTK test passes. Final debug binary
  timestamp2026-09-29 11:16:55 -0400. Retry repaired two covers and one icon; remaining unavailable
  cases were intentional fixture inputs. Reports: /tmp/ludomere-p87-backend-report.md,
  /tmp/ludomere-p87-ui-report.md and /tmp/ludomere-p87-review.md. Source frozen; no package rebuilt.
  All P87 tests used private profiles; no live GOG credentials/profile accessed. Live gates remain.

## Current synchronization feedback work

- User reports fresh login after reset loads names and an initial image batch, then leaves grey
  placeholders without spinners. Requests stage-labelled bottom-left synchronization, detail loading
  indicators and visible synchronization errors. R19/P86 captures this authorized work.
- Backend acquisition, UI/grid/docs compatibility, independent assurance security. Reproduce using
  disposable delayed/multi-batch fixtures; never inspect or reset the user's profile/credentials.
- Existing batch50, lazy metadata, native/Proton actions and no-background-navigation constraints
  remain. No package build or unrelated ordinary-auth changes are included.
- Initial inspection found no first-50 cover cap. Cover errors are silently skipped and completion
  still reported; fatal sync errors are logged without visible notification. UI uses one static
  Updating library label and decoder has no pending/error feedback. Exact reported stall cause is
  not yet proven. Owners agreed queued/started/finished cover events and consolidated real-work UI.
- Independent QA will use a disposable source copy with documented local endpoint/auth fixture
  substitutions to exercise the cold HTTP→backend→GUI path; production TLS/origins remain intact.
  Backend queue fixtures independently test unchanged source with >50 items and controlled failures.
- User clarified the spinner disappeared, narrowing investigation to silent completion/partial or
  fatal failure rather than an indefinitely running pipeline. Existing HTTP200 non-image bodies can
  also be cached as usable and fail only during silent decode; scoped cover validation/repair is
  authorized, with regression evidence required. Neither finding proves the user's live CDN cause.
- Backend 19-test suite passes: 125 local-HTTP covers all attempted with four workers; 121 loaded,
  one unavailable and three classified failures (HTTP503, corrupt HTTP200 and persistence error),
  with later work continuing. Corrupt cache repair, stale session and consumer early-exit pass.
  Evidence: /tmp/ludomere-p86-online-tests.log and /tmp/ludomere-p86-backend-report.md.
- Independent cold GUI reached all120 fixture covers and visibly reported two failures. It exposed
  two required UI defects missed by warm/signed-out testing: empty-library presentation not becoming
  populated, and authenticated Refresh retaining a RefCell borrow across the mutable sync call.
  Both are assigned narrow corrections to UI owner, with independent re-exercise required.
- Candidate required checks pass 312 unit/six integration tests, fmt, Clippy and debug build; new
  GTK regression passes with120 distinct delayed images, decode failure and stale binding/cache
  generation checks. Integrated independent GUI review remains pending.
- Reviewer disclosed one initial backend-test invocation lacked private XDG: two pre-existing tests
  created/self-removed PID-specific synthetic image artifacts in the default cache. No existing
  account/profile data or credentials were read. Reviewer is rerunning with explicit private roots
  and documenting exact artifacts; all cold GUI testing remains isolated. Do not claim no cache writes.
- Integrated cold GUI now passes fatal second core batch→persistent error/Retry, 50/50/20 recovery,
  all120 cover outcomes, queued/loading/unavailable/error states, retry repairing two failed covers,
  immediate named detail indicators, available Play/Download, selected-detail preservation and
  authenticated Refresh remaining on Collections. Manager inspected loading screenshots.
  Evidence: /tmp/ludomere-p86-acceptance-evidence.log and /tmp/ludomere-p86-qa-7m8i0n2e/.
- Final tiny symmetric navigation guard confines empty/populated library presentation changes to
  Home/Empty; zero-result Collections preservation and exact-final checks are pending. No package
  build, user account traffic or real-game execution. Exact original live failure stays unverified.
- P86 final disposition: complete, no unresolved scoped QA/security blocker. Final zero-result
  Collections proof passed; exact-final fmt, warnings-denied Clippy,312 unit/six integration tests,
  debug build and diff check pass. Separate120-image GTK regression and19 independent isolated
  online tests pass. Final debug timestamp2026-09-29 10:50:43 -0400. Manager reviewed all handoffs,
  final logs and actual loading screenshots. Reports: /tmp/ludomere-p86-backend-report.md,
  /tmp/ludomere-p86-ui-report.md, /tmp/ludomere-cover-sync-review.md. GUI evidence:
  /tmp/ludomere-p86-acceptance-evidence.log and /tmp/ludomere-p86-zero-evidence.log.
  Initial isolation omission affected only synthetic PID-specific files from two existing cache
  tests; cleanup attempted but absence unverified because PID was not retained. No broad cache
  inspection/deletion followed. Corrected independent run used private roots. No real account used.

## Current startup regression

- User cleared profile and reported cargo run abort. Supplied backtrace identifies
  rebuild_library library.rs:479 -> synchronous stack notify -> window.rs:94 borrow_mut panic.
- Root verified cargo build succeeds. Holding read AppModel borrow during set_visible_child_name
  is the confirmed code defect; theme/Vulkan/accessibility messages are not this panic's cause.
- P84 assigned minimal fix and empty-profile GUI regression to compatibility, independent fresh
  startup/source assurance to security. Previous 501 cached-game checks missed the empty path.
- No real profile/credentials will be accessed or reset by agents; tests use disposable roots.
- Independent reviewer reproduced the exact panic with the preserved pre-fix executable and a
  fresh private profile: exit -6, window.rs:94 RefCell already borrowed via library.rs:479.
  Evidence: /tmp/ludomere-empty-before.log and /tmp/ludomere-empty-before-evidence.log.
- P84 minimal borrow-order fix and actual GTK empty-profile regression are implemented; final
  checks and independent after-fix startup passed.
- User also requested a Settings logout/cache control (R18/P85). The interview resolved the
  control type and removal scope; installed games/installers/Proton/runtimes remain outside reset.
- Interview settled: automatic-on-sign-out toggle, full profile reset including preferences,
  favorites/tags/activity/queue; preserve installed/downloaded payloads, Proton and runtimes.
- P84 complete: independent failing-before/fresh-and-cached passing-after GUI evidence, actual
  GTK regression, and all required checks pass (300 unit/six integration plus separate GTK test).
- P85 acquisition owns reset backend; compatibility UI/config/docs; security independent review.
  Toggle opt-in and clearly explains closure after reset. Only disposable profiles are used by
  agents; no real login/profile is reset. Active operations and stale workers require safe handling.
- P85 lifecycle inspection found detached cache/config/database writers and shutdown paths that do
  not join every worker. A narrowly scoped self re-exec cleanup phase after GTK exits is authorized
  to close old handles and terminate stale writers before reset; no external helper or dependency.
  Backend/UI owners are coordinating active-operation refusal, constrained reset inputs and payload
  protection. Independent security review is involved before implementation is finalized.
- P85 early review requires persisted manual Proton selections to be protected alongside configured
  game/download roots; overlap with reset targets must refuse the whole reset. This finding is with
  the backend owner, with a disposable regression required before completion.
- Existing ordinary sign-out can race a token-refresh keyring write. P85's enabled reset eliminates
  that race by deleting credentials only after process replacement; broad ordinary-auth redesign is
  outside this change and remains a separately reported pre-existing issue.
- P85 UI/config/docs are implemented. Seven focused disposable backend tests initially passed;
  required full checks and independent GUI verification are pending. Reviewer prepared a private
  fake Secret Service and payload-sentinel profile, without host credentials or user data.
- Both backend owner and reviewer identified Linux flock's failed shared-to-exclusive upgrade can
  release the shared lock. Owner is replacing this profile-only primitive with atomic OFD locking
  and strengthening concurrency evidence before handoff. No reset completion claim yet.
- P85 independent enabled-reset GUI passed: persisted toggle alone erased nothing; sign-out exited
  successfully after cleanup; fake OAuth entry/profile files disappeared; six payload/save sentinels
  survived; fresh launch was signed out with default-off setting. Ordinary toggle-off sign-out also
  preserved durable counts/cache. Injected keyring deletion failure retained the manifest/database,
  exposed recovery-only controls, and explicit Retry completed successfully.
- Required candidate checks passed 308 unit/six integration tests and the separate GTK startup
  regression. Final narrow malformed-control FIFO/FD validation and remaining refusal/Close GUI
  checks are being verified before P85 handoff; no real account or payload operations exercised.
- P85 final disposition: complete. Manager inspected exact-final logs and all three handoffs.
  Fmt, warnings-denied Clippy, 308 unit/six integration tests, debug build, diff check and separate
  GTK startup regression pass. Seven independent backend tests pass. Final recovery GUI confirms
  failure→Close→relaunch recovery-only→Retry→fresh signed-out/default-off profile; protected Proton
  overlap refuses without deletion and leaves Settings usable. No unresolved scoped blocker.
  Reports: /tmp/ludomere-p85-backend-report.md, /tmp/ludomere-p85-ui-report.md and
  /tmp/ludomere-profile-reset-review.md. Logs: target/p85-{test,clippy,build,gui-regression}.log.
  Final debug build timestamp: 2026-09-29 04:24:52 -0400. No package rebuild or real profile reset.
  App-wide branch encryption key is retained for other profiles; actual branch credentials are
  removed with SQLite. Actual game/helper cancellation and live-keyring/account behavior remain
  unexercised; exclusion has source and disposable activity-test evidence.

## Current library performance work

Final disposition: implemented and independently reviewed with no unresolved blocker in reviewed
scope. Source/debug build ready for user testing. Final exact-candidate checks pass: 300 unit tests,
six integration cases, five UMU tests, formatting, Clippy with warnings denied, debug build and
diff checks. Independent five section regressions and isolated 501-game GUI checks passed.
Reports: /tmp/ludomere-library-performance-review.md and /tmp/ludomere-p81-report.md;
full test evidence: target/p81-test.log. No new package was built; existing dist artifacts predate
these performance changes. Milestone history below retains the findings and their resolution.

- User approved implementation of the investigated proposal with 50 products per request.
- Anonymous live checks returned every requested product at batch sizes 10, 25, 45, 46 and 50.
  This proves tested lightweight request support, not a whole-library speed guarantee.
- Names then covers; rich details lazy; actions usable while metadata loads; filters visibly
  loading/incomplete with retry; sparse cache and ownership preservation are required.
- P80 backend: acquisition. P81 sync/details/actions: compatibility. P82 grid/filters: grid.
  P83 independent QA/security follows implementation. No package rebuild planned.
- Existing prior working-tree changes and dependency behavior remain preserved.
- Agreed integration contract separates Product, Metadata, Artwork and Acquisition requests;
  per-product merges and readiness share existing observation storage. Core compatibility/language
  fields use existing JSON storage, with no planned schema boundary change. Session generation
  guards protect stale work; UI passes visible-cover priorities to bounded background workers.
- P80 implementation checkpoint: 50-ID core requests, four cover workers, rate-limit-safe fallback,
  session-guarded entitlement commits, preserved recorded pack entitlements, tile-only writes and
  shared-image path locking implemented. Focused regressions and integration are still pending.
- Review found installer choices gated by build/depot acquisition. Workers are separating Builds
  from Acquisition so offline installer availability does not depend on Galaxy metadata.
- P82 handed off incremental grid, two decode workers, filter/collection readiness and retry UI.
  Focused image tests added; compile remains blocked by P80/P81 integration, so no pass claimed.
- Independent P83 source/control review started; isolated synthetic-library GUI checks follow a
  working build. P81 owns final integration and coordinated required checks.
- Narrow P81 install-dialog preparation support delegated to acquisition after backend tests,
  with explicit shared-file regions; compatibility retains detail sections and Download chooser.
- P83 found returned pack records with absent DLC fields could erase cached inherited entitlement
  edges. Sent to P80 for absent-versus-empty handling and regression; unresolved until re-reviewed.
- Preliminary tests exposed sandbox restrictions on local fixture servers/image loader IPC plus
  a new fixture initialization defect. Workers are correcting the fixture and rerunning affected
  checks with appropriate local-test permissions; no aggregate pass claimed.
- P83 prepared an isolated 501-game fixture with fresh/unknown metadata, cover paths, favorites,
  activity and one inert native installation marker. Awaiting a stable build for actual GUI checks;
  no account tokens or game execution involved.
- Manager inspected passing P80 logs: five product tests and 30 state tests. Includes 50-ID
  batches, missing IDs, no transport/throttling fanout, sparse core fields and pack entitlements.
  Evidence: /tmp/ludomere-p80-product-tests.log and /tmp/ludomere-p80-state-tests.log.
- Library now compiles; independent GUI review can begin. Full checks and final install-dialog
  integration remain outstanding. P83 found queued grid snapshots could miss early image results;
  P81 owns the narrow insertion-from-current-model correction, pending verification.
- First P83 isolated GUI observation: cached 501-game fixture showed names at approximately
  1.823 seconds and all 501 at 2.509 seconds after spawn, including AT-SPI polling overhead;
  no crash. Reviewer later identified invalid image fixtures in that first run. Final valid-cover
  fixture observed names at approximately 6.5 seconds and all 501 at 7.0 seconds, including large
  AT-SPI tree traversal. Neither is a first-paint or authenticated network-refresh benchmark.
- P80 online/media suite: 15 passed, manager inspected /tmp/ludomere-p80-online-tests.log.
- P83 GUI verified installed fixture shows Play, cached description and playtime while Product
  and Artwork requests fail with separate Retry controls. No game launched. Reviewer verified
  source corrections for pack/sparse/session/DLC/queued-card findings; final review still pending.
- Manager identified cache-clear/explicit-refresh readiness integration; P81/P80 are invalidating
  lazy freshness appropriately without restoring eager full-library enrichment.
- P80 backend and delegated install-dialog preparation handed off, pending final review. Focused
  backend total is 50 passing tests (5 product, 30 state, 15 online). Install shell is immediate,
  local preparation runs off GTK, and Galaxy choices complete/retry in place without resetting
  entered choices. P81 now owns coordinated final formatting/Clippy/full tests/debug build.
- Final candidate P81 checks pass: cargo fmt --check, all-target Clippy with warnings denied,
  cargo test (299 unit tests, six integration cases and their subprocess checks), five UMU Python
  adapter tests, and git diff --check. Tests used disposable XDG roots with local fixture/IPC
  permissions. Manager inspected target/p81-test.log. Updated target/debug/ludomere built.
- Final P83 GUI pass against rebuilt candidate is pending. No Arch package build/install requested
  for this change; prior dist package does not contain these new source changes.
- P83 rebuilt-candidate GUI passed signed-out Download chooser's explicit sign-in state (no
  unsolicited login/spinner), Retry preserving Play/cache, and rapid selection's final page.
- Final P81 review found owned downloaded DLC lacked computed local action state and could show
  Download instead of Install/Update. Essential regression fix authorized with focused regression
  and refreshed required checks/debug; P83 narrow action re-review follows.
- Final P83 install probe passed: immediate plan dialog, populated local/storage fields, empty
  installer choices keep Install disabled, Galaxy absence is explicit, close preserves Play.
  No installer/game/helper executed. Controlled delayed-response tab/scroll behavior remains
  source-backed rather than automated live proof; real GOG timing remains unmeasured.
- Final DLC action regression fixed and independently re-reviewed. Two obsolete account-epoch
  preparation shells now close instead of retaining a spinner. Exact-final required checks rerun
  and pass (300 unit, six integration); P83 reports no unresolved functional/security blocker.
- Live account switching, authenticated synchronization and performance, exact scroll/tab behavior
  under deliberately delayed responses, and actual game/DLC installation remain user acceptance
  criteria, not passing claims. Earlier real-game/desktop gates below also remain outstanding.

## Current decisions

- Arch Linux x86-64 only; pacman packaging supersedes AppImage/Ubuntu.
- Bundle UMU with Ludomere. User explicitly retained Ludomere's UMU adapter after its two runtime
  Python hook replacements were disclosed. Proton/runtime acquisition continues to require prompts.
- Discover external Proton installations without modifying them; GE > UMU > Valve for automatic
  selection. Save the first default, allow per-game overrides, ask when a selected runtime disappears.
- Offer stable current/historical GE and UMU downloads; Valve is discovery-only. No bundled Proton
  or Steam Linux Runtime.
- Bundle **official, unmodified Comet 0.3.2 and its service helper**. User accepts upstream automatic
  peer downloads/updates. Explain that behavior; no custom peer consent, verification or rollback
  promise. No proprietary peer files are bundled.
- Check Comet/peer metadata on startup and manually without background dialogs/navigation.
  Comet binary updates require confirmation and verified official assets, use private storage,
  preserve prior/package files, and defer to a newer packaged helper.
- Before modifying/rebuilding a dependency to satisfy a behavior requirement, explain the conflict
  and let the user decide. The rejected Comet patch/source-build/custom-feed approach and generated
  modified helpers were removed. No such files remain in the delivery.
- No host package installation, commits/pushes/publication, credential access, or external messages.

## Delivered artifact and documentation

- Package: `dist/ludomere-0.1.0-1-x86_64.pkg.tar.zst`, 13,517,508 bytes.
- SHA256: `e7a6b78a2e8939219459202914cd03f3611dcf2f34bfd15df93ee7c5a2a6a940`.
- Source: `dist/ludomere-0.1.0.tar.gz`.
- Source SHA256: `9538888498bde547da88fc4b1f8783c5a30da3797a6f509841f04b0654c4a977`.
- README documents exact Arch requirements, local checks, isolated test state, pinned container,
  helper preparation, package generation, Proton/Comet behavior and guided game verification.
- Scripts and GitHub Actions build/check pacman/source artifacts without installing or publishing.
  The workflow's equivalent Arch container commands ran locally; remote Actions was not triggered.
- No schema boundary/revision changes were needed. Proton preferences use atomic separate JSON.

## Verification reviewed by the manager

- Final clean Arch build: fmt, all-target Clippy with warnings denied, **285 unit tests**, six main
  integration cases plus subprocess checks, **five UMU adapter tests**, release build and makepkg.
  Evidence: `target/arch-package-official-comet-final.log`.
- Independent isolated compatibility suite: **38 passed**,
  `/tmp/ludomere-p60-official-tests.log`.
- Actual package independently audited for ownership/modes, exact official helper bytes, licenses,
  source snapshot, absence of peer/Proton/runtime payloads and obsolete patch/feed files.
  Evidence: `target/official-comet-package-audit.log` and independent final assurance report.
- Official Comet SHA256:
  `2d6694d544fd3155d90d540e70bc1be767a6b9fdda130275f2b79616ff14e843`.
  Service SHA256:
  `c7695267da363a861af99db95cafe68b732ae743e5830b4feea1bc7ee745f99d`.
- Final packaged GUI in isolated Xvfb/HOME/XDG/session buses passed startup metadata/no dialogs,
  automatic-peer explanation, decline/no payload, cancellation cleanup, fresh retry confirmation,
  official Comet helper-pair download/install/hash verification, current-version/manual check and
  offline error display. Older official helpers were inert fixtures; no Comet/peer/game code ran.
- Prior independent Proton GUI evidence covers default persistence/replacement, folder selection,
  GE/UMU catalogs, actual historical UMU-Proton download, Steam runtime download, cancellation,
  publication and already-ready state. No downloaded runtime/game was executed.
- `git diff --check` passed. The manager reviewed worker inventories, source changes, final build
  evidence and the independent final report.

## Independent disposition and closed findings

Current report: `/tmp/ludomere-final-assurance-report.md`, with detailed
`/tmp/ludomere-official-comet-security-report.md` and
`/tmp/ludomere-official-comet-qa-report.md`. Earlier Proton QA:
`/tmp/ludomere-qa-report.md`; current control inventory: `/tmp/ludomere-p48-report.md`.

Independent reviewer found **no unresolved blocker in the reviewed delivery**. Corrected findings
include Comet credential logging, unsafe full UMU-source extraction, VDF notice omission, real Valve
manifest size, changed-helper service registration, newer-package preference, update baseline,
publication/cancellation/retry/path checks, and makepkg stripping that altered official Comet bytes.
The stripped intermediate package was rejected and replaced with the exact-byte final artifact.

The earlier automatic-peer consent finding is superseded by the user's accepted upstream policy.
Upstream peer downloader limitations are recorded in
`/tmp/ludomere-comet-upstream-security-report.md`; they are not Ludomere-owned integrity guarantees.
Licence research is complete in PEER_LICENSE_REVIEW.md; no further permission investigation is
requested. Historical patched-Comet test evidence is superseded by the final official-binary report.

## Outstanding user-assisted gates

Use README's Guided game verification with the user's own Arch session and GOG credentials.
Credentials must not be shared with agents.

- Real GOG login/keyring and owned-game install, launch, patch/update/repair and uninstall.
- Actual Windows game use of global/per-game Proton, replacement behavior and GPU/runtime support.
- Native Linux game operation independent of Proton/UMU.
- Authenticated Comet online features and upstream automatic peer preparation for a supported game.
- Desktop/portal/keyring/Wayland behavior. Cached-peer status edge cases have unit/source evidence,
  not exhaustive graphical coverage. Remote GitHub Actions has not run.

The user already agreed to assist. Request results from the concrete checklist; do not declare
Prototype ready or silently count unexercised checks as passed.

## Ownership

Manager /root owns records and gate decisions; it wrote no product implementation.
P10/P40/P47: /root/compatibility. P20/P46/P48: /root/acquisition.
P30: /root/ui. Prior QA: /root/qa. Final independent P50/P60 extension: /root/security,
who implemented none of the reviewed product code. All current implementation workstreams have
handed off; P84/P85/P86/P87/P88/P89/P90/P91/P92 are complete. User-assisted live acceptance remains outstanding.
