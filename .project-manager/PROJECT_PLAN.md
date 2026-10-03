# Project plan

## R96 publication and clean upstream merge

- P283 (complete, ui_review): bump only application0.2.5 Cargo manifest/lock, PKGBUILD and
  AppStream metadata; retain historical releases and pkgrel1. No dependency/schema changes.
  Verify locked metadata, XML/version consistency and focused format checks; root builds.
- P284 (complete, depot): read-only review newly fetched upstream2b7a083 shutdown journal
  race fix and overlap with our changes. Recommend relevant merge verification; no edits.
- Root commits reviewed R95 fixes separately, preserves each post-commit build, and previews
  merge conflicts without altering working tree. If clean, merge and run focused merged checks,
  commit version and publish existing branch to origin. No conflict resolution without user input.
  SSH fetch failed; public HTTPS fetch succeeded and existing gh credentials verified usable.
- Completed commits037e57c/e5f1e74/ae55b45 and clean merge5d9d9a5; each exact post-commit build
  passed. Merged manager32 and recovery4 tests, fmt and all-target Clippy PASS. Branch published
  to origin over HTTPS without permanent remote changes; no PR. Final records-only closeout is
  built and pushed separately; no package, full-suite repeat or user-profile operation.

## R95 focused manual-test fixes

- P280 (complete, ui_review): trace initial onboarding Proton selection and implement the
  minimum correction in setup/proton UI. Visible initial valid selection must be persisted and
  Next must work without toggling; preserve custom/unavailable selection and async/session guards.
  Private synthetic GTK regression; no real runtime/helper/game operations.
- P281 (complete, depot): investigate credential-service preparation and first post-reset
  save failure in auth.rs and pinned keyring implementation. Return evidence/proposal before
  edits if cause requires changing credential behavior. Synthetic provider/error tests only;
  never inspect/call real wallet or read tokens. No retries that bypass consent or storage fallback.
- P282 (complete for scoped synthetic QA, storage_audit): independent QA/security review of both changes and relevant
  regression results. Root coordinates Cargo and private HOME/all XDG/TMP tests, reviews diff,
  maintains records. No commits/push/PR/package unless separately authorized for this follow-up.
- P280 cause confirmed: discovery selects first row while busy, suppressing autosave; Next
  correctly rejects unsaved selection. Reuse guarded save notification once after discovery only
  for global onboarding without saved choice or save error. Preserve Settings/per-game semantics.
- P281 approved proposal: explicit login resolves standard service/default wallet readiness and
  unlocks before the single keyring save. Pinned keyring3.6.3 swallows get_collection unlock errors
  via fallback that can attempt create_item on a locked collection; user-specific failure remains
  unproven. No vendor edit/new dependency, alias/collection mutation or secret read. Pin service
  owner; private worker MainContext and sender/path-scoped prompt; bounded readiness and120s prompt
  wait; original-session checks; distinguish missing/default, dismissal, timeout and access errors.
  Best-effort dismiss own prompt on cancellation; no credential-save retries. Independent review
  requires synthetic sequence/early-completion/owner-change/cancellation and redaction coverage.
- Final evidence: focused auth12 PASS (including twelve private-provider scenarios), private GTK3
  PASS; new actual wizard regression rerun on final binary PASS. Root fmt/diff, all-target Clippy
  with warnings denied, and cargo build --locked PASS. No full suite or real wallet/profile action.
  Reports /tmp/ludomere-p280-report.md, /tmp/ludomere-p281-report.md and
  /tmp/ludomere-p282-review.md. User's actual KDE first post-reset sign-in remains unverified;
  pinned preflight does not make the subsequent existing keyring connection atomic.

## R94 closeout

- Complete within the 20:53:48–22:53:48 UTC authorized window: 22 separate product commits on
  improvement/ux-performance-audit-2026-10-02, every exact post-commit build passed. Full build
  process passed at8d02889; final CSS-only a456d48 passed fmt, all-target Clippy, independent review,
  rebuilt isolated startup and its post-commit build. Root commits final records and builds once
  more, then stops. No PR/push/package. Ledger records deferred P273, finite QA limits and the
  disclosed P263 test-isolation incident; no exhaustive readiness claim.

## P279 R94 final startup warning cleanup

- The optional startup smoke test passed but emitted an unsupported GTK CSS overflow-property
  warning. Approve a one-declaration exception to the feature freeze: ui_review removes only
  ignored overflow:hidden from .active-transfer-header in ui/mod.rs. No widget clipping/layout
  behavior added. Independent source review, fmt/Clippy/build and rebuilt existing startup smoke
  test verify the warning disappears. No full-suite repeat for this CSS-only correction; prior full
  suite remains recorded at8d02889. Separate commit/build, no other cleanup, hard stop22:53:48.

## P278 R94 final build and bounded assurance

- Root runs tools/check.sh once as final build process, with private HOME/all XDG/TMP, offline
  Cargo, no desktop/helper overrides and a20-minute process deadline. Source21 commits frozen.
  Depot reviews cumulative backend performance/correctness claims; storage_audit reviews changed
  session/reset/path boundaries; ui_review reconciles changed controls and user-facing inventory.
  Read-only reports only unless a concrete release-blocking regression requires a scoped repair.
  No new feature work after22:25 UTC; hard stop22:53:48. No real account/game/keyring/cloud/helper
  actions and no PR/push/package. Root records limits, deferred P273, isolation incident and results.
- Final full process PASS:539 library tests,6 integration tests,5 Python helper tests;35 tests
  ignored by normal suite, with changed GTK cases already exercised separately. Read-only final
  reviewers report no introduced blocker. Optional existing empty-profile startup smoke test may
  run only after a read-only safety check confirms private HOME/display/bus cannot reach real
  credentials, games/helpers or mutate user state; otherwise record as unexercised. No new code.
- Safety review found the existing test replaces XDG_RUNTIME_DIR and startup probes Secret
  Service/public versions. Approved storage_audit's temporary harness: private bus with no service
  activation, private HOME/XDG/TMP, dead-loopback proxy, Broadway started after runtime replacement
  via a private utility wrapper, unchanged integration binary/fixture,90-second deadline and
  owned-process cleanup. No signed-out marker shortcut or product/test edits; report actual outcome.

## P277 R94 source migration lifecycle follow-up

- Independent cumulative review found strong Close/Continue callback cycles in P252 MigrationView
  and missing initial-preflight profile activity. ui_review owns game_settings.rs narrow fix:
  weak captures, original-session/activity checks before inspection, preserve explicit migration
  consent and existing progress/close behavior. Retained private GTK weak-release and stale/reset
  tests; no real migration/helper. Independent review and separate commit/build required.

## P276 R94 authoritative checksum filename safety

- storage_audit found pre-existing parse_gog_checksum accepts raw XML filenames which verification
  may join to a destination before deleting a corrupt/missing download. Approve narrow verify.rs
  validation proposal, then reject unsafe XML and fallback filenames before exposing GogChecksum;
  never normalize an authoritative traversal into another filename. Synthetic parser/path tests,
  no file deletion/network. Preserve valid names, checksum/size semantics. Independent review and
  separate commit/build. Source frozen P272 first; no destructive diagnostic test needed.
- Approved strict basename validation after XML/fallback selection: reject empty/whitespace-only,
  dot/dotdot, separators, controls and non-single-normal components; retain accepted original name.
  Invalid explicit XML must not fall back to a safe name. Pure valid Unicode/space/multipart and
  traversal/absolute/control rejection tables; existing protocol fallback normalization unchanged.
- Root review also confirms malformed MD5 currently counts as a corrupt payload and can trigger
  repair deletion. Same bounded parser set must reject non-32-hex MD5 before exposing authoritative
  checksum metadata; accept upper/lowercase valid hashes, preserve original value. No hashing or
  deletion test required; parser fixtures use valid hashes when testing names.

## P275 R94 installation status-log efficiency audit

- Root found UmuLogStatusMonitor rereads its entire growing log every100ms in executor.rs, then
  processes only new bytes. Depot read-only audit next: propose bounded incremental reading while
  preserving complete-line/status behavior, truncation/replacement and prompt worker shutdown.
  Synthetic files only, no helper/process/game launch. No edits until proposal reviewed; avoid
  generic log framework or unrelated logging changes. Independent tests/review if approved.
- Approved executor.rs-only incremental reader: inode/device+offset,64 KiB reads, stop checks
  between chunks,100ms idle wait,16 KiB pending-line bound with oversized status lines skipped
  until newline (raw saved logs untouched). Reset pending/dedup on truncation/replacement; decode
  completed lines to preserve split UTF-8. Private append/unchanged/truncate/replace/oversize and
  synthetic monitor shutdown tests. No real helper launch; no generic log infrastructure.

## P274 R94 achievement loading lifecycle and truthful failures

- ui_review confirmed the only achievement worker opens cache without profile activity/session
  precheck, discards cache failures and claims cached achievements exist even when absent/offline.
  Own ui/achievements.rs: original-session check and activity around worker, session-aware UI
  results, sanitized diagnostics retaining useful cache failure when refresh fails, truthful empty
  offline feedback. Preserve cache-first rendering, refresh and Comet policy; no network/backend
  protocol change. Private synthetic offline cache/error/reset tests and independent review; never
  real account, keyring, achievements or profile operations. Separate commit/post-commit build.

## P272 R94 downloaded-file verification feedback

- storage_audit identified start_product_verification presenting a completion modal from a
  background callback, losing full details from the persistent status and mislabeling unavailable
  checksums as no downloads. Own files.rs verification path only: retain sanitized completion/error
  detail inline, remove unsolicited terminal modal, distinguish empty input from unavailable checks.
  Preserve consent, repair/deletion decisions, cancellation and session checks. Synthetic events/
  private GTK or pure outcomes only; no real files/helper/network. Independent review and focused
  tests, separate commit/build; stop for scope expansion or unresolved deletion-policy ambiguity.
- Approved minimum lifecycle correction: capture original online/auth sessions in request and
  ephemeral verification state (ui/mod.rs field extension allowed); profile activity covers worker,
  session checks between stages, account commit guard only for short mutations/delete+enqueue,
  never network/hash/traversal. Reject stale confirmation and UI results. Preserve existing selected
  corrupt-file decisions and explicit Verify-and-repair consent. No real verification execution.

## P273 R94 further library responsiveness audit

- Depot read-only audit library.rs rebuild/title and grid-scroll cover-priority work. Establish
  actual redundant work and propose bounded improvements with source evidence and synthetic
  verification; no edits before manager accepts proposal. Preserve live-state checks, row identity,
  priority ordering and account/closing behavior; no persistent broad model-index redesign.
- Confirmed title refresh makes two per-widget linear searches and unconditional title writes.
  Depot owns library.rs bounded fix: ephemeral ID→title map preserving first-duplicate behavior,
  release model borrow before GTK mutations, skip identical labels. Keep idle new-row live-model
  relookup unchanged. Private retained GTK unchanged/changed/removed IDs, focus/selection and
  row/card identity plus independent review required. Scroll throttling deferred due timing surface.

## P270 R94 organization write lifecycle

- Confirmed root review: organization.rs hide/tag worker writes use UI epoch only for results;
  queued work can open/create the profile after sign-out/reset, unlike the guarded Favorite path.
  storage_audit next after P268 commit owns organization.rs: capture account session at action,
  acquire profile activity then use with_account_session for SQLite mutation/readback, preserve
  UI pending/epoch behavior and add immediate visibility-save activity plus sanitized failures.
  Existing tag/visibility semantics unchanged. Private synthetic stale-session/reset exclusion and
  existing behavior tests, independent review required. No real profile reset or account activity.

## P269 R94 batched library classification evidence

- Depot after P263 commit owns state.rs/storage.rs: reuse existing P251 selected-product revision
  loader in400-ID batches for library_evidence, including retired revisions. Preserve represented
  product scope, parsing failures, category/fallback and cross-product semantics; no narrow JOIN
  that silently weakens old validation. Expose existing internal helper only as needed. Synthetic
  equivalence, malformed/unrelated and retained-part tests; quantify source query reduction.
  No schema/dependency/network/profile changes; independent review and separate commit/build.

## P268 R94 explicit patch-row activity

- storage_audit after P266 commit owns files.rs explicit archive-row Run Patch only: reuse row
  status/progress for preflight/applying/terminal feedback, full sanitized errors, no background
  completion/error modal, distinguish disconnection from cancellation. Preserve explicit consent,
  backend patch validation and action restoration. Include session/lifetime guards as needed for
  this path; private synthetic events/GTK tests only, no patch/helper invocation. Depot independent
  review after P263 ready. Separate commit/exact build required.

## P267 R94 account-bound manual cloud actions (before P264)

- Confirmed safety finding: Game Properties force-sync confirmation can outlive its account;
  run_cloud_action captures auth session only inside sync when worker starts. ui_review owns
  game_settings.rs bounded fix before P264: bind manual normal/force actions to originating
  Properties auth/account session, recheck confirmation and worker dispatch, call existing
  sync_for_session, suppress stale result/control updates with actionable account-change text.
  Preserve destructive consent and all backend safeguards. No network/keyring/save tests;
  private GTK stale-confirmation controls and synthetic dispatch verification. storage_audit
  independently reviews session boundaries/security and behavior. No cloud-builder redesign.
- Also bind the existing Check now inventory path to its originating session and discard stale
  result updates. ui_review additionally owns cloud_saves/mod.rs inventory signature (one caller)
  to pass original auth session explicitly; no new compatibility wrapper or changed cloud policy.

## P266 R94 update error diagnostics

- storage_audit owns update_policies.rs only: preserve actionable sanitized actual errors for
  global update checks and per-game policy loading, distinguish worker disconnection. Keep current
  retry/close-reopen controls and save policy; no new framework/backend/network actions. Focused
  existing policy/sanitizer checks, independent source review and separate commit/build gate.

## P263–P265 R94 targeted refresh and compatibility preference loading

- P263 depot next after P259 commit: storage.rs/sections.rs root-only Game Files compatibility
  inspection for targeted local refresh; preserve full root/ancestor/infrastructure and archive
  checks, own-target reconciliation, and full Storage child diagnostics. Document that lightweight
  snapshots do not contain child issues (no model consumer uses them). Regression for partial
  sibling, unsafe roots/overlap and archive type checks. No cached-validation bypass.
- P264 ui_review next after P260 commit: game_settings.rs Compatibility fixes initial load only.
  Worker SQLite read; disabled controls with loading/error/Retry; populate under reset suppression,
  enable only when loaded and installed. Preserve save/default-reset behavior, session/reset and
  weak-lifetime checks; private GTK synthetic/database tests, no real profile/helper changes.
  Cloud Saves initial read deferred because its large record-dependent builder needs separate
  design; managed-detail label async rewrite also deferred to avoid unbounded call-site churn.
- P265 nonimplementer reviews and focused checks for each; root separate commit/exact build gates.

## P260–P262 R94 filter counts and patch feedback

- P260 ui_review owns library.rs: direct Game predicate plus per-refresh matching IDs to remove
  repeated ID scans for counts/section headings. Preserve all filters, playable/hidden behavior,
  missing metadata, collapse/selection and existing GTK callbacks; no persistent index/cache.
  Focused equivalence and private GTK regressions; measured/sourced work reduction.
- P261 storage_audit owns files.rs preferred-patch flow: move DB/file inspection to worker;
  visible inspection, explicit errors/normal no-patch result, existing patch consent, in-place
  patch progress and terminal/disconnect feedback. No completion modal/focus change; closing
  progress does not cancel. Account/reset/lifetime guards required; synthetic GTK only, no patch
  execution or game/profile mutation. Preserve patch choice/version rules and update fallback.
- P262 independent cross-review after P259–P261 source ready; root coordinates compilation,
  relevant tests, separate commits and post-commit builds. No new dependency/schema/policy.

## P259 R94 automatic-install planning efficiency

- Depot approved next separate set after P256 commit: reuse request job IDs already resolved by
  intent and read base DLC membership once; preserve selection ordering, validation and consent.
  Own src/download/auto_install.rs and relevant existing tests only. Synthetic queue/DLC/legacy
  identity fixtures, focused intent/completion/reopen tests and independent review required.
  No schema, filesystem policy, helper or real profile actions. Await wave2 commit before edits.

## P255–P258 R94 second wave

- P255 ui_review owns details.rs operation-log loading/error/Retry: preserve available sources,
  display installation errors without files, stale-refresh/session guards; private GTK synthetic
  success/error/empty/disconnect/retry. No actual discard/log-folder action.
- P256 depot owns online.rs image FIFO VecDeque scheduling, preserve selected priority/cancel/
  workers semantics; focused scheduler tests. P257 storage_audit owns account.rs embedded-login
  loading/error/retry using sanitized generic messages and ephemeral WebKit; preserve OAuth,
  redirect/session/cancel behavior, no real network/account tests. Each separate reviewed commit.
- P258 independent cross-review follows each; root serializes Cargo and uses immutable copied
  test binaries plus exact-commit build checkout. No new work beyond these approved scopes yet.

## P251–P254 R94 bounded audit, first wave

- Approved first sets: P251 state.rs bulk normalized-library child data loading, preserving narrow
  lookup/order/errors, measured on synthetic500-game fixture; P252 game_settings.rs async source
  migration inspection/preparation, phase feedback/duplicate prevention and worker-stop errors.
  P252 login page busy/error/retry and P251 image FIFO VecDeque are queued separate later sets.
- P251 depot audits backend state/download/installation inefficiency, nonblocking work and bounded
  IO. P252 storage_audit audits UI async actions/loading/error/cancellation feedback in dialogs and
  settings. P253 ui_review audits library/details/rendering/event-refresh responsiveness. Initial
  read-only findings with exact evidence; root approves narrow disjoint edits within R94 before
  implementation. No new feature policy or security loosening; defer ambiguity without questions.
- P254 nonimplementer cross-review and focused regression/private GTK checks per set. Root alone
  owns records/Git, stages separate coherent sets, runs appropriate fmt/lint/focused checks and
  verifies cargo build --locked after each commit. Other workers pause mutations during each gate.
  At end run build-process checks once if time allows; no package/PR/push. Stop starting edits by
  22:25 UTC, reserve final verification time, hard stop22:53:48 UTC or user interruption.
- All agents read AGENTS and relevant skills, preserve new upstream d836fbe changes. Reports in
  /tmp/ludomere-p25*-report.md; durable change ledger .project-manager/R94_IMPROVEMENTS.md.

## P249–P250 R93 publication

- P249 storage_audit owns version0.2.4 in Cargo/lock/PKGBUILD/AppStream only; keep package release1
  and preserve release history. Verify metadata consistency/XML/package syntax; no dependency or
  product changes. P250 depot independently reviews accumulated release inventory and metadata.
- Root prepares comprehensive commit and concise PR comment, checks prior evidence plus final
  formatting/build as needed, commits and pushes fork main without force and posts requested PR6
  comment. Verify remote head/comment. User explicitly authorizes publication; no package build,
  actual profile/helper actions, new features or full suite repetition required for version bump.

## P245–P248 R92 actionable per-game recovery

- Status complete: owner implementations and independent cross-reviews verified. Scoped tests
  total38 storage/recovery/preferences plus2 confirmation tests and2 retained actual GTK tests pass;
  final fmt/diff/all-target Clippy/debug build pass. Reports /tmp/ludomere-p245-report.md,
  p246-report.md, p247-report.md and p248-{backend,ui,preferences}-review.md. No full suite, real
  user-file/helper operations or publication. Read-only actual file-manager/reboot behavior not
  exercised; scope and limits recorded in status. Root preserved all work without committing.
- Agreed interface: LibraryStatus.game_issues(path,reason), root/direct-child browsing validation,
  per-target game validation. Healthy siblings remain usable. P245 explicit known-product/selected
  root reset may remove unrecognized direct-child contents only after exact-path UI consent, with
  directory identity recheck, conflicting readable ownership rejection and unowned prefixes kept.
- Approved corrupt-journal recovery extension: explicit preparation stores bounded path/product/
  directory+journal identity/hash/current boot. Same boot refuses reset; after reboot unchanged
  evidence allows journal quarantine and another explicit reset confirmation. No automatic deletion
  or system reboot; no inferred process quiescence. P246 owns uninstall.rs Browse and Install again.
- P247 approved malformed Proton preference recovery: confirmed original-file backup and defaults
  under existing locks/session guards, plus confirmed per-game DLL reset; own compatibility/proton.rs,
  ui/proton.rs,dll_overrides.rs and coordinated README. No broader startup/config rewrite.
- P245 depot owns storage/backend discovery: distinguish root configuration failures from child
  game problems, scope destination validation, reuse existing repair/reset ownership controls.
  Report concrete design and exact owned paths before edits; no blanket validation bypass.
- P246 storage_audit owns game-level UI recovery flows (details/files/chooser/settings-storage
  as coordinated): offer repair/browse/confirmed start-over and remove manual-file-work dead ends.
  Wait backend interface agreement; preserve progress, account/lifetime guards and action consent.
- P247 ui_review independently inventories related settings/prefix/default recovery dead ends;
  report bounded fixes and coordinate ownership before edits. Reuse existing prefix backup/repair,
  preference defaults and typed-library tools; no unrelated recovery framework.
- P248 cross-review by nonimplementers, focused backend and private GTK tests, then root fmt/lint/
  build. Root owns records only. Read AGENTS/terse-code/scope-creep; preserve clean a13f8c6 baseline.
  Routine local reads/scoped edits/tests authorized. No secrets, real profile/library mutation,
  installer/helper execution, full suite/package/publication or new dependencies/schema.

## P243–P244 R91 publication

- P243/P244 complete: all four version files consistent, scoped metadata/XML/package syntax and
  final fmt/diff checks pass; cargo build --locked builds0.2.3. Independent accumulated-diff and
  commit/PR accuracy review passes. Root proceeds with explicitly authorized commit/push/PR edit;
  remote publication and CI results are verified separately after commit creation.

- P243 storage_audit owns version metadata0.2.3 in Cargo/lock/PKGBUILD/metainfo and scoped
  consistency checks, no dependency changes or package build. P244 depot independently inventories
  accumulated changes/version metadata and reviews release-summary accuracy; no product edits.
- Root prepares comprehensive commit and concise PR-body additions, verifies existing review and
  test evidence, commits/pushes to current fork branch and updates upstream PR6 on explicit user
  authorization. No force push; preserve remote edits/PR text and inspect new CI result.

## P239–P242 R90 autosave and optimization

- Complete for this bounded pass: P239 autosave implementation7 focused tests+4 private shipping
  GTK tests pass; independent P242 actual wizard/Rename fixture and source/security review pass.
  P240/P241 performance/dead-code reviews pass. Final fmt/diff, all-target Clippy -D warnings and
  debug build pass after cleaning the private fixture's Cargo artifacts. No full suite, package,
  publication, new dependency, schema change or real user profile/helper action performed.

- Approved evidenced P240 fixes: state.rs batch revision-parts queries rather than one per revision,
  filter cached pack relations in SQL, remove stale dead-code allowances on actively called paths.
  Approved P241 fixes: remove four cfg(any()) unreachable DLC UI implementations, avoid repeated
  identical footer-icon loading, index Downloads widgets once per progress update instead of
  traversing for every operation. Preserve cached-image refresh and widget lookup semantics.
  P240 also owns config.rs unchanged-read no-write optimization (creation/normalization still
  persist); P241 owns narrow sync.rs footer-icon invalidation on newly streamed media. P242A
  depot independently reviews P241; P242B ui_review independently reviews P240 then P239.
- P239 identified DLL Save, per-game Proton Apply, launch fields Enter-only, onboarding final bulk
  save and Storage Rename confirmation. Automatic valid field persistence with short debounce,
  ordered workers and visible failures; optional untouched onboarding suggestions remain opt-in.

- P239 storage_audit owns options/onboarding/Properties autosave inventory and implementation,
  including setup/settings/proton/dll_overrides/game_settings and relevant config persistence if
  needed. Confirm before broad backend changes. Reuse current workers; preserve action consent,
  validate partial fields, order/coalesce writes and show failure/retry. Focused tests/private UI.
- P240 depot audits backend state/download/installation for concrete bottlenecks and dead/old
  paths; report evidence before bounded edits, own only approved backend paths. No schema/version
  changes, no security/data-integrity weakening, no vendor modifications or new dependencies.
- P241 ui_review audits UI/library refresh/rendering for inefficiencies and dead/obsolete paths,
  excludes P239-owned controls; report concrete candidates then implement approved bounded fixes.
  No background focus/navigation or GTK-thread traversal. Coordinate shared paths explicitly.
- P242 independent cross-review after implementations (review others' owned paths), private
  changed-control exercise and root integrated fmt/Clippy/build. All workers read AGENTS.md,
  terse-code/scope-creep, retain previous edits, write reports not project records. Routine scoped
  terminal/tests/private synthetic fixtures authorized; no full suite per small change, user
  profile mutation, secrets, real helpers, packages or publication. Escalate policy choices.

## P235–P238 R89 ineffective controls

- Complete for the bounded control audit: independent private GTK matrix passes actual Properties
  persistence/rollback/retry, favorites without navigation, account invalidation, launch errors,
  gallery, Logs, tray and Storage controls. Final fmt/diff, all-target Clippy -D warnings and debug
  build pass after cleaning private fixture Cargo artifacts. All14 reviewed source hashes match;
  fixture markers absent from shipping executable. No full suite/package/publication performed.

- P235/P236/P238 implementation review with focused passing evidence complete; P237 independent
  changed-control GUI/source review running. Windows Move is refused with explicit limitation,
  native Move preflights and reserves games; no unsafe prefix migration. P238 also fixed P237's
  two stale-account Saving labels in game_settings.rs. Root final integrated checks pending.

- Confirmed inventory: launch EntryRow signal mismatch; silent compatibility/cloud/favorite
  persistence failures; single-screenshot no-op arrows; silent picker/desktop-launch errors;
  enabled no-op Storage remove/move/add paths; inert network header; log/tray feedback gaps.
  P235 additionally owns favorite section window.rs and gallery/URI controls. P236 owns shared
  file_open helper, cloud export picker and network header section (coordinate window edits).
  P238 depot will own logs.rs/tray.rs after P237 releases slot, then P237 resumes independent QA.
  Ordinary Config.save failures get visible feedback only: asynchronous snapshot writer would
  introduce cross-writer stale overwrites, so no unrelated configuration architecture change.

- P235 ui_audit: audit/fix library, details, game settings, files and Downloads controls;
  report concrete no-op reproduction and bounded changed-control verification. Own those UI
  modules and narrow supporting callbacks only; coordinate shared paths before edits.
- P236 storage_audit: audit/fix Settings/onboarding/account/Proton/Comet controls and narrow README;
  preserve helper consent/security. Own settings/setup/proton/comet/auth UI surfaces, no backend
  changes without demonstrated necessity and coordination. No real helper/account execution.
- P237 ui_review: independent inventory of remaining controls (navigation/actions/notifications/
  logs/screenshots/cloud/achievements), report findings to owners, then independently verify final
  fixes with private GTK/source/focused tests. Review only; root owns project records. Routine
  scoped tools/private fixtures/build allowed; no full suite/package/publication/user files.

## P231–P234 R88 performance and responsiveness

- Complete within the bounded audit: final root-reviewed source/reports, independent backend/
  storage/GTK reviews and shipping build checks pass. bash tools/check.sh: fmt, Clippy -D warnings,
  518 unit tests/13 ignored,6 top-level integrations/3 helper opt-ins ignored,5 Python tests.
  Explicit14088-file probe and private GTK matrix passed separately. cargo build --locked passes;
  reviewed UI hashes unchanged and fixture markers absent from shipping executable. Real Terraria
  installation/hardware timing remains user verification. No global Prototype ready declaration.

- P231/P232 implementation and P234A/B independent backend/storage source reviews complete with
  focused passing evidence; root reviewed reports/diffs. P233 implemented with10 focused tests.
  P234C independent private GUI interaction review in_progress. Final integrated build waits for
  fixture Cargo completion/provenance. No package/publication requested.

- P231 depot worker: download/depot.rs and installation/manager.rs; verify small-file journal
  bottleneck, batch safe checkpoints, eliminate evidenced repeated work, provide measured local
  progress. Synthetic high-file-count before/after and resume/cancel/integrity tests required.
- P232 storage worker: storage.rs and directly necessary filesystem evidence helpers; investigate
  reset/partial-install incompatibility and repeated validation costs. Preserve typed separation,
  reject ambiguous paths; report any policy choice before implementing it. Focused regressions.
- P233 UI worker: src/ui and README; audit long-action feedback and GTK blocking, integrate P231
  materialization progress and fix confirmed silent/stuck interactions. Inventory changed controls
  and test bounded success/failure/loading states. Coordinate backend interfaces, no backend edits.
- P234 pending independent QA/security cross-review by non-implementers, plus integrated build
  checks after workers' focused checks. Root owns records only. Routine scoped tools/isolated
  fixtures authorized; no real credentials/user-file mutation/helper/package/publication. All
  implementations report evidence, changed paths, remaining risks and gaps before completion.

Ownership refinements: P231 also owns installation/depot.rs progress plumbing and exact-path
bounded progress persistence/admission lock review. P232 owns profile_reset.rs retained payload
identity without resume plans. P233 owns installation/windows_executable.rs narrowly to prevent
out-of-payload symlink traversal alongside asynchronous chooser. These are evidenced R88 defects,
not additional features. Cross-review: UI reviews storage/reset; storage reviews depot; independent
UI reviewer follows when a slot frees. Preserve finite changed-control matrix and build provenance.

## P229–P230 R87 build failure

- Fix/checks complete: isolated before/after regression confirmed missing persisted config and
  outdated fixture layout. Only launcher test setup changed; lifecycle assertions/production guards
  retained. tools/check passes508 unit/6 integration/5 Python plus fmt/Clippy. P230 independent
  source review passes. Root commits/pushes and checks new CI under existing authorization.

- P229 compatibility owns diagnosis/reproduction/minimal fix for failed launch sign-out fixture
  at launcher.rs:965, and directly necessary affected paths. Reproduce exact test then tools/check
  build checks under isolated profile; no real game/helper/account. Report evidence/limits.
- P230 security independently reviews root cause, fix and no weakened lifecycle/storage guards;
  no product edits or duplicate full build. Root verifies logs, records, commits and pushes on
  user authorization, then monitors CI. No new features, package install or unrelated refactor.

## P227–P228 R86 version and publication

- P227 ui owns0.2.2 Cargo/lock/package/metainfo synchronization and scoped metadata checks.
- P228 compatibility read-only accumulated-diff inventory/release-note accuracy review. Root
  prepares descriptive commit and concise upstream PR6 comment, commits/pushes after checks,
  verifies remote head and comment. User explicitly authorizes both external mutations. No new
  product changes or full suite/package build; attribution Codex (GPT-6).

## P225–P226 R85 repair feedback and setup details

- Complete: manager reviewed source/counter semantics, nonempty Details/preparation screenshots,
  P226 independent scoped PASS with11 private assertions and cleanup. Final fmt/diff, all-target
  Clippy and production debug pass. No live repair/helper or full suite/package/publication claim.

- P225 ui in_progress owns download_chooser.rs and narrowly necessary UI/README: investigate
  empty Details and repair feedback gaps, populate useful stage detail and retain progress across
  preparation/accepted repair. Relevant checks, no backend expansion without evidence/coordination.
- P226 security independent review of truthful progress/nonempty details, consent/account/close
  behavior, bounded private physical GUI. No product writes; root records only. Reports
  /tmp/ludomere-p225-report.md and p226-review.md. Routine scoped tools/private tests/build allowed,
  preserve all work, no real logs/profile/helper/network execution/full suite/package/publication.

## P223–P224 R84 manual update progress

- Complete: manager reviewed scoped UI change and owner checks, P224 independent no-material-
  finding disposition and all12 isolated physical assertions, plus process cleanup. Confirmation
  hands accepted work to current detail stages; cached offline consent preserved. Relevant test,
  fmt/diff, Clippy and production debug build pass. No live update/helper or publication claimed.

- P223 ui in_progress: trace manual update confirmation/admission and detail operation rendering;
  close accepted dialog, retain actionable refusal, show download and final setup in current detail
  view. Own narrow UI paths and README, relevant regressions; backend edits only if evidenced
  necessary and coordinated. Preserve accumulated work/R83 recovery modal. No full suite/live data.
- P224 security independent review: admission/close/error/session lifetime and actual private GUI
  confirmation→detail stages→completion/failure. Review only, no source edits; report findings to
  P223. Reports /tmp/ludomere-p223-report.md and p224-review.md. Root records only. Routine scoped
  inspection, isolated fixture/tests and build allowed; no real credentials/helpers/publication.

## P220–P222 R83 recovery dialog and progress

- Complete within requested scope: root reviewed P220/P221 reports, three focused backend
  regressions, P222's 22 isolated physical UI assertions and independent no-material-finding
  disposition. Final fmt/diff, all-target Clippy and production debug build pass. Real-game
  recovery remains user verification; no real prefix/helper, full suite, package or publication.

- P220 ui owns recovery dialog/copy/progress continuation in ui/proton.rs, launch receivers and
  narrowly needed download_chooser/README. Replace footer-only interaction with launch-request
  modal, preserve backup consent/stale guards, show current component and honest progress through
  setup completion/errors. Relevant checks only, no real helper/file mutation.
- P221 compatibility traces existing setup progress/snapshots/cancel/result APIs, supplies precise
  UI contract; owns backend changes only if essential missing progress, coordinate first.
- P222 security independently reviews focus/session/consent/progress semantics and exercises
  bounded isolated GUI states. No product edits; root records only. Reports /tmp/ludomere-p220-
  report.md, p221-report.md, p222-review.md. Preserve all prior edits; no publication/full suite.

## P218–P219 R82 missing prefix recovery offer

- Complete: actual ownerless scaffold diagnosis, narrow marker-proven backed-up recovery,
  pre-cloud prefix validation and actionable refusal implemented. Root reconciled actual-launch
  regression, prefix7, final format/lint/debug and P219 independent PASS. No live recovery claim.

- P218 compatibility in_progress: trace typed launch errors and preparation rejection, minimally
  fix backend with focused actual-error regression; own installation/compatibility, no UI unless
  coordinated. P219 security independent review of cause, regression and preserved safety.
  Read-only structural metadata for reported prefix allowed, no saves/registry contents/secrets
  or real mutation/helper execution. Root records only; tests and debug verification scoped.

## P215–P217 R81 prefix recovery

- Complete within requested scope: manager reviewed backend6 + guard7 tests, P217 independent
  source/private-GUI PASS, final offline-DLC retry correction, documentation and P216 production
  fmt/Clippy/debug/provenance. No real game/helper execution, full suite, package or publication.

- P215 in_progress on focused lifecycle tests; P216 review (source landed, combined compile pass);
  P217 in_progress independent source and private GUI review. User explicitly confirmed backups.
  Final production checks follow fixture build with crate-only artifact clean.

- Minimal staged design authorized: typed failure→confirmed backed-up managed-prefix rebuild→
  durable SetupRequired→direct existing Repair continuation. Clear receipt only correct prefix/
  setup successful completion. Depot uses installedbuild repair, not update; offline needs explicit
  installer repair. No bare-prefix-ready claim, autolaunch or background dialog. Backup default
  communicated; no actual user data mutations by agents. Review path/process/crash guards.

- P215 compatibility investigates/owns minimal managed-prefix recovery backend in compatibility/
  installation paths and typed launch failure metadata. Define contract with UI, reuses locks/
  source identity/process tracking, preserve installed payload/user prefs. Investigate prerequisite
  replay needed for fresh prefix; no incomplete success claims. No real profile/helper execution.
- P216 ui owns launch-failure recovery offer, explicit confirmation/progress/results and README;
  no background focus/window changes, no error-string parsing for ownership. Await backend contract
  and user old-prefix disposition before dependent implementation.
- P217 security independently reviews destructive/path/process boundaries and private synthetic
  regression + relevant UI action controls. Review only, no product edits. Reports
  /tmp/ludomere-p215-report.md, p216-report.md, p217-review.md. Root records only; preserve work.

## R80 manual per-game update follow-up

- Complete for requested scope: root reviewed P212/P214 reports, corrected revision/cache-identity
  findings, targeted tests and P213 independent scoped PASS. Manual UI boundary checks and actual
  backend safety tests are distinct; no live GOG transfer claim. Final format/Clippy/build pass.

- P214 acquisition after initial-language fix owns backend manual Depot check→offer→confirmed
  queue contract, reuse existing update/discovery/validation without changing automatic policy.
- P212 ui after autosave owns Play availability and arrow action/check status/confirmation,
  worker-only IO and immediate in-place state; coordinate stable backend contract with P214.
- P213 independent review extends only to changed update flow/safety. User confirms both Depot
  and offline-installer sources; reuse installed source and supported acquisition paths. Relevant
  tests only, no real downloads or account access.

## P212–P213 R79 policy autosave

- Complete including P214 initial-language routing: autosave private UI persistence/failure/close
  checks, selector4 tests and independent source review pass; Save/Apply buttons removed.
  Final combined checks recorded with R80; no schema/package/publication changes.

- P212 ui owns update_policies.rs and narrow README; reuse existing persistence, serialize or
  coalesce async writes so latest choice wins, suppress initialization saves, retain account guards,
  surface failures; user steering removes the immediate language reconciliation button. No schema
  changes. Focused regression/build checks only. Report /tmp/ludomere-p212-report.md.
- P213 security independently checks autosave ordering/load/close/error and account protections,
  private synthetic GUI change/reopen/inheritance including rapid edits. No actual credentials,
  libraries/network/downloads; reviewer no product edits, root records only. Report p213-review.md.
  Coordinate Cargo and isolated fixture provenance; no full suite/package/publication.
- P214 acquisition traces initial Depot language selection, corrects any missing per-game override
  using existing global/default fallback and worker persistence reads. Own download_chooser.rs
  and narrowly necessary acquisition code/tests; no overlapping update_policies.rs/README.
  Report /tmp/ludomere-p214-report.md; P213 review includes this bounded acquisition selection.

## P210–P211 R78 sidebar remap crash

- Complete: manager reviewed narrow files.rs diff, exact pre-fix crash/fixed callback regression,
  latest hidden-label evidence and independent P211 PASS. Final format, Clippy10.17s, clean debug
  build23.77s and shipping artifact provenance pass. No broad GTK/full-suite claim.

- P210 ui owns files.rs/library.rs narrowly as necessary, identify reentrant map callback and
  fix without stale menu labels or silently lost updates. Preserve all accumulated edits; relevant
  regression, format/lint/compile only. Report /tmp/ludomere-p210-report.md.
- P211 security independently reviews exact fix and private synthetic remap reproduction, including
  hidden-label freshness and safe borrow lifetime. Reviewer reports findings, no product edits.
  Private isolated GTK fixture only, no live account/game/download; report p211-review.md.
  Coordinate Cargo artifact isolation; root records only. No package/publication/full suite.

## P207–P208 R77 Properties update controls

- P207/P208 complete: manager reviewed focused policy test, source/backend trace and private
  GUI override/Inherit save/reopen persistence with no duplicate controls or queued work.
  P209 complete: source and checkbox/Next/Back/discard review pass; final source build22.79s,
  post-clean Clippy9.00s and artifact provenance pass. No live GOG update tested.

- P209 ui additionally owns setup.rs and narrow README: checkbox on Game Files step for existing
  global auto_update_galaxy_installations, default-on/existing-value preserved, normal wizard draft
  save/discard behavior. P208 independently verifies wiring and visible checkbox. Relevant setup
  tests plus formatting/compilation only; no full suite or package/publication.

- P207 ui owns game_settings.rs/update_policies.rs and narrow README correction: replace Updates
  placeholder with single working existing per-game group, remove duplicate Installation location,
  accurately explain Inherit/On/Off scope/schedule, retain language feature. Preserve async saves
  and account guards. Relevant existing policy test and combined format/Clippy/debug only.
- P208 security independently traces persistence→policy resolver→automatic queue and checks
  isolated Properties Updates selection/save/reopen/inheritance without real account/downloads.
  No backend/schema changes absent concrete necessary defect; no unrelated cleanup. Reports
  /tmp/ludomere-p207-report.md and p208-review.md. Root records only, no publication/package.

## P205–P206 R76 sign-in credential-store investigation

- Complete for scoped implementation/verification: final P205 evidence and P206 PASS reviewed.
  Real credential persistence/login remains an explicit user-retry limit, not a passed live test.

- P205 implementation authorized after source evidence: worker-only standard Secret Service
  owner/activatable detection at explicit sign-in save; only if neither exists, bounded normal
  activation of advertised KDE compat name then standard-interface readiness verification.
  Standard providers take precedence; never override disabled settings/refusal or weaken secure
  persistence/session/tombstones. Fixed-safe error categories, relevant synthetic tests and README.
  Own auth.rs plus direct gio0.22 declaration using already-locked graph only. P206 independently
  reviews final code/tests; no actual session activation or credential probes by agents.

- P205 acquisition owns auth/keyring source diagnosis, minimal safe diagnostic recommendation,
  and relevant synthetic reproduction; no implementation until evidence/contract reviewed.
- P206 security independently reviews provider/error classification and permitted metadata checks;
  no actual credential reads/writes or desktop security changes. Root coordinates user clarification.
- Preserve all previous changes. No full suite/package/publication or fallback storage authorized.

## P202–P204 R75 onboarding and Storage sections

- Complete for requested scope: root reviewed implementation, focused test/build evidence and
  independent P204 scoped PASS. All requested UI/default/preservation behavior accounted for;
  physical checks and source-only limits documented in /tmp/ludomere-p204-review.md. No publication.

- P202 compatibility owns setup.rs and necessary narrow default config changes plus README.
  One plain editable path+browse per typed page; sensible suggestions, optional skip, preserved
  existing extras/default identities, worker validation, correct Back/Next/Proton/runtime/login.
- P203 ui owns settings.rs/settings/storage.rs: expandable Storage navigation with Game Library
  default and typed archive sections, retaining per-type management. Preserve existing storage
  deep links and selection indication; no backend/schema changes.
- P204 security independently reviews exact R75 diffs and isolated GUI changed controls after
  freeze. Use prior private fixture tooling only when needed, no real profile/account/network/
  game execution; keep fixture artifact separate and rebuild production after copy-out. Owners
  run relevant setup/storage/config tests and combined fmt/Clippy/debug only, not full suite.
  All owners return paths/evidence/limits in /tmp/ludomere-p202/203/204 reports; root alone records.

## P201 independent R73/R74 review

- Complete for scoped review: manager reviewed /tmp/ludomere-p201-review.md, concrete private
  deletion/sentinel evidence and final build provenance. P195/P196/P198/P199/P200 implementation
  and relevant tests complete. Review findings returned to owners and corrected; final literal
  title checks passed. No material scoped finding remains. Live account/game and unexercised UI
  permutations are explicit limits, not passes; no broad Prototype-ready or publication claim.

- Security owns read-only source review and isolated GTK/sourcecopy fixtures after P198/P200 handoff.
  Scope all new library controls, typed enforcement/migration/copy identity/update/deletion protection;
  no real profiles, credentials, accounts, game/helper execution or network. Synthetic deletion only
  under asserted private /tmp root. Report control evidence, exact source snapshot, cleanup and any
  blockers in /tmp/ludomere-p201-review.md. Known UI-owner finding: move per-file resolve_job_id off
  GTK before final acceptance. Final shipping binary rebuilt after instrumentation copied out.

## R74 archive deletion steering

- P199 compatibility additionally owns uninstall.rs default-off archive-deletion checkbox verification
  and corresponding recovery behavior. P198/P200 ui additionally owns game_settings.rs Properties
  buttons and existing files.rs/cleanup.rs type-filtered confirmation/removal workflow. Controls are
  independent of installedness; preserve other archive type, game files/saves and unrelated products.
  Explicit confirmation lists affected file scope. Private focused regression fixtures only.

## P195–P197 — R73 storage discovery and implementation preparation

- Discovery complete; implementation authorized after all user choices. P195 acquisition owns
  config/storage authoritative type+compatibility guards, managed/download identity, state migration,
  installation/update/reset protection and sections/window snapshots. Retain schema25, devrev8 only
  for necessary multi-copy managed identity; update canonical baseline24 migration and retained steps.
  P196 compatibility owns setup/settings/storage/update_policies/README. P198 ui owns chooser/files/
  details and DownloadDialogWidgets in mod.rs. P197 security returns for independent review later.
  Contracts: LibraryKind and three Vec<GameLibrary> sets, independent defaults; one opt-in per
  optional type; worker-only storage inspection/action validation. No optional fallback to Game
  Files, no file moves/deletes. Incompatible blocks all use until corrected; preserve root protection.
  Artifact category LanguagePack/Patch/Installer routes offline; Bonus routes extras. Install target
  stays Game Files. New jobs and file matching retain separate destination identity. Tests include
  focused config/schema/routing/updates/protection and private UI controls; no fullsuite/package.
  Required UI-B glue also owns AppModel.library_statuses and widgets/file_open.rs guarded content
  opens; preserve nonlibrary log access and Settings correction controls. Backend snapshots publish
  worker results. Strict unknown root content is incompatible, unreadable/missing is unavailable;
  use marker/journal/managed layout evidence without recursively traversing installed game payloads.
- Backend follow-on split after UI source completion: P199 compatibility owns installation planning,
  execution/launch/recovery gates plus profile_reset and download/auto_install protection; P200 ui
  owns updates.rs and download/cleanup.rs per-root eligibility/retention. P195 retains config/storage/
  state/managed/download.rs+manager and shared snapshots. Preserve existing explicit per-game offline
  policy overrides; new optional-type flags set defaults, membership restriction always applies.

- P195 acquisition: read-only backend inventory of typed storage/config, download routing, install
  from offline files, update selection and necessary persistence changes. Return minimal proposed
  contract/tests, not implementation, while user migration/update-scope answers are pending.
- P196 compatibility: read-only UI inventory of onboarding, Storage, destination prompts and
  missing-folder list states. Identify exact owned surfaces and acceptance checks; preserve R70–R72.
- P197 security: read-only storage safety/migration/cleanup/reset review; propose path separation
  rules and risks without accessing user profiles or libraries. Root records only.
- Authorized: source/record inspection and reports in /tmp; no product edits, real account/files,
  downloads or execution needed for discovery. Implementation ownership follows user clarifications.

## P192–P193 — Factory reset failure follow-up

- P192/P194 complete; P193 independently verified actual private reset with failed credential
  deletion in both workers, retained marker and fresh-process restore refusal, visible error/retry
  and preservation checks. Manager reviewed reset-result.json: profile removed, seven sentinels
  preserved, cleanup/fresh restore exit0. Focused auth8/reset9, fmt/Clippy/fresh debug pass. Actual
  desktop credential-store failure cause remains unknown; no real profile accessed or reset.

- P194 acquisition owns auth.rs reset-only cleanup contract/tests; P192 owns inline Settings
  progress/errors and profile_reset integration. P193 independently checks confirmed durable marker,
  credential failure/restart and full private cleanup. Actual reported blocker is credential delete;
  allow only that best-effort external cleanup, preserve other safety gates. No real keyring access.
- P192 compatibility owns reset diagnosis/minimal correction and focused tests/docs; retain all
  uncommitted R70/R71 changes. Investigate actual error once supplied; do not assume the cause.
  P193 security independently reviews lifecycle/path protection and private failure controls.
  Root records only. No live profile reset, credential access, full suite/package/publication.
- Acceptance: preparation failures visible in Settings, successful existing close/cleanup preserved,
  evidenced root cause corrected without weakening protection. Return scoped reports in /tmp/
  ludomere-p192-report.md and -p193-review.md; distinguish simulated checks from user incident.

## P189–P191 — Explicit factory reset and sign-in investigation

- Implementation and scoped verification complete: 17 focused cases, fmt/Clippy/fresh debug and
  independent P191 private controls/reset process-replacement PASS. No material scoped finding;
  actual user login cause still requires retry with new diagnostic source. Preserve this limitation
  in handoff, no claim of live GOG authentication success. No publication requested.
- P189 acquisition owns auth source investigation, bounded evidenced fixes and relevant tests;
  source/public diagnostics only while user symptom/build reply pending. Coordinate window.rs
  overlap before edits. No real logs/profile/credentials/account interaction or speculative fixes.
- P190 compatibility owns settings reset control, config option retirement, window/reset integration,
  safe existing backend reuse, README and focused fixture checks. Preserve all library payloads and
  protected paths, close writers before cleanup, errors actionable. No actual user reset/deletion.
- P191 security independently reviews safety and affected private inert controls; reviewers do not
  edit product. Root owns records. P190 proposes lifecycle/legacy-option contract before edits.
  Owners coordinate shared files/serialized checks. Focused tests/fmt/Clippy/debug as appropriate;
  no full suite/package/publication or dependency/schema changes. Reports /tmp/ludomere-p189-report.md,
  -p190-report.md, -p191-review.md. User login symptom/build question is pending.

## P187–P188 — Version 0.2.1

- Metadata/checks and independent review complete; manager approves seven-path publication.
- Acquisition owns four release metadata files, scoped metadata/format checks and approved commit/
  normal origin-main push after review. Security independently reviews exact diff and consistency;
  root alone owns records. No source behavior, dependency/schema changes, builds/full suite/package,
  tag/release or PR mutation. Preserve older AppStream entries; pkgrel resets to 1.
- Evidence: /tmp/ludomere-p187-report.md and /tmp/ludomere-p188-review.md. Publication must return
  local/remote SHA and clean tree. Root records freeze before commit; final outcome in handoff.

## P186 — Publish reviewed library/menu fixes

- Acquisition owns explicit 14-path commit, normal push to origin main and concise new PR6 comment;
  root reviews inventory/message and final evidence. User authorizes these external mutations.
- Reuse completed P183–P185 verification; inspect diff/status and confirm PR head/base/remotes before
  mutation. No force push, new implementation, tests/build/package or PR body replacement. Stop on
  unexpected changes/divergence. Return commit SHA, remote match, comment URL/readback, clean status
  in /tmp/ludomere-p186-report.md. Records frozen before publication, outcome recorded in handoff.

## P183–P185 — Library state, update colors, installer recognition and grid menu

- Complete: owner reports, 12 scoped tests, final fmt/Clippy/shipping debug and independent P185
  scoped PASS reviewed. Actual Add/root/menu/Favorite controls exercised privately; limitations in
  status/README retained. No full suite/package or publication. Private processes stopped.

- R68 user-approved follow-up: P184 also owns minimal files.rs Favorite dispatch correction using
  the existing explicit Hide binding pattern and a focused detach regression. P185 rechecks list/
  grid favorite target, persistence and no-navigation; repeat only affected checks and final build.

- R66/R67 completed. P183 acquisition owns Storage Add/default-installer controls, minimal config/
  managed/state changes and sections/window rematching integration. P184 compatibility owns sidebar
  colors, reusable row/tile context menu, collection tiles, README and final combined checks.
  P185 security independently reviews source and private physical controls; root owns records only.
- Acceptance: library additions update without click, agreed update/run color precedence, truthful
  update tooltip, explicit installer root actually used, late metadata rematches affected files
  without losing unrelated records, grid/list menu parity with no secondary-click navigation.
- Reuse existing bounded off-GTK workers, validate current account/root/operation lifetime, preserve
  independent folder settings and durable data. No global scan per metadata event or unapproved
  import/schema changes. Authorize focused regressions, private inert GTK fixtures, fmt/Clippy/debug
  for integration; no full suite/package/live profile/account/helper/game/payload/publication.
- P183 proposes transaction/root guard contract before dependent implementation; owners coordinate
  shared UI declarations and serial checks. P185 returns findings for owner fixes, never product edits.
  Evidence /tmp/ludomere-p183-report.md, -p184-report.md, -p185-review.md.

## P178–P179 — Version 0.2.0

- Metadata implementation/review complete. Locked offline metadata, parsed TOML/XML consistency,
  full lockfile dependency preservation comparison, fmt/diff checks pass; independent P179 PASS.
  Manager reviewed four-file diff/reports and approved seven-path commit/message. Records frozen
  for normal push; final remote/clean-tree evidence in /tmp/ludomere-p178-report.md and handoff.

- R65 in progress: acquisition edits version metadata only and verifies locked offline Cargo
  metadata/TOML/XML consistency and diff; security independently reviews. Root records only.
  After review/freeze, commit explicit paths and push normally; verify remote and clean tree.
  No full tests, binary/package build, tag/release, schema change or dependency updates.

## P176–P177 — Build failure diagnosis and correction

- Local fix/verification complete. Manager inspected exact before/after makepkg logs, one-line diff,
  source hashes/exclusions and independent package/helper/license audit. No unresolved local finding.
  Approved four-path commit/message, normal push, narrow PR update; records frozen before publication.
  Remote CI remains a separate pending check, never inferred from local success.

- Reproduction confirmed five missing-fixture compile errors after successful release build. Narrow
  .zlib source inclusion corrected them; complete makepkg now passes fmt/Clippy, 467 unit tests,
  six main integration cases, five Python boundary tests and package creation. Twelve unit tests
  remain intentionally ignored by this general run. P177 independently verifies source/package.
- Complete prior branch/PR delivery: after reviewed artifacts/frozen records, acquisition commits
  the four changed paths and pushes normally; security updates PR packaging/verification mentions
  and checks new CI state. No package artifacts committed, manual rerun, force push or merge.

- User confirmed local pacman and CI failure. Package/full-check reproduction authorized in existing
  pinned Arch environment; capture output, no host/package installation or remote rerun. Prior
  3dcaf6e CI formatting failure is already fixed by a7408a4; latest run still in progress, so current
  package issue remains unestablished. Owner checks existing logs before rebuilding.

- R64 in progress. Acquisition owns fresh CI/build logs, reproduction and minimal evidenced fix;
  security independently reviews clean-environment/configuration gaps and final correction. Root
  owns records; no product edits. Report actual failing stage before fixing and preserve checks.
- Authorize read-only GitHub runs/logs, cached local compile and relevant tests; broader build checks
  only if needed to reproduce the failing build. No package build until established relevant scope,
  host installation, credentials/profile/game/helper access, dependency patches, commit/push or CI
  rerun. Reports /tmp/ludomere-p176-report.md and /tmp/ludomere-p177-review.md; logs retained safely.

## P172–P173 — Proton Settings parity with onboarding

- Complete for scoped local assurance. Manager reviewed P172/P173 reports, final source diffs,
  five focused-test/check logs and actual Settings/GOG/wizard screenshots. Independent review reports
  no open material finding; fixture stopped. Exact-final active-Cancel guard passed fmt/Clippy/debug.
  P174 copy/spacing complete. P175 commit message/eight-path inventory and updated P171 PR body
  approved; records frozen for normal push and verified PR readback. No broader readiness claim.

- Owner implementation/checks complete; independent review closing. Five focused cases pass (Settings
  GTK, wizard GTK, runtime metadata, two folder cases), fmt/all-target Clippy/debug/diff checks pass.
  Final publication P175 acquisition follows reviewed report/frozen records; normal origin-main push
  and PR6 update by security. No full suite/package. See /tmp/ludomere-p172-*.log.

- P174/R63 acquisition owns exact GOG Online Services/Check for Updates copy and small top margin
  in online-settings presentation only. No adjacent redesign; source review and combined formatting/
  build suffice for copy/spacing. Coordinate distinct paths with compatibility; publish together.

- R62 in progress. Compatibility owns Proton Settings/shared essential setup glue, scoped tests
  and README; security independently reviews source and isolated controls, then updates the retained
  PR draft. Root records only. Acceptance: automatic explicit saves, Custom-only chooser/last option,
  always-visible Proton downloads, bottom runtime state with automatic checking and explicit missing
  download; preserve saved choices, wizard, per-game semantics, cancellation and stale-result guards.
- Authorize focused selector/runtime/setup tests, fmt/Clippy and debug build for isolated GTK review.
  Private HOME/all XDG/bus/Xvfb and inert files only; coordinate fixture processes. No full suite,
  package, live account/profile, real game/helper or real runtime payload. Minimal reuse, no schema/
  backend changes. Publish only after owner/reviewer evidence is reviewed; P171 draft waits final push.


## P170–P171 — Publish onboarding wizard and update PR

- P170 complete, P171 held for R62; user-authorized R60/R61 publication. P170 acquisition reviews and commits the six
  changed paths, pushes normally to verified origin main, and verifies remote HEAD. P171 security
  prepares a concise PR6 body retaining existing coverage, adds wizard/refinements, and publishes
  after push verification and manager review. Root owns records and release decision.
- No product edits, tests, builds, lint, package, force push or merge. Preserve prior work and
  distinguish earlier validation from the latest untested source-only correction. Acceptance:
  accurate commit, matching local/remote commit, clean tree, verified PR body/base/head.


## P168–P169 — Default detected Proton rather than Custom

- R61 follow-up; complete after source-only review. Compatibility owns minimal selector/Next glue; security independently
  reviews source only. Keep saved choice else detected-first, Custom last/explicit including empty
  discovery; preselected visible choice must be accepted by Next without change-away/back. Preserve
  read-only discovery and all prior work. Root records only. User explicitly forbids tests; no tests,
  GUI fixtures, build/lint/package/publication. Scoped source/diff inspection only; report limits.
- Manager reviewed the two-file incremental diff and reports /tmp/ludomere-p168-report.md and
  /tmp/ludomere-p169-review.md. Saved choice retained, detected version suggested without discovery
  writes, empty prompt precedes Custom, and direct Next confirms the suggestion off GTK. No open
  material source finding. This correction remains uncompiled and unexercised per user instruction.

## P166–P167 — Simplified Proton/runtime wizard steps

- R61; complete for scoped local assurance. P166 compatibility owns setup/proton presentation, necessary minimal shared
  UI composition, scoped regressions/README. P167 security independently reviews changed controls
  and async selection/save/readiness/consent boundaries. Root owns records only.
- Acceptance: exact requested copy, automatic user-driven Proton persistence, conditional custom
  picker/download group, runtime auto-check on entry with truthful loading/found/missing/error and
  download enablement; remove redundant controls. Preserve defaults on failed/cancelled custom
  choice, reject stale callbacks, prevent selection races, retain explicit acquisition and separate
  optional login. Skip semantics pending user answer only; completed-profile behavior unchanged.
- Authorize scoped source/docs/tests, private HOME/allXDG/bus/Xvfb inert fixtures, relevant tests,
  fmt/Clippy/debug. No full suite, real profile/credentials/account/helper/game/network payloads,
  schema/dependency/package/commit/push. Preserve all R60 work. No unrelated Settings redesign;
  report essential shared changes and material ambiguities before changing policy. Return changed
  paths, controls/evidence, test logs, risks/limits; reviewer never fixes product code.
- Accepted minimal design: wizard-only auto-save mode on existing selector with guarded population,
  serialized saves/custom sentinel and detection state; runtime controls expose check results on
  entry/reentry/transfer completion. Ordinary Settings unchanged. Initial/default onboarding uses
  exact requested default copy; existing per-game repair retains per-game persistence/context.
  Unsupported/no-runtime-required states remain truthful; no external runtime/backend redesign.
- Pending optional Skip answer does not block retaining its existing close/discard behavior, stated
  to user. Removing deferral checkboxes must not leave an invisible prior deferred flag bypassing
  completion readiness; successful normal Finish validates readiness and clears deferral.
- Manager reviewed /tmp/ludomere-p166-report.md and independent -p167-review.md scoped PASS,
  final source hashes, actual screenshots and logs. Four focused tests, fmt/all-target Clippy/debug/
  diff pass; exact final UI matches physically tested source. Auto-save/custom/empty-discovery,
  cross-group busy/Skip, selected-runtime readiness/error/cancel/retry, save/login/restart pass.
  No unresolved material finding; private fixtures stopped. Real payload/auth/runtime execution
  untested, no full suite/package/publication or global Prototype ready declaration.

## P164–P165 — Welcoming sequential onboarding

- R60; complete for scoped local assurance. P164 compatibility owns setup wizard presentation, minimal shared Proton UI
  composition and README/tests. P165 security independently inventories/exercises changed controls
  and reviews persistence/consent/lifecycle boundaries. Root owns records; no product edits.
- Acceptance: Welcome, individual game-folder/download-folder/Proton/runtime steps and final
  settings completion before optional GOG sign-in; Back retains drafts, Next validates relevant
  input, step position is clear, explicit downloads and existing choices preserved. Close/failure/
  retry/account change cannot commit incomplete defaults or open login unexpectedly. Existing
  completed profiles do not automatically repeat onboarding; Finish setup remains usable.
- Authorize scoped source/docs/tests and private HOME/allXDG/bus/inert GTK fixtures, focused
  tests/fmt/Clippy/debug. No actual profile/credentials/account login/helper execution/network
  payloads, schema/dependency/package/commit/push/full suite. Preserve prior behavior except the
  requested presentation. Report design/API before edits that change persistence/acquisition;
  escalate material product choices. Owner/reviewer reports contain paths/evidence/limits.
- Accepted composition: five steps Welcome/Game folder/Download folder/Proton/Windows runtime,
  then existing separate optional login after durable defaults save. Explicit Proton selection and
  downloads retain immediate persistence with truthful copy; folder drafts save only on completion.
  Minimal scoped acquisition busy/cancel handles protect navigation/Skip/Close, preserving Settings.
  P165 uses private inert result boundaries for acquisition/login ordering, with limitations stated.
- Manager reviewed source, /tmp/ludomere-p164-report.md and independent -p165-review.md PASS,
  actual screenshots and logs. Two focused tests/fmt/all-target Clippy/debug/diff pass. Navigation,
  pickers/drafts, explicit selection, busy Cancel/Skip, readiness/deferral, save failure/retry,
  save-before-login and restart pass isolated controls. Final source delta reconciled, fixtures
  stopped; no unresolved scoped finding. Real acquisition/auth and host portal variants untested;
  no full suite, package or publication, and no global Prototype ready declaration.

## P161–P163 — Publish prefix cleanup and concise complete PR

- In_progress. P161 acquisition inventories/stages/commits/pushes R59 to fork main; P162
  compatibility audits current PR/body and complete divergence, drafts concise feature/fix-complete
  replacement and updates PR6 after push. P163 security independently reviews both publication
  inventories/messages and feature coverage/claims. Root alone owns coordination records.
- Acceptance: reviewed9-path R59 commit with useful message, normal fork push and matching refs;
  existing KonoTyran/ludomere PR6 remains main-targeted, shorter clean Markdown retains all earlier
  feature/fix mentions plus later implemented changes, and API readback matches reviewed body/head.
- Authorize necessary local Git/read-only remote comparison and authenticated normal fork push/PR
  description update, explicitly requested by user. No force/upstream branch write/merge/comments,
  product edits/package/build/full-suite/secret output. Prior verified evidence remains applicable.
  Stage and publish only after independent and manager review. Stop for divergence/new scope;
  return exact inventories, drafts, coverage mapping, results, refs and URL. No stale PR assumptions.
- P161 manager/independent content review clears exact9 paths and full R59 message; source hashes
  match verified P160 candidate and fresh fork main matches be82a43. Stage/index check then normal
  same-fork authenticated HTTPS commit/push authorized; no new test or build needed. P162/P163 PR
  coverage review proceeds separately, and body publication follows the verified push.

## P158–P160 — Remove managed prefixes during uninstall

- R59; complete for scoped local assurance. P158 acquisition owns safe backend uninstall/recovery/prefix cleanup and
  focused tests. P159 compatibility owns confirmation/partial-failure presentation, README and
  applicable UI tests. P160 security independently reviews destructive boundaries and controls.
  Root owns coordination records only. Agree APIs/file boundaries before shared edits.
- Acceptance: healthy and incomplete Windows uninstall removes only the corresponding managed
  prefix after affected writers stop; absent prefix succeeds; errors remain actionable/retryable;
  symlink targets/other games/external saves/preferences remain untouched. Native behavior stays
  independent, downloads stay opt-in. Confirmation accurately warns about prefix-contained saves.
- Authorize scoped source/docs/tests and isolated HOME/allXDG/bus inert filesystem/GTK fixtures,
  focused tests/fmt/Clippy/debug. No actual profiles/logs/credentials/user deletion, real game/helper
  execution, dependency changes/schema/package/commit/push/full suite. Preserve prior work; stop
  for broader destructive boundaries or uncertain ownership. Return source/evidence/limits.
- Confirmed gaps: healthy Depot removes prefix after payload (losing retry evidence on failure),
  Windows offline and incomplete recovery retain it. Reuse anchored nofollow cleanup and existing
  recovery receipt with optional validated prefix identity; preserve prefix-only retry after payload
  removal, reject replacement/conflicting ownership, never let a legacy receipt authorize a new root.
  Preview exposes exact prefix targets/counts; normal paths validate before destructive work and
  remove after writers drain. No new recovery store/schema or broader reset changes.
- P160 identified vendor leader-exit/error can leave descendants; necessary offline-uninstall
  protection reuses existing SetupProcessGuard in the same recovery receipt. Arm before spawn,
  record known group, clear only proven drain; uncertain same-boot recovery stays blocked. No new
  supervisor/store/schema. Cover failure/crash and markerless-native preservation in focused tests.
- Manager reviewed P158/P159 reports, independent /tmp/ludomere-p160-review.md PASS, source,
  final logs and actual confirmation/result screenshots.17 backend+2 UI focused tests, fmt,
  all-target Clippy/debug/diff pass. Physical Cancel/unchecked cleanup/permission failure/prefix-only
  retry/native preservation/missing-prefix checked recovery pass; exact source reconciled and
  fixtures stopped. No scoped blocker. Real vendor/runtime execution remains untested; no full
  suite/package/commit/push or global Prototype ready declaration.

## P157 — Commit and push state, logout, DLL and Depot corrections

- User explicitly authorizes publication of current R53–R58 work. In_progress; acquisition owns
  inventory/message/staging/commit/push, security independently reviews scope and claims. Root
  owns records only. Include existing source/tests/README/records; exclude artifacts/private logs.
- Acceptance: comprehensive useful message, exact reviewed inventory, diff checks, successful
  normal push to LegendaryLinux/ludomere main and matching remote/local refs. Prior verification
  remains applicable; no unnecessary test/build rerun. No force, upstream write or product edits.
- Authorize necessary local Git and authenticated fork network operations without printing secrets;
  stop on divergence, unexpected content or new permissions. Return paths/message/review and refs.
- Content review complete: exact35 source/README/record paths and comprehensive message cleared
  by manager and independent /tmp/ludomere-p157-review.md. Staged diff checks pass; fresh fork main
  matches local0078c84. Commit and normal same-fork HTTPS push authorized. No code or test rerun;
  final publication result is verified through local/remote refs and reported in the handoff.

## P155–P156 — Multiple-container Depot preparation

- R58/R49; complete for scoped local assurance. P155 acquisition owns dependency/manifest diagnosis, minimal correction,
  focused regressions and necessary README. P156 security independently reviews integrity and
  regression evidence. Root records only; preserve P146–P154 uncommitted work.
- Suspected boundary: combined_manifest offsets internal container indices, then canonical_json
  serializes through a single-container wire format. Establish actual call chain and reproduce.
- Acceptance: valid multi-container game/dependency combination prepares without losing container
  references or weakening path/hash/size validation; invalid references still fail; meaningful
  before/after regression plus focused tests/fmt/Clippy/debug and independent review.
- Authorize repository/public primary metadata reads, scoped source/test edits and private HOME/
  allXDG/bus inert fixtures. No credentials/real profile or new private logs, user deletion, actual
  helpers/games, dependency patches, schema/package/commit/push or full suite. Escalate broader
  behavior/trust changes. Reports include cause, paths, evidence, limits and outstanding findings.
- Accepted approach: explicit versioned internal snapshot in existing manifest JSON field, strict
  network/wire parser unchanged; typed combined validation reuses existing path/hash/chunk rules
  plus full reference size/bounds checks. Installed snapshot/current-manifest readback understands
  internal format and existing wire snapshots. Multi-container identities bind indices; preserve
  existing valid single-container fingerprints. No database schema or dependency changes.
- Known pre-P136 multi-depot markers may have the earlier unbound fingerprint despite the current
  snapshot serializer regression. Permit exact legacy identity comparison only when reconstructing
  an existing marker from original strict wire current_sources; never new typed snapshots/targets.
  Preserve depot/build provenance and cover tampered-source refusal. Preparation also checks final
  snapshot size before payload work to prevent a late persistence-size failure.
- Manager reviewed source, /tmp/ludomere-p155-report.md and independent -p156-review.md PASS,
  owner15 focused passes and reviewer3 repeated targeted passes, fmt/all-target Clippy/debug/diff
  evidence. Preparation, installed cache and journal/Resume preserve multiple containers; malformed
  data still fails. Debug09:20:17UTC. No full suite, actual game/helper, package or publication;
  fresh Witcher installation remains user acceptance. No unresolved scoped finding.

## P152–P153 — Persistent controller detection failure

- R57 reopened. P152 compatibility owns source/runtime differential diagnosis and a concrete
  evidence-based correction proposal; P153 security independently reviews diagnosis and any probes.
  In_progress. Root records only; preserve all prior uncommitted changes.
- Authorize repository/public primary/runtime-source reads and inert private diagnostic preparation.
  No speculative controller defaults, dependency edits, user-prefix mutation, real games, raw input,
  host permission changes, package or commit. User-private diagnostic access pending scoped answer.
- Acceptance: establish whether the previous XInput correction applied, selected runner and device
  visibility differences, separate proven findings from hypotheses, then fix demonstrated own-code
  defects with focused regression/review. No hardware success assertion without actual evidence.
- Workers return safe reports with sources, paths, tests, findings, limits and needed next checks.
  Escalate new private access/execution or policy choices before proceeding. Relevant tests only.
- User approved exactly the offered read-only diagnostics (launch logs/settings/XInput prefix
  entries/controller metadata), excluding credentials/saves/raw input. Latest cargo run retained
  unchanged overrides. P152 owns reads; P153 coordinates independent review, no duplicate broad scans.
- P154 acquisition reactivated for the DLL composer's wildcard-versus-bare lookup proposal,
  src/compatibility/dll_overrides.rs, focused tests and coordinated README only. P153 found Wine's
  environment precedence applies per lookup key, so native wildcard registry entries may outrank
  bare environment defaults. Exact runtime source verification/proposal precedes any edits.
  Preserve inherited/per-game priority, conditional receipt policy and unrelated uncommitted work.
- Exact GE11-7 Wine46b29104 confirms the conditional gap. P154 authorized paired bare/wildcard
  composition for chosen DLLs, conservative managed-only exclusion for explicit upstream native
  XInput configuration, focused source-backed lookup regressions and precise README guidance.
  P153 independently reviews. No runtime probe needed before this evidenced correction; actual
  loaded module/controller outcome remains user acceptance. Relevant tests/fmt/Clippy/debug only.
- P152 diagnosis and P154 correction complete for scoped local assurance; manager reviewed
  /tmp/ludomere-p152-report.md, -p152-composer-report.md, exact source and final logs. P153 review
  finds no scoped blocker. Five focused tests/fmt/all-target Clippy/debug/diff pass. No UI changes,
  full suite, dependency code changes, package/commit or user-profile mutation. R57 actual hardware
  acceptance remains open: retry BIT.TRIP automatically, Witcher requires explicit Builtin rows.

## P149–P151 — DLL overrides and controller compatibility

- Complete for scoped local assurance. P149 acquisition owns DLL persistence/editor/launch integration for R56;
  user chose per-game only. P150 compatibility investigates R57
  own UMU/Proton command environment and public primary Lutris/runtime evidence, proposing and
  implementing only confirmed necessary fixes. Agree ownership before edits to shared compatibility
  paths. P151 security independently reviews and exercises affected controls after implementation.
- Acceptance: editable/persistent validated DLL modes with deterministic composition, actionable
  errors and native independence; source-supported controller diagnosis, correction with relevant
  regression evidence, no unproven hardware success claim. No schema bump absent necessity.
- Root records only. Authorize source/public-primary read-only research and scoped changes/tests,
  private HOME/allXDG/bus/inert fixtures, final fmt/Clippy/debug. No full suite, real user profile or
  game/helper execution, raw input capture, credentials, system changes, package/publication.
  Relevant device metadata-only diagnostics may be proposed when needed; no blanket host scans.
  Escalate ambiguous semantics, new access/execution or unconfirmed broad work. Reports must list
  paths, evidence, limitations and findings; preserve all P146–P148 uncommitted changes.
- P149 design accepted: existing atomic proton.json gains default-empty per-game DLL maps, five
  typed modes with name validation, worker-backed Game Settings/Compatibility rows. Foreground
  Windows launch only; managed defaults then inherited/command then explicit per-game choices.
  Owner paths compatibility DLL module/proton/mod, launcher, UI DLL editor/game_settings/mod and
  coordinated README. P150 owns dependency_setup plus bounded receipt helper/tests and supplies
  provenance defaults; no shared launcher edits without P149 coordination.
- P150 correction accepted after primary evidence: remove blanket native xinput from DirectX
  recipe; only a validated successful old-method receipt bound to current prefix identity adds
  launch-local builtin defaults for its four affected XInput DLLs. Preserve inherited/per-game
  intent, no registry or DLL mutation. Missing/malformed/recreated-prefix evidence does not trigger.
  User hardware cause remains unverified. P149 owns final proportionate checks after source freeze.
- Manager reviewed P149/P150 reports and independent P151 scoped PASS, source, final logs and
  representative actual editor screenshot. DLL4/preference7/GTK1/uninstall1/receipt1 focused checks
  pass (overlapping filters distinguished); final fmt/all-target Clippy/debug/diff pass. GUI tests
  cover five modes, validation, cancel, save/load failure retries, removal/reopen and delayed save
  after close. Repeated-suffix and oversized-publication findings closed; previous preferences
  preserved on failure. Private fixtures stopped. No full suite/package/commit/push; user hardware
  verification and broader original gates remain outstanding, no global Prototype ready claim.

## P146–P148 — Immediate state, sidebar colors and recoverable logout

- R53/R54: P146 compatibility owns detail/action refresh, sidebar styles and relevant UI tests/docs.
  R55: P147 acquisition owns auth/download/install/reset lifecycle backend and coordinated sign-out
  UI region in window.rs. Agree file regions/APIs before edits; root records only. P148 security
  independently reviews lifecycle/privacy plus physically exercises affected controls.
- Complete for scoped local assurance. Starting source0078c84 was clean and pushed. No unrelated refactor/schema/
  dependency/package/publication. Scoped source tests/docs and private HOME/allXDG/bus/local HTTP/
  inert-process/GTK fixtures allowed; no real profile, tokens, game/helper execution or user deletion.
- Acceptance: visible detail action settles immediately after file completion without navigation;
  sidebar follows green running/blue downloading/white installed/grey otherwise; sign-out remains usable across
  failed and active operations, no stale-account callbacks/credential resurrection, recoverable
  interruption with honest partial/error handling and no blocking GTK work. Reset/running-game
  choices are settled below. Proportionate tests/fmt/Clippy/
  debug and independent review; no full suite solely for this change. Escalate material new scope.
- Interview settled: full reset discards resume records/settings but retains files; running games
  stay running. P147 minimal auth-generation/logout marker, install pause/drain/resume, account
  callback guards, separate game/cloud activity and conditional reset shutdown approved as R55
  necessities. Reset errors leave signed-out UI with existing-button retry; no new screen/schema.
- After UI freeze, P146 also owns narrowly scoped cloud_saves/{mod,api,sync}.rs captured-session
  cancellation guards/tests necessary to keep running games from uploading after sign-out. Agree
  sync_for_session API with P147 launcher owner; no broader cloud feature/refactor. P148 reviews.
- Confirmed final R55 contention correction adds sections.rs and online.rs to P147: preserve the
  sign-out detail page lifetime, move backend invalidation to its worker, and make generation reads
  nonblocking while retaining the existing serialized persistence barrier. No broad session redesign.
  Exact journal pause failures must surface, and durable logout prevents replay until normalization.
- Manager reviewed final P146/P147 reports and independent P148 scoped PASS, source/diff checks,
  focused tests, actual private GUI evidence and source reconciliation. Final fmt/all-target Clippy
  with warnings denied/debug pass; no full suite/package/publication. Same-page actions, all four
  colors, held-barrier logout, post-logout Stop-to-Play, reset/retry/fresh launch and file preservation
  pass. Private fixture processes stopped. Real-service/runtime acceptance remains user-assisted;
  no global Prototype ready declaration. New changes remain uncommitted for cargo run testing.

## P145 — Commit and push verified installation improvements

- User authorizes current changes committed and pushed. In_progress; compatibility owns exact
  inventory/message/staging/commit/push, security independently reviews content and claims. Root
  records only. Include accumulated R42–R46/R49–R52 work, tests/docs and user game acceptance.
- Acceptance: comprehensive accurate message, reviewed explicit paths without secrets/generated
  payloads, diff checks, normal push to origin main (LegendaryLinux/ludomere), remote hash matches.
  No new product edits/builds/package/full suite, force push or upstream mutation. Prior verified
  evidence remains applicable. Stop on unexpected divergence or unrelated content.
- Manager reviewed exact40-path inventory and complete message; independent review cleared prose,
  public metadata fixtures and privacy. Final staged check precedes commit and normal fork push.
  Fresh fork main57574b3 is an ancestor of local a99f277, so push also publishes that prior local
  commit. SSH unavailable; authenticated HTTPS to the same fork is allowed without remote changes.
  Publication outcome is verified by matching remote/local refs and reported with the final SHA.

## P143–P144 — Persistent certificate failure

- R52 follow-up; complete for scoped local assurance. P143 compatibility traces own command→UMU→Protonfixes→downloader
  and confirms a narrow correction; P144 security independently reviews causal evidence and trust.
  Prior source-only proposed fix did not establish runtime success; do not repeat that claim.
- Read-only primary sources/repository/system helper code and scoped diagnosis allowed; preserve
  prior edits/user state. Relevant private-profile inert tests and own-code fixes after cause
  confirmation only. No fullsuite, real helper/game/installer execution, userprefix/privateenv/
  credentials, dependency patches, host changes, package or commit. A concrete runtime-only probe
  may run after independent isolation review as ordinary reversible/read-only debugging; platform
  approvals remain binding. This excludes Proton, installers, games and user-prefix access. Root records only.
- Acceptance: demonstrated propagation/override cause, focused secure regression or clearly stated
  external blocker, no new guessed CA default. Reports include paths/evidence/remaining limits.
- New actual retry identifies inherited SSL_CERT_FILE/SSL_CERT_DIR as the skip cause despite
  absence in the shell. Owners trace Cargo/library initialization and propose a minimal path/trust
  correction before implementation; no startup-only provenance assumption without evidence.
- Isolated Cargo/direct comparison confirms Cargo adds both SSL keys. Approved minimal own-code
  translation of inherited readable absolute CA file/directory paths through proven host mounts,
  preserving selected trust, unmapped/invalid values and directory-list semantics. Generic Arch,
  no user-specific locations; multi-layout/private fixtures and independent review required.
- Manager inspected final source, P143 report and P144 independent PASS, nine focused tests,
  fmt/Clippy/debug/diff checks and actual runtime HTTPS200/TLS verification0 with translated
  SSL_CERT_FILE/DIR. No scoped blocker remains. Actual Proton/DirectX retry is user acceptance;
  no full suite, dependency modification, package or commit. All probe processes stopped.

## P141–P142 — Winetricks certificate handling

- R52; complete for scoped local assurance. P141 compatibility owns source/primary diagnosis and minimal own-adapter/
  command environment correction plus focused regression and necessary README. P142 security
  independently verifies cause, TLS preservation, command boundaries and relevant evidence.
- Authorize repository/helper-source reads, bounded public primary references, relevant system
  certificate metadata checks (no environment dump/private credentials), disposable HOME/allXDG/
  bus tests and proportionate fmt/Clippy/debug. No full suite, user prefix changes, real runtime/
  installer/game execution, dependency patches, host package/system CA changes or commit/push.
- Acceptance: establish cause versus hypothesis, preserve secure trust selection and explicit user
  CA intent, reproduce corrected command/download behavior safely; report paths/evidence/limits.
  Escalate any required external runtime execution/access or trust-policy tradeoff. Root records only.
- Narrow final change: Winetricks-only mapped canonical host CA default and two inert regressions
  in umu.rs plus README. Six relevant tests/fmt/Clippy/debug/diff pass; manager inspected source,
  reports /tmp/ludomere-p141-report.md and independent -p142-review.md. TLS/custom trust preserved.
  Source-supported runtime-path diagnosis, actual steamrt/DirectX retry unverified. No fullsuite,
  dependency/host/prefix modification or package/commit. No unresolved scoped finding.

## P139–P140 — Dependency endpoint selection

- R51/R49; complete for scoped local assurance. P139 acquisition owns narrow dependencies.rs/shared endpoint fix and
  relevant regression tests, proportionate fmt/Clippy/compile/debug checks. User now reserves the
  full suite for build process; no full cargo test for this narrow correction. P140 independently reviews
  real official endpoint shape and origin/redirect/credential integrity. Root owns records only.
- Authorize bounded anonymous official metadata/primary-source reads, disposable inert payload
  acquisition if necessary for transfer verification (never execution), private HOME/all XDG/bus
  fixtures and source/test edits by owner. Preserve all prior work; no schema/UI redesign, account/
  profile/credentials, host package, commit/push or dependency code modification.
- Acceptance: reproduce exact refusal, verify official response→URL→verified acquisition, meaningful
  regression, required checks and independent no-unresolved-finding report. Escalate new security/
  scope boundaries; return cause/paths/commands/evidence/limits. No live game success claim.
- Final change confined to dependencies.rs URL handling/test and AGENTS.md verification policy.
  Manager reviewed /tmp/ludomere-p139-report.md and independent -p140-review.md, source and logs.
  Nine relevant tests/fmt/Clippy/debug/diff pass; actual production OpenAL acquisition/checksum/
  cache reuse passes. No full suite, package, execution or account access. All scoped findings closed.

## P136–P138 — Catalog-backed GOG dependencies

- R49/R50 following P135; complete for scoped local assurance. P136 acquisition owns catalog resolution, bounded official
  metadata, verified artifact acquisition/cache and minimal depot transfer reuse plus focused tests.
  P137 compatibility owns execution/checkpoints and installation/Resume integration, UI/preflight/
  offline alternative, docs and final checks. P138 security independently reviews and exercises
  changed boundaries/controls. Root records only; preserve all existing uncommitted work.
- Manager reviewed /tmp/ludomere-p136-report.md, -p137-report.md and independent -p138-review.md,
  final logs/source and actual UI screenshots. Exact fmt/Clippy430unit/sixintegration/debug/diff
  checks pass;112 focused installation plus independent eight catalog/six setup cases pass.
  All scoped findings closed; actual review/Cancel/offline/retry/Resume/long-error/closed-result
  controls pass in inert fixtures. No real vendor/runtime/game success claim; P70 gates remain.
  README documents conservative uncertain-process/corrupt-journal recovery limits. No schema,
  dependency-source modification, package, commit or real-user-data operation. Fixtures stopped.
- Agree exact typed plan and file/API ownership before edits. Prefer GTK-free dependency module(s)
  and existing transfer/process/journal patterns; no new external downloader/dependency patches.
  No invented numeric product IDs or unnecessary schema changes. If persistence schema is necessary,
  retain target25 with canonical24→25/development revision policy and full migration verification.
- Acceptance: exact openAL catalog resolves before game transfer; whole dependency list preflight;
  supported EXE/MSI/local-content plans, safe verified cache reuse, success-only per-prefix/revision
  checkpoints, partial Resume/prefix replacement, aggregate errors/explicit offline choice; preserve
  account/generation reset and native behavior. Never claim real title success from inert tests.
- Authorize scoped source/tests/docs edits, primary public metadata/reference reads, disposable
  HOME/allXDG/bus/local HTTP/Xvfb tests and fmt/Clippy/full serial tests/debug. No user profile/logs/
  credentials, real helper/installer/game execution, host changes, package/commit/push or dependency
  code modification. Stop for new scope/security/access decisions. UI owner coordinates final checks;
  reviewer sends findings to owners without fixes. Reports include paths/evidence/limits.

## P135 — Systematic Depot dependency investigation

- R48/R26/R42. Complete (investigation/proposal only). Acquisition audits own dependency/setup architecture and public GOG
  contracts; compatibility researches primary Steam/Lutris/Winetricks/Heroic approaches; security
  independently checks proposed bounds. Read-only product investigation, root records only.
- Acceptance: exact openAL cause, source-backed broader gaps and implementable alternatives with
  tradeoffs, testing and scope. No speculative dependency aliases or success guarantees. No code,
  actual user logs/profile/credentials, runtime/game/helper execution, package, commit or publication.
- Authorize scoped source reads and bounded anonymous primary-source web/metadata research and
  inert /tmp reports. Never print full GOG metadata/client secrets. Escalate necessary new access
  or behavior changes; return sources/findings/limits and proposed next work.
- Manager reviewed /tmp/ludomere-p135-backend.md, -comparison.md and independent
  -security-review.md plus own source/primary documentation. Exact official openAL is absent from
  the mapper; full IDs available before late failure. Recommend typed GOG catalog planning,
  aggregate early preflight, verified cached artifacts, per-prefix/version successful checkpoints
  and explicit offline choice. No product/test/build/runtime work performed. Implementation pending
  user direction; current lookup-table-only approach is not complete Depot prerequisite support.

## P132–P134 — Incomplete-game recovery

- R47 withdrawn by user after locating Logs tab. Only R46 remains; optional downloaded-file deletion
  retains existing unchecked default. No running-log changes or further interview needed on that point.
- Residual policy settled: delete unknown/modified leftovers including in-directory saves after
  explicit path-labelled confirmation, per user answer. Preserve prefixes/external saves and optional
  downloads unless selected; no recovery-folder feature. Healthy idle normal uninstall unchanged.
- R46 (R47 withdrawn). P132–P134 complete for scoped local assurance. P132 acquisition owns cancellation/quiescence and safe managed-file/
  transient-state cleanup backend; P133 compatibility owns discoverable actions, confirmation and
  recovery presentation/docs. P134 security independently reviews destructive boundaries and real UI
  controls. Root records only; preserve all prior uncommitted work.
- First inventory existing reset/uninstall/cancel/cleanup APIs and propose minimal safe contract.
  No deletion of actual user files, broad recursive cleanup, prefix/save reset or schema changes
  without established necessity. Durable prefs/activity/native independence remain invariant.
- Acceptance: available action across active/incomplete/error states, wait for affected writers,
  preserve unrelated files/preferences and truthful partial errors/retry, prevent auto-resume from
  recreating removed files, immediate in-place refresh and existing session/lifecycle safeguards.
  Download-cleanup default remains unchecked; running-log request is withdrawn.
- Authorize scoped source/test/docs and disposable HOME/allXDG/bus/Xvfb/synthetic file+queue
  fixtures. No real profile/prefix/credential or game/helper execution, actual deletion, dependency
  patch, package/commit/push. Stop for new destructive/access/scope ambiguity. Agree backend/UI API
  before overlapping edits. UI owner coordinates final fmt/Clippy/fullserial/debug after source freeze.
- Necessary R46 support approved: minimal bounded, atomic/no-follow recovery identity receipt under
  existing staging retains safe explicit retry after metadata deletion/partial failure/crash. Created
  only after confirmed quiescence/revalidation, no automatic deletion/resume or new schema/queue;
  preserve until requested removal succeeds. P134 independently reviews boundaries and necessity.
- Final evidence reviewed: /tmp/ludomere-p132-report.md, /tmp/ludomere-p133-report.md and independent
  /tmp/ludomere-p134-review.md. Exact fmt/Clippy411 unit/six integration/debug/diff checks pass;
  affected GTK case separately passes. Actual local HTTP drain/unrelated-job preservation and
  physical checked/unchecked cleanup, exact warnings, unsafe-path refusal, partial deletion/receipt
  retry and healthy normal preview/Cancel pass. No unresolved scoped finding. Physical in-progress
  cancellation/account switching and actual vendor execution remain explicit limits. Fixtures stopped;
  no real user files, package, commit or dependency changes. Broader P70 gates remain open.

## P129–P131 — Setup invocation and View Error

- R45/R39/R42/R44. Status complete for scoped local assurance. P129 acquisition owns backend invocation/diagnostics/tests;
  P130 compatibility owns View Error UI and necessary regressions/docs; P131 security independently
  reviews both. Preserve all existing uncommitted changes; root owns records only.
- Acceptance: identify observed installer invocation error from relevant logs/metadata, correct
  confirmed contract defects without skipping required setup, retain action/exit/log context;
  physical View Error click opens readable full failure without unwanted navigation or stale-account
  exposure. Meaningful inert regressions and final fmt/Clippy/full serial tests/debug required.
- Authorize narrow source edits, primary public metadata/reference reads, bounded existing authorized
  Gungeon log inspection and exact inert GOG setup-script metadata inspection if necessary. Private
  HOME/allXDG/bus/Xvfb fixtures only; no real profile/prefix/credential access, installer/game/helper
  execution, dependency patch, package/commit/push. Stop for unapproved execution/access or scope.
  Owners agree file boundaries and report cause/evidence/limits; UI owner coordinates final suite.
- Final corrected source supplies metadata-identified Galaxy wrapper context, repairs old cached
  setup metadata by exact selected repository refresh, retains detailed action/exit/log failures,
  and keeps View Error mapped across timer ticks. Reports /tmp/ludomere-p129-report.md,
  /tmp/ludomere-p130-report.md and independent /tmp/ludomere-p131-review.md reviewed by manager.
  Fmt/Clippy401 unit/six integration/debug/diff pass;95 installation+42GOG focused pass. Physical
  current/recovered held-click/full-copy/account-clear regressions pass; private fixtures stopped.
  No scoped finding remains. Actual Gungeon setup/crash resolution remains user acceptance;
  no prefix manipulation, dependency modification, package or commit.

## P127–P128 — Resumed Depot support executable

- R44/R42/R26. Status complete for scoped local assurance. P127 acquisition owns narrow installation/Depot backend diagnosis,
  confirmed correction and regressions; P128 security independently reviews safe path/setup/resume
  behavior. Preserve current uncommitted logging changes. Root alone edits records.
- Acceptance: trace exact ExecutableMissing origin; safely resolve required setup executable or
  report actionable missing support data without claiming installation complete. Regress actual
  affected flow, including resume after missing files, then fmt/Clippy/full serial tests/debug.
- Authorize source/docs/tests and isolated HOME/allXDG/bus filesystem fixtures, existing relevant
  operation-log authority and primary public references as necessary. No real profile/credentials,
  prefix reads, real game/helper execution, destructive user-file operations, dependency patch,
  package, commit or push. Escalate broader scope/access; return paths, cause, evidence and limits.
- Confirmed exact executable is a support file in official metadata; bare Execute app-only lookup
  corrected to safe exact app-first/support fallback. Manager reviewed source/manifest, owner
  /tmp/ludomere-p127-report.md, independent /tmp/ludomere-p128-review.md and final check logs.
  Eight focused tests, fmt/Clippy397 unit/six integration/debug/diff pass, including manual deletion
  with retained journal then verified rematerialization. No scoped finding remains. Actual Gungeon
  setup/launch remains user acceptance; no GUI/dependency/package changes or executable test needed.

## P124–P126 — Depot diagnostics and game-run logs

- R42/R43. P124 acquisition investigates generic failure source/runtime execution/persisted errors;
  P125 compatibility inventories existing capture/log controls and proposes minimal logging change.
  P126 security independently reviews diagnostics/privacy and eventual implementation. Status
  complete for scoped local assurance. Interview settled: relevant user operation logs may be inspected; live+saved logging
  implementation authorized. P124 owns runtime capture/storage/backend and operation diagnosis;
  P125 owns log UI/README/tests; P126 independent QA/security. Coordinate minimal API before edits.
- Authorize repository/source reads, reports and isolated HOME/allXDG/bus/local inert app fixtures.
  No real profile/logs pending permission, credentials, game/helper execution, external mutation,
  package operation or commits. Preserve clean a99f277 baseline; root alone edits records.
- Return concrete causes vs hypotheses, affected controls, minimal scoped plan and necessary tests.
  User asked whether agents can inspect operations; explain existing runtime capture and limits.
- Updated authority: P124 may inspect only relevant user operation logs with bounded/redacted
  output and safe running executable identity (no environment/credentials); other profile content
  remains excluded. Implement R43 scoped source/tests/docs, isolated inert process fixtures,
  fmt/Clippy/full tests/debug; no real game/helper execution or dependency patches. Preserve old
  logs, account guards, off-GTK I/O and no background windows. No automatic log deletion proposed.
- Confirmed R42 backend correction: skip already-installed requested Winetricks verbs before UMU
  invocation, preserve all missing setup failures/unknown dependency refusal and commit-on-success.
  Own-Ludomere code only; bounded exact-format read and synthetic retry/partial/all/readerror tests.
  Actual user's prefix remains outside inspection authorization. Reviewer examines this boundary.
- Final evidence: /tmp/ludomere-p124-report.md, /tmp/ludomere-p125-report.md and independent
  /tmp/ludomere-p126-review.md. Manager reviewed source, reports, exact-final check logs and GUI
  screenshot. Fmt/Clippy395 unit/six integration/debug/diff pass; new GTK regression separately passes.
  Physical history/live/pause/scroll/Copy/folder-retry/read-retry/account clearing pass. Two explicitly
  authorized inert native launches preserve distinct0600 logs and redact displayed synthetic secrets.
  Three scoped findings corrected and independently verified; no unresolved scoped blocker.
  Private fixtures stopped. Real Gungeon resume/Windows/desktop acceptance remains P70. No package,
  commit or push; user will test with cargo run.

## P123 — Commit error presentation, decimal units and runtime fixes

- User authorizes local commit with a useful message. Compatibility owns exact15-file inventory,
  message and commit; security independently reviews staged scope/message. Status complete.
- Include R39–R41 source/README/project records; preserve earlier committed history and exclude
  generated artifacts/private logs. No new code, package operation, push or PR mutation. Existing
  final390 unit/six integration/fmt/Clippy/debug and scoped GUI evidence remains applicable.
- Acceptance: complete accurate message, reviewed explicit inventory, diff checks and clean local
  commit result. Record user-confirmed Witcher launch without generalizing live acceptance.
- Independent content/message review cleared exactly15 intended files. Local commit succeeded with
  clean status; these closure records are included in that same commit. No push or new product edits.

## P122 — Confirmed MSVC2019_x64 variant

- R41 follow-up. Acquisition implements exact x64 mapping to existing vcrun2019 and extends
  focused mixed-variant/coalescing regressions in manager.rs; security independently reviews.
  Status complete for scoped local assurance. Same P120 isolation/authorization boundaries, no extra runtime families,
  dependency changes or new UI. Required fmt/Clippy/test/debug; actual game remains user acceptance.
- Exact x64 alias/deduplication/coalescing verified; final390 unit/six integration/fmt/Clippy/debug
  pass. Manager reviewed source/reports/logs; independent /tmp/ludomere-p122-review.md PASS.

## P120–P121 — Identified MSVC2019 Depot failure

- R41/R39/R26. P120 acquisition owns narrow dependency mapping and meaningful regressions in
  src/installation/manager.rs, minimal necessary compatibility validation and README if affected.
  Verify primary Winetricks/UMU semantics first; no guessed aliases or unrelated runtime expansion.
  P121 security independently reviews correctness, required-dependency enforcement and resume safety.
  Status complete for scoped local assurance. Prior uncommitted P117–P119 changes preserved.
- Acceptance: MSVC2019 requests supported runtime setup, keeps unknown requirements failing and
  does not mark dependencies installed before success; regression and required fmt/Clippy/test pass.
  Source/fixture assurance only; no real Witcher installation success claim.
- Authorize scoped edits by P120, public primary-source reads, private HOME/allXDG/bus fixtures,
  required checks/debug. No real profile/credentials/game/helper execution, package build/install,
  commit/push/PR mutation or dependency edits. Stop for broader scope or unsafe runtime assumptions.
  Return source references, changed paths, evidence and limits; root owns records.
- Final mapping and tests confined to installation/manager.rs. Manager and P121 reviewed pinned
  Winetricks semantics and source; exact final fmt/Clippy390 unit/six integration/debug checks pass.
  Independent /tmp/ludomere-p121-review.md reports no scoped blocker. Actual runtime/game execution
  remains user acceptance; prior-prefix/partial-runtime retry limitations were not expanded here.

## P117–P119 — Readable installation failures and decimal units

- R39/R40. P117 compatibility owns UI error ingress/no-navigation/readable notifications and
  decimal formatting (domain.rs shared formatter plus UI), focused regressions/README. P118
  acquisition traces backend error detail/sanitization/dependency mapping; investigate the Witcher
  case, fix confirmed narrow propagation defects, propose necessary dependency-support changes
  before implementation if identity/semantics cannot be established. P119 security independently
  reviews and exercises real input/error paths. Status complete for scoped local assurance.
- Acceptance: full sanitized native/Depot/launch failure visible/copyable without page change;
  error-control clicks don't merely redirect to Downloads. Correct decimal boundary/rate displays;
  existing background focus/account invariants retained. Do not suppress unsupported requirements.
- Authorize scoped source/docs/tests, public primary-source investigation, private HOME/allXDG/
  bus/Xvfb/local inert fixtures, required fmt/Clippy/test/debug. No real profile/account/credentials,
  game/helper execution, dependency downloads/installations, package build, commit/push/PR mutation.
  Owners coordinate APIs and return paths/evidence/limits; manager owns records, no product edits.
- Final evidence: /tmp/ludomere-p117-report.md, /tmp/ludomere-p118-report.md and independent
  /tmp/ludomere-p119-review.md. Manager inspected source, final checks and actual GUI screenshots.
  Fmt/Clippy388 unit/six integration/debug and dedicated GTK regression pass. Physical full-copy,
  persisted Depot recovery, no-navigation, account guard and corrected compact/control layout pass;
  two scoped layout findings closed. Exact Witcher dependency remains unknown; mapping unchanged.
  No real game/account/helper execution or new package/commit. Live acceptance remains P70.

## P116 — Upstream pull request

- User explicitly requests a PR against upstream main with a clean Markdown inventory of all
  implemented features/fixes. Compatibility owns fresh remote comparison, PR draft and creation;
  acquisition audits earlier feature inventory; security independently checks completeness/claims.
  Status complete. Root manages review and local records only.
- Target KonoTyran/ludomere main from LegendaryLinux/ludomere current committed branch. Verify
  remotes and existing PRs, do not duplicate or push upstream. Fork push only if required for the
  authorized PR; no force push, merge, code edits, package rebuild or new implementation commit.
- Acceptance: accurate complete user-facing Markdown, validation and honest limits, reviewed
  base/head and published PR URL. Current verified tests remain valid; no unnecessary reruns.
- Published https://github.com/KonoTyran/ludomere/pull/6, open non-draft; main from
  LegendaryLinux:main637c2b1. API readback matches reviewed title/body and branch scope. No push,
  merge, additional comments or new commit; local coordination updates remain outside PR source.

## P115 — Commit package and Comet corrections

- User authorized a local commit with a helpful message. Compatibility owns explicit seven-file
  staging/message/commit; security independently reviews content. Status complete.
- Include R38 source, check runner, README and records; exclude generated helpers/packages/logs.
  No product edits or push. Verify message accuracy, diff checks and clean status after commit;
  existing exact-final386unit/sixintegration/fivePython/package/debug evidence remains applicable.
- Independent review cleared exactly seven intended files and message; local commit succeeded
  with clean status. Coordination closure included in the same commit; no push.

## P112–P114 — Package failure and Comet availability

- R38. P112 compatibility owns build-script/PKGBUILD/documentation diagnosis and necessary fixes;
  P113 acquisition owns Comet discovery/official-release diagnosis and necessary narrow fixes;
  P114 security independently verifies confirmed changes and package/helper contents. Complete for
  scoped local assurance; original uncaptured error not asserted identical to reproduced failure.
- Acceptance: reproduce/classify actual failure, distinguish source-run lookup from helper failure,
  verify authoritative release availability, meaningful regressions and final required checks plus
  corrected local pacman artifact when feasible. No invented live-game/Comet authentication pass.
- Authorize source reads, official anonymous metadata/assets, scoped edits, private HOME/allXDG/
  bus/test fixtures and repository/container package builds necessary to reproduce this reported
  failure. No host package install, real profile/account/credentials/helper/game execution, remote
  mutation, dependency patch/rebuild without confirmed need under R38, or new commit/push.
  Root owns records. Owners report findings/paths/tests/risks; stop for new scope/access boundaries.
- Final package passes fmt/Clippy386unit/sixintegration/fivePython/release; debug builds and actual
  Settings without override reports verified0.3.2. Independent package/source/helper audit passes.
  Reports /tmp/ludomere-p112-report.md, -p113-report.md and -p114-review.md. No dependency build,
  host installation, game execution or new commit. Original stage preserved; private fixtures stopped.

## P111 — Commit accumulated features and fixes

- User explicitly authorized a local commit of current work with all features/fixes in its message.
  Owner compatibility; independent content reviewer security; status complete.
- Scope: accumulated selected-feature R26–R32, contention/refresh R34 and UI R35–R37 changes plus
  README/tests/project records. Explicitly inventory/stage intended sources; exclude build artifacts,
  private fixtures and packages. No new implementation, package operation, push or publication.
- Acceptance: comprehensive accurate message, clean diff checks and reviewed staged inventory,
  successful local commit and reported SHA/status. Existing exact-final tests remain applicable.
- Independent review cleared exactly50 intended files and the comprehensive message. Local commit
  succeeded with a clean working tree; final coordination closure included in the same commit.

## P108 — Sidebar visibility and notification presentation

- R35/R36; owner compatibility, complete; notification interview settled.
- Own src/ui except comet.rs; scoped README and meaningful tests. Reuse current visibility action,
  preserve in-place filters/collections, fix actual right-click input and move Show hidden to Library.
- Use minimal bounded in-memory session history for results/warnings/errors; live progress separate.
  Modal, hover popover,10-second text lifetime, keyboard/
  empty/long-message/burst handling need actual GUI verification. No unrelated UI refactor.
- All P108–P110 operations: source/docs/test edits only for implementation owners; private HOME/all
  XDG/runtime/bus/Xvfb/local inert fixtures for tests; fmt, all-target Clippy -D warnings, cargo test,
  debug build. No real profile/credentials/account/game/helper use, package build/install, commit,
  external mutation or dependency modification. Preserve earlier work. Root owns records.
  Stop for unapproved scope, security or access needs. P108 coordinates final checks.

## P109 — Clear Comet Settings status

- R37; owner acquisition, complete. Own src/ui/comet.rs and its focused tests only; coordinate
  any necessary component API/README changes with manager/P108. No dependency/backend redesign.
- Separate installed Comet state, actionable update/install status and Comet-managed peer state;
  retain explanation and existing confirmed acquisition/cancel flow. Unknown/offline/checking must
  not imply missing installation. Return control inventory, source findings and focused evidence.

## P110 — Independent UI assurance

- R35–R37; owner security, complete for scoped local assurance. Read-only product review; disposable
  instrumented fixtures permitted with exact diff retained and no executor/account authority.
- Exercise physical sidebar context Hide/Unhide, Library filter placement, notification button/
  modal/hover/timer/long history and Comet checking/missing/current/update/failure/cancel states.
  Review account isolation, bounded notifications, markup/error disclosure and background focus.
  Report findings to owners, never quiet fixes. Real service/desktop limits remain explicit.
- Final evidence: /tmp/ludomere-p108-report.md, /tmp/ludomere-p109-report.md and independent
  /tmp/ludomere-p110-review.md; manager reviewed reports, exact-final check logs and screenshots.
  Fmt/Clippy384unit+6integration/debug/diff and6 separate GTK checks pass. Physical contextHide,
  LibraryShowhidden/Unhide, hover/modal/timer/historybounds/signout and inert Comet controls pass.
  Three confirmed scoped defects closed; no unresolved finding. Private fixtures stopped.

## P105 — Database access and truthful completion persistence

- R34 items1/5 and backend support for2–4. Status complete; owner acquisition.
- Own state.rs, download backend, installation.rs and necessary installation backend; coordinate
  APIs with P106 before shared boundary edits. No UI/records. Preserve preceding feature work.
- Current schemas open without writes; retain shape/revision validation and recheck under migration
  lock for concurrent first opens. Genuine writes atomic; no WAL/timeout increase/schema bump.
- Provide scoped/batched inventory and marker APIs needed by UI. Terminal persistence errors remain
  recoverable and visible without falsifying payload state or replaying completed destructive work.
- Evidence: held-writer read-open regression, concurrent initialization/migration/durable preservation,
  terminal bookkeeping failure/recovery tests, focused checks and report.

## P106 — Bounded local-state refresh and completion presentation

- R34 items2–5; status complete, owner compatibility. Own src/ui/, README, focused tests.
- Coalesce progress before database work; affected product/parent/DLC refresh takes priority over
  broad scans; share loaded state across detail/sidebar/filter presentation, limit obsolete workers.
  Preserve page/tab/scroll/focus and last-known state on errors; preview fresh/retryable and no GTK I/O.
- Coordinate minimal backend interfaces with P105. No unrelated architecture/features/dependencies.
- Evidence: synthetic500+ library/progress bursts and selected/unselected install/uninstall/download/
  cleanup updates, stale/account/failure handling, preview retry/Cancel, meaningful final GUI checks.
- P105/P106 authorization: scoped source/tests/docs, private HOME/allXDG/runtime/bus/Xvfb/local
  inert fixtures, fmt/Clippy/test/debug. No real profile/credentials/account mutation/game/helper
  execution, package build/install, commits/pushes. No WAL or public schema change. Root owns records.
  Stop for new scope/access/security boundaries. P106 coordinates final required checks.

## P107 — Independent contention and refresh assurance

- R34; status complete for scoped local assurance, owner security. Read-only product review and disposable fixtures only.
- Verify P105/P106 tests and changed controls, schema safety, atomicity and durable data,
  file-outcome/bookkeeping distinction, no GTK blocking/unsolicited navigation and bounded refresh.
- Return reproducible findings to owners, never quiet fixes. Same private-fixture restrictions as
  P105/P106. Required final fmt, all-target Clippy -D warnings, cargo test and debug for GUI evidence.
- Final evidence: /tmp/ludomere-p105-report.md, /tmp/ludomere-p106-report.md and independent
  /tmp/ludomere-p107-review.md. Manager inspected source, reports, logs and actual GUI screenshots.
  Final fmt/Clippy/379 unit/six integration/debug/diff checks pass; five GTK cases separately pass.
  Held-writer current reads under1ms,2000 progress events zero DB work, selected/unselected terminal
  updates and permission/preview retries independently exercised with505 synthetic products.
  No unresolved scoped findings, WAL unchanged. Live game/account/desktop gates remain P70.

## P104 — Database contention and local-state latency proposal

- R33; complete (investigation/proposal, implementation pending). Manager coordinates read-only audits by acquisition (database) and
  compatibility (UI/event/local refresh). No product/test/configuration edits authorized.
- Acceptance: source-backed causes distinguished from hypotheses, minimal fix proposal, schema/
  concurrency/lifecycle implications and regression plan. Inspect event completion ordering and
  unnecessary whole-library work. Preserve all existing working-tree changes.
- Authorize source/dependency reads, primary documentation and reports/private disposable probes
  under /tmp only. No actual user profile/credentials, game/helper execution, network account use,
  package build/install, commit or external mutation. Escalate scope/access needs. Manager owns records.
- Evidence: /tmp/ludomere-p104-db-report.md and actual production-library private-profile probe
  /tmp/ludomere-p104-db-probe.log. Manager independently inspected both UI refresh/event chains and
  helper call counts. No full GUI latency benchmark or identification of the user's live writer.
  Proposed fixes require a subsequent implementation task; no application changes made here.

Manager: /root. Specification: PROJECT_SPEC.md. Arch Linux x86-64 only. Proton management precedes
package implementation. Workers never edit .project-manager/. No product changes by the manager.

## P100 — Persistence, Depot policies, scheduler and safe retention

- R26–R32 shared persistence; R26/R29/R30 backend. Status complete; owner acquisition.
- Own src/state.rs (sole schema/API writer), src/updates.rs, download backend/cleanup, minimal
  Depot/installation integration, config.rs and lib.rs exports by coordination. No UI/records.
- One local development revision7 under schema25; merge only selected tables/fields and canonical
  migration, preserve intents/user data and reject foreign same-revision shapes actionably.
- Coordinate hidden/tag/achievement/cloud APIs with P101/P102. No upstream social/outbox/saved-view/
  global-queue tables. Migrate the exact former default source order once; preserve customized orders.
- Acceptance: defaults/inheritance, fresh bounded scheduled acquisition, account/reset/busy safety,
  verified replacement and protected Trash with truthful partial reporting. Existing auto-install,
  native behavior and local profile survive. Meaningful migration/queue/cleanup regressions required.

## P101 — Depot/defaults, organization, achievement and policy presentation

- R26–R30/R32 UI; R28 achievement API. Status complete; owner compatibility.
- Own src/ui (coordinate P102 cloud module), src/gog/achievements.rs plus minimal exports, README,
  focused tests. No state/config/backend ownership except agreed interface contracts.
- Acceptance: Depot-first new acquisition with fallback, complete existing operations/language flow;
  hide/tag filters update in place, achievement sections show loading/cache/error/retry, all network/
  filesystem work off GTK and stale-account results rejected. Policy controls inherit accurately.
- Provide hooks/contracts to backend scheduler, cleanup status and P102 cloud UI. Hidden titles stay
  eligible for updates and are excluded from normal browsing/search/collections. No new social UI.

## P102 — Cloud export and selected remote deletion

- R31. Status complete (deletion policy accepted); initial owner grid, final integration acquisition
  (cloud backend) and compatibility (cloud UI/docs). Own src/cloud_saves/ and new src/ui/cloud_management.rs,
  focused tests; coordinate only insertion/export hooks with P101 and persistence with P100.
- Acceptance: use existing supported cloud integration, verified path-safe explicit exports, selected
  deletion with recovery copy and revision confirmation, durable suppression and partial failure
  handling; retain account/reset/activity safety. Only supported installed Windows games are in scope.
  User accepted the disclosed concurrent-update race; confirmation warns to stop games and other
  cloud clients, without claiming a verified server-side atomic compare-and-delete guarantee.
- Export/delete only fixture saves during tests. No real credentials/profile or external mutation.

## P103 — Independent selected-feature assurance

- R26–R32. Status complete for available independent assurance; live acceptance remains P70.
  Independent owner security (no product implementation).
- Inventory/exercise changed controls in private GTK/XDG fixtures; review schema/data preservation,
  network/session boundaries, automatic update/installation/cleanup and cloud deletion integrity.
  Review findings return to owners; no quiet fixes. Manager reviews final evidence and disposition.
- All P100–P103 authorization: scoped source/docs/tests, read-only authoritative upstream reference,
  private HOME/allXDG/runtime/bus/local HTTP fixtures, fmt/Clippy/test/debug and required meaningful
  GUI checks. No actual account, credentials, user profile, helper/game execution, system/package
  install/build, dependency modification, commits/pushes or unrelated cleanup. Report any necessary
  new dependency before adding. Required final cargo fmt --check, clippy --all-targets -D warnings,
  cargo test; one owner coordinates final formatting/checks. Manager alone edits records.
- Final evidence: /tmp/ludomere-p100-report.md, /tmp/ludomere-p101-report.md and independent
  /tmp/ludomere-p103-review.md. Exact-final fmt/Clippy/366 unit/six integration/debug/diff checks pass;
  four display-dependent unit cases and startup separately pass. Actual new controls exercised with
  disposable local services. No unresolved confirmed scoped blocker. Real GOG/game/Comet/Wayland,
  timer soak and exhaustive lifecycle/collection combinations remain explicit limits, not passes.
  No Prototype ready declaration; no new package or commit. Final debug17:39:10 -0400.

## P91 — Download defaults and managed-file cleanup

- Requirements R24; status complete. Backend acquisition, UI compatibility, independent security.
- Scope: existing managed downloads/delete APIs and minimal uninstall wiring; config/chooser default,
  Manage action/confirmation, optional unchecked uninstall cleanup, scoped README and regression tests.
  No real user deletion, schema/dependency/package changes or broader cleanup architecture.
- Acceptance: Extras defaults off for new profiles, retains saved preference; action only for present
  managed downloads; cancel changes nothing;
  delete preserves installed payload/saves/preferences, respects active download/install work and
  symlink/shared-root boundaries; uninstall checkbox correctly honors choice with visible failures.
- Authorization: scoped source/tests/docs and private HOME/allXDG/bus/GUI/local fixtures, required
  checks/debug; no actual profile/account/helper/game use. Escalate new scope/security boundaries.
- Evidence: backend contract, destructive-path sentinel regressions, changed-control GUI inventory,
  final checks and independent QA/security report. Root records only.
- Result: new/missing Extras preference off, existing choices preserved; Manage confirmation and
  unchecked uninstall cleanup implemented. Independent cancel/symlink partial failure/fresh retry
  preserve payload/saves/preferences;5 cleanup and opt-in uninstall regressions pass. Exact-final
  required checks pass332 unit/six integration. Reports /tmp/ludomere-p91-{backend,ui}-report.md and
  /tmp/ludomere-p91-review.md. No scoped blocker; real uninstaller execution remains unverified.

## P92 — Detail-image reliability and failed screenshot layout

- Requirements R25 and R14/R19/R20; status complete. Backend acquisition, UI compatibility;
  independent QA/security security. Own online.rs/backend and UI media/gallery/details respectively.
- Acceptance: diagnose reproducible cause(s), failed/empty screenshots do not overlay information,
  independent images settle/retry sensibly and usable cache survives partial failures; no speculative
  retry flood or full-library refresh. Preserve screenshot keys, lazy loading and stale/session guards.
- Authorization: scoped code/tests and bounded anonymous primary-service diagnostics if needed;
  private HTTP/GUI fixtures, no credentials/private caches. No dependency/package changes.
- Evidence: concrete diagnosis, request counts and failure/cache regressions, real GTK loading/
  failure/partial/retry/empty screenshot layout and independent report. Live original cause remains
  explicitly unverified unless reproduced; no unrelated UI changes.
- Result: empty/failed screenshot layout and bounded validated locked cache fixed; useful Artwork
  survives optional failures with Retry. Independent physical Retry→keys/wrap/Escape and empty/
  partial views pass; two GTK regressions and final checks pass. Intermediate window-only key fix
  rejected after real-input test; final overlay/direct-user-focus fix verified. DLC parser inference
  disproved and unnecessary change removed; exact live Product failure remains unverified. Same reports.

## P90 — Setup pickers, full Proton paths and separate final sign-in

- Requirements: R23, R22. Status: complete. Owner /root/compatibility; independent QA/security
  /root/security. Manager owns records only.
- Scope: src/ui/setup.rs/proton.rs and minimal necessary account integration; focused tests/README.
  Necessary config.rs normalization correction preserves independently chosen download paths on load.
  Exclude backend queue/schema/dependencies/packages/unrelated changes.
- Acceptance: actual folder pickers select/cancel safely; game choice derives downloads child;
  download override cannot change game folder and prefilled values stay intact until user editing;
  full long Proton paths readable by mouse/keyboard selection; only successfully saved setup advances
  to separate optional sign-in modal. Cancel/skip/no-account/known-account states preserve settings.
- Authorization: scoped source/docs/tests, private HOME/allXDG/runtime/bus/Xvfb and inert/local HTTP
  fixtures, required fmt/Clippy/test/debug. No real profile/keyring/account traffic/helper execution,
  package build/install or external mutation. Stop for new scope/security/permission boundary.
- Evidence: source/control inventory, actual picker/path/final-step GUI states, config persistence
  and save-failure behavior, required checks, independent report. No new requirements interview needed.
- Result: both picker select/cancel and one-way path changes pass, full Proton paths readable with
  pointer/keyboard, save failure cannot advance, final optional login follows successful save only.
  Independent restart verifies distinct folders persist and cancelled drafts neither save nor sign in.
  Corrected fmt/Clippy/debug and serial324 unit/six integration pass; initial parallel Comet lock
  failure retained as a test limitation. Reports /tmp/ludomere-p90-ui-report.md and
  /tmp/ludomere-p90-review.md. No scoped blocker; live desktop/account gates remain P70.

## P88 — Targeted image retry and footer/download defaults

- Requirements: R21. Status: complete. Backend owner /root/acquisition; UI owner
  /root/compatibility; independent QA/security /root/security.
- Acceptance: retry issues only failed cover/icon requests, preserving good cache and core catalog;
  notification dismissal persists through ordinary UI refresh but new failures remain visible;
  full re-sync is explicit in Settings, Downloads truly centered and status on right. Install after
  downloading defaults on and works with saved defaults, with no background dialog/focus theft.
- Paths: acquisition online/state and minimal download/install queue backend by agreement;
  compatibility src/ui/config/README/tests. Existing disabled install-after placeholder requires
  lifecycle inspection before implementation; owners report minimal contract before shared edits.
- Authorization: scoped source/docs/tests/debug, disposable HTTP/GUI fixtures and required checks.
  Every test private HOME/XDG/runtime/bus; no real account/profile, payload/helper execution, package
  build/install, dependency changes or unrelated cleanup. Escalate schema/consent/scope conflicts.
- Evidence: request-count retry regressions, dismiss/new-failure and footer pointer GUI, download
  completion lifecycle with inert fixtures, required checks and independent reports. Depends on
  R22 choices only where first-launch/compatibility setup behavior changes.
- Necessary backend support: persist automatic-install intent in SQLite, preferring safe reuse of
  existing operation plans; if a dedicated table is needed, keep target25 and advance only internal
  development revision, update canonical24→25 and verify every retained revision. No new file-backed
  parallel queue. Require idempotent completion/recovery, removal cancellation and visible failures.
- Result: physical Retry requests exactly failed images; Dismiss/new-failure/Settings full refresh/
  centered footer pass GUI. Default-on installation uses durable intent and existing installer queue;
  cancellation/recovery/session protections pass inert regressions. Final fmt/Clippy/323 unit/six
  integration/debug/diff checks pass. Reports: /tmp/ludomere-p88-backend-report.md,
  /tmp/ludomere-p88-ui-report.md and /tmp/ludomere-p88-review.md. No scoped review blocker;
  actual game installation remains user acceptance.

## P89 — Guided defaults and unobtrusive Windows prerequisite handling

- Requirements: R22 and R02–R06. Status: complete; interview complete.
  UI lead /root/compatibility; backend support /root/acquisition; independent /root/security.
- Scope: first-launch defaults flow, existing runtime/default reuse, clear missing-component actions,
  source-run UMU detection/setup documentation. No helper modification/rebuild or package build.
- Acceptance: guide folders, Proton, runtime readiness and optional GOG sign-in; existing profiles
  get one-time guide prefilled with current settings; allow deferral and Finish setup. Ready Windows
  actions proceed using defaults without settings modal. Preserve explicit acquisition consent and
  native independence; startup guide is explicitly requested, unrelated background dialogs excluded.
- Authorization: scoped source/config/docs/tests, local build-artifact inspection and disposable
  fixtures/debug checks. No actual credentials/profile inspected or payload/helper execution.
- Evidence: exact UMU cause, guided-flow/control inventory and persistence/cancel/missing/ready tests,
  ordinary ready install with no compatibility modal, independent security/GUI review.
- Result: debug source run finds staged UMU without CWD/PATH search; ready Windows action proceeds
  without modal, missing selection exposes Finish setup. Fresh/existing guide prefill, optional sign-in
  cancel, invalid-folder refusal, chosen-folder persistence, deferral and one-time relaunch pass GUI.
  Inert real-GTK ready/missing test passes. Reports above and /tmp/ludomere-p89-ready-gtk.log,
  /tmp/ludomere-p89-fresh-evidence.log. Real game/component/desktop gates remain P70.

## P87 — Screenshot keys, parallel icons and usable footer controls

- Requirements: R20, R13–R19. Status: complete. Backend: /root/acquisition;
  UI/tests/docs: /root/compatibility; independent QA/security: /root/security.
- Scope: bounded icon acquisition/persistence alongside covers in online.rs/state.rs; screenshot
  modal keyboard controls, footer structure/retry and sign-in status lifecycle in src/ui/.
  Preserve unrelated edits, details laziness, hero retry and existing account/session guards.
- Acceptance: more than50 sidebar icons arrive before details open; image failures are independent;
  physical pointer Retry starts network retry without navigation; distinct Downloads works;
  real key events navigate modal with existing bounds; completed sign-in text clears.
- Dependencies: owners agree event contract; independent GUI exercises final candidate. No schema,
  dependency, package or broader authentication redesign. README only if behavior needs updating.
- Authorization: scoped edits, isolated local HTTP/private D-Bus/Xvfb fixtures, required fmt/Clippy/
  tests/debug build. Every test must explicitly use private HOME/XDG/runtime roots. No real profile,
  credentials, account traffic, game execution, package build/install or external mutation.
- Evidence: changed-control inventory, targeted failures and keyboard/pointer GUI checks, required
  checks, source/security review and reports; record remaining live-account limitations honestly.
- Result: sidebar icons interleave with covers in four workers, modal keys share wraparound buttons,
  footer controls are independent, sign-in status settles and failure text is sanitized. Final fmt,
  Clippy,313 unit/six integration tests/debug/diff checks pass; separate GTK regression passes.
  Independent physical pointer/key,120-game cold icons/covers and synthetic real exchange lifecycle
  pass with no scoped blocker. Reports: /tmp/ludomere-p87-{backend,ui}-report.md and
  /tmp/ludomere-p87-review.md. Live GOG/desktop/game acceptance remains with P70.

## P86 — Complete image loading and visible synchronization state

- Requirements: R19, R13–R17. Status: complete. Backend owner: /root/acquisition; UI/grid/docs
  owner: /root/compatibility; independent QA/security: /root/security.
- Scope: diagnose fresh-login cover pipeline across batches; narrowly fix queue/event/cache defects;
  meaningful stage/progress and loading/error presentation in grid, details and synchronization.
  Preserve batch size 50, lazy metadata, available Play/Download, profile reset and existing edits.
- Acceptance: fixture with more than 50 games completes all cover attempts; delayed/missing/failed
  images are visibly distinguished; game-list/grid/actual metadata work has truthful stage feedback;
  detail loading appears immediately and settles/retries; sync failures notify without navigation or
  exposing credentials/URLs. Cancellation/stale-account responses cannot corrupt current state.
- Ownership: backend online.rs and necessary gog/state code/tests; UI owner src/ui and README plus
  focused GUI tests. Agree event contracts before editing shared boundaries. No schema/dependency
  changes, packaging, real account/profile access or unrelated auth redesign.
- Authorization: scoped source/tests/docs, disposable local HTTP fixtures/private GUI sessions,
  fmt/Clippy/full tests/debug build. No real credential reads, live game/helper execution or reset.
- Evidence: concrete cause, multi-batch/delayed/failure regressions, control inventory and independent
  fresh-profile GUI review. Report live-account timing as unverified unless user supplies evidence.
- Result: silent per-cover errors, whole-stage persistence abort, poisoned image cache, stale decoder
  results and misleading status corrected. Cold-library presentation and authenticated Refresh panic
  also fixed in the exercised synchronization path. Exact-final fmt/Clippy/312 unit/six integration/
  debug checks pass; separate120-image GTK regression and19 independent online tests pass.
  Independent120-game HTTP→GUI fixture covers fatal batch failure, partial images, repair/retry,
  detail loaders/actions and page preservation, with a final zero-result Collections proof.
  No scoped blocker remains. Reports: /tmp/ludomere-p86-backend-report.md,
  /tmp/ludomere-p86-ui-report.md and /tmp/ludomere-cover-sync-review.md. Live cause remains unverified.

## P84 — Empty-profile startup crash regression

- Requirements: R13, R17; user report of cargo run failure after clearing profile, with backtrace.
- Status: complete. Owner: /root/compatibility; independent verification /root/security.
- Cause: rebuild_library changes the GTK stack while holding Ref<AppModel>; synchronous stack
  notify attempts borrow_mut in window.rs:94 and aborts. Compilation succeeds.
- Scope: minimal src/ui/library.rs/window.rs borrow ordering fix and focused empty-profile GUI
  regression coverage. No unrelated warnings, renderer, dependency or account changes.
- Acceptance: actual fresh-profile startup stays open; the exact failing signal path is covered;
  existing cached startup/navigation intact; fmt/clippy/test/debug build pass; independent review.
- Authorization: scoped source/test edits and isolated Xvfb/disposable XDG verification. No real
  credentials/profile access or deletion, game execution, dependency changes, package installation.
- Evidence: failing-before/passing-after reproduction, changed paths, checks and review disposition.
- Result: exact fresh-profile failure independently reproduced; minimal borrow-order correction
  passes fresh and cached 501-game GUI checks. New GUI regression fails before/passes after;
  300 unit/six integration tests, fmt/Clippy/debug/diff checks pass. Reports:
  /tmp/ludomere-p84-report.md and /tmp/ludomere-empty-startup-review.md.

## P85 — Settings logout/cache control

- Requirement: R18. Status: complete. Backend owner: /root/acquisition. UI/config/docs owner:
  /root/compatibility. Independent reviewer: /root/security; no blocking finding remains.
- Scope: Settings/account control and narrow backend reset logic/tests/documentation. Preserve
  payloads, external installations and unrelated work; no dependency or schema boundary changes.
- Acceptance: off-by-default toggle persists; enabling alone does nothing. User sign-out with
  toggle enabled clears keyring login and full profile (config/preferences/activity/queues,
  replaceable metadata/images/logs), preserves all payloads, safely quiesces or rejects active work,
  prevents stale tasks restoring data, closes app and next launch creates a clean signed-out profile.
  Errors surface clearly; normal sign-out unchanged when toggle off. No symlink/path escape or
  arbitrary recursive deletion; reset uses explicitly owned files/directories and isolated tests.
- Dependencies: P84 startup fix; user answer on reset behavior/data scope; independent review.
- Authorization: scoped backend/UI/config/docs/tests and isolated disposable reset fixtures,
  local build/fmt/Clippy/tests and private GUI verification. No actual user-data reset, real keyring
  credential access, game/helper execution, package rebuild or external mutation by agents.
- Implementation boundary: a fixed private reset manifest, profile lifetime lock and self re-exec
  cleanup are necessary to stop detached writers before keyring/database deletion. Refuse active
  payload operations or protected-path overlap before deletion; pending failures expose Retry/Close
  without starting ordinary profile workers. No generic cleanup CLI or external helper dependency.
  The pre-existing ordinary-sign-out token refresh race is recorded separately; this change must
  prevent credential resurrection on the enabled full-reset path without broad auth redesign.
- Evidence: 308 unit/six integration tests, fmt, warnings-denied Clippy, debug build and separate
  empty-profile GTK regression pass. Seven independent backend regressions and disposable GUI
  default-off, enabled-reset, payload preservation, overlap refusal and recovery Close/relaunch/Retry
  pass. Reports: /tmp/ludomere-p85-backend-report.md, /tmp/ludomere-p85-ui-report.md and
  /tmp/ludomere-profile-reset-review.md. Real account/active payload acceptance stays with P70.

## P80 — Progressive library backend and safe partial persistence

- Requirements: R13–R17. Status: complete. Owner: /root/acquisition.
- Scope: src/online.rs, src/gog/, src/state.rs and backend model definitions as coordinated;
  focused backend tests. Exclude UI, documentation and manager records.
- Acceptance: core batches of 50 without rich expansions; covers before enrichment; targeted,
  independently completing details/filter/acquisition APIs, bounded deduplicated work and cache
  freshness. Preserve ownership/DLC and sparse persistence; respect schema baseline policy.
- Dependencies: agree event/request/cache contracts with P81/P82 immediately.
- Authorization: scoped edits, public primary-source reads and anonymous bounded requests,
  disposable fixture tests/builds; no private account/credentials, host installs or dependency edits.
- Evidence: changed paths, regression tests, request ordering/count evidence, risks and limitations.
- P81 support after backend tests: acquisition owns only install-dialog preparation functions in
  src/ui/download_chooser.rs by explicit region agreement with compatibility; move marker/installer
  inspection off GTK and separate build readiness. Compatibility retains Download chooser ownership.

## P81 — Responsive sync and lazy detail/action presentation

- Requirements: R13–R17. Status: complete. Owner: /root/compatibility.
- Scope: src/ui/sync.rs, details.rs, download_chooser.rs, files.rs, mod.rs, window.rs and minimal
  necessary adjacent detail UI; README behavior/test instructions. Exclude P82-owned library/media
  files, backend and manager records unless ownership explicitly coordinated.
- Acceptance: drain bounded event batches; in-place detail sections with loading/retry; stale result
  guards and current action inputs; immediate usable Play/Download; no GTK blocking work.
- Dependencies: P80 APIs and P82 model/filter integration. Preserve prior Proton/Comet edits.
- Authorization: scoped edits/builds/tests and isolated GUI fixtures, no actual account or game use.
- Evidence: changed-control inventory, tests/checks, cold/warm/failure behavior and remaining gates.

## P82 — Incremental grid, cover workers and filter readiness

- Requirements: R13, R16, R17. Status: complete. Owner: /root/grid.
- Scope: src/ui/library.rs, src/ui/widgets/media.rs and filter controls/collections by agreement.
  Exclude P81-owned sync/details/model files and backend except coordinated contracts.
- Acceptance: incremental cards, bounded off-GTK cover decode with visible priority; affected filter
  loading/error/retry and incomplete-results feedback, available local filters usable.
- Dependencies: P80 metadata readiness and P81 shared model/wiring; coordinate before shared edits.
- Authorization: scoped edits and isolated tests/builds; preserve user data and existing edits.
- Evidence: large-library request/render behavior, interaction inventory, tests and limitations.

## P83 — Independent performance-change assurance

- Requirements: R13–R17. Status: complete (available independent review; live acceptance remains
  with P70). Owner: /root/security (independent of implementation).
- Scope: read-only QA/security review, isolated fixture and GUI checks; no quiet fixes.
- Acceptance: fmt/clippy/test pass; review data preservation, request bounds, stale responses,
  action availability, every changed control's loading/error/success behavior and large-library
  evidence. No authenticated performance or real-game claim without user evidence.
- Dependencies: P80–P82. Authorization: ordinary local checks/disposable fixtures; no credentials,
  host installation, publication or live-account operations. Return findings with reproductions.
- Final evidence: /tmp/ludomere-library-performance-review.md; /tmp/ludomere-p81-report.md;
  target/p81-test.log. Final candidate passes 300 unit/six integration/five UMU tests, fmt, Clippy,
  debug build and diff checks. Independent five section regressions and isolated 501-game GUI
  checks passed; no unresolved blocker in reviewed scope. Authenticated timing/account switching,
  deliberately delayed scroll/tab behavior and real game/DLC installation remain live acceptance.

## P10 — Proton discovery, persistence, and launch integration

- Requirements: R02, R03, R04, R06, R09; repository invariants.
- Status: review (implementation and focused checks passed). Owner: /root/compatibility.
- Scope: src/compatibility backend/types/UMU and new discovery/selection module;
  minimal persistence and installation call sites needed to carry the selected Proton consistently.
  Own src/compatibility/mod.rs exports. Exclude acquisition implementation, UI, packaging, records.
- Acceptance: discover native/Flatpak Steam and libraryfolders plus Heroic/Lutris/managed versions;
  validate/deduplicate, GE > UMU > Valve with newest stable within family; first choice saves global
  default; per-game overrides survive uninstall; missing selection never falls back; selected Proton
  applies to install, launch, patches, winetricks, and Comet registration; native games unaffected.
  Missing runtime is a typed/preflight condition with no implicit download or background dialog.
- Dependencies: coordinate public acquisition/preflight contracts with P20 and UI with P30.
- Evidence: focused fixtures and command-construction/persistence regressions, changed-path report.
- Authorization: scoped edits, primary-source research, local builds/tests, routine dependency reads.
  Stop for schema-policy conflicts, incompatible UMU contracts, or unsafe external-directory writes.
- Implementation decision: use a dedicated atomic proton.json preference file under the existing
  config root, with serialized updates. Existing Config/UI snapshots otherwise overwrite background
  first-default selection. This avoids unrelated settings rewrites and schema changes.

## P20 — Explicit Proton and runtime acquisition

- Requirements: R05, R06; R02 integration; no bundled Proton.
- Status: review (manifest regression and actual GUI acquisitions passed; real-game gate pending).
  Owner: /root/acquisition.
- Scope: new src/compatibility acquisition module(s), focused tests, a minimal private helper adapter
  under resources/helpers/ if needed to enforce consent, necessary Cargo.toml/Cargo.lock
  dependencies. Exclude discovery/config/UMU launch module exports, UI, packaging, records.
- Acceptance: stable historical/current GE and UMU releases from authoritative upstream; explicit
  download operations with progress, cancellation, bounded streaming, provenance/integrity checks,
  safe extraction and atomic publication; no deletion of external versions. Runtime acquisition is
  separate and explicit; normal launch cannot silently download or update missing components.
- Dependencies: agree APIs with P10 and P30 before implementation; no UI blocking or dialog work.
- Evidence: primary source/version contracts, local failure/security tests, acquisition report.
- Authorization: authoritative upstream metadata/download research, scoped reversible fetches,
  necessary constrained libraries after provenance/license review, tests in disposable directories.
  No unreviewed helper execution with broad access. Stop for unsupported upstream contracts or
  supply-chain/security tradeoffs; notify manager of new native dependencies.

## P30 — Proton settings and user-action prerequisite flow

- Requirements: R03, R04, R05, R06, R09.
- Status: review (implementation/check/clippy passed). Owner: /root/ui.
- Scope: src/ui/ including new compatibility settings module. Exclude backend/packaging/records.
- Acceptance: global selector with persisted first/default choice, per-game override and inherit
  action, refresh and folder choice, stable release catalog and explicit acquisition progress/errors/
  cancellation. Missing chosen version asks for replacement. Missing Proton/runtime offers happen
  on user action, with no background focus theft; all blocking work is off GTK. Native unaffected.
- Dependencies: P10/P20 APIs. Initial work may inspect UI and agree interfaces; integrate once ready.
- Evidence: full inventory of changed controls and states, compile/static checks, testable preflight
  behavior, manual verification instructions for reviewer/user.
- Authorization: scoped UI edits, local compilation/testing, established GTK/libadwaita patterns.
  Stop for extra features, backend ownership conflicts, credential access, or unsafe UX behavior.

## P40 — Arch packaging, bundled helpers, docs, local/CI checks

- Requirements: R01, R02, R07, R08, R09.
- Status: complete (final package/checks and independent exact-byte/source audit passed).
  Owner: /root/compatibility.
- Scope: PKGBUILD, helper preparation/build/check scripts, container definition, GitHub Actions,
  README, THIRD_PARTY_NOTICES, minimal bundled-helper lookup in Comet/UMU as coordinated.
- Acceptance: reproducible documented Arch environment; fmt/clippy/test; build pacman artifact with
  bundled UMU and Comet/helper, licenses, no Proton/runtime payload; no host UMU dependency; verify
  package contents and helper resolution. CI uploads package artifacts, does not publish releases.
- Dependencies: P10–P30 implementation checkpoint.
- Evidence: clean/container checks where available, release/package build, payload/license audit,
  exact commands and limitations. No privileged host installs.
- Authorization: build-time downloads explicitly allowed; pinned trustworthy sources, isolated
  package build/test. Escalate system changes or resource/security constraints.

## P45 — Determine GOG peer redistribution terms

- Requirements: R07 and 2026-09-29 user clarification preferring bundled peers.
- Status: complete (manager reviewed current terms and all eight artifact inventories). Owners: /root/security (official terms), /root/compatibility (artifact notices).
- Scope: authoritative GOG SDK/developer/user terms, public peer metadata/artifact license evidence,
  relevant upstream integration context. Research only; no product or project-record edits by workers.
- Acceptance: identify an applicable redistribution grant and conditions, or exact missing grant/
  restriction with primary-source evidence and a concrete path to resolve it. Distinguish use,
  public download, game-developer redistribution, and third-party launcher redistribution.
- Authorization: public primary-source reads and bounded official artifact downloads/inspection
  in disposable paths; no binaries executed, credentials, terms accepted, accounts, or messages sent.
- Stop for gated terms, account/credential requirements, or authority outside the documented scope.
- Evidence: URLs, short relevant clauses/sections, applicability/uncertainty, artifact provenance,
  report with findings and unresolved questions. Preserve other working-tree changes.

## P46 — Component availability checks using upstream metadata

- Requirements: R11, R12; R07 integration.
- Status: complete (metadata tests and independent startup/manual GUI checks passed).
  Owner: /root/acquisition.
- Scope: GTK-free component metadata checks and compatibility exports; remove superseded custom
  peer downloader/storage/tests and unused ZIP dependencies. Exclude Comet launch/package/records.
- Acceptance: official HTTPS bounded metadata checks; compare available Comet/peer versions with
  local version evidence without triggering payload downloads. Read upstream peer metadata from the
  agreed Comet data location. No claim of Ludomere-managed peer verification, rollback or consent.
  Coordinate status APIs with P47/P48; preserve Proton/runtime acquisition.
- Authorization: scoped source/tests, vetted dependencies after reporting provenance, official
  fetches and isolated tests. No peer execution, credentials, global install or publishing.
- Evidence: metadata/version/error tests and authoritative upstream format validation.

## P47 — Unmodified Comet packaging and confirmed official-binary updates

- Requirements: R07, R11, R12; closes SEC-02.
- Status: complete (official updater GUI, package checks and independent audit passed).
  Owner: /root/compatibility.
- Scope: official pinned helper preparation, comet.rs/comet_update.rs, package/build/check scripts,
  README/notices; remove local Comet patch/source-build/custom-feed tools and superseded artifacts.
  Exclude peer metadata implementation/UI/records. Coordinate data-root/status APIs with P46.
- Acceptance: package official unmodified Comet/service; do not modify/recompile dependency code.
  Allow upstream automatic peer acquisition, remove custom peer-readiness guards, retain credential
  suppression. Confirmed updates use official release assets with publisher integrity metadata,
  bounded transfer, atomic private selection, retry and working-version preservation; a newer package
  supersedes an older private helper. No proprietary peers/Proton/runtime in package. Rebuild/check
  Arch artifact and update docs, preserving system package ownership. No custom update feed needed.
- Authorization: vetted pinned source/build dependencies, scoped builds and disposable smoke tests.
  No real credentials/game or peer execution, host install, publishing. Report new supply-chain
  dependencies/risks and incompatible upstream assumptions promptly.

## P48 — Upstream behavior explanation and component update controls

- Requirements: R11, R12, R09 UI invariants.
- Status: complete (compile/static/focused and final packaged GUI checks passed).
  Owner: /root/acquisition; UI thread unavailable.
- Scope: src/ui/ only, minimal app-start wiring if necessary by agreement; no backend/package/records.
- Acceptance: explain upstream automatic peer downloads/updates; remove custom peer consent gates,
  payload controls and launch interception while preserving Proton behavior. Startup checks run off
  GTK, render status in place, never present/navigate; manual check button reports loading/current/
  newer/failure states. Newer Comet binary updates require confirmation, progress/cancel/retry and
  app-owned storage. Peer updates remain Comet-managed with accurate UI copy.
- Authorization: scoped UI edits, isolated compile/UI checks; no host desktop/credentials/game use.
- Evidence: complete changed-control inventory, testable flows, worker-thread and no-focus review.

## P50 — Independent QA

- Requirements: all R01–R12 and skill QA gate.
- Status: review (automated/package/credential-free GUI checks passed; real-game/desktop gates pending).
  Owner: /root/qa for prior report; /root/security for independent P46–P48 extension because the QA
  thread is unavailable. The extension reviewer implemented none of the reviewed product code.
- Scope: read-only review plus disposable test artifacts; all changed behavior and relevant existing
  interactions. Inventory every user-visible control/interaction and record exercised/unverified states.
- Acceptance: reconcile requirements, run repository checks, assess package from representative clean
  environment, exercise automatic/missing/download/native paths and supported graphical behavior.
- Dependencies: P10–P40; R10 user participation for authenticated real-game testing.
- Evidence: reproducible findings, commands, coverage/control inventory, explicit blockers.
- Authorization: safe local testing only; no product fixes or real credentials without authorization.

## P60 — Independent security review

- Requirements: all applicable R01–R12 and skill security gate.
- Status: complete (no unresolved blocker in reviewed official-binary delivery).
  Owner: /root/security; evidence /tmp/ludomere-final-assurance-report.md.
- Scope: read-only review of archive/network/process/path/persistence/bundled dependency/CI trust.
- Acceptance: no unresolved material finding; inspect provenance, downloads, host/external path
  boundaries, subprocess environments, credentials, progress/error redaction, and CI permissions.
- Dependencies: P10–P40. Confirmed fixes go back to workers and return for re-review.
- Evidence: severity/impact/reproduction/remediation report; no quiet fixes.
- Authorization: primary-source research, safe local probes, no external active attack or secrets.

## P70 — Guided user acceptance and readiness gate

- Requirements: R10 and all skill readiness gates.
- Status: ready (final package and README checklist available; awaiting user-assisted results).
  Owner: manager coordinates user and independent QA.
- Acceptance: user verifies real GOG install/launch, chosen/default/override Proton, Comet behavior
  for a suitable game, package desktop operation, and no regressions in native Linux behavior.
- Dependencies: P50/P60 results and reviewable artifacts/checklist.
- Evidence: actual user results reconciled by QA; unexercised gates never count as passes.
- No Prototype ready declaration until all required gates have evidence.
