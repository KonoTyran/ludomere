# GOG peer-library redistribution review

Reviewed 2026-09-29 for R07/P45. Manager: /root. Researchers: /root/security (terms) and
/root/compatibility (official artifacts). This is a source-based licensing assessment, not legal advice.

## Finding

No applicable public grant was found for distributing the peer DLLs separately inside Ludomere.
The user subsequently chose an explicit download offer instead of bundling (see PROJECT_SPEC R11).
Bundling is not implemented or license-cleared. An applicable SDK agreement or specific written GOG
permission could establish distribution rights if that design is revisited.

Latest user decision: trust Comet's author concerning upstream dependency-download permission and
use official, unmodified Comet, accepting its automatic peer downloads. This supersedes the earlier
custom explicit-download implementation. No further permission investigation or external contact is
requested; the historical redistribution findings below do not block that accepted integration.

## Primary evidence

- [Current GOG GALAXY licence](https://support.gog.com/hc/en-us/articles/360023617474-GOG-GALAXY-Licence-Agreement),
  section 3.B: a redistribution grant exists, but 3.B.i limits it to the entire application and
  restricts third-party integration. Later conditions require licence inclusion/acceptance,
  preservation of notices and generally unchanged software, with no remuneration. Keeping the
  DLLs separately licensed would not resolve the whole-application/integration conditions.
- [Current GOG User Agreement](https://support.gog.com/hc/en-us/articles/212632089-GOG-User-Agreement),
  11.1(c): prior-permission route for distribution not otherwise allowed; 23.1 lists legal@gog.com.
- [GOG Distribution Terms](https://sites.google.com/gog.com/terms-and-conditions/terms-and-conditions),
  1.4: publisher/game-build rights refer to additional SDK terms. No general independent-launcher
  redistribution permission is expressed there. The additional SDK agreement was not accessible
  without developer-portal access; no account was used or agreement accepted.
- [Official SDK FAQ](https://docs.gog.com/faq/#general-sdk-questions), General SDK Questions 15,
  identifies GalaxyPeer.dll as a client redistributable, with manual inclusion in games for old
  SDK versions. This technical instruction does not specify a launcher distribution licence.
- [Public Windows peer manifest](https://cfg.gog.com/desktop-galaxy-peer/7/master/files-windows.json):
  distribution version 1.2.33.1, eight ZIPs, each containing one DLL. The researcher inspected all
  eight and verified the enclosed DLL hashes. No LICENSE/EULA/NOTICE/README or manifest licence
  pointer was present. PE resources identify GOG Galaxy Peer, version 1.114.12.0 and GOG copyright.
  Embedded third-party notices do not provide a grant for the complete peer DLLs.

Applicability is an assessment of the published terms, not proof that no private or SDK-specific
permission exists. Public download access and copyright attribution alone do not supply the
missing peer-only distribution grant. A direct-user-download design would avoid Ludomere hosting
copies, but is not itself proof of complete licence compliance. The user approved that design after
this limitation was explained.

Current agreement bodies were verified via the public article APIs because the browser tool rendered
only page chrome. Licence body last-update date: 30 January 2026; current articles' API update:
7 July 2026. Archived agreements effective only until 9 March 2026 were not used as current terms.

## Concrete resolution

Obtain either the applicable SDK licence covering this launcher use or written GOG permission
covering the following request. No message has been sent and no proprietary file added to the
repository or pacman package. Inspection downloads remained disposable and were never executed.

### Draft request to GOG Legal — not sent

Subject: Permission to bundle GOG Galaxy Peer libraries with Ludomere for Linux

We maintain Ludomere, a GPL-3.0-or-later GOG game launcher for Arch Linux. We would like to
distribute the unmodified Windows GalaxyPeer.dll and GalaxyPeer64.dll files for MSVC 15–18 from
GOG's desktop-galaxy-peer manifest alongside Ludomere and the open-source Comet communication
service. The manifest currently identifies distribution 1.2.33.1; the DLL resources identify
version 1.114.12.0. Windows games would load these libraries through Proton for GOG Galaxy features.

The DLLs would retain their GOG licence and notices and would not be relicensed under Ludomere's
GPL. We would bundle these components without the complete GOG GALAXY client. Please confirm
whether an existing licence permits this arrangement or grant specific permission covering the
parts and integration conditions in section 3.B.i of the GOG GALAXY licence.

Please clarify whether permission covers build-time downloads from GOG, binary package hosting,
community mirrors/redistribution and subsequent library updates, and specify any required EULA
acceptance, attribution, notices, distribution-channel or remuneration conditions.

## Evidence retained during this session

- /tmp/ludomere-peer-terms-report.md
- /tmp/ludomere-galaxy-license.json and .txt
- /tmp/ludomere-user-agreement.json and .txt
- /tmp/ludomere-peer-artifact-license-report.md

The coordination record above preserves the conclusion and primary links if disposable evidence
is later removed. Existing package verification remains valid for the package without peer DLLs;
the Comet implicit-download finding and real-game acceptance gates are still open.
