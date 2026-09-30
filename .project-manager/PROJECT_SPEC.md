# Project specification

## Diagnose and fix failing build — 2026-09-30

R64 user: "The build is failing. Figure out why and correct it." Reproduce from actual build
output, correct the evidenced cause and verify the affected build path without weakening checks.
Build type is being clarified while recent CI/logs are inspected. Preserve R60–R63; no unrelated
cleanup, host installation, dependency modification or new publication requested.
User confirms both local pacman package build and GitHub Actions/PR checks fail. Reproducing the
documented package build and its full checks is now in scope; do not install the produced package.
Continue the previously authorized branch/PR delivery with this packaging correction so remote
checks receive the corrected source; no merge, host install or manual CI rerun.

## Simplify Proton Settings — 2026-09-30

Follow-up R63: Rename the Settings tab to "GOG Online Services" and its update button to
"Check for Updates"; add a small margin above that button to separate it from Official Release.
Include in the same current publication; no other Comet behavior change requested.

User: "The Proton settings should get the same treatment as the onboarding. When the user changes
the proton version there, set it as selected automatically. Only show the folder choice if custom
is selected. Proton downloads should always be shown there. Include a section at the bottom for
the Steam Runtime. Do this and run tests, then push and update the PR."
- R62: Apply automatic explicit selection and conditional Custom picker to Proton Settings, retain
  detected/saved choice and Custom-last behavior, keep Proton acquisition visible, and put Steam
  Linux Runtime readiness/acquisition below it. Reuse the wizard's automatic checks and explicit
  downloads, with errors and retry. Preserve per-game choices, onboarding, and prior preferences.
- Focused tests and relevant validation now expressly authorized for this change, followed by
  commit/push and PR6 update. No full suite, package, real account or game execution requested.

## Publish onboarding wizard — 2026-09-30

User: "Commit and push, then update the PR to reflect the new additions." Publish the current
R60/R61 wizard and refinements to the fork and update existing upstream-main PR6, retaining its
feature/fix coverage concisely. No new implementation, tests, builds or package requested.

## Simplify welcome wizard controls — 2026-09-30

Follow-up: "The custom proton directory should not be selected by default. It should be placed at
the bottom of the list." Preserve saved selection; otherwise prefer detected versions, keeping
Custom an explicit final option. User explicitly requests no tests for this correction.

User requests these corrections to R60:
- R61 Step 2 description: "Choose the directory where your games will be installed."
- Step 4 title: "Select Proton Version". Description (user's subsequent text-only correction): "Select the version of Proton you wish to
  use by default below. This version will be used to launch all Windows games. You may
  override this choice on a per-game basis later." Apply user selection changes automatically;
  remove the Apply button. Add "Custom Proton Directory" to the selector and show a directory
  chooser button only for that option. Remove the Windows-later checkbox. Show acquisition only
  when no existing Proton versions are detected, titled "Download Proton", description:
  "No Proton versions have been detected on your system. You may browse for and download Proton
  runtimes here."
- Step 5 title: "Verify Steam Linux Runtime". Description: "Proton also needs a Steam Linux
  Runtime. If it has not been detected on your system, click the button below to download it."
  Automatically check on entry; disable download when found and show "Steam Linux Runtime found!
  You're all set!" Otherwise enable download and show "Click the button below to download the
  missing runtime." Remove Check Windows requirements, Windows-later checkbox and their existing
  explanatory text. Keep explicit downloads and Skip for now.
- Clarifying only Skip on Steps 4/5: save folders/defer Windows/continue to sign-in versus existing
  close-without-folder-save behavior. Independent copy/selection/readiness work may proceed.
  Preserve prior uncommitted wizard work; no package/publication/schema/dependency change requested.
- Optional Skip question remains unanswered; manager explicitly states preserving the already
  accepted close/discard-folder-drafts/Finish-later behavior meanwhile. This is continuity of prior
  behavior, not acceptance of the proposed new save-and-continue option. A later answer may steer it.

## Guided first-launch onboarding — 2026-09-30

User: "Update the initial launch setup modal which configures defaults and then prompts for GOG
login to an onboarding wizard. Greet the user, say welcome, and guide them through each individual
step of the initial setup process."
- R60: Replace the all-in-one initial setup form with a welcoming sequential wizard, individual
  existing setup steps, clear progress and Back/Next navigation. Retain folder pickers/one-way
  download suggestion, readable Proton paths, explicit component acquisition, prefilled settings,
  validation/durable save and optional GOG sign-in only after defaults are completed.
  Previously agreed Windows deferral remains, as confirmed below. No new components,
  account access, forced re-onboarding, schema, packaging or publication requested.
- User confirms: "Yes—keep Skip and Finish setup later (Recommended)."
  `{"user_answers":{"windows_setup_deferral":"keep_skip_and_finish_setup_later"}}`

## Publish prefix cleanup and condense upstream PR — 2026-09-30

User: "Commit and push, then update the PR with all the new features and fixes. The PR text is also
very long, so try to be a bit more terse in your feature descriptions without omitting any feature
or fix mentions." Publish R59 and update existing upstream-main PR6 to cover the full current fork
diff, retaining every feature/fix mention in concise Markdown. No merge, new implementation, package
build or redundant full-suite run requested.

## Delete a game's prefix on uninstall — 2026-09-30

User: "Uninstalling a game should also delete its prefix directory."
- R59: Confirmed game uninstall, including incomplete-game removal/reset, removes that game's
  managed compatibility prefix along with its game files. This supersedes earlier prefix retention
  on uninstall. Explain removal of saves/settings stored inside that prefix in the confirmation.
  Preserve external saves, other games' prefixes, Proton installations, durable Ludomere preferences/
  activity and optional downloaded-file deletion. Profile reset behavior is unchanged. No actual
  user-file deletion, game/helper execution, new package, commit or push requested in this task.

## Commit and push accumulated fixes — 2026-09-30

User: "Commit what you have and push." Publish current R53–R58 source, tests, documentation and
coordination records to the configured fork branch with a comprehensive accurate commit message.
No new features, package build, full-suite rerun, force push or upstream mutation requested.

## Fresh Witcher Depot preparation failure — 2026-09-30

User removed downloads/install directories and reset the profile, then received:
"Could not prepare required Depot components: wire depot manifest has an invalid small-files
container index. Retry preparation or choose Offline installers and extras."
- R58: Diagnose and correct this Depot preparation failure while preserving valid container/file
  associations, required dependency integrity, safe paths and explicit offline fallback. Preserve
  prior uncommitted work; relevant tests only, no actual user-data deletion or game execution.

## Controller failure after initial correction — 2026-09-30

User subsequently confirms BIT.TRIP detects the controller with automatic correction, and Witcher
detects it with the four explicit DLL overrides. Full profile reset removes those overrides and
Witcher detection fails again. User asks for explanation and feasibility of detecting required
overrides; no new repair/reset policy or implementation is authorized by this question.

User reports the controller remains undetected in BIT.TRIP Runner and Witcher 3. R57 remains
unresolved for actual hardware. Verify applied launch policy, runner version and runtime/device
visibility before further defaults. Prior source/fixture checks are not hardware acceptance.
Narrow read-only diagnostic access and exact latest test settings are being clarified.
User authorizes the offered read-only launch logs, selected Proton/DLL settings, XInput-related
prefix entries and controller device/permission metadata; exclude credentials, saves and raw input.
User confirms latest test used cargo run with DLL overrides unchanged.
`{"user_answers":{"controller_diagnostics":"scoped_read_only_approved","latest_controller_test":"cargo_run_overrides_unchanged"}}`

## DLL overrides and controller investigation — 2026-09-30

User requests DLL override support similar to Lutris, and investigation/correction of clear
deficiencies preventing an Xbox controller working in Ludomere's Wine/Proton games. Witcher 3
and BIT.TRIP Runner do not see it; user confirms Witcher 3 controller works through Lutris.
- R56: Provide editable DLL load-order overrides integrated with existing Windows compatibility
  settings and launch behavior. User chooses per-game only, with the five offered modes.
  Preserve built-in compatibility fixes and explicit user intent; no downloaded DLLs requested.
- R57: Compare relevant Ludomere launch/environment behavior with unchanged UMU/Proton and Lutris,
  investigate controller visibility, and correct evidenced own-code deficiencies. Connection type,
  comparison: USB Xbox; Lutris GE-Proton10.29 works with Steam both running and closed.
  Generic Arch behavior required; no
  speculative host drivers/udev/permissions changes or dependency patches. Preserve existing work.
No new package/commit/push requested; use relevant tests and proportionate independent review.
`{"user_answers":{"dll_override_scope":"per_game_only","controller_connection":"usb","lutris_runner":"GE-Proton10.29","steam_comparison":"works_running_and_closed"}}`

## Immediate action state, sidebar colors and unrestricted sign-out — 2026-09-30

User requests three follow-ups after the push:
- R53: Correct the game-detail primary action remaining stale after file operations. Completion
  must update the current detail view in place without navigating away/reopening the game.
- R54: Game-list text is green while running (user follow-up), blue while downloading, white
  when installed, grey otherwise, in that precedence order. Update immediately with operation state.
- R55: Downloads or failed/in-progress operations must never prohibit logout. Interrupt work
  safely and retain sufficient state for resumption/recovery after exit or later login; do not
  require manual repair merely to sign out. Preserve account isolation and filesystem integrity.
Interview settled: ordinary logout retains recovery state; optional full profile reset retains
files but discards automatic resume records/settings, as before. Running games remain running
on sign-out, including when full reset closes the client. Interrupt downloads/setup work safely.
`{"user_answers":{"reset_interrupted_work":"full_reset_discard_automatic_resume_retain_files","running_game_signout":"keep_running"}}`
No package build, new commit/push or unrelated redesign requested. Use focused tests for changes.

## Game verification and publication — 2026-09-30

User confirms the existing Witcher 3 installation runs, BIT.TRIP Runner successfully runs, and
Coffee Talk was downloaded and runs. This accepts those reported game paths, including the
BIT.TRIP certificate/dependency retry; it does not establish every game/service/platform gate.
User explicitly requests committing current changes and pushing them to the configured fork.

## Persistent DirectX certificate failure — 2026-09-30

User retries R52 and again receives curl77 naming `/etc/ssl/cert.pem`; successful dependency
receipts skip OpenAL/MSVC execution. Reopen the integration diagnosis: establish why the intended
CA configuration did not reach/effect the failing downloader before another correction. Preserve
TLS and user trust, no prefix reset/dependency source modifications. Relevant tests only.
User confirms cargo run after closing Ludomere and all three certificate variables absent in the
launch terminal. Subsequent instrumented retry reports inherited SSL_CERT_FILE/SSL_CERT_DIR and
skips the managed default. Diagnose launcher/library additions rather than treating presence alone
as proof of user customization.
User additionally requires the solution to work on generic Arch installations, not just their
machine. Discover and translate system trust paths without user-specific paths or certificate
contents; retain explicit trust selection and secure verification.

## DirectX certificate failure — 2026-09-30

User supplies BIT.TRIP Runner log: OpenAL and MSVC2010 proceed successfully; DirectX Winetricks
download fails with curl77, unable to set certificate file `/etc/ssl/cert.pem` under UMU/GE-Proton.
- R52: Diagnose and correct the demonstrated certificate/download integration failure without
  disabling TLS verification, modifying dependency source or resetting the user's prefix. Preserve
  successful setup receipts/Resume and distinguish unrelated prefix warnings from the fatal cause.
  Use focused tests only; actual runtime/game execution is not authorized by this pasted log.

## Verification policy — 2026-09-30

User: "don't run the full test suite if you only change a small amount of code. Run relevant tests
only. The full suite should be reserved for the build process."
- Small changes require relevant tests and proportionate checks; reserve the full suite for the
  build process. This supersedes earlier routine full-suite requirements for narrow edits.

## Dependency endpoint failure — 2026-09-30

User reports BIT.TRIP Runner installation fails with `No supported official dependency download endpoint`.
- R51: Diagnose and correct the dependency endpoint selection against actual official GOG response
  shape; preserve validated origins, hashes, cancellation and existing Resume. Regress the real
  response-to-transfer path rather than only synthetic direct chunk requests. No real installer/
  game execution, account access, package or commit requested.

## GOG dependency resolution implementation — 2026-09-30

User authorizes the P135 proposal: "Implement proper GOG dependency resolution."
- R49: Resolve the complete required GOG dependency set from its official catalog before game
  transfer, with actionable aggregate unsupported/unavailable failures and explicit offline choice.
  Acquire verified reusable dependency artifacts, support catalog-declared executable/MSI and
  game-local content using typed plans, and retain narrowly evidenced Wine compatibility exceptions.
- R50: Apply required setup through unchanged UMU and selected Proton/prefix, with visible progress,
  cancellation and full sanitized errors; track success per prefix identity and dependency revision,
  retry unfinished setup without re-downloading intact game files, and never silently skip requirements.
  Existing Resume records must resolve missing plans safely. Preserve recovery generation/account
  guards, native independence, successful-only installed markers and existing working installations.
- Required dependency acquisition is part of the explicitly selected game installation/update;
  present required components in the plan and preserve existing Proton/runtime consent. Offline
  installers remain an explicit alternative, never an automatic second payload download.
- Verify representative real catalog metadata plus inert/local HTTP failure, retry, cache, unsafe
  path and recreated-prefix fixtures, independent QA/security and repository required checks.
  No host package/commit, real account or real installer/game execution is requested.

## Recurring Depot dependency failures — 2026-09-30

User reports BIT.TRIP Runner fails with `unsupported required GOG dependency openAL` and asks
whether recurring installation failures can be avoided, comparing Steam and Lutris.
- R48: Diagnose this exact failure and assess a systematic dependency-handling solution using
  primary evidence. Distinguish Ludomere mapping refusals from runtime installer failures; explain
  feasible prevention and limitations. Broader replacement/runtime execution policy needs a concrete
  proposal before implementation. Preserve prior fixes and unmodified dependency policy.

## Incomplete-game removal and visible running logs — 2026-09-30

User reports:
> Enter the Gungeon now launches. Show the uninstall button even for games mid-download or in an error state. The user should be able to delete files and reset the game state even if a problem occurs with the files. I also do not see where to view the log window when a game is running.

- R46: Make confirmed uninstall/reset available for active-download, incomplete and failed games.
  Safely quiesce affected work before removing managed game files and transient operation state;
  preserve durable activity/preferences and protected payload boundaries. Do not require a healthy
  installation marker or working vendor uninstaller to recover from a failed installation.
- R47: Make running-game log access visible from game controls. Existing Logs tab is currently the
  only entry; clarify separate live window versus direct tab navigation before dependent UI work.
- User confirms real Enter the Gungeon launch after R45, settling that game's immediate acceptance.
  This does not establish every runtime/game/service gate. Optional downloaded-file cleanup choice
  and log-window presentation are being interviewed. No actual user-file deletion by agents.
- Interview settled: keep downloaded-file deletion optional, retaining its existing unchecked
  default. User found the Logs tab and explicitly withdrew R47: "Disregard this feature request."
  No log UI changes belong to this task. R46 remains active.
- Residual-file interview explicitly warned that untracked files may include saves; user answered
  "Delete them too." R46 recovery may delete all remaining files, including untracked files and
  in-directory saves, in the exact confirmed game directory. Explain this in confirmation. Prefixes,
  external saves, unrelated directories and durable preferences/activity remain protected; managed
  downloaded installers/extras retain their separate optional unchecked deletion choice. No recovery
  folder is requested. Existing healthy idle normal uninstall behavior remains unchanged.

## Gungeon setup crash and inactive View Error — 2026-09-29

User reports:
> Enter the Gungeon opens an installer, and when I select OK on the language, it crashes and Ludomere reports the following error: Enter the Gungeon: Depot operation failed / GOG setup action failed. Also, the View Error button does nothing. There is a real problem with dependency handling here.

- R45: Investigate and correct the demonstrated Depot setup invocation failure and restore usable
  View Error details. Audit the full required setup invocation contract rather than assuming another
  missing runtime. Preserve required actions and unmodified dependencies; no success claim without
  evidence. Existing relevant operation-log authority persists; no real installer execution authorized.

## Gungeon resumed Depot setup executable — 2026-09-29

User reports deleting local files manually and downloading again through Resume, followed by:
> Enter the Gungeon: Depot operation failed
> ExecutableMissing("/home/chris/GameFiles/ludomere/enter_the_gungeon/galaxy_enter_the_gungeon_2.11.0.13.exe")

- R44: Diagnose and correct the resumed Depot installation's missing executable failure, preserving
  required setup, safe path resolution and resumability. Distinguish reproduced source defects from
  assumptions about manually removed files. Preserve R42/R43 changes; no package/commit requested.

## Enter the Gungeon diagnostics and game logs — 2026-09-29

User reports:
> I installed Enter the Gungeon and got this error: Enter the Gungeon: Depot operation failed; see Downloads //// That's not a very helpful error. It looks like Ludomere has real problems installing required dependencies. Are you able to run Ludomere and inspect operations yourself? I also want a log for running games, similar to what Lutris has.

- R42: Investigate the uninformative Enter the Gungeon Depot failure and improve diagnosed error/
  dependency problems. Distinguish actual failure evidence from assumptions; isolated app execution
  is permitted. Access to the user's relevant operation logs is pending explicit answer; no credential
  access or real game/helper execution authorized by the current investigation.
- R43: Provide usable game-run logs. Inspect existing capture/view behavior before redesigning;
  live view plus saved per-launch logs versus saved-only preference is pending clarification.
- Interview settled: user permits inspection of relevant local Enter the Gungeon operation logs,
  excluding credentials and with sensitive values redacted from reports. User selects live,
  scrollable game log view plus saved per-launch logs, Copy and Open log folder actions.
  No authorization to execute actual user games/installers or inspect credential stores.
- User explicitly authorizes the isolated capture test after automatic approval rejection: "Yes,
  run that isolated logging test." Only the temporary synthetic stdout/stderr script with a one-
  second wait and exit may run through Ludomere; no real game or compatibility helper is authorized.

## Identified Witcher 3 Depot dependency — 2026-09-29

User supplied the complete error after R39:
> The Witcher 3: Wild Hunt — Remastered: Depot operation failed
> unsupported required GOG dependency MSVC2019

- R41: Resolve the identified MSVC2019 dependency mapping in the existing Depot Windows setup
  flow, using verified upstream runtime semantics. Preserve required-dependency refusal for unknown
  identifiers and existing prefix/resume/commit behavior. No dependency source modification.
- Follow-up confirmed by user: the next required identifier is MSVC2019_x64. Include this exact
  architecture variant in R41, using the existing runtime's x64 support and deduplicated setup.
- User subsequently reports: "The game does launch now." This is live Witcher 3 launch acceptance
  after the dependency fixes, not acceptance of unrelated games or every service feature.

## Installation errors and decimal download units — 2026-09-29

> When a game installation fails, include full details in the alerts, don't redirect to downloads. Downloads didn't have any further information. When attempting to launch Witcher 3, I got a truncated error, which reads: "Installation Failed unsupported required GOG depe..." [Image #1]. Also, change the download units from MiB to MB, etc. Nobody wants to see Mibibytes or Mibibits.

- R39: Preserve full actionable installation/launch failure details in readable alerts/notification
  history; never redirect to Downloads on failure. Trace the reported unsupported-required-GOG-
  dependency failure without guessing or bypassing necessary dependency checks. Use existing
  notifications, readable/copyable text and session/privacy guards. Screenshot supplied in chat.
- R40: Show download sizes and transfer rates using decimal byte units (kB/MB/GB/TB and per-second
  equivalents), converting with1000 rather than relabeling binary magnitudes. Preserve byte-based
  storage/accounting; no transfer/protocol changes.

## Package failure and Comet availability — 2026-09-29

> The build failed. Also, based on the settings view, I think the Comet integration isn't functional. It may need to be compiled from source if there is no release available to download.

- R38: Diagnose and correct the failed documented pacman build and Comet availability/integration
  problem. Obtain exact build and Settings errors; independently inspect helper discovery and
  official release availability. A source build is conditional on no suitable official release,
  not authority to patch dependency code. Preserve prior unmodified-Comet policy unless the user
  approves a specific necessary change. Verify corrected build without installing on the host.

## Sidebar hiding, notifications and Comet clarity — 2026-09-29

User request:
> Right clicking on a game in the games list and choosing "Hide Game Locally" does not hide the game. In the game detail view, clicking the gear and choosing "Hide Game Locally" does hide the game. Messages which appear in the lower right corner of the client are frequently cut off and therefore not readable as they cannot be expanded. Messages queued for user notification should be placed into a notifications list, openable in a modal with a button in the lower right where the notifications are now. Clicking this button should open the notifications modal. Hovering on the button should show a popover with the full text of the most recent notification. After ten seconds, the truncated notification text should disappear, leaving only the button. "Show Hidden" in the filters list should appear under the "Library" category rather than the "Operating System" category, where it is now. The GOG online services tab in settings is a bit confusing. The description of what Comet is is fine, but the availability and installation is confusing. Please clean that up.

- R35: Sidebar right-click Hide/Unhide must use the same persistent action as detail Manage and
  immediately update normal browsing/search/collections in place. Move Show hidden into Library.
- R36: Replace lower-right transient messaging with notification history accessible through a
  persistent button/modal. Hover shows full latest message in a popover; compact latest text hides
  after10 seconds without removing the button/history. User selected current-session-only history
  of results, warnings and errors; keep live progress separate. Preserve centered Downloads and
  unrelated synchronization controls. No persistent notification database.
- R37: Clarify Comet Settings availability/installation/update presentation, retaining its existing
  explanation, unmodified official helper behavior, confirmation and cancellation. Distinguish local
  Comet, available updates and automatically managed peer libraries. No dependency modification or
  new installation authority.

## Database contention and action refresh investigation — 2026-09-29

> Investigate database problems like this and propose a fix to reduce the locking behavior where not necessary. I've also noticed the UI doesn't update immediately upon completion of a game uninstall, installation, and other actions involving game files and downloads. It seems to wait upwards of ten seconds to update the games list game color and detail view button to represent the state of the game (play, download, install, etc).

- R33: Investigate unnecessary database contention and delayed local-action presentation; propose
  a concrete correction before implementation. The reported database error appears in the uninstall
  confirmation before pressing Uninstall. Preserve filesystem-authoritative installedness, durable
  user data, schema validation and in-place asynchronous UI behavior. This stage is analysis only.

Implementation authorized:
> Definitely fix those problems. 1-5. WAL mode can wait.

- R34: Implement the five P104 corrections: current-schema opens without initialization writes;
  progress-only UI updates without database queries and coalescing before expensive work;
  affected-game filesystem-authoritative completion refresh shared by actions/colors/filters;
  shared datasets and bounded broader refreshes; retryable previews/refresh errors preserving known
  state and truthful distinction between file success and bookkeeping failure. WAL/journal mode is
  unchanged. Preserve schema25/dev7 validation and atomic migrations, user data and account/reset guards.

Source: user messages in the repository conversation, preserved below. Material specification
interview decisions are settled; implementation is authorized by the original request and answers.

## User-supplied request

> Let's fix up this documentation and testing environment setup, along with providing necessary components to enable core features without additional downloads. Use the $project-manager skill to accomplish this, and interview me about specific requirements. A build script which outputs an AppImage would also be good. The ability to download proton versions and choose the one to run with should be an added requirement, and potentially access to already-installed proton versions, including GE versions installed via ProtonPlus or similar programs.

## Clarifications — 2026-09-28

User response concerning bundled components:

> I think specifically I want the program to use proton versions installed by other programs, and to decouple umu as a herd requirement. Most users will have a method of acquiring proton versions, but it may be advisable to include umu in the package to allow ludomere to invoke its own proton version downloads. What is Comet?

User response concerning supported environments:

> What are the consequences of supporting more than Arch?

The interviewer proposed this Proton scope:

> download UMU-Proton and GE-Proton releases, choose a global default, override it per game, automatically discover existing Steam/ProtonPlus installations, and allow manual folder selection. Externally managed versions would be used without modifying or deleting their installations.

The user accepted it with this condition:

> I accept this scope, provided umu is packaged with ludomere to allow the acquisition of proton versions.

The user selected testing setup C: local setup/check scripts and documentation, a reproducible
container for builds and automated tests, and GitHub Actions for checks and AppImage artifacts.

The user requested that delivery priorities and limits be expanded into separate questions.

### Further user clarifications

Regarding the launch backend:

> What are my options for launching proton? Is umu the best option?

Regarding packaging and verification:

> If we support Ubuntu, how do you plan to test that? What are my other options for packaging? Is a pacman package reasonable?

The user selected Comet option A: bundle Comet and its Windows service helper, preserving the
existing integration without its first-use component download.

The user specified delivery order:

> Proton management first, then packaged builds.

The user specified the download policy:

> Downloads are acceptable during the build process. Proton should not be packaged with ludomere. If the user does not specify a proton version and one can not be automatically detected, ludomere may then offer to download a proton version for the user.

### Settled backend and packaging decisions

The user supplied these answers:

> 1. Bundle UMU, yes.
> 2. If missing, Ludomere should offer to download it.
> 3. GE, then UMU, then Valve
> 4. Ask for a replacement.
> 5. Drop Ubuntu support. ArchLinux is the only supported platform for now, so a pacman package is the target.

Answer 1 selects bundled UMU as the Windows launch backend. Answer 2 concerns the Steam Linux
Runtime. Answer 3 sets automatic Proton family priority. Answer 4 concerns an explicitly selected
Proton installation that disappears. Answer 5 supersedes the earlier AppImage direction, including
the CI artifact format; Ubuntu and other distributions are outside the supported platform scope.

### Final interview answers

> 1. When first choosing the proton runtime, save that as the default runtime and allow the user to override it for individual games.
> 2. A
> 3. A
> 4. I can help with real-game verification, yes.

Answer 2 selects available stable GE-Proton and UMU-Proton releases, including older releases.
Valve Proton is discovered from existing installations rather than downloaded by Ludomere.
Answer 3 selects native and Flatpak Steam locations, additional Steam libraries, and standard
Heroic/Lutris Proton locations, including versions installed there by ProtonPlus. Manual folder
selection remains available. Answer 4 commits to guided user real-game verification without
sharing credentials.

## Current accepted requirements

### Follow-up discussion — 2026-09-29

The user initially approved an explicit peer download offer, then requested bundling all peer
libraries. Before implementation, the user paused that direction to ask:

> Hold up. Are these peer libraries readily available for download? Would explicitly offering their download to the user, along with a description of what they are for, be a problem?

The user then clarified:

> I think bundling the proprietary files is fine. Users are already integrating with a proprietary service. Figure out the license part.

Following the licence review, the user chose the explicit-download design:

> Okay, go with that. Explicitly prompt the user to download the files.

The user added:

> Ludomere should check if a more recent version of Comet and its dependencies is available on launch, and have a button to check manually.

This supersedes the proposed peer bundling. Comet and its service helper remain packaged; proprietary
peer files are acquired from GOG only after an explicit, explanatory user prompt. Check Comet and
its support components on application startup and via a manual control. Startup checks must obey
the existing no-background-dialog/focus rule. No silent component installation/update is authorized.

The user resolved that question:

> Offer an in-app Comet update after confirmation

At this point the design required application-owned Comet updates, preservation of a working version
on failure, and an explicit peer-download boundary. The peer boundary and modified-dependency
approach were subsequently rejected below; package ownership and Comet update confirmation remain.

Further steering:

> If comet is downloading its dependencies automatically, let's trust its author has permission to do so.

The user accepts Comet's upstream dependency-download permission without a separate investigation.
This is a project decision, not newly established legal evidence. The user then selected:

> Ask before every dependency download or update

This was subsequently superseded by the user's explicit dependency policy:

> Have you modified Comet? If so, don't. If comet's inbuilt functionality is to automatically download peer dependencies silently with no option to override that, so be it. Don't recompile dependency code to satisfy my requirements. Just tell me if a dependency doesn't work the way I want and then I will decide if I want to modify and rebuild it.

Use unmodified official Comet binaries. Remove the local Comet patch/source build and custom
peer-download enforcement. Automatic upstream peer acquisition is accepted, superseding per-peer
confirmation and Ludomere-owned peer verification/rollback promises. Explain that behavior in the
UI/documentation. Comet itself still has confirmed in-app binary updates and startup/manual metadata
checks. Before modifying or rebuilding a dependency to resolve any future behavior conflict, explain
the conflict and let the user decide. Do not infer authority for such changes from feature requirements.

The manager disclosed that Ludomere's UMU adapter replaces two Python acquisition hooks at launch
without editing or rebuilding UMU files. The user explicitly selected:

> Keep Ludomere's UMU adapter and download prompts

The UMU adapter and explicit Proton/runtime acquisition policy therefore remain approved. This does
not authorize a Comet patch or other dependency modifications.

- R01: Correct local build/test requirements documentation and provide local setup/check scripts,
  a reproducible build/test container, and GitHub Actions checks and package artifacts.
- R02: Use bundled UMU as the Windows launch backend, removing the separately installed host UMU
  requirement. This supersedes the repository's hard-coded /usr/bin/umu-run contract for this work.
- R03: Discover existing Proton installations, including Steam/ProtonPlus-managed versions, and
  allow manual folder selection. Search native and Flatpak Steam, additional Steam libraries, and
  standard Heroic/Lutris locations. Do not modify or delete externally managed installations.
- R04: Offer global Proton selection and per-game overrides. Automatic family priority is
  GE-Proton, then UMU-Proton, then Valve Proton. Ask for a replacement when an explicitly selected
  installation disappears; do not silently fall back. The first chosen version becomes the saved
  application-wide default. It remains the default until changed; games may override it.
- R05: Provide UMU-Proton and GE-Proton downloading. Never bundle Proton with Ludomere. When no
  Proton version was specified and none can be detected, offer the user a download. Offer available
  stable releases, including older releases. Valve Proton is discovery-only.
- R06: Offer to download the Steam Linux Runtime when it is missing.
- R07: Bundle Comet and its Windows service helper, preserving the current integration without
  downloading those bundled helpers on first use. Proprietary GOG peer libraries are excluded from
  the package and managed by unmodified Comet as described in R11.
- R08: Support Arch Linux only and produce a pacman package. Ubuntu support and AppImage output
  are superseded. Retain the repository's existing x86-64 package target unless the user changes it.
- R09: Implement Proton management first, then packaged builds. Build-time downloads are allowed.
- R10: The user will assist with guided real-game verification on Arch. Credentials remain with
  the user. Do not mark interactive or real-game acceptance passed without actual evidence.
- R11: Use unmodified official Comet binaries and explain that upstream automatically acquires and
  updates proprietary peer libraries. No peer bundling or custom Comet patch/source rebuild; its
  upstream automatic acquisition is accepted. Preserve normal operation when Comet is disabled.
- R12: Check for newer Comet and its dependencies at Ludomere startup and through a manual button;
  report status in place without unsolicited dialogs or silent downloads. Offer in-app Comet updates
  after confirmation using verified official assets; install in application-owned storage and
  preserve package-owned helpers. Dependency behavior conflicts must be explained before any
  proposed dependency modification or rebuild.

## Acceptance and repository constraints

### Library loading — approved 2026-09-29

User specification:

> Find ways to improve this library load time. The priority for loading data should be: game names, then game images for the grid view. Game details views should download the necessary data when opened for the first time, with a small loading indicator on each widget as it loads. If a game's data is not loaded, buttons to download and play the game should remain available and usable.

> Can we request more than ten games at a time? Filters should show a loading indicator if their required data is not yet available.

Implementation authorization:

> Implement the identified and suggested improvements with requests for 50 games at a time.

- R13: Fetch lightweight core products in batches of 50; names precede grid covers, with bounded
  work and cached results reused. Remove the one-event-per-50ms UI bottleneck and avoid repeated
  full-grid rebuilds. Prioritize visible covers and move image file/decode work off GTK.
- R14: Load rich detail data on first opening, independently per section with small loading and
  failure/retry states. Update in place without changing focus, selected page, tab or scroll.
  Deduplicate requests and prevent stale/account-crossing results from affecting current UI.
- R15: Play remains usable from local installation state while remote metadata is unavailable.
  Download opens its chooser promptly and fetches only required acquisition data with loading
  feedback; decorative data does not gate actions. Preserve existing prerequisite and update rules.
- R16: Show loading for filters missing required metadata and fetch that data in the background,
  prioritizing active filters. Unknown data must not count as a confirmed non-match; partial results
  are labelled and failed loads offer retry. Available local filters remain usable.
- R17: Partial cache updates preserve ownership, pack-derived DLC entitlement, user data, rich
  cached fields, platforms and relationships. Verify cold/warm/offline large-library behavior,
  independent failures/retries, rapid page changes and persistence. No new platform/package scope.

### Logout cache control — requested 2026-09-29

> Also a button to settings to clear the cache when you log out.

- R18: Add a Settings toggle that clears the full profile whenever the user signs out. User chose:
  "A toggle that clears cache whenever I sign out" and "Full profile reset, including settings,
  favorites, tags, playtime, and queue records". Installed games, installer payloads, Proton and
  runtimes remain intact. Toggle defaults off; its copy explains the complete reset and application
  close afterward so old workers/open database handles cannot recreate erased state. Toggling alone
  never clears data. Agents exercise reset only on disposable test profiles, not user data.

### Synchronization feedback and stalled images — requested 2026-09-29

> When logging in after a cache clear, the sign-in process loaded my list of games and started loading images, but seemed to stop after loading the first batch. The remaining grid view images did not load, and the grey boxes in their place did not have a loading icon on them. The loading indicator in the bottom left which indicates the library is being synchronized should be updated to also show the stage of the synchronizing process. Loading games list, loading grid images, loading metadata, etc. The game detail view should also show loading indicators while that data is loading, to show the user the program is not being idle. If a synchronization error occurs, the user should be notified.

- R19: Diagnose and fix cover loading that appears to stop after the first batch following a fresh
  login/cache clear. Keep 50-game core requests and lazy detail loading. Grid placeholders and
  detail widgets visibly indicate pending work; settle to loaded, unavailable or error states rather
  than silent placeholders or indefinite spinners. Bottom-left synchronization feedback identifies
  current real work (game list, grid images, metadata) with progress where known. Surface sync and
  partial failures visibly without unsolicited focus changes; retain usable cached data and actions.
  Follow-up answer: the synchronization spinner "disappeared" when the remaining images stopped.

### Screenshot navigation, sidebar icons and footer — requested 2026-09-29

> The browse screenshots modal should allow changing screenshots with the left/right arrow keys. The game icons on the games list are not populating until the game is clicked on - these images should populate in parallel with the grid images. The "Retry" button in the lower left opens the downloads view, but does not initiate a retry button. The retry button for the hero image on the game detail view does seem to work properly. Then the process is finished, the "Signing in to GOG..." text remains in the lower right. Looking closer, it appears the full bottom-bar is a button which opens the downloads view.

- R20: Screenshot modal Left/Right keys navigate with the existing previous/next semantics.
  Sidebar game icons acquire alongside grid covers without opening details, with bounded work and
  persistent cache. Footer Retry performs synchronization retry without navigating to Downloads;
  Downloads has a distinct navigation control. Clear obsolete sign-in status after completion.
  Preserve independent hero retry, lazy details, 50-game batches and page/focus invariants.

### Failed-image retry and initial setup — requested 2026-09-29

> The Retry button on the footer retries the full synchronization, not just the failures. That button should only retry the failed images. A full re-sync should be a button in the options. It should also be possible to dismiss the failure notification. The "Downloads" button should be centered on the footer, with the "Signed in to GOG" text on the right. The "install after download" button should be checked by default. When attempting to install a game after downloading, a "Windows Compatability" modal opens, and informs me UMU is unavailable. The whole "Windows Compatibility" modal should really not appear like this. If anything, these settings should all assume the defaults set in Ludomere. When Ludomere launches for the first time, it should guide the user through the necessary steps to set up all required defaults.

- R21: Footer Retry retries only failed images, without ownership/core product resynchronization or
  unrelated metadata work. Full resynchronization belongs in Settings/options. Failure notification
  is dismissible without falsifying error state. Center Downloads in the footer and keep sign-in
  status on the right. Install after downloading is selected by default and must work as labelled.
- R22: Use saved Ludomere defaults for ordinary Windows installation/launch rather than presenting
  the full Windows compatibility modal each time. Diagnose missing bundled UMU in the development
  flow. Provide first-launch guided setup of necessary defaults, retaining explicit consent for
  missing Proton/runtime downloads, external Proton discovery and per-game overrides. User chose
  folders, default Proton, runtime checks/download offers and optional GOG sign-in; allow deferring
  Windows setup with Finish setup available; existing profiles get a one-time guide prefilled with
  current settings. No dependency modifications or native-Linux dependence on Windows components.

Interview answers:
```json
{"user_answers":{"setup_scope":"folders_proton_runtime_optional_gog","windows_setup_deferral":"allowed_with_finish_setup_action","existing_profiles":"one_time_prefilled_guide","constraints":{"platform":"Arch Linux x86-64"},"preferences":{"downloads":"explicit_user_confirmation"}}}
```

### Setup folder selection and final sign-in step — requested 2026-09-29

> Adjustments to the initial setup modal: 1. There should be a direcotry select modal available for both paths. The download folder should automatically update to /chosen/directory/downloads when the game folder is chosen, but still allow the user to change the download folder, which should not change the game folder path. // 2. The path to the proton file is not fully visible. It should be possible to see the full path when selecting the version. // 3. GOG sign-in should occur after other settings are completed. This should be the last step, and be presented in a different modal.

- R23: Both setup folder fields have directory pickers and remain editable. Choosing a game folder
  sets its download suggestion to that folder's downloads child; changing downloads is independent.
  Preserve existing prefilled paths until the user changes them. Full Proton paths are readable during
  selection. Remove sign-in from the settings form; after successful completion/save, offer optional
  sign-in as the final step in a separate modal. Invalid/failed/cancelled setup must not advance.
  Retain Windows deferral, no silent component downloads and existing preference preservation.

### Download cleanup and detail media — requested 2026-09-29

> In the download window, in optional content, "Extras" should be uncheked by default. The manage menu on a game should include an option to delete downloaded files if they are present, like installer files, goodies, etc. It should also be a checkbox when uninstalling a game. If screenshots fail to download, the navigation arrows appear in place under the library information. [Image #1]. Also, image downloads seem to fail frequently on game detail pages.

- R24: Extras defaults unchecked for new profiles; retain the Settings preference and existing
  saved choices, per user's follow-up answer below. Game Manage offers deleting present
  downloaded installers/extras and related managed downloads, also optional in uninstall. Delete
  downloaded files only, preserving installed payloads/saves/preferences except the normal separately
  confirmed uninstall. Uninstall cleanup checkbox defaults off. Reuse managed-file boundaries and
  exclude active payload work; verify cancel, empty, partial failure and UI refresh.

User answered the Extras preference question:
> Keep the preference, but default it off for new profiles

Interview record: `{"user_answers":{"extras_default":"keep_preference_default_off_new_profiles"}}`.
- R25: Failed/empty screenshot content cannot leave navigation arrows overlapping library information.
  Investigate detail image failures and correct demonstrated request/cache/loading/retry defects,
  preserving lazy independent sections, page/focus, available actions and safe bounded requests.
  Screenshot supplied at /tmp/codex-clipboard-c2D0BW.png; live cause is not established from image alone.

- Preserve native Linux independence, installed payloads, durable preferences, activity, marker
  formats, and repository schema policy. Use existing persistence shape where possible.
- Filesystem traversal, network requests, and process operations run off the GTK thread. Background
  operations update state in place and never navigate or present windows. Offers are triggered by
  direct user actions; Ludomere-owned queued/recovered work must not silently acquire components.
  Comet's automatic peer acquisition is the user-approved exception.
- Validate Ludomere-managed downloads and archives, constrain its writes to application-owned
  storage, and preserve external Proton installations. Upstream Comet's peer downloader retains its
  own behavior. Never shell-concatenate commands or expose credentials.
- Run cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test, and the release/package
  checks appropriate to the changes. Verify discovery, persistence, command propagation, missing
  components, download failure/cancellation, native independence, and packaging payload contents.
- Provide an Arch build/check environment and CI artifact workflow with pinned or constrained,
  verified bundled components and license notices. No release publishing or host package install
  is requested.
- Independent QA and security review are required by the invoked project-manager skill. Outstanding
  interactive or real-game checks remain explicit gates; the manager does not perform implementation.

No time, storage, or artifact-size cap was supplied. Use ordinary scoped project resources and
escalate any material resource or security tradeoff before expanding the work.

## Selected upstream features — authorized 2026-09-29

User selection:
> I definitely want full GOG Depot support as the default download option. I want to be able to hide games locally, I want achievement support, automatic old-installer cleanup, global and per-game update policies, cloud save support including exports, delete selected cloud saves, personal tags.

User clarification and implementation authorization:
> Full GOG depot support should include generation-two depots with offline installer fallback. Generation one does not need to be supported at this time. For automatic updates, I want depot auto-updates on with offline backup downloads and cleanup opt-in. Achievement support should have an achievement browser with existing comet integration. Since my chosen features are all feasable, do implement those. Ask me questions about ambiguities or gaps in feature requests.

- R26: Prefer generation-two Windows Depot acquisition, with explicit offline-installer fallback
  and source choices. Preserve existing install/update/repair/DLC/branch/resume behavior; complete
  installed-language controls and correct Depot primary-action priority. Generation one excluded.
  Existing-profile source-order migration is pending the user's answer; do not alter installed sources.
- R27: Local persistent Hide/Unhide and Show hidden. Exact normal-view/search exclusion and hidden
  auto-update behavior are pending clarification. Preserve hidden state across refresh/uninstall.
- R28: Native achievement browser with cached unlock state/dates/descriptions and progress where
  provided, independent loading/error/retry, offline use and account isolation. Retain unmodified
  Comet integration; no desktop unlock notifications, social features or remote writeback.
- R29: Inherited global/per-game Depot auto-update, offline-backup-download and old-installer-cleanup
  policies. Defaults: Depot updates on, other two off. Implement post-sync/periodic online scheduling,
  targeted fresh acquisition data, running/busy-game exclusion and safe account/reset lifecycle.
- R30: Opt-in automatic removal of superseded managed installer revisions only after complete
  replacement verification, using Trash and existing protected cleanup/operation boundaries.
  Preserve in-use/queued-install files and user payloads; report partial failures accurately.
- R31: Preserve current cloud synchronization/conflict handling and add explicit verified exports
  and selected remote deletion with confirmation/recovery export/revision checks, unchanged-local
  reupload suppression and visible partial errors. No actual user-account mutation during agent tests.
  Installed-only versus uninstalled-game management is pending the user's answer.
- R32: Extend existing local personal tags with assignment removal, global rename/delete and any/all
  filters. Preserve case-insensitive semantics and active-filter coherence; no GOG tag synchronization.

Implementation constraints: selectively adapt upstream 4717092, never overwrite this fork's lazy
loading, setup, reset, UMU/Comet, cleanup or auto-install behavior. Keep public schema25 and one
canonical24→25 migration; consolidate these changes into one new internal development revision and
verify retained local revisions. Upstream same-number development revisions are different shapes,
not implicitly compatible databases. Friends/chat/saved views/custom art/global queue reordering/
bandwidth controls are outside this phase. Arch-only; no package build, commit or push requested.

Final interview answers: change the old default order to Depot-first and preserve customized orders;
hide from normal browsing and keep update policies active; supported installed Windows cloud games
only for this phase. These settle all pending choices above.

```json
{"user_answers":{"depot_format":"generation_two_with_offline_fallback","update_defaults":"depot_on_backups_cleanup_opt_in","achievements":"browser_existing_comet","source_order_migration":"old_default_only_preserve_custom","hidden_games":"exclude_normal_browsing_keep_updates","cloud_scope":"supported_installed_windows"}}
```

Cloud-deletion clarification, 2026-09-29: the user was told that GOG's atomic conditional-delete
guarantee could not be established and that a concurrent upload after the final recheck might be
deleted without being present in the recovery copy. The user selected:
> Include deletion with recovery copy, rechecks and explicit warning (Recommended)

R31 therefore includes enabled selected deletion with verified recovery, revision rechecks and an
explicit confirmation warning to stop games and other cloud clients. Do not claim guaranteed atomic
compare-and-delete. This authorizes the product behavior, not tests against the user's real saves.
