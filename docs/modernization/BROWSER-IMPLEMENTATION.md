# Browser ownership and credential submission

## Defects confirmed by source and execution

The old browser cleanup deleted singleton locks in the user's actual Chrome profile and a shared chromiumoxide runner directory. The main browser/login paths shared a fixed working profile; pool profiles used only PID identity. Concurrent runtimes could collide, and startup copied personal Chrome cookies/storage without an explicit import selection or copy limit.

The `authenticate` tool used the old typed accessibility protocol, guessed fields across the page, injected vault credentials without checking the saved service URL, and returned “Authenticated” after clicking submit. A real current-Chrome fixture failed before insertion with `Auth: ax tree failed: uninteresting`, confirming protocol incompatibility rather than merely a theoretical review concern.

## Implemented behavior

Main browser, pool and interactive login launches now receive unique private profiles under `TEMM1E_DATA_DIR/browser-profiles`. Ownership follows the runtime/session rather than a guessed PID directory. Neither startup nor shutdown removes personal/shared Chrome locks. Normal main-browser close requests graceful Chrome termination and waits for the owned process before profile cleanup, with a bounded graceful interval and owned-child fallback. Interactive login now retains and aborts its CDP task, including failed initialization.

Personal Chrome discovery/copy is off by default. An operator can explicitly set `TEMM1E_BROWSER_IMPORT_FROM` to a selected Chrome `Default` profile directory; `TEMM1E_CLEAN_BROWSER=1` suppresses that import. Only cookie/network/session storage paths are considered, with a combined 128 MiB copy budget, 10,000 entries, depth 16 and rejection of symlink/non-regular entries. Failure aborts startup and cleans the partial new profile; the source is not modified. Importing a live browser's changing database or OS-encrypted cookies is not a verified session migration. Prefer Tem's explicit login/vault session capture for authentication continuity. Old fixed profiles are preserved, not silently deleted or imported.

Startup reserves 8 GiB free space. Chromium disk/media cache settings are 64/16 MiB; these are browser cache settings, not a hard total-profile quota. Application storage and crash-left directories still need total-profile retention enforcement. Temp-directory cleanup is best effort after abnormal termination, and Windows browser/profile cleanup needs live platform acceptance.

Credential submission requires the current page and form action to match the saved HTTP(S) service origin, including effective port. URL userinfo, opaque URLs, missing destinations, downgrade or cross-origin actions fail before credential insertion. Destination checks repeat before sensitive effects. Tool actions on one browser instance are serialized so concurrent navigation cannot interleave with a login action through that tool.

A single visible standard HTML form must identify exactly one password field, one username/email field by standard autocomplete/type/name, and one submit control. Ambiguous or non-standard forms require interactive login. DOM backend-node handles and CDP text insertion carry credential values; values are never interpolated into JavaScript. After submit, the tool reports **unverified authentication**, shows a bounded public page observation with exact known credential values removed (including short passwords), and leaves success assessment to positive account evidence. Absence of a login prompt during session restore is likewise reported as a heuristic, not proof of authentication.

## Validation and practical limits

Pure tests cover origin/port/downgrade/lookalike rejection, exact short-secret removal, conservative submission wording and bounded imports. A separately invoked ignored test uses the actual installed Chrome, a local HTTP fixture and synthetic in-memory credentials. It exercises rejection on a different origin, actual valid-form submission, challenge detection without claiming login, secret-free post-submit output and owned-profile removal. This is not acceptance against real accounts, CAPTCHAs or SSO providers.

Origin checks do not defeat malicious scripts already executing on an authorized origin or provide atomic navigation/form binding against hostile page mutation. Browser sandbox flags, per-chat page/image isolation, vault session principal namespaces, full modern accessibility/observation protocol coverage, and process containment beyond best-effort descendants remain separate audit items. Do not advertise complete browser isolation from these changes alone.

Final checkpoint 29 validation: `cargo test -p temm1e-tools --features browser` passed 493 tests, with one ignored real-Chrome fixture; the fixture was separately invoked with `TEMM1E_HEADLESS=1 TEMM1E_CLEAN_BROWSER=1` and passed in 9.70 seconds. Full workspace/all-feature/all-target clippy with warnings denied passed. Every Cargo invocation used the disk guard and an isolated regression profile. Earlier real-Chrome failure and lint failures are preserved; the removed obsolete AX formatter accounts for 11 fewer unit tests. No real account credentials were used by the Chrome fixture.
