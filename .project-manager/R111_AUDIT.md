# Interactive audit, 2026-10-06

Window: 08:13:55–20:13:55 UTC. Branch: `improvement/interactive-ux-audit-2026-10-06`.
Main fast-forwarded to `5c0d5cb`; no upstream merge, push or new PR.

## Closeout

- Completed82 separate local change commits through`71ecd7b`, each with an exact post-commit
  `cargo build --locked` pass. Final full build and affected native-pointer/GTK/real-app gates
  pass as detailed below. This records-only closeout is separate from those82 changes.
- Final source verified at20:01:44 UTC, within the authorized12-hour window. All actual and
  synthetic app windows are closed; no download, installation or game remains active from this
  audit. No publication, version bump, schema change or package build/install was performed.
- Main outcomes: wallet discovery and authenticated Depot reuse; native Linux/preference and
  prerequisite handling; responsive file/Storage/cloud/tray work; consistent search, keyboard,
  compact layout, cancellation, diagnostics and download completion feedback. Git history and
  the task entries retain individual changes and their failed-before/corrected-after evidence.
- Real test games were Gungeon, Coffee Talk and BIT.TRIP Runner. Conservative reservation16GB
  of40GB includes repeated transfers/cache/staging; it is not measured new disk consumption.
  Gungeon's three archives are restored (900.7MB total including its extra). Coffee's Audit Test
  save remains preserved and normally synchronized; no deliberate cloud-save deletion occurred.
- Remaining user-assisted checks: cold-start/locked-wallet interoperability, cloud export through
  the native destination picker, applying an actually available game update, and fuller gameplay/
  controller/accessibility coverage. Normal Coffee cloud sync passed; export did not start and
  its picker was canceled through normal app close. No exhaustive prototype-readiness claim.
- Deferred technical findings remain in PROJECT_STATUS and the source reports: complete stale
  Properties identity, malformed preference recovery, broader native popup/Cloud-page retention,
  and unmeasured secondary performance proposals. P411 corrects the reproduced repair warning;
  it does not establish the stripped native stack's exact internal cause.
- Final source gate: `/tmp/ludomere-r111-full-build-pd7d9sna/build.log`.
  Exact P411 commit build: `/tmp/ludomere-r111-p411-postbuild.log`.
  Records-only closeout follows the same post-commit build procedure, with its output retained
  at `/tmp/ludomere-r111-closeout-postbuild.log`.

## Consolidated verification checkpoint

- Final P411 source fresh consolidated build PASS: formatting, all-target Clippy-Dwarnings,
  574 library tests, six integration tests, five Python helper tests and cargo build.95 library
  entries ignored by default; affected GTK gates separately executed. Evidence:
  /tmp/ludomere-r111-full-build-pd7d9sna/build.log. Project artifacts rebuilt after clearing the
  shared-checkout cache; private profile/process/network environment excludes actual data.

- At927556d consolidated private build PASS: fmt, all-target Clippy-Dwarnings,574 library tests,
  six integration tests, five Python helper tests and build.95 library entries ignored by default;
  affected GTK gates separately run.80 exact post-commit builds PASS. Evidence
  /tmp/ludomere-r111-full-build-_ihxotmp/build.log. Actual profile/helpers/network excluded.

- At26e8c50 consolidated private build PASS: fmt, all-target Clippy-Dwarnings,574 library tests,
  six integration tests, five Python helper tests and build.95 library entries ignored by default;
  affected GTK gates separately run.77 exact post-commit builds PASS. Evidence
  /tmp/ludomere-r111-full-build-mppzqivn/build.log. Actual profile/helpers/network excluded.

- At37aa551 consolidated private build PASS: fmt, all-target Clippy-Dwarnings,574 library
  tests, six integration tests, five Python helper tests and build.93 ignored library entries;
  affected GTK gates run separately.72 exact post-commit builds PASS. Evidence
  /tmp/ludomere-r111-full-build-9gwc2kj3/build.log. Actual profile/helpers/network excluded.

- At54345f2 consolidated private build PASS: fmt, all-target Clippy-Dwarnings,574 library
  tests, six integration tests, five Python helper tests and build.92 library entries ignored
  by default; affected UI gates run separately.68 exact post-commit builds PASS. Evidence
  /tmp/ludomere-r111-full-build-uv_o7udz/build.log. Actual profile/helpers/network excluded.

- At99eeb3d consolidated private build PASS: fmt, all-target Clippy with warnings denied,
  574 library tests, six integration tests, five Python helper tests and build.86 library tests
  ignored by default; affected GTK tests separately passed.58 exact post-commit builds PASS.
  Log /tmp/ludomere-r111-full-build-ufr3s228/build.log. No actual profile/helpers/network exposed.

- Ata3641c1 consolidated private build gate PASS: formatting, all-target Clippy, default Rust
  tests, five Python helper tests and build. Log /tmp/ludomere-r111-full-build-vcdb940n/build.log.
  All47 commits have exact post-commit build PASS; affected ignored GTK tests separately passed.

- At5a05fe8 consolidated private gate PASS: formatting, all-target Clippy with warnings denied,
  571 library tests, six integration tests, five Python helper tests and build.77 ignored library
  entries excluded from default suite; affected GTK fixtures run separately. Log:
  /tmp/ludomere-r111-full-build-ebwyytx7/build.log. All43 commits have exact post-commit build PASS.

- Atb3f9227 the next private full build gate PASS: formatting, all-target Clippy with warnings
  denied,570 library tests, six integration tests, five Python helper tests and build.69 ignored
  library entries excluded by default; changed GTK paths independently exercised. Log:
  /tmp/ludomere-r111-full-build-8kbq3zr4/build.log. All34 commits have exact post-commit build PASS.

- At6b7328d the private build gate passes cargo fmt, all-target Clippy with warnings denied,
 566 library tests, six integration tests, five Python helper tests and cargo build --locked.
 57 ignored library entries are excluded by default; changed GTK paths were exercised separately.
 Log: /tmp/ludomere-r111-full-build-odjoz3gv/build.log. No exhaustive UI coverage claim.
- Two earlier harness failures retained: symlink helper blocker destination, then absent synthetic
 UID records preventing private D-Bus authentication. Wrapper now resolves/deduplicates blocked
 helpers and mounts only generated current-UID passwd/group/files-only NSS records. No real account
 database, wallet, desktop or external networking exposed; product tests were not weakened.
- Live P343/P345/P346 retest passes: healthy uninstall hides Retry/collapses alternative recovery,
 retains warnings/archive-default, restores blue Download/arrow in place, and source switching in
 the same chooser shows full paths while hiding Depot feedback for offline selections.

## Live testing

- P411 final focus-order regression p370a-iiba2nqc PASS (initial Cancel, forward Offline/Confirm,
  backward Offline/Cancel, all retained layout/cancellation assertions). Freshly compiled final
  native-pointer diagnostic p411-hk_w2z0i PASS: no target ancestry critical, live consent focus
  retained, all inert/backend/lifecycle gates unchanged. One cached cross-checkout compilation
  unexpectedly reused the shared target; no runtime gate was accepted from it. Cleaned only
  Cargo's Ludomere package artifacts and rebuilt diagnostic freshly, then cleaned that package
  again for the final main consolidated build. Dependencies and real user/game files untouched.
  Independent final QA/safety source GO. Only four production lines and the existing fixture
  changes enter main; all temporary diagnostics stay outside the repository.

- P411 restoration preview focuses the new Cancel action after replacing content. The first
  existing fixture failed because it presented a fresh dialog after construction, unlike all
  three production callers. Independently reviewed fixture correction presents inert preparation
  content first; strict Cancel/layout checks then pass p370a-eyompo1g. Actual Coffee review has
  zero Gtk-/GLib-GIO-CRITICAL or panics; Tab→Shift+Tab→Space cancels and returns Play. No setup
  admitted, save/prefix/payload unchanged. Actual app closed normally. Final traversal/build
  gates pending; diagnostic remains outside main.

- P411 one-line main preview passes fmt/compile/build and retained private consent regression
  p370a-jlp0n0wr. Actual Coffee repair preparation now emits no Gtk critical; Escape closes
  review without component execution and restores Play. Tab first selects component text but
  later focus visibility is inconclusive. The underlying tab highlight after Escape is not
  proof of focus leakage: the pointer remains over that tab at the former Start coordinates.
  Do not count full keyboard acceptance yet; candidate remains uncommitted pending narrow review.

- P411 bounded default-forwarding stack observer independently reviewed and compiled. Native-
  pointer run p411-cqz2a1hq again passes assertions but reports the critical. Stack places it
  in deferred native event processing during consent observation, not directly inside Rust's
  set_child call; native frames are unsymbolized. One isolated focus-clear experiment is under
  review; no product remedy or precise toolkit attribution follows from this partial stack.

- P411 repeat p411-8wy0b4uh again completes all inert/native-pointer assertions with the same
  content-replacement warning. Reviewed argument-free debugger attachment to that synthetic PID
  was refused by the OS (ptrace Operation not permitted); no stack obtained, no security setting
  changed and no real process inspected. A bounded test-only default-forwarding log observer is
  being assessed instead. Actual app remains closed.

- P411 native-pointer reproduction succeeds diagnostically: p411-441xwxv0 reaches actual
  Gear/Manage/Repair/Start/components/Cancel; all Rust assertions pass, wrapper intentionally
  exits2 for exact gtk_widget_is_ancestor critical immediately after preparation release.
  No profile/database/queue/backend mutation. Previous observer-only p411-dgtqtpg8 established
  nested leave before enter with containment false; independently approved native-enter count
  preserves actual-input proof and all other gates. No product fix or internal cause claimed yet.

- P411 diagnostic compiles. Two unchanged native-pointer runs reach the Manage submenu but
  fail its diagnostic Legacy hover counter (second22.20s, not the45s timeout); no target GTK
  ancestry critical yet. Approved diagnostic-only correction observes the single existing
  Motion controller's enter/contains-pointer, matching actual production hover dispatch.
  Other ButtonPress proofs and backend guards remain strict; no product fix inferred.

- P411 frozen inert X11 diagnostic received independent UI and safety source GO. Root-reviewed
  persistent inspection/preparation interception, Confirm/Offline refusal, disabled unrelated
  actions and private bus/profile wrapper. Compilation/runtime pending; this remains outside
  main and is neither a product fix nor proof of a toolkit cause.

- P412 commit655f5fe exact post-build PASS;81 built audit commits. Coffee Export invoked only
  its native folder picker; it is unavailable through current X11 automation. No destination
  selected/export started. External Request.Close correctly refused unmatched caller; normal
  app tray Close succeeded and its portal request subtree is now empty. Temporary chosen-to-be
  export directory remains empty. No save/cloud deletion, no access-control bypass; export
  live gate remains unverified. Actual app closed cleanly before synthetic X11 diagnostic.

- P412 source/fmt/build PASS: recovery Retry now reads Preparing reset… while its existing
  validation/scheduling worker runs; error/disconnect restore Retry reset before re-enabling.
  Exact three-label-setter independent review; no reset/credential or policy change/test run.
  Closing source assessment found no introduced blocker within reviewed P401–P410; cfg(test)
  accounts for about75% of net Rust growth. P408 steady completion check adds O(K×J) borrowed
  ID comparisons, no measured stall; typicalK1 and existing scans noted, no speculative refactor.

- Current927556d live P375 repeats: nested Coffee Manage→Repair shows corrected confirmation
  with no warning; Start→required-components emits one gtk_widget_is_ancestor invalid-widget
  critical at15:01:31 local. Consent and Cancel work; canceled before component execution, game/
  prefix/save untouched. Screenshots repair-late-*; only exact safe GTK line examined. P411
  authorizes isolated current-source real-pointer diagnostic proposal with fail-closed backend
  seams; no product fix/debugger or toolkit attribution. P410 exact post-build PASS,80 commits.

- P409 commit17939d1 exact post-build PASS;79 built audit commits. Final Gungeon archive
  restoration returns3/3 and900.7MB; app-download-feedback-preview.log contains zero Gtk-CRITICAL,
  GLib-GIO-CRITICAL or panic markers. P410 one-literal Proton Cancel independently reviewed,
  fmt/build PASS; live p410-proton-loading verifies wording during metadata lookup. No runtime
  download/selection change, no extra test framework for this reversible text correction.

- P408 live PASS: observed Windows archive downloading/completion, then deleted that exact
  one file without leaving detail. DOWNLOAD COMPLETE disappeared in place; native Play,
  Linux/macOS archives and remaining counts retained. Screenshots p408-live-completed/cleared.
  P409 live PASS p409-live-idle-history: explicit idle/history paragraph visible above graph,
  current network/disk0, historical peak retained, queue/completed controls reachable. Windows
  archive restoration pending; no other game files touched. No target GTK critical observed.

- P408 commit4eba7fb exact post-build PASS;78 built audit commits. P409 retained actual renderer
  gate p378-_o_stb4z PASS19.04s, normal/enlarged text, history variants and positive-history
  paused/failed/queued precedence; known private GSettings warnings only. Root reserves another
  1GB (total16GB/40) for exact Gungeon archive deletion/reacquisition state and historical-graph
  visual gates. No new game or installer execution; native payload/other archives remain protected.

- P408 fixed-production identical actual-panel gate p408-gimihv_q PASS3.82s: tracked deletion,
  unrelated retention/removal, partial Paused, multi-job batch, subsequent batch and no-managed
  invalidation retain native Play and widget identity. Known private GSettings warnings only.
  Independent implementation GO, fmt/compile/all-target Clippy PASS; actual repeat pending.

- Live26e8c50 native Gungeon manual Check for Updates returns explicit no-update/keep-playing
  result; no update applied. Downloads actions remain reachable at enforced1150x720 minimum;
  restored1330x850 afterward. Proton Refresh retains configured GE-Proton11-7, runtime reports
  found with disabled download action; Load releases visibly loads then offers its list without
  downloading/selecting a different runtime. Loading used misleading Cancel download copy,
  delegated a separate literal-only proposal. No preference change or new game data.

- P408 exact new actual-panel fixture on unchanged production fails at expected stale visible
  completion after tracked job deletion (p408-e7zxpkbe, details.rs4534); unrelated job and
  downloaded-product membership retained. Same fixture now being gated with reviewed ID fix.
  P409 paragraph/renderer test independently reviewed GO; runtime pending. Synchronous logo
  decode assessment is deferred: normal cached logos already bounded360x125, no measured stall,
  safe async reuse requires separate contain-mode queue design. No rushed card-crop substitution.

- P407 actual preview PASS: Coffee Cloud basic enable/status/locations/sync controls appear
  first; scrolling reaches unchanged Export/Manage warnings and actions. Native Gungeon retains
  managed-UMU-only explanation. No cloud action/settings change. Screenshots p407-cloud-basic-first,
  p407-cloud-management-below and p407-native-cloud. Focused loader/fmt/build PASS; source reviewed.
  Gungeon cancellation restoration completed3/3 archives900.7MB; native Play remains available.

- On0bf8c27 Gungeon Windows archive deletion restores a single Download/count2 in place,
  preserving native Play and Linux/macOS archives. First reacquisition finished before Pause;
  the subsequent primary click therefore launched the native game, which was normally exited.
  Second transfer was promptly paused at8.7MB; Downloads trash opened confirmation on its
  first click and confirmed cancellation removed that queue entry. Earlier P358 first-click
  failure did not reproduce; no causal fix claimed. Restoration transfer is now running.
  Total reservation remains15GB. Screenshots gungeon-*-retest/timed/cancel under evidence root.
  Deletion leaves a stale DOWNLOAD COMPLETE status while unrelated archives remain; delegated
  bounded source investigation before implementation. No cloud deletion or installer execution.

- P407 Cloud group-order patch independently source-reviewed; existing full loader/hidden-tab
  focus regression p369-_nplom9x PASS2.63s, known private GSettings warnings only. Native/basic
  and supported management behavior otherwise unchanged; root actual visual gate pending.
  Bounded disconnected-worker review found no additional concrete busy-forever defect in
  reviewed file/log/settings receivers; scope limits retained in busy-worker-assurance report.

- On0bf8c27 Coffee Talk launches through configured GE-Proton11-7, sidebar turns green and
  primary becomes Stop. Root created an explicitly named Audit Test profile in an empty game
  slot and reached the opening scene. This newly creates test save data (unlike earlier title-only
  runs); normal sync is authorized. Logs show the saved run; Follow off/Refresh gives paused
  guidance, Follow restored. Normal game windowquit triggers visible Uploading cloud saves then
  Play/white name and playtime12min in place, preserving coffee search. Properties records last
  successful sync18:29 UTC; read-only inventory reports one remote save/28.0KB. No forced sync,
  cloud deletion or export. Named coffee-*-audit.png evidence; app still running for next test.
  P406 commit0bf8c27 exact post-build PASS,76 separately built audit commits.

- Next live transfer gate reserves an additional1GB within the allowed40GB (total conservative
  reservation15GB), for deleting/reacquiring only Gungeon's test Windows installer and exercising
  paused-transfer cancellation. Native installed payload and Linux/macOS archives stay intact.
  Root alone owns actual actions; no deliberate cloud/save deletion or new game introduction.

- P406 one-literal log status correction independently reviewed; forced Refresh no longer
  promises live following when Follow is off. Existing actual-reader/folder/Follow/Refresh
  regression p399-1jkhg1gq PASS; fmt/compile PASS, known private schema warnings only. No reader
  policy or test assertions changed. P403 commit0c6152e exact post-build PASS,75 built commits.

- P403 corrected exact gate PASS p403-3132_dmv with no GTK warnings: frozen admission creates
  no DB, activity exists before worker entry and through result send, duplicate ticks do not
  dispatch, raw/session/owner/logout retirement rejects work/results, mutable-model notify
  reentry retires without next dispatch. Existing P338 pure matching/loader PASS
  p338-y55d9lnk/sg17spnr; actual presentation PASS p338-x0n0cpvx (known schema warnings only).
  Independent final/correction reviews and fmt/compile PASS. Initial fixture failure retained;
  no production policy or original strict assertion changed to obtain PASS.
  P405 commitdbdf61a exact post-build PASS,74 built audit commits.

- P405 actual full-path row passes950px,680px and enforced561px minimum allocation; at561px
  path wraps across two lines with no ellipsis and Browse remains accessible. Focused Ctrl+A
  selects the complete displayed path; clipboard untouched. Screenshots p405-path-{normal,
  narrow,wrapped,selected}.png. Native word-char wrapping/source independently reviewed, fmt/
  build PASS. No actual path containing markup was created. Test app normal Close, no panic or
  Gtk-/GLib-GIO-CRITICAL in app-installed-path-preview.log. P404 menu→Properties also worked live.

- P403 first private run p403-6w0np3zf fails fixture epoch==1 at6060. Confirmed fixture had no
  usable library, causing real renderer to write count then missing-library guidance (two
  notifications). Minimal synthetic compatible-library fixture correction under independent
  review; strict assertions unchanged, production patch removed from main pending rerun.
  P404 commit9acdda6 exact post-build and retained P385 popup test p385-fh4xdgnt PASS.

- P404 actual unopened-builder negative/positive gate proves explicit Rust capture-cycle
  removal: old code fails strict weak-release at files.rs7363 in p404-ixo6aymj; identical
  fixture passes with three weak-capture blocks in p404-pbsnm7lk. Retained harmless actions/
  controller still work after release. Mapped action/filter/submenu close-order test passes
  p404-c5c_3fnl. Independent final review/fmt/compile/Clippy PASS; only known private GSettings
  schema warnings. P393 mapped native retention remains deferred, not solved or relabeled.

- Read-only actual Gungeon Properties Installation/Installed Files/Updates/Compatibility pages
  inspected; source/version/native explanation and automatic-save guidance render. No policies,
  runtime or files changed. Installation directory is heavily ellipsized; targeted UI/source
  proposal requested. Named app-only screenshots properties-*-audit.png, app-properties-pages-audit.log
  has no panic/Gtk-/GLib-GIO-CRITICAL; application exited normally.

- Documentation assessment finds README/runtime/PKGBUILD consistently use bundled UMU1.4.4
  and Cometv0.3.2; current user-supplied AGENTS standalone /usr/bin/umu-run sentence diverges.
  No dependency/runtime/package or user-instruction change made. Keep this discrepancy for
  user decision; no automatic host fallback or architecture migration added during audit.

- P393 native-only diagnostic p393-rqkua9mf: never-mapped ownership transfer releases its
  popover; both filtered-anchor and ordinary visible-anchor mapped dismissal retain one native
  reference after close/unparent. This reproduces retention without the application builder,
  narrowing it beyond Hide-specific behavior; it does not establish a toolkit cause or fix.
  Temporary diagnostic removed, original strict acceptance still unmet. Candidate stays out.
  P402 commit37aa551 exact post-build PASS,72 built audit commits.

- P402 live one-file summary and confirmation PASS (p402-one-file-summary.png and
  p402-confirmation-properties.png); three-file category remains plural. Confirmation canceled,
  all archives preserved, test app normally closed. No panic/Gtk-/GLib-GIO-CRITICAL in
  app-archive-count-preview.log. Independent two-expression review/fmt/build PASS.

- P400 review correction: chooser polling was already worker-side at baseline5c0d5cb and
  current5764b79. Earlier claim of GTK SQLite polling was incorrect; assurance report amended.
  Actual worker profile-lifetime handling is a separate proposal under investigation.
  P401 commit5764b79 exact post-build PASS,71 built audit commits.

- P401 live General-first PASS: new Gungeon Properties opens General, Cloud remains accessible,
  invoking Properties again preserves Cloud in the same existing window. Screenshots
  p401-general-default.png and p401-existing-cloud-retained.png; app-properties-default-preview.log
  has no panic/Gtk-CRITICAL/GLib-GIO-CRITICAL. Source review/fmt/build PASS; no settings changed.

- P393 staged-owner diagnostic p393-uau89w2h observes unchanged retention after dropping hover,
  Refresh and Hide owners separately. Diagnostic exit0 is not original acceptance PASS; original
  strict lifecycle assertion remains unmet. Temporary diagnostics and candidate removed again.
  Independently reviewed native three-case baseline authorized, without application/backend code.
  P399 commit624bf6a exact post-build PASS,70 built audit commits.

- P399 private actual-view/reader gate PASS p399-35yn4yyi: sanitized selectable folder failure
  survives two real synthetic-log updates, selection/pause/Refresh, and clears on retry/success/
  retirement. Queued late error and signal-time retirement cannot dispatch/update obsolete views.
  Final independent source GO, fmt/compile/Clippy PASS. Reader policy unchanged; no real desktop
  error induced. Live Gungeon saved-log view rendered and exposed current run/Refresh/Copy/folder.
  Actual tag/log preview closed normally after restoring tags; app-tag-log-preview.log retained.

- Actual P394 focused keyboard Enter saves a clearly marked temporary Gungeon tag, displays its
  chip and Tags saved; clicking its remove control restores empty original tags, clearing input
  disables Add. Earlier XSendEvent-to-window Return did not submit; normal focused keyboard
  event did. Screenshots p394-tag-added-focused.png and p394-tag-restored.png. No test tag remains.
  Generic Properties currently starts on unsupported Cloud Saves for this native game; bounded
  General-first P401 approved. Separate catalog feature/play-mode near-duplicates preserved.

- P400 cumulative source assurance finds no introduced blocker in its bounded reviewed paths;
  report /tmp/ludomere-r111-p400-assurance.md separates independent authored-by-others coverage
  from consistency checks of own code. No runtime, credential exposure or full readiness claim.

- P397 commitcd34f77 exact post-build PASS (69 built audit commits). P393 animation-disabled
  diagnostic also fails identically (p393-qnn3ynpn); simple animation-dependent explanation is
  unsupported. All P393 candidate/temporary changes removed from main again. Further owner/
  native-baseline diagnosis required; no timeout or lifetime assertion weakened.

- P397 exact private actual-worker gate PASS p255-3fwlcft_: activity exists before worker entry,
  frozen/retired admission cannot create log/database paths, staged revocation prevents later DB/
  publication work, partial errors/Retry and newer-row ownership retained. Independent source GO;
  compile/fmt/Clippy PASS, known private GSettings warnings only. Actual user logs/helpers untouched.
  P395 copy and P398 folder predicate preserved by contextual integration.

- P398 one-expression GTK stat removal independently reviewed; only Downloads folder-button
  construction changes. Nonempty missing paths now use existing asynchronous validation/error
  feedback on explicit click; empty paths remain disabled. No safety/path policy or helper change.
  Formatting/build PASS; recent P396 private opening/error gate retained, no mirror unit test.

- P393 diagnostic repeats failure (p393-3vm0882h): original menu releases, outer popover remains
  unparented/unrooted/unmapped with one non-diagnostic reference; its submenu/Manage descendants
  remain. Native focus is the list row, not the popup. Exact remaining owner not established;
  possible clientless-display transition retention is a hypothesis only. Temporary diagnostics
  and candidate removed from main while author investigates; no production scope expansion.

- P396 commit9bcc659 exact post-build PASS (67 built audit commits). P393 temporary fixture-only
  diagnostics preserve its failed strict release assertion and original production candidate;
  root rerun pending. P397 frozen operation-log admission and P398 one-expression folder
  eligibility remain separate, with source review/gates before integration.

- P395 commit7f3a6d8 exact post-build PASS (66 built audit commits). P396 exact private helper
  gate PASS p396-ttu00nkh: current signed-out activation, held-worker heartbeat, error/disconnect,
  account/auth retirement, hidden/closed retained parent and weak destruction. Only one captured
  inert launch, no profile files created. Original validation path unchanged; fixture checks
  outcomes/lifetimes, not real path validation/file-manager execution. Independent source GO and
  fmt/compile/Clippy PASS; only known private GSettings warnings.

- P394 commitf6a1264 exact post-build PASS (65 built audit commits). P395 independent final
  source GO: four recovery strings now name Home→reopen, matching actual detail identity reset;
  no backend/navigation/guard change. Two unwrapped operation-log labels shortened, original
  Achievements/Logs labels already wrap. Copy-only scope warrants formatting/build, no new fixture.

- P394 private actual editor/SQLite gate PASS p270-o18ygr8z: empty/whitespace/cleared eligibility,
  Entry activation trims and saves once, effective parent sensitivity blocks programmatic duplicate
  signals, refusal restores correct eligibility and preserves tags. Prior Manage/visibility/session
  regressions retained. Independent source GO, fmt/compile/Clippy PASS; only known private
  GSettings warnings. No actual personal tags changed; signal test is not physical keyboard proof.

- P392 commit7654459 exact post-build PASS (64 built audit commits). P393 first actual-builder
  fixture fails its component weak-release wait (p393-0yxu5614, files.rs7353), after earlier
  action ordering assertions; retained P385 passes p385-n8oy19t9. Compilation/fmt/Clippy pass.
  Candidate removed from main pending author ownership diagnosis; no lifecycle assertion waived.
  No product commit for P393 yet. P394 tag interaction gates proceed independently.

- P392 strict launch gate PASS p392-dlgfz8zp under independently reviewed root-only X11
  wrapper: private profile/bus and inert backend, real desktop focus only for its verified PID/
  title. Original active/post-Starting assertions unchanged; Broadway failure retained. Only
  known private GSettings warnings remain. Menu and retained P389 gates already passed;
  compile/fmt/Clippy/build PASS. Separate real app277842 tray AboutToShow lists only the three
  test games; Gungeon tray launch reaches native2.1.9 title/menu, then normal windowquit.
  App normal tray Close exits; no panic/Gtk-CRITICAL/GLib-GIO-CRITICAL in this run.
  Evidence p392-gungeon-tray-title-settled.png, app-tray-profile-preview.log. No new transfer.

- P390 commit435ddc1 exact post-build PASS (63 built audit commits). P392 menu gate PASS
  p392-90kfrtg6 and retained P389 lifecycle PASS p389-r5z6gw4p; fmt/compile/Clippy PASS.
  P392 launch fixture reaches its explicit active-window wait, which fails on clientless Broadway
  (p392-xbz5__e6, tray.rs1096). No assertion weakened; active-window environment remains a gate.

- P390 final private gate PASS (p390-6yecu2en), including separate real library/account counters,
  reentry, terminal restoration, closed/stale/no-start/local paths. Correct native fixture parent
  eliminates row-focus criticals; only known private GSettings-schema warnings remain. Final
  fmt, all-target Clippy and test compilation PASS. Independent container/spacing corrections GO.
  Actual Settings progress/completion already passed on identical production logic before6px margin.

- P390 corrected counter design passes independent source review and initial private actual-control
  gate (p390-5uk_o904), fmt/Clippy/compile. Fixture ActionRow incorrectly used a Box parent,
  producing focus criticals; independently reviewed fixture-only PreferencesGroup correction
  restores the native hierarchy before final gate. Production real Settings run had zero panic,
  Gtk-CRITICAL or GLib-GIO-CRITICAL markers. One explicit Refresh immediately showed disabled
  Refreshing and Game list250/569, then enabled Refresh and synchronization-finished text.
  Screenshots p390-maintenance-{idle,running,finished}.png; app-maintenance-preview.log.
  Normal tray Close exits the test instance. Small local feedback spacing correction pending.

- P392 frozen source GO includes review correction: same-session Windows foreground refusal
  after Starting repeats existing retry guidance instead of leaving false Starting text;
  revoked-account rejection stays silent. P393 weak-management-popup implementation source GO.
  Both await root focused compile/runtime gates; no actual reset or game launch used by fixtures.

- P390 pre-integration review caught and corrected a shared proposal mistake: begin_library_session
  advances LIBRARY_SESSION, not raw ACCOUNT_SESSION. Earlier adoption design/review superseded;
  no mistaken draft entered main. Root and both agents independently reread the two APIs.
  Corrected local observer retains original account/auth/epoch, follows model sync_generation,
  and rejects every account invalidation. This removes unnecessary adoption machinery.

- P389 commitde41bc5 exact post-build PASS;62 built audit commits. P392 isolation starts from
  that committed tray lifecycle; its profile-work guards remain separate. P390 contract reduced
  to current-sync feedback, O(1) idle snapshots and no original/newer attribution machinery.

- P389 corrected exact lifecycle gate PASS (p389-5k6sf0cd); disposal helper waits for producer
  disconnect and rejects duplicate acknowledgments. Independent source/correction GO;
  fmt/Clippy/compile/build PASS. Real KDE app270006 advertises expected tray commands, normal
  window X hides while process survives, explicit tray Open restores same window, tray Close
  exits normally. Immediate search after Open preceded100ms dispatch; settled check passes.
  p389-{started,tray-reopened}.png and app-tray-startup-preview.log; zero panic/Gtk-CRITICAL/
  tray-unavailable markers in that run. No real delayed-watcher claim beyond private driver.

- P389 initial private lifecycle gate fails an immediate channel-disconnection assertion after
  explicit disposal acknowledgment (p389-a2k30y5f, tray.rs692). Compilation/fmt/Clippy pass;
  no commit. Author investigates producer-drop timing, preserving exactly-once disposal and
  eventual disconnection checks. No real tray/profile touched in this fixture.

- P391 commitb89ed5f exact build PASS, bringing61 separate built audit commits. The last
  file-popup and tab-preview application logs contain zero panic/Gtk-CRITICAL/GLib-CRITICAL
  markers; this is limited to those two runs and does not resolve deferred earlier P375 evidence.

- P391 Rename starts disabled and follows trimmed entry text, avoiding silent empty submission.
  Existing actual Manage tags fixture PASS (p270-u8jsxqlm): empty/whitespace/valid/cleared,
  Cancel/no-write and valid trimmed SQLite rename; stale/reset checks retained. Initial fixture
  compile failed on nonexistent AlertDialog.response; independently reviewed correction clicks
  the actual mapped native response button. Final fmt/Clippy/compile PASS; production unchanged
  by correction. No real personal tags changed by this test.

- Actual Gungeon grid Hide → empty results → Show hidden → Unhide round trip PASS.
  Search remains intact; count0/1 and notification update correctly; Show hidden remains under
  Library category. Restored original visible game and disabled Show hidden. Evidence
  grid-hidden-result.png, show-hidden-filter.png, grid-unhide-menu.png, grid-visibility-restored.png.
  Running preview is0d10311 plus rejected P388 visual flag; this flow is unaffected by that flag.

- P388 one-property non-overlay scrollbar candidate rejected by live narrow gate: visible bar
  consumes tab allocation and clips labels (p388-narrow-settled.png); wide fits normally.
  Independent source reassessment finds native Automatic measurement does not reserve the bar
  through natural-height propagation alone. Entire product delta reverted; no commit/acceptance.
  Existing keyboard/wheel-reachable tab layout remains. Discoverability improvement deferred
  rather than add speculative geometry handlers. Preview process retains old candidate until restart.
  P387 commit0d10311 exact build PASS;60 separate built audit commits.

- P387 reviewed one-expression copy correction renders1 part instead of1 parts, preserving
  None omission and plural counts. Formatting/diff PASS; no new fixture for this copy change.
  P386 commitf8a9cb7 exact build PASS, bringing59 built audit commits before P387.

- P386 private actual-component lifetime PASS (p386-g0thah12): inventory row/button and
  Advanced popup/force buttons release after ordinary detachment while host remains mapped.
  Retained P267/P369/P372/P379/P384 PASS (p267-9l7q8joe,p369-djwfcbfv,p372-8nrw9w17,
  p379-oy6idog7,p384-g3yo6mpj). No actual cloud operations; whole-page retention still true.
  Independent source review, fmt/Clippy/compile PASS. Scope is two proven strong-capture cycles.

- P385 actual pointer handoff PASS: wallpaper File actions closes before destination chooser;
  first Download click closes chooser and reports existing files. Reopened menu/Delete closes
  popup; first confirmation Delete removes only wallpaper, restoring single Download and totals.
  Direct Download restores1,732,183B wallpaper and900.7MB total; game/three installers preserved.
  Screenshots p385-{chooser,first-download-click,delete-confirm,first-delete-click,restored}.png.
  Exact private builder p385-y36qpfty, archive completion/delete p296-j4hilar1, retained proxy
  p296-bpjt14e6 all PASS; fmt/Clippy/compile/build PASS. Independent source review GO.
  This resolves the observed two-click Files dialog handoff, not deferred P375's warning.

- P384 commitb2922a9 exact post-build PASS;57 separate built audit commits before P385.

- P384 new actual-control fixture PASS (p384-qe7bjzdg), retained P369/P372/P379 PASS
  (p369-p_o635lf,p372-3p1ewhvt,p379-7_xamkxt), final fmt/Clippy/compile PASS. Real mkdir
  failures and activity/origin/launcher guards exercised under private roots; no real file
  manager or cloud operation invoked. Whole-page retention diagnostic remains true, not fixed.
  P381 commit4de6019 exact post-build PASS;56 separate built audit commits before P384.

- P381 final preview: real wallpaper delete/redownload completes with status on its own row;
  Play/alternate/gear/favorite and selected Files tab fit at1150x700 and1330x850. Hidden status
  has no empty second row. Evidence detail-preview3-status-fixed.png and -status-wide.png.
  Existing status/action fixture PASS (p341-ssfa9q3a) and Files renderer PASS (p354-aoorftjj),
  final fmt/Clippy/compile/build PASS. Earlier actual native keyboard and filter gates retained.
  App-local GDK_DPI_SCALE attempt did not establish visibly enlarged text, so no full-app enlarged
  font claim: only separate native-tab1.35font diagnostic covers that variation. Actual active/
  failed status was not newly manufactured; retained synthetic state tests cover its controls.
  These finite limits remain recorded; no exhaustive responsive/accessibility readiness claim.

- Live P382b on3146197: Gungeon manual Check for Updates returns No update is available with
  no unusable Update button or spinner; alternate-action popup closes correctly. Screenshot
  manual-update-live-fixed.png. Closed normally; no work queued. P383 commit8554fc0 exact
  post-commit build PASS, bringing55 separate built audit commits.

- P383 focused gates PASS: exact unchanged-predicate cases (p296-k1i_y819), actual held-worker
  completion/deletion/reentry/session fixture (p296-jf1ufpq1), retained unmounted proxy chooser
  (p296-hoqgb97x). Final fmt/Clippy/compile PASS. Existing nonfatal private GSettings warnings
  remain; no actual game/account/helper access. Independent source review GO and completion
  policy unchanged. P382b commit3146197 exact post-commit build PASS.

- P382b actual-dialog initial-outcome fixture PASS twice (p382b-z7k06fmm and final
  p382b-luxeqhzm). Initial Clippy rejected the cfg-known-None test interception; independently
  reviewed closure correction removes the lint without suppression or changed worker behavior.
  Final fmt/all-target Clippy/compile/build PASS. Available/confirmed update paths have source
  ordering coverage only; no real available update was manufactured. Live no-update follow-up
  remains pending alongside next layout preview. Hidden terminal Update and spinner verified.

- P381 preview2: settled narrow/wide populated Files fit after wrapping header; platform and
  language filtering work and values survive resizing. Actual paced native keyboard traversal
  selects Logs/Files/Overview and reveals offscreen first/last tabs after settled allocation.
  Private normal/enlarged native-focus API diagnostics also complete; they are supplementary,
  not real-key evidence. Retained P354 renderer PASS (p354-4f5b1ctv), compile/fmt/Clippy PASS.
- P381 still blocked by actual completed-download panel: wallpaper-only delete/redownload
  succeeds, but settled Download Complete status pushes gear/favorite offscreen at1150x700.
  Evidence detail-preview2-complete-status-narrow.png. Bounded separate status-row proposal
  requested before further changes; preview patch temporarily removed for independent P382b gate.
- File-menu test correction: GTK uses a separate X11 popup excluded by main-window screenshots.
  Menus/actions were opening; repeated inputs spawned additional choosers, all dismissed without
  further downloads. Actual one-file wallpaper Delete confirms first click dismisses underlying
  menu but leaves confirmation; second click deletes. Direct-row Download then succeeds on first
  confirmation click. Possible retained popup grab is under source review; no cause claim yet.
  Wallpaper restored (about1.7MB), game/three installer files preserved;14GB reservation remains.

- P379 commit44f13da exact build PASS. P382a idle graph predicate passes the extended actual
  Downloads renderer (p378-99n30h98), fmt/Clippy and compile. Initial private display startup
  was sandbox-blocked before tests; rerun with authorized private socket creation passed.
  Zero-only completed history hides the graph; disk/network history and active/error controls
  remain covered. No sample/history or transfer policy changed.

- P379 corrected private GTK and retained P372 gates PASS (p379-hauwyn4h, p372-u_3e4cm_),
  with final fmt/Clippy PASS. Existing SQLite/backend/P369 gates passed earlier. New Choose
  destruction and stale-result/activity assertions pass. Whole-page retention remains unresolved:
  both never-clicked baseline and completed-save detached pages remain retained; no cause proven.
  This does not claim a global Properties lifetime fix. Independent review and scoped guarantees
  retained; current override persistence no longer blocks GTK or loses newer choices to discovery.

- P380 missing-fixture-status diagnosis confirmed: production initialize_library_loading was not
  called in P325. Independently reviewed one-line fixture initialization retains every assertion;
  final exact P325 PASS (/tmp/ludomere-p325-s1h1rbzf). Prior P340/P334/P337 remain PASS.
  Final fmt/Clippy and commit3bd6d20 exact post-build PASS;51 separate local audit commits built.
- Gungeon macOS archive completes (~261MB actual), giving3local installers/900.7MB total archives
  and Extras. Active-download capture preceded settled window resize, so additional action-row
  clipping is not yet established from that frame. By the next
  attempted Pause click transfer had completed and button had changed to Play, launching the
  already-authorized native Gungeon. Root closes its game window normally; no macOS install
  occurred. Screenshot detail-width-paused-download.png is actually running-game state, not Pause.

- Reserve14GB of40GB for another bounded Gungeon archive transfer (macOS installer catalog260MB),
  solely to inspect active/paused presentation; no additional game or macOS installation admitted.
  Existing native/Windows installers and game remain in place. Live preview remainsbb20ed0+P381.

- P381 preview improves narrow Overview: primary/gear/favorite are inside viewport and horizontal
  scrolling reveals final tab. Selecting Offline Installers exposes a remaining active-page width
  issue and clips favorite again. Keyboard focus-scrolling not yet established. No acceptance or
  commit yet; author reviews active header minima. Preview patch temporarily removed from main
  (retained isolated/artifact, running preview unchanged) to verify P379 independently.
- P379 independent source review GO after neutral malformed-row messages (Choose can be disabled)
  and valid Error-status fixture correction. Compile/fmt PASS; actual SQLite reservation merge,
  backend effective-return/error and existing round-trip private tests PASS; GTK gates pending.
  New GTK gate fails whole-page weak release (/tmp/ludomere-p379-etgkqz8_); retained P372 fails
  old Retry-sensitive completion sentinel when resolved availability now disables hidden Retry
  (/tmp/ludomere-p372-0h0fab2z). P369 loader passes (/tmp/ludomere-p369-bm08jemk). Corrections
  under independent review; no lifetime assertion silently dropped or P379 commit yet.
- P380 compile/fmt PASS; extended P340 accessibility/renderer and retained P334/P337 PASS
  (p340-vkynb3k0,p334-ubibeg1o,p337-sgvlkwh3). Extended P325 fails missing fixture status label
  (p325-kltfe2_0); author investigates narrow fixture construction. No product regression inferred.

- Onbb20ed0 small Gungeon wallpaper transfer completes normally; new Disk write rate label
  fits actual Downloads header, thin section dividers/compact empty queue visible. File remains
  in Extras (about1.7MB), two offline installers/game untouched;13GB reservation retained.
  Empty zero-rate graph after this fast transfer and disabled Update on Up-to-date modal are
  P382 source-only feedback assessment. P381 minimal native tabviewport preview built, live
  precommit verification starts; no complete responsive-layout claim yet.

- Gungeon manual Check for Updates returns No update available without queuing work. Modal
  retains a disabled Update button (minor feedback follow-up); root closes normally. Actual
  narrow window1150x700 including decorations clips detail gear/favorite/right tabs on both
  Overview and Patch Notes; P381 source proposal isolates tab-width pressure, no guessed fix yet.
- Livea3641c1 closes normally through tray; aggregate log check shows0 panics/RefCell/GTK or
  Adwaita criticals for this run. Restarted verifiedbb20ed0 for Downloads layout live verification.

- P378 independent correction review and exact private fixture PASS
  (/tmp/ludomere-p378-60rgtjrj), compile/fmt/Clippy PASS. Initial failure is retained: box-drawing
  glyph has1px negative ink bearing with visible overflow; revised assertion keeps logical bounds
  strict and tests transformed ink against actual clipping ancestors/window, without tolerance
  or product padding. Normal/enlarged text, divider thickness, compact empty queue, ordered history,
  nonempty queue and paused Resume eligibility verified. No action/network worker launched.
  Commitbb20ed0 and exact post-commit build PASS;50 separate local audit commits built.

- Gungeon Achievements loads four unlocked entries; explicit Refresh visibly shows Loading
  achievements and disables the action, then restores Up to date/current cache time and the
  same page. No game launch or achievement mutation. Live screenshots gungeon-achievements-*.png.

- Ona3641c1 Gungeon screenshot viewer opens at6/16; focused Right advances to7/16 and Left
  returns6/16, with matching images. Initial SendEvent-style key did not advance; normal focused
  keyboard input does. Screenshots gallery-open/right-focused/left-focused.png. No navigation fix
  inferred. Overview's generic Location shows installer root despite installed native payload;
  sidebar says1 games. P380 source assessment only.
- P378 first exact private layout fixture fails at metric ink starting outside label allocation
  (/tmp/ludomere-p378-4imeucus). Formatting, compilation and Clippy pass. Author adds measurement
  diagnostics under independent review; no assertion weakening or build/commit acceptance yet.

- P376 reviewed worker/eligibility correction passes actual synthetic SQLite reserved-lock,
  heartbeat, exact-key preservation, raw-session, activity and reentrant controls fixture
  (/tmp/ludomere-p376-cdf3kkr1), retained P363 branch controls (/tmp/ludomere-p363-1pvjukij),
  compilation/fmt/Clippy. New Forget IO is off GTK; no keyring/decrypt/network or global account
  mutex added. Current-selection restoration and Started handoff preserve busy feedback.
  Already-issued DELETE cannot be undone by revocation; no cross-window serialization claim.
  Commitefa4410 and exact post-commit build PASS;49 separate audit commits built individually.
- Actual completed-history trash presents explicit history-only confirmation. Enter default-
  response test was blocked by approval because focused response was unverified; root instead
  clicked the visible Cancel successfully, preserving all three records/files. No keyboard
  activation PASS inferred. Screenshot history-cancelled.png confirms unchanged history.

- P377 independently reviewed two-literal setup-stage correction verified by existing mapped
  setup report regression (/tmp/ludomere-p332-wglbg_xg), fmt/Clippy/compile PASS. Fresh installs
  now say Saving game files; required-component preparation no longer implies every processed
  byte is downloaded. No phase/worker/progress/callback changes or new test scaffolding.
  Commitd5d3c51 and exact post-commit build PASS;48 separate audit commits built individually.
- Reinstalled BIT.TRIP renders its title screen ona3641c1; normal quit removes its window and
  returns Play/search with2min retained playtime. Notifications records uninstall/setup results,
  without transient Checking Windows requirements/Ready entries (live P373 confirmation).
  No panic/RefCell/GTK critical found in this app run at this checkpoint. Earlier repair-only
  warning remains open; no exhaustive game/controller or progress-frame coverage claim.

- Ona3641c1 BIT.TRIP normal uninstall completes from first confirmation click; actual payload
  and prefix both absent afterward. Detail updates to Download without reopening; playtime
  remains1min. Root begins authorized reinstall from default Windows Depot source, no extra
  game admitted and13GB budget reservation retained. Screenshots bittrip-uninstall-*.png.
  Fresh reinstall completes successfully, Details shows26.4MB Depot download,57.5MB actual
  writes and58.8MB full estimate distinctly; Play restored in place. Intermediate committing
  stage captured, operation finished before background/Downloads capture, so no live full-phase
  or active-header claim. Misleading Saving repaired files on fresh install assigned P377copyfix.
- P375 independently reviewed cfg(test)-only diagnostic remains isolated, main unchanged.
  First isolated compile fails because its fixture supplied nonexistent Game.owned; author
  correction/review requested. No stale test binary or live debugger run.
  Independently approved field/assertion correction compiles. First exact runtime
  (/tmp/ludomere-p375-osbekox5) fails at actual confirmation Start mapped/focus assertion after
  inspection release. No ancestry critical before that boundary (only known harness GSettings
  warnings). Author diagnosis requested; no assertion weakening, product fix or main integration.
  Corrective instrumentation retains strict actual Start mapping/focus, waits specifically for
  the response control and records safe frame/ancestor metadata. Independent review/compile PASS.
  Complete exact diagnostic PASS (/tmp/ludomere-p375-sak6uovy), no target ancestry warning;
  full actual callback chain reached with fail-closed synthetic results. Broadway frame count
  advances1to3 and sheet opacity0 at observation; this is not proof of live X11 animation
  coverage. Xvfb absent, no package installed. Diagnostic remains isolated, no speculative fix.

- P374 actual Downloads constructor/updater counter parity and constrained action layout fixture
  PASS (/tmp/ludomere-p374-aulqb_j1), plus pure matrix (p374-2cemoaa_), existing processing/
  persisted-count tests (p374-t5ruwpfb,p374-_m65czif) and full diagnostic layout/refresh tests
  (p333-pkdlxdr6,p326-hx1yk1ql). Independent review/compile/fmt/Clippy PASS. Separate actual
  writes/full estimate replace misleading write percent; writing activity only in relevant
  phases, valid local phase fractions retained, inactive meters stop, constructor and updates
  share labels/full tooltips. Backend counters/actions/rates/ETA unchanged; no live benchmark.
  Reviewer caught test-only wrong active-footer button assumption before runtime; corrected to
  exact phase-specific existing controls, retaining visibility and geometry assertions.
  Commita3641c1 and exact post-commit build PASS;47 separate audit commits built individually.

- P372 original-session metadata Retry, pre-spawn activity and weak page/window result lifetime
  independently reviewed and verified by actual-handler inert fixture (/tmp/ludomere-p372-qg0zylkz).
  P369 local loader (/tmp/ludomere-p369-0a7d8pkw) and P267 cloud actions
  (/tmp/ludomere-p267-3kq6nmtf) regressions PASS; compile/fmt/Clippy PASS. Terminal receiver
  retirement precedes widget updates and Retry unlock, preventing reentrant old-result overwrite.
  No actual discovery/cloud save access. Broader location override race remains separate.
  Commitec4a91f and exact post-commit build PASS;46 separate audit commits built individually.

- On5a05fe8 Gungeon's2.1MB-catalog wallpaper uses an Extras-only destination chooser with the
  full configured path and working Download action. Actual1,732,183B ZIP lands under
  extras/enter_the_gungeon/extra; row becomes1/1 downloaded/checkmarked, total rises to639.7MB,
  completion appears in footer/detail, Play remains available and two installers stay retained.
  No installer launch. Budget reservation remains13GB/40GB, including this small artifact.
  Extras Delete confirmation needed a repeated click in this live attempt (first left it open;
  cause unknown, active/focus remained main window). Second click removes only the wallpaper,
  returns row to single Download/0of1 and total638MB, records full deletion success. Confirmed
  ZIP absent; game/two installers retained. P375 notified of possible menu/dialog input relation,
  without claiming causality. No deliberate cloud deletion.

- On5a05fe8 Coffee Talk Repair preview reaches the corrected component consent: full-width
  header, matching left-aligned text/list, visible Cancel/alternate/Install actions. Root cancelled
  at consent; no new repair or component installation. Screenshot components-live-fixed.png.
  Actual log again emits one GTK ancestor invalid-widget critical at09:40:05 local near the
  confirmation-to-components transition; no visible failure. P375 source diagnosis assigned.
- P373 inert actual-footer/producer/Notifications GTK fixture PASS
  (/tmp/ludomere-p373-zwk5myd8), compile/fmt/Clippy PASS; source independently approved.
  Overlap, reentry, errors, stale sessions, destruction and compact geometry exercised without
  actual helpers. Existing ready-action/missing-selection regression PASS
  (/tmp/ludomere-p373-ready-xy2yu368) with reviewed private non-executable text helper.
  No real helper execution. Temporary checks own independent weak footer labels; overlap and
  reentry preserve one visible line, actual errors/results/Finish setup/history remain intact.
  Commit698bd22 and exact post-commit build PASS;45 separate audit commits built individually.

- P370b independent review, pure counter matrix and actual mapped setup Details fixture PASS
  (/tmp/ludomere-p370b-_en33cny, /tmp/ludomere-p332-tmoxo3nd). Compile/fmt/Clippy PASS.
  Payload bytes written are distinct from full estimate; no inferred reuse or manufactured
  completion fraction. Component/unknown-origin counters use processed wording; known-zero
  Depot download is scoped. Backend counters, controls and progress policy unchanged.
  Commit8602458 and exact post-commit build PASS;44 audited commits built individually.
- On5a05fe8 Coffee Talk Properties displays the saved enabled cloud policy, last successful
  sync and full save location without navigation. Root invoked read-only inventory check;
  no cloud export, force or deletion. Inventory completes in place with No remote save files/0B.
  Screenshots cloud-loader-live-properties.png and cloud-loader-live-inventory.png retained.

- P369 new actual Cloud local-loader fixture PASS (/tmp/ludomere-p369-1qnzpsyc): reader-blocking
  SQLite lock/GTK heartbeat, accurate saved/missing records, real read error/Retry, unchanged
  database, signed-out local access, original sessions, weak closed/hidden/detached pages and
  synchronous Retry reentry. Existing P267 cloud-action regression PASS
  (/tmp/ludomere-p267-5uva2eia). Independent mechanical extraction/lifecycle review and
  compile/fmt/Clippy PASS. No cloud request or automatic discovery; metadata/override issues
  remain separate. Live Properties retest pending next app restart.

- Liveb2629ce app log aggregate after these tests: no panic/RefCell borrow/Adwaita critical and
  no Gtk warnings, but one Gtk critical at08:49:52 local (`gtk_widget_is_ancestor` invalid widget)
  near the Coffee repair flow. No observed crash or stuck UI; cause not established. Retain as
  an open diagnostic, do not claim a warning-free live run or infer a speculative source fix.

- P370a consent spacing independently reviewed; final actual mapped short/long text,400px footer
  and error scrolling fixture PASS (/tmp/ludomere-p370a-5o79fn4z). Compile/fmt/Clippy PASS.
  Two earlier fixture failures assumed flush HeaderBar bounds; installed libadwaita stylesheet
  explicitly uses-1px side margins. Final header-only assertions permit only that outward extent
  while requiring full coverage/no inset; actual text/action/scroll assertions unchanged. No
  consent/admission callbacks changed; full source and corrective review retained in p370a reports.

- Additional authorized under2GB test selected: BIT.TRIP Runner. GOG's official product page
  https://www.gog.com/en/game/bittrip_runner lists100MB Windows storage; chooser defaults to
  Windows Depot. Conservatively raise reservation from9GB to13GB of40GB for payload, prefix,
  staging and prerequisites before download. No other new test game admitted.
- BIT.TRIP Runner fresh Windows Depot install succeeds onb2629ce. Observed game-file download
  progress, clicked Run in background, then observed DirectX stage2/4 with visible in-place progress
  and disabled primary action. Completion restores Play/white list title without navigation or
  a background modal. Required OpenAL/MSVC2010/DirectX/MSVC2010_x64 setup did not fail. Launch
  renders the title screen. Measured payload56,697,541B and prefix409,323,147B (about466MB);
  conservative13GB total reservation retained for caches/staging/repeated tests.
- P371 duplicate MSVC consent diagnosis needs no backend fix: source already checks exact recipe
  history in the same prefix before each helper and retains separate vendor/provenance receipts.
  Root read-only aggregate log check confirms exactly one vcrun2010 command for the two entries;
  no raw logs or credential values shared with agents. Missing history remains deliberately pending.

- P368 independent review and final private actual-worker lifecycle fixture PASS
  (/tmp/ludomere-p368-dgo76u9m); existing P367 geometry fixture PASS
  (/tmp/ludomere-p367-p0fiwve5). Inspection registers activity before dispatch and checks original
  raw sessions without requiring login for local reads. Initial fixture exposed inherited repeated
  close on already-closed dialog; narrow early return independently approved and warning absent
  on final run. Compile/fmt/Clippy PASS. No claimed atomic cancellation of in-flight reconciliation
  or post-reset surviving worker; this protects queued work, stale results and reset preflight.

- Onb2629ce Coffee Talk's recognized Depot Repair presents explicit repair confirmation and
  required-component consent, then Setup complete without launching the game. Retains current
  installed version. Completion succeeded too quickly between root observations to capture active
  progress; do not infer a full intermediate-progress check. Details contains real stages; repair
  reused files (0 downloaded/written) but planned558.3MB total looks unfinished, assigned P370
  diagnosis. Repaired game renders title screen; normal close ends CoffeeTalk.exe and restores
  Play/white title/search in place. No deliberate cloud deletion, reservation remains9GB of40GB.

- Onb2629ce real Gungeon Manage/Repair shows the padded, titled native fallback; Review
  Reinstallation opens the full680x620 chooser with Linux2.1.9,255.3MB, full installation
  destination and visible Cancel/Install. Root cancelled; no installation/files changed.
  Evidence: repair-fallback-fixed.png and repair-chooser-fixed.png in the private audit directory.

- P367 repair presentation independently reviewed and verified: padded scrolling fallback with
  clear Repair/Update title and hidden stopped spinner; reused reinstallation chooser restores
  standard680x620 dimensions/title. Actual mapped constrained-geometry fixture PASS
  (/tmp/ludomere-p367-8jvgzmkd), existing P346 source/destination fixture PASS
  (/tmp/ludomere-p346-r96dc1gv). Initial Clippy test-type complexity corrected with an equivalent
  cfg(test) alias, independently reviewed; final compile/fmt/Clippy PASS. No execution/consent
  changes; live retest passes above and inspection lifecycle addressed separately in P368.

- On695d738 the Account Factory Reset-to-Connection gap is visibly corrected; Connection remains
  Online and authenticated. No reset, sign-in exchange or settings mutation performed.

- Native Gungeon Repair offers reinstall/Browse/reset instead of Depot repair; no files changed.
  Review Reinstallation then remains in420x180 dialog, visibly clipping installer choices while
  Install stays available. Rechecked after loading, cancelled without installing. P367 assigned
  concrete handoff-size/layout fix; do not claim native repair end-to-end PASS from this flow.

- Settings Proton shows full selected runtime path, automatically reports Steam Linux Runtime
  found and disables its unnecessary download action. Public Load releases populates choices and
  enables download selection without changing the installed default or downloading a runtime.
  Account Connection shows Online and authenticated. No sign-out/reset or runtime change tested.

- Coffee Talk Properties onb3f9227 reproduced Switch-on-installed-Master refusal without changing
  branch/password; P363 addresses the verified pointless action. Read-only Cloud storage Check now
  completes with No remote save files/0B, in place. Normal sync setting remains enabled; no export,
  forced sync, cloud deletion or other settings change. Properties closed normally.

- Onb3f9227 initial Offline Installers shows both retained Gungeon archives/checkmarks/638MB.
  Confirmed native removal with archive deletion unchecked reports one game directory removed,
  zero prefixes and two archives retained. Detail changes to Download without reopening; arrow
  offers Install from Offline Installer. Chooser selects downloaded Linux2.1.9 and correct root.
  Cached install completes, Play launches native executable without Windows setup/repair, and
  the fresh marker is schema1/base.operating_system=linux with no UMU field. Game intro and main
  menu render; normal quit ends EtG.x86_64 and restores Play/white title/search in place.
  No extra transfer or deliberate cloud/save deletion; reservation remains9GB of40GB.

- Archive-only Gungeon macOS transfer paused at134.8/260MB. Initial featured trash clicks showed
  no dialog despite responsive navigation; after revisiting Downloads, ordinary clicks show Cancel
  download confirmation. Confirmed cancellation removes that transfer, preserving both completed
  Linux/Windows installers and native payload. Source diagnosis finds no proven dead handler;
  original transition remains unexplained, no speculative product change. Reserve9GB of40GB.

- Onc9f5b99, existing native Gungeon installation launches and renders its intro; normal window
  quit ends EtG.x86_64 and restores green Play/white title/search in place. The original mixed
  marker hash remains unchanged before/after launch, proving read-only recovery. No Windows repair
  accepted or deliberate game save/cloud deletion. Intro, not a completed gameplay test, verified.
  Further cached reinstall deferred until P355 saved launch-options preservation is corrected.

- P351 live Collections Tab shows a clear game-card focus ring and Enter opens its detail view.
  Gungeon's temporary favorite was restored; screenshot favorite-restored confirms Removed from
  favorites and an empty star. P353 Storage wording fits the990x700 settings window; normal outer
  scroll reaches lower controls. P350 Recheck completed too quickly to capture its busy appearance;
  deterministic fixture, not the live screenshot, establishes the pending state.

- Additional live controls: Comet manual check shows immediate checking/disabled button and
  returns to up-to-date; downloaded-file index reports one indexed/matched archive without changing
  payloads. Metadata refresh shows Game list500/569 and completes with Library synchronized in
  Notifications. On-demand detail artwork subsequently loads; metadata-search incomplete reflects
  intentionally lazy metadata, not a persistent synchronization spinner.
- Gungeon Logs shows its saved run and successful exit; inner scrolling pauses follow mode with
  an explanation, outer page scrolling exposes the full log panel and installation-log actions.
  No log export, clipboard copy or unrelated desktop inspection performed.

- Root launched Ludomere on the real KDE desktop with X11 for scoped automated input.
  User reports wallet already unlocked. No passwords or tokens captured or shared with agents.
- Reproduced search deletion/hiding when enabling Installed; zero-result Home pane is blank.
  App-only screenshots retained privately under `/tmp/ludomere-r111/`.
- Enter the Gungeon Depot preparation failed before game transfer with a credential-service
  D-Bus disconnect. Desktop metadata checks report no standard Secret Service owner and KWallet
  disabled, despite its service owning a bus name. User was notified; wallet settings unchanged.
  Source review identifies redundant keyring reads during Depot preparation despite UI-held token.
  Follow-up corrected the initial diagnosis: the legacy wallet compatibility API being disabled
  does not establish the user's actual Secret Service wallet is disabled. Activating the advertised
  org.kde.secretservicecompat provider restored org.freedesktop.secrets and the user unlocked.
  Existing discovery already supports that provider during explicit sign-in, but saved-token reads
  bypass it. P330 addresses that path; no wallet settings changed or credentials reset.
- Missing configured installer/extras directories created explicitly by root; existing files untouched.
- Enter the Gungeon Windows offline installer (about 382 MB) downloaded through the detail action,
  automatic installation completed, and Play appeared without reopening details. Game process/window
  launched, sidebar turned green and Stop appeared. Root removed its X11 game window afterward;
  later checked xdotool's manual and corrected that this destroys the window rather than requesting
  normal process exit. Do not infer game exit/stop defects from that automation action.
- Measured Gungeon payload339,363,121 B, prefix355,554,544 B and archive382,662,456 B: about1.078 GB
  on disk. Coffee Talk Depot preparation succeeded after wallet activation; component/install consent
  accepted through UI. Track subsequent transfer/disk changes before adding more test games.
- Test game transfer budget initially0.383 GB, then Coffee Talk Depot (557 MB installed payload).
  Measured combined payload/prefix/archive plus dependency cache approximately2.15 GB; reserve
  a conservative4 GB of the40 GB allowance including download overhead until exact totals settle.
  No intentional cloud
  deletion or real uninstall performed. Installation used the application's normal helper flow.

## Fixes

### P411 — preserve focus when preparation becomes component consent

- Clear the old dialog focus before replacing its content, then focus visible Cancel on the
  new component review. This corrects the repeatedly reproduced GTK ancestor critical while
  retaining keyboard access, explicit consent and automatic no-review behavior.
- The same strict native-pointer diagnostic reproduces before and passes after; final run
  p411-hk_w2z0i retains focus and emits no target critical. Existing consent regression now
  uses the actual presented-dialog lifecycle and checks Cancel plus four native traversal
  targets; p370a-iiba2nqc passes. Real Coffee Tab/ShiftTab/Space cancellation passes without
  component execution or critical. Independent QA/safety and fresh full build pass.
- Main includes four production lines and a small existing-fixture extension. Temporary
  observers, pointer fixture, stack logging and debugger experiments remain outside main.
  Failed fixture assumptions and partial native-stack evidence are retained above; no broad
  GTK cause, warning suppression or whole-application keyboard guarantee is claimed.

### P365a — save launch fields without overwriting unrelated preferences

- Atomic partial SQL updates only executable/arguments/updated timestamp, preserving raw runtime
  and pending-profile JSON, update policies, original creation timestamp and activity. Absent rows
  receive launch defaults and retain marker-based Windows runtime fallback; no schema change.
- Seed initial submitted text so unchanged Enter/focus/unmap does not save. Actual edits and
  failures remain retryable. Register profile activity before queued work and reject old session
  before filesystem/database access. No global operation lock or unrelated-download restriction.
- Independent review, four focused private SQL/actual-entry/old-signal/branch regressions,
  formatting, Clippy and compilation PASS. Evidence p365-qjlangm4, p365-4111ep92,
  p365-9774kct8, p363-w2ixj27h. No real profile changes for these tests.
- Full P365 design was rejected: its global gate would block editing during unrelated downloads,
  and unknown marker data would disable new windows until reopened. Stale installation-specific
  launch fields and broader Properties refresh still need a separate non-disruptive design.

### P366 — collapse unused Factory Reset feedback space

- Bind the reset-status scroller's own visibility to its label, collapsing the empty reserved
  area while retaining real progress and full selectable, capped-height error scrolling.
- Independent review, extended actual Account layout/status/lifetime fixture (p359-anwwrai6),
  formatting, Clippy and compilation PASS. Synthetic feedback expands, scrolls and collapses to
  the original measured height; no Factory Reset action, account change or private data in test.

### P364 — show update-check progress and results inside Settings

- Immediately disable duplicate check activation and show local pending/result/refusal/error text.
  Keep full redacted diagnostics selectable/wrapping, restore deliberate retry after terminal or
  stale sessions, and release weak page controls on close. Background work retains normal behavior.
- Keep pending feedback local so automatic/duplicate checks cannot clear unrelated main progress.
  Main result notifications retain existing policy; already-running does not promise a notification.
  Original click-time epoch/auth/online checks reject stale UI results, not already-queued work.
- Independent review, actual production-dispatch/inert-outcome mapped fixture (p364-17h3duxl),
  existing backend stale test (p364-fls7izo3), formatting/Clippy/compilation PASS. Fixture covers both
  duplicate orderings, reentrant retry, session changes, long redacted errors and destruction; no
  real batch updates run. Initial lint test placement and hidden-main effective-visibility assertion
  corrected in tests only, preserving main sentinel own visibility/text and mapped Settings checks.

### P363 — explain unavailable branch actions

- Branch Switch is disabled for invalid/current branch, Forget Password for invalid/Master, with
  useful tooltips. Preparation disables both actions; selection changes cannot bypass it. After
  handoff Switch stays disabled while named-branch Forget returns. Errors restore current eligibility.
- No credential-presence probe, branch backend or password policy changed. Independent review,
  actual mapped controls/lifecycle fixture (p363-0w7fepo0), formatting, Clippy and compilation PASS.
  Fixture invokes no real branch/password worker; no real branch switch claimed.

### P362 — retain notification hover across remapping and clean up on destruction

- Bind manually parented notification popover cleanup to its anchor's destruction, using a weak
  reference and parent check. Surface unrealization no longer permanently detaches the popover;
  never-presented windows now also clean up. History, timers, focus and click behavior unchanged.
- Independent source/lifecycle review PASS. Strict never-presented, mapped detach/reinsert,
  hide/show and pending-timeout teardown fixture PASS, along with unchanged notification action
  and Account status tests. Root verified no children-left/popover/surface warnings in all logs;
  isolated harness's pre-existing GSettings source diagnostics remain separate.
- Evidence p362-xqiig396, p362-cjd5fixj, p359-_vwcgyk0; formatting, Clippy and compilation PASS.

### P361 — explain unavailable Storage moves before clicking

- Move follows current selection and destination: disabled for empty/missing/same-target choices,
  existing Windows-prefix refusal, or active work. Tooltips explain why; changing selection or
  rebuilding rows cannot re-enable active Move. Backend guards and destination choice unchanged.
- Independent source review, mapped actual-controls regression and existing backend destination
  preflight PASS; formatting, Clippy and compilation/build PASS. Initial fixture incorrectly
  requested invalid selection in a nonempty autoselect dropdown. Corrected only the setup to empty
  its real model, explicitly assert no selection, then retain every sensitivity/tooltip/busy check.
  Evidence p361-q5cbyzon (initial), p361-5nevxott (pass), p361-htizftam (backend pass).

### P359 — keep Account session status current

- Refresh the existing GOG session subtitle from in-memory network, token expiry and logout state.
  Weak row/model references stop updates after destruction; unchanged text produces no updates.
  No keyring, database or network probes, identity/action redesign or navigation added.
- Actual mapped-row transition/time-expiry/borrow/focus/destruction fixture, independent review,
  fmt/Clippy/build PASS. Fixture-only nonexistent stack field corrected to real content stack;
  first old-binary run correctly rejected zero tests. Strict assertions retained.
- A separate pre-existing notification-popover teardown warning appeared in this test and is
  assigned P362 diagnosis; do not attribute it to status refresh or claim a clean global teardown.

### P355 — retain launch options through cached reinstalls and source migration

- Fresh cached-offline and offline migration plans now retain saved arguments and full Windows
  runtime preferences while preserving selected source/destination and native independence.
  Empty saved rows remain authoritative; migration uses installed values only when no row exists.
  Existing repair/DLC plans keep explicit current options. Windows completion retains pending
  profile data without applying it or changing active-profile resolution/target-prefix derivation.
- Cached preparation holds profile activity from admission through hydration/save/enqueue, with
  original session checks. Migration reads preferences before backup/removal; returned errors stop
  preparation. No GTK I/O or helper policy change. The shared fresh constructor replaces duplicates.
- Independent review and eight private actual-constructor/save/refused-handoff/completion/native
  regressions PASS; formatting, Clippy and build PASS. No actual source migration was exercised
  by these fixtures. Malformed JSON defaults inside StateStore remain a separate deferred issue.

### P354 — prepare initial downloaded-file rows off GTK

- Files immediately shows an inspection shell while one tracked worker prepares base/owned-DLC
  paths, saved jobs, completeness, historical copies, folder availability and summary values.
  Reuse one connection/jobs scan and prepared rows; no initial per-row GTK DB/stat fallback.
- Keep unknown local state non-actionable, retain prior totals on returned errors and offer Retry.
  Weak view/request/account/auth/local-revision/input guards reject stale results and coalesce
  changes. Revoked sessions stop loading with exact Home-then-reopen guidance; no automatic
  replacement-session work. Ordinary errors keep in-place Retry; later row policies are unchanged.
- Review caught and corrected cached-local fallback totals, cross-kind grouping and stale-session
  loading feedback. Two new actual-entry lock/heartbeat/prepared-data/lifecycle fixtures plus four
  existing archive/queue/summary regressions PASS. Initial existing-test runs refused the wrong
  private prefix; corrected harness invocation passed without weakening guards or product code.
  Independent final review, formatting, Clippy and build PASS. No real-file work in fixtures.
- Existing persistence APIs may suppress malformed row/JSON errors internally; this pre-existing
  limitation remains explicitly deferred. One connection is not an atomic multi-query snapshot.

### P357 — track Storage inspection during profile reset

- Both choice-list and selected-library inspection open SQLite, so both now acquire profile
  activity before spawning and check originating sessions before reads/reconciliation. Reset
  draining accounts for delayed work; results still require current request/account/session.
- Independent review caught a new page-lifetime session snapshot that prevented reuse after
  sign-in. Corrected to per-admission snapshots; a deliberate current-session Recheck works in
  the same Settings page, while old results cannot paint or start stale profile reads.
- Actual delayed-worker/activity-drain/stale-no-database/same-page-retry and existing P350 feedback
  regressions PASS in private profiles/displays. Independent re-review, fmt/Clippy/build PASS.
  No actual profile reset or credential deletion used for this test.

### P352 — separate native installs from retained Windows preferences

- Real Linux Gungeon reinstall inherited a saved UMU profile, wrote a contradictory marker and
  incorrectly offered Windows prefix repair. Explicit Linux preparation, completion, marker
  construction and reconciliation now keep native runtime state independent of Windows settings.
- Preserve complete durable Windows preferences, including pending profile, when native executable
  discovery saves preferences. Windows marker fallback and missing-prefix installedness remain.
  Recover only the identified native/offline/managed-UMU marker shape in memory; future and other
  contradictory markers stay strict. No prefix deletion, on-read rewrite or schema change.
- Independent source review and four private regressions PASS, covering multipart preparation,
  both native completion points, exact recovery/rejected counterexamples and repeated full/targeted
  reconciliation with activity/preferences/raw-file preservation. Formatting/Clippy/build PASS.
  At this checkpoint live native retry was pending. Subsequent live launch and fresh schema1
  reinstall passed; see the later Live testing entries. No failed-before evidence is discarded.

- The first live retry found Storage's separate protected marker reader still called strict
  validation directly, blocking launch. Follow-up shares the exact pure normalization after each
  reader's existing checks; storage no-follow, bounded regular-file reads and strict writes remain.
  Actual launch-admission/readiness fixture plus the four prior regressions PASS, including rejected
  symlink/oversized/malformed/future metadata and unchanged raw bytes. Independent review and
  fmt/Clippy/build PASS. This corrected missing admission coverage; subsequent live native
  launch/reinstall results are recorded above and supersede this checkpoint's pending retry.

### P325 — retain search while changing library filters

- Preserve the query for boolean, language and metadata filters; keep search visible/enabled
  beside active chips. Preserve explicit Clear Filters behavior and view selection.
- Production change removes 20 lines from library/window; actual GTK regression exercises both
  input orders, all filter handler types, matching grid/sidebar counts, focus/widget identity,
  chip removal and Clear Filters without starting metadata workers or creating a real database.
- Independent UI review PASS. Focused private Broadway/DBus test PASS, formatting and all-target
  Clippy PASS. Commit717e8da and exact post-commit cargo build --locked PASS.

### P326 — show active downloads and retain full errors

- Featured transfer selection prefers active Depot/archive work over paused/failed history.
- Show full wrapped, selectable archive and Depot failures separately from changing ETA labels;
  retain Depot row errors through progress updates and refresh changed error text in the same state.
- Failed archive header action is labeled Retry download. Existing pause/resume/delete behavior
  and navigation are preserved.
- Independent source/UI review PASS. Two focused unit tests and two private GTK regressions PASS;
  formatting, all-target Clippy and cargo build --locked PASS. Fixtures prove control/text behavior;
  exhaustive visual clipping or real failure cases are not claimed from unmapped GTK widgets.

### P328 — use the authenticated session for Depot actions

- Carry the current token and original auth/online generations through direct Depot preparation,
  review and admission; avoid redundant saved-login reads. Reject expiry, account mismatch and
  stale sessions before new cache/observation/queue stages. Tokens remain absent from journals.
- Protected branch keyring work runs outside SQLite writes; revalidate before persistence.
  Migration restores once before its installation boundary, with original-session guards.
- Independent security/lifecycle review PASS; eight focused private regressions, formatting,
  all-target Clippy and cargo build --locked PASS. Existing updater API still captures its auth
  generation at worker entry; this change does not claim broader dispatch-provenance repair.
- Coffee Talk real Depot setup exposed a separate .NET4.5.2 exit194, queued as P331. DirectX
  setup completed; retain downloaded game files and investigate before retrying or resetting.

### P330 — discover the credential service before reading a saved login

- Saved-login reads reuse existing bounded Secret Service discovery/advertised provider activation.
  Standard providers remain preferred; KDE compatibility activation is used only when advertised.
  No wallet settings changed, new provider hardcoding, plaintext fallback or token cache added.
- Preserve original authentication generation and durable sign-out barriers before discovery,
  before credential read and after read. Discovery itself does not request unlock or collection
  access; the keyring's existing credential-read behavior still applies afterward.
- Independent security/lifecycle review PASS. Seven focused synthetic/private-bus regressions,
  formatting, all-target Clippy and cargo build --locked PASS. User-unlocked real provider remains
  running; no forced wallet restart/lock was used to claim a cold-start integration test.

### P329 — refresh managed-file summaries without blocking GTK

- Move refreshed archive counts/sizes and legacy-file metadata reads to workers. Share one refresh
  callback per files page/product, discard older requests and detached/account-stale results,
  and reserve profile activity before worker entry so reset cannot race profile recreation.
- Keep prior summary on read failure and update existing labels in place. Initial page construction
  still has older synchronous reads; this does not claim every Files path is now asynchronous.
- Independent source/lifecycle review PASS. Three private regressions PASS: summary equivalence,
  SQLite-lock/GTK heartbeat with stale/reset cases, and existing archive arrival/deletion controls.
  First existing-test invocation stopped at its required fixture-prefix guard; corrected isolated
  invocation passed. Formatting, all-target Clippy and cargo build --locked PASS.

## Further findings
### P331 — finish native installer restart requirements

- Coffee Talk's .NET vendor log confirms successful setup with HRESULT0x80070BC2 and restart
  request. Its Windows3010 status became Unix194 and Ludomere incorrectly rejected it.
- Recognize documented3010/1641 low-byte results only for native MSI and the exact official
  dotNet45/NDP452 installer. Other executables, Winetricks and interpreters remain strict.
- Drain installer children, then run the selected prefix's Wine restart routine through normal
  tracked UMU execution before recording completion. Also finalize after zero success, ensuring
  a retried already-installed component cannot bypass an interrupted restart. No host reboot.
- Independent source/security review PASS. Seven focused private regressions PASS, including
  explicitly synchronized inert process/drain/cancel checks, receipt/retry, strict failures and
  existing durable guard/MSI-path tests. Formatting, all-target Clippy and build PASS.
  Real Coffee Talk retry remains pending the rebuilt binary; no live success claimed yet.

### P332 — keep setup outcomes visible while reading diagnostics

- Pin concise sanitized failure/status above a diagnostics-only scroller, keep Close outside it,
  hide finished progress bars, and title terminal states Setup complete/failed/stopped using the
  existing structured outcome. Put full selectable error before operation/stage history.
- No process, retry, launch or background-window behavior changed. Independent review PASS;
  formatting/error test and mapped400px GTK scrolling/control regression PASS. Initial GTK test
  looked for a collapsed expander child too early; corrected fixture only, then passed.
- Formatting, all-target Clippy and cargo build --locked PASS. Live retry on new code next.

### P327 — confirm and track permanent Depot cancellation

- Confirm Downloads cancellation, retain Keep as default, and show Cancelling while cleanup runs
  off GTK. Preserve published payloads and current temporary-file safety checks; full errors become
  terminal failures with retry controls instead of stale busy state or silent refusal.
- Retain cleanup ownership/profile activity through terminal publication and reject competing
  resume/pause/cancel admissions. Independent review found two races; corrected recovery admission
  ordering and preserve an installation's proven success if it beats a late cancellation request.
- Three focused backend and one GTK regression PASS, including protected-directory refusal,
  journal preservation/retry, held-gate responsiveness, stale admission and late completion.
  Fixture-only AlertDialog API/unnamed-widget lookup issues corrected before successful run.
  Independent UI/backend re-review, formatting, all-target Clippy and build PASS.
- Coffee Talk real retry completed setup, updated Play/installed title in place, enabled normal
  cloud sync and rendered language/title/profile-selection screens. No game save was deliberately
  created or cloud file deleted. Root used Ludomere Stop after discovering X11 windowclose does
  not end the game process; this is a test-driver correction, not a product defect.

### P334 — explain empty library search results

- Show no-match and hidden-games guidance in the existing Home view, without clearing filters,
  replacing cards or navigating from another page. Preserve existing empty-account guidance.
- Independent review PASS; mapped empty/filter/scroll/focus regression and existing search/filter
  regression PASS. Formatting, all-target Clippy and build PASS before commit.
- Coffee Talk Stop returned to Play and no Coffee Talk/Gungeon process remained. Live Stop exposed
  a separate search/sidebar inconsistency, assigned P337. Normal game exit was not simulated by
  destroying its X11 window; no automatic post-exit cloud-sync success is inferred from that action.

### P333 — keep Downloads actions visible with long failures

- Full selectable archive/Depot diagnostic labels scroll inside a capped viewport, leaving
  Retry/Cancel outside. Preserve existing cancellation/session behavior and all diagnostic text.
- Independent review and four focused GTK tests PASS. Mapped real-CSS fixture at1100x600 kept
  archive/Depot headers at210/241px for roughly16KB errors; inner scrolling leaves actions and
  outer scroll fixed. Formatting, all-target Clippy and build PASS.

### P335 — reuse the cached-library database connection

- Startup Metadata/Acquisition readiness reuses one store and at most one cached-product
  reconstruction per game. Preserve independent observation errors, strict TTL boundaries,
  malformed-product handling, immediate game-list delivery and existing receiver guards.
- Source-derived connection opens drop from1+2N to1; observations remain2N and maximum product
  reconstructions drop from2N toN. This is not a measured whole-application timing claim.
- Independent review, two focused private regressions, formatting, all-target Clippy and build PASS.

### P337 — retain search and selection after game exit

- Release the mutable model borrow before synchronous alphabetical filter invalidation; otherwise
  callbacks fall back to admitting every row despite the actual query/count. Clear GTK's selected
  flags before detaching retained sidebar rows so reorder can restore the selected row correctly.
- The strict mapped regression initially exposed the retained-row selection defect after its
  baseline was established; production correction now passes both sorts, real row/card filters,
  selection, activity ordering, current detail, focus and scroll. No assertion was weakened.
- Live Coffee Talk normal exit returned to green Play/white sidebar title with only Coffee Talk
  shown for the active query. Mapped regression, formatting, all-target Clippy and build PASS;
  independent final re-review PASS before commit.
- Live Home no-match guidance verified. Test-driver xdotool type consumed subsequent words in
  one attempt; corrected by separate invocation. No product defect inferred from that input error.

### P339 — identify archive size honestly

- Label the game detail, DLC catalog and refreshed subtitle byte count as Downloaded files;
  a remaining installed game no longer has an unexplained0B value after deleting its archive.
- No byte-counting or traversal changes. Independent review, existing async-summary regression
  extended through nonzero/zero/repeated refresh, formatting/all-target Clippy/build PASS.

### P341 — clear stale transfer colors

- Neutral empty/preparing/downloading archive rows clear earlier error styling and tooltip;
  actual failures remain red and verified completion green. Active archive Pause gets operational
  blue styling, with paused/failed/complete paths restoring normal idle styling.
- Independent review and three focused GTK regressions PASS, covering real row-event transitions,
  deletion/retry/error/complete and existing library-selection queue proxies. Fixture recreated a
  deleted inert parent for simulated transfer completion; production deletion was not changed.
- Formatting, all-target Clippy and build PASS. Live evidence originally reproduced both colors.

### P338 — batch installer chooser polling

- Keep one lazy job index per poll, preserving canonical artifact identity, newest/last-tie
  selection, managed-file precedence, completed-file validation and independent read fallbacks.
  Empty malformed artifact records cannot panic the new index. No timer/session/UI receiver changes.
- Source-derived job reads fall fromU to at most1 forU groups requiring fallback; all-managed
  groups perform no job read. Saved-job identity work falls fromU×J toJ per pass.
- Independent review, three private pure/file regressions and actual-row GTK regression PASS;
  formatting, all-target Clippy and build PASS.

### P342 — keep free-space inspection read-only

- Only compatible existing destinations start a read-only free-space query; opening the chooser
  no longer creates missing directories or replaces an unavailable diagnosis with free bytes.
- Reject missing/non-directory/symlink leaves in the query boundary. No new mount assumptions or
  folder-creation workflow. Independent review, private no-write regression, formatting/Clippy/build PASS.

### P340 — native Home grid keyboard activation and accessible titles

- Use one native FlowBox activation path for mouse/keyboard, preserve secondary context menus,
  label each retained grid cell with its full title, and restore visible keyboard focus styling.
  Reject hidden, detached, stale-model and logout-pending activation without background navigation.
- Independent review, native activation/cursor/accessibility fixture and three existing filter/
  empty/exit regressions PASS. Fixture waits for actual layout and enters native focus handling;
  strict cursor/activation assertions retained. Formatting/all-target Clippy/build PASS.
- Live restarted app: Tab/Shift-Tab shows a clear card focus ring; Enter and Space each open the
  focused Gungeon detail, preserving its search; grid right-click retains the normal action menu.
  Private screenshots home-keyboard-focus5, home-keyboard-enter, home-keyboard-space-confirmed,
  home-context-menu under the audit evidence directory. Collection game tiles remain unchanged.

### P344 — explain external uninstaller prompts

- All existing uninstall stage-message writers and UI fallback now say to follow any prompts in
  the uninstaller window. This addresses the real Gungeon Yes/No/OK prompts without detecting
  windows, stealing focus or altering helper execution. Independent review/fmt/Clippy/build PASS.

### P343 — simplify normal uninstall confirmation

- Hide Retry until a removal check fails; collapse alternative file-reset choices for healthy
  installations and expand them on failure. Browse, prefix/save warnings, optional archive cleanup,
  default Cancel and explicit destructive consent stay visible and unchanged.
- Independent review, two private mapped recovery/preview regressions, formatting/Clippy/build PASS.
  Initial fixture compilation used private marker APIs and an incorrect label return type; corrected
  fixture only before execution. No protection or destructive-confirmation assertion weakened.

### P345 — restore idle colors and subsequent transfer controls

- Synchronize Download/Install/Update versus Play container styling for both main and arrow buttons;
  reuse the idle helper on paused/failed archives. Completed archive resets its active visual latch,
  allowing a later download to show Pause instead of retaining idle content.
- The expanded strict repeated-operation test first failed, proving that existing latch defect.
  Minimal correction, independent re-review and the unchanged six-case regression now PASS;
  formatting/all-target Clippy/build PASS. No timers, execution or download states changed.

### P348 — refuse busy cancellation promptly

- Capture/check recovery generation with one nonblocking admission attempt instead of waiting on
  another operation's journal persistence. Hold successful admission through owner registration;
  preserve profile activity, protected-file cleanup and late-completion semantics.
- Both cancellation surfaces explain busy/refreshed state and allow retry. A rejected permanent
  Depot cancellation cannot fall through to unrelated offline cancellation.
- Independent review and five private regressions PASS, including actual GTK callback/heartbeat
  while another admission is held, unchanged owner on refusal, retry, stale generation, late success,
  protected cleanup and confirmation. Formatting/all-target Clippy/build PASS. Initial test-local
  missing glib import fixed without adding a production GTK dependency.

## Earlier implementation checkpoints and live evidence

These entries preserve intermediate progress. Later Live testing and consolidated verification
entries above supersede their pending statements: P349/P350 runtime gates, P351 physical-key
activation, P353 narrow layout and the full suite subsequently passed. Current limitations are
listed in PROJECT_STATUS.md; these historical checkpoints are not a current blocking checklist.

### P353 — explain drive capacity versus library contents

- Add compact Drive usage label and wrapping explanation that managed categories count this
  library while capacity/free/Other cover the containing drive. Short Other legend avoids widening
  the category row. No calculation, scan, selection or action behavior changes.
- Independent source/layout review and formatting/Clippy/build PASS; no new wording-only test.
  Live narrow-window visual confirmation follows the next app restart.

### P351 — keyboard access within Collections

- Native game-grid activation and full-title accessibility now match Home; preserve collection
  index Buttons and secondary context actions. Use exact collection membership, not Home filters;
  refuse hidden/removed/busy-model/stale-grid/account/logout activation and retain focused children.
- Independent review, strict private mapped native-focus/cursor/activation/accessibility/lifecycle
  regression, formatting/Clippy/build PASS. Physical-key live corroboration remains pending.

### P350 — visible Storage recheck state

- Disable/relabel Recheck while its selected inspection is pending; clear old legend totals with
  the old chart. Restore only current request outcomes; keep library switching available and no-
  library Recheck disabled. Preserve worker/reconciliation semantics and recovery actions.
- Independent review, actual-receiver private GTK success/error/disconnect/stale/account/switch/
  empty regression, formatting/all-target Clippy/build PASS. No real filesystem workers in fixture.

### P349 — bound repeated grid-scroll work

- Coalesce adjustment notifications into one75ms trailing pass reading the latest viewport;
  weak grid references reject hidden/destroyed work. Preserve immediate map/rebuild updates,
  visible-first/nearby order and existing exact bounds; filtered/unmapped rows skip geometry work.
- Independent review, strict private500-card geometry/burst/filter/lifecycle/focus regression and
  formatting/Clippy/build PASS. A burst of20 changes produces one deferred publication instead of
 20 scans. Each scan remains linear; no measured frame-rate or virtualization claim.

### P346 — accurate install destinations and source-specific feedback

- Show full configured Game Files roots and the actual existing installation folder or chosen
  root/slug. Remove chooser-only mount-point lookup and its now-unused helpers. Long paths wrap.
- Keep Galaxy preflight/preparation feedback scoped to Depot while generic/offline errors remain
  visible; preserve source choice, preparation ownership and multipart queue behavior.
- Strict mapped and existing multipart tests PASS, final independent reviews and fmt/Clippy/build
  PASS. Gate caught and corrected a new hidden-parent visibility loop and widget ownership cycle;
  fixture now waits for destination allocation without weakening geometry assertions.

- P346 install chooser final runtime and review gates pass.
  P349 grid-priority coalescing and P350 Storage feedback pass source review, runtime gates pending.
  P351 Collections keyboard parity is isolated. Consolidated build suite has not yet run.
- P347 cross-check's cancellation issue is corrected in P348; no new material auth/session/setup
  issue was found in that bounded source review. This is not an exhaustive security assessment.
- Gungeon Depot launch rendered the actual game menu; normal window-close ended EtG.exe. Details
  displayed completed setup history and Close remained accessible. Planned/recorded counter totals
  differ; potential clarification is deferred rather than treating approximate counters as corruption.

- Gungeon's Depot reinstall completed all declared prerequisite setup successfully before the
  attempted cancellation interaction; this is successful reinstall evidence, not a cancellation pass.
  Conservative transfer/storage budget reservation7GB of40GB after repeated Gungeon acquisition.

- Gungeon uninstall completed through its own Yes/remove, Yes/keep-saves and final OK prompts.
  Ludomere restored Download without reopening details and retained its offline archive; external
  prompt guidance and stale green Download styling are corrected in P344/P345; live retest pending.

- Real Gungeon archive-only deletion/re-download verified: one confirmation removes one archive,
  restores single Download/counts while preserving installed Play; explicit library Download
  transfers/registers382.7MB, restores archive menu/checkmark/counts, and does not run installation.
  Subsequent Depot tests raise the conservative aggregate reservation to7GB of40GB.

- Deferred proposals: clearer drive-wide capacity wording, same-destination Move enablement,
  nested Genre scroll discoverability, initial Files page synchronous reads and grid size rebuilds.
  Missing-library creation needs mount-identity evidence; no speculative automatic recreation added.

Source findings are proposals until assigned, implemented and verified. Full interactive or game
coverage is not claimed from source review alone. Reports: `/tmp/ludomere-r111-{ui-critic,performance,reliability,depot-auth}.md`.
