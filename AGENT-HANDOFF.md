# Temm1e modernization — agent handoff

**Read this before continuing. Updated: September 9, 2026.** The creator is away and authorized autonomous work through completion. This handoff must be updated in **every modernization commit that is pushed**, together with implementation notes and validation evidence. Do not conclude that the operation is finished merely because one checkpoint is green.

## Authority and release rule

- Branch: `codex/modernize-temm1e-research`; draft PR: <https://github.com/temm1e-labs/temm1e/pull/74>.
- Immutable A/B baseline: `da503c09ca5c0f41c308c99e42d4736aa3611f8a` (main, version 5.8.1 when the operation began). Do not move or patch the baseline runtime to make comparisons pass.
- The creator authorized implementation, intermittent branch pushes, and merging/releasing **6.0.0 only after broader A/B shows no material regression and real-user readiness/release checks pass**. Optimization superiority is not required. No main merge, version bump or release has happened.
- Preserve the creator's vision from `VISION.md`, `FEATURES.md`, `docs/TEMM1E_VISION.md`, feature design documents and `tems_lab`. Research and audited mappings are in `docs/modernization`.
- Preserve selected-model default, Tem identity/palette, messaging-first operation, and the accepted compact transcript TUI with expandable tools/optional panels. Do not quietly retire library-only/cloud/growth features.
- No messages to other people or external bug reports are authorized. No additional live paid provider accounts are authorized merely because credentials happen to exist locally.
- The creator supplied a Z.ai coding-plan key for `glm-5.3-flash`. Keep it outside Git; use `TEMM1E_ZAI_KEY_FILE` pointing at the existing private local file. Never print, copy into documents, or commit its contents. Subscription usage is not zero-cost API usage.

## Read next

1. `docs/modernization/IMPLEMENTATION-STATUS.md`: chronological implemented checkpoints with evidence and limits.
2. `docs/modernization/FEATURE-COVERAGE.md`, `03-FINDINGS.md`, `MATH-AUDIT.md`: feature/vision/math audit, not claims of complete runtime acceptance.
3. `docs/modernization/05-IMPLEMENTATION.md` and `DECISIONS.md`: contracts, dependencies and unresolved product choices.
4. `docs/modernization/BENCHMARK-PROTOCOL.md`, `PILOT-02.md`, `docs/RELEASE_PROTOCOL.md`: actual release conditions. Four pilot tasks are not broad noninferiority evidence.
5. Relevant focused implementation document before editing that subsystem.

## Current position

Checkpoint **35** fixes TUI provider/endpoint/credential mixing and model-switch reconstruction. Read `TUI-CONNECTION-IMPLEMENTATION.md`. `AgentSetup::resolve` captures configuration or one saved file; the bridge never rereads saved keys/endpoint. Matching headers/key pools remain scoped. The handle keeps its setup for switching/rollback, allows registered/custom-endpoint models, and saves only a still-matching saved record. No API key is invented for Codex OAuth.

Validation: **50 TUI tests**, real Unix PTY chat→custom model switch→second turn, restart/history and onboarding passed. Four synthetic provider requests used the selected key/endpoint; the deliberately unrelated saved endpoint received zero. Saved credentials stayed unchanged, Unicode echo33.7ms, resize/exit/exact termios restoration passed. Final TUI/all-target/all-feature lint passed20.65s and cleaned1.7GiB. Logs: `implementation-tui-connection-final-tests.log`, `implementation-tui-connection-pty.log`, `implementation-tui-connection-clippy.log`. A Rust2021-incompatible let-chain formatting attempt was corrected to an equivalent Option filter before final lint. No active Cargo process remains.

Previous push **34 `e5dc5d9`**, CI **34267201313**, pending. **33 `a424b74`**, CI34265542264, **32 `816a75d`**, CI34263918328, **31 `df4e6bd`**, CI34263300717, and **30 `9df96c9`**, CI34261794031, are fully green. **29 `06d9bbd`**, CI34261489651, remains a recorded Windows failure (Hive stale readiness race repaired in32), not erased by a rerun.

Next concrete task: **TUI shared budget and model-switch budget continuity.** `agent_bridge.rs` still constructs unrelated TemDOS/JIT trackers and passes raw provider to Perpetuum's usage-discarding bridge. Allocate the owning budget once, inject it into AgentRuntime/TemDOS, set JIT parent to agent.budget, wrap only auxiliary Perpetuum with MeteredProvider, and retain the same budget through model switch/rollback. Do not wrap already-accounted foreground calls. Add a real fixture showing that switching does not reopen exhausted spending admission. Then continue root alternate paths and typed durable goal/evidence contracts, broader feature acceptance/A/B. Preserve pending D01/D02 defaults.

Final browser-feature tests passed **493 tests, one ignored real-Chrome test** (`implementation-browser-post-review-tests.log`). Eleven obsolete typed-accessibility formatter tests were removed with the unused formatter; schema tests remain. The real-Chrome test was separately invoked and passed (`implementation-browser-real-auth-final.log`): wrong origin rejected without submission, valid form submitted once, authentication explicitly unverified, reflected credentials redacted and owned profile removed. Final workspace/all-feature/all-target clippy passed (`implementation-browser-final-workspace-clippy.log`); the guard removed 1.2 GiB. Earlier lint failures are retained as evidence. No Cargo process from this validation is running.

Files for checkpoint 29:

- `crates/temm1e-tools/src/browser_auth.rs`: exact HTTP(S) origin/effective-port policy; known-secret report redaction; no unsupported success claim.
- `browser_profile.rs`: private unique profiles under the selected Tem data directory, 8 GiB startup reserve, explicit bounded import (128 MiB/10,000 entries/depth 16), owned temporary-directory cleanup.
- `browser.rs`: removes personal Chrome discovery/lock deletion; explicit `TEMM1E_BROWSER_IMPORT_FROM` opt-in (`TEMM1E_CLEAN_BROWSER=1` disables it), serialized tool actions, standard DOM form selection, repeated page/form destination validation, CDP credential insertion, unverified result, bounded graceful close. A real fixture found and replaced the old broken typed accessibility login path (`uninteresting` enum error).
- `browser_pool.rs`, `browser_session.rs`: unique owned profiles; interactive CDP task retained/aborted instead of detached.
- `Cargo.toml`/lock: tools adds already-locked `fs2 0.4.3`. Cargo may rewrite lock format 3 to 4; keep format 3 if that is the only unrelated change.
- `BROWSER-IMPLEMENTATION.md`: detailed behavior, migration/environment options, evidence and remaining boundaries. Review it against final code.

After pushing checkpoint 35, inspect CI and continue TUI budget continuity above. Keep the following boundaries visible; the browser checkpoint is not complete product acceptance.

## Validation evidence currently available locally

Logs are kept **outside `target`**, in the checkout's parent directory. They are not all committed artifacts. Preserve failure logs rather than replacing them with invented passes.

- Browser: `implementation-browser-profile-tests.log`, `implementation-browser-final-tests.log`; real Chrome `implementation-browser-real-auth-v2.log` (actual old accessibility failure), `-v3.log`, `-v4.log`, `implementation-browser-real-auth-final.log` (pass). Initial `implementation-browser-real-auth.log` was a fixture compile error, not product acceptance. `implementation-browser-final-workspace-clippy.log` is the final successful workspace lint; preceding lint failures remain preserved.
- TUI history: `implementation-history-page-tests.log` (9 conversation tests); `implementation-tui-history-final-tests.log` (45); `implementation-tui-history-final-pty-v2.log` (chat/restart/history commands/onboarding pass; restore made zero provider requests); scoped final lint `implementation-tui-history-post-review-clippy.log`.
- Gateway: `implementation-gateway-onboarding-tests.log` (55+2), `implementation-gateway-root-tests.log` (79), `implementation-gateway-onboarding-smoke.log` (two real no-key starts, same profile, liveness 200/readiness 503, SIGTERM exit 0), `implementation-gateway-final-clippy.log`.
- Anthropic: 79 provider unit tests + retry + five stream fixtures; two-turn actual HTTP native-history fixture and eight agent lifecycle integration tests. See checkpoint 26 logs and focused document.
- Earlier full workspace tests and CI are documented in status. They do not substitute for CI on the final release revision.

## Disk and runtime rules — mandatory on the creator's machine

The creator has very little storage. Do not run unmanaged Cargo builds or simultaneous guards. Every local build/check/test/clippy uses:

```sh
TEMM1E_DATA_DIR=<isolated-regression-profile> python3 scripts/cargo_guard.py -- <cargo arguments>
```

The guard owns `target/guarded`, disables debug/incremental artifacts, reserves 8 GiB free, caps target output at 8 GiB and cleans at completion. `--keep-cache` is only for a short validation/binary-extraction batch; finish with cleanup. Exit **75 means unfinished**, never passed. `cargo fmt` is safe outside the guard. Keep logs elsewhere. Do not build Docker locally; use CI. Do not delete user profiles, source, secrets, baseline binaries or benchmark evidence. Recent free space was about 21 GiB; recheck before expensive work.

A local private companion resume note exists outside Git at the operation workspace's `work/MODERNIZATION-RESUME.md`, with exact machine paths and secret **file location only**. If continuing on another machine, ask for a local environment/file reference rather than asking the creator to paste a key.

## Implemented foundations — retain and do not redo

The first 28 pushed checkpoints include source-backed frontier research, 65-family coverage mapping, corrected statistical math, modern provider catalog/pricing knownness, Z.ai coding-plan support, private locked OAuth refresh, direct/native OpenAI Responses and Anthropic replay, real bounded SSE, final context fitting and provenance-backed compaction, canonical scoped conversation heads, explicit legacy import/recovery, transactional final-reply outbox, owned background work/shutdown, tool-effect journaling/process cleanup, safer RBAC bootstrap, temporary-index snapshots, structurally keyed channel routing, typed bounded Mission Control, verified updater, runnable persistent Docker and real PTY-tested TUI. README and 18 uniform pixel images were already regenerated earlier in the operation. Follow detailed documents for exact limitations; this list is not full release acceptance.

## Next priorities and genuine remaining gates

1. **Browser/current critical findings:** validate remaining pool/interactive launch paths on supported platforms. Total browser/storage retention, sandbox policy, principal-scoped browser page/image/vault sessions and generic modern accessibility handling remain separate gaps. Current origin checks do not defeat a malicious script on the authorized origin or make page mutations atomic. Do not claim complete browser isolation.
2. **Shared runtime and accounting:** remaining durable/global budget and all auxiliary/retry/cancel accounting; Initial root Perpetuum is metered in33; other composition paths still need coverage; Engram curator commonly cancels at short shutdown. Preserve explicit unknown costs. Complete actual entrypoint composition and durable goal/evidence linkage; legacy TaskQueue is not a wired production goal manager.
3. **Provider/context completion:** modern menu IDs are exposed in checkpoint 30 with existing defaults preserved; capability-knownness and Gemini generateContent native streaming is implemented in checkpoint 31; Interactions and live acceptance remain open, reasoning controls/quota experience, whole-wire/auxiliary context and cache acceptance. Only GLM coding plan is live-authorized; other accounts require fixtures/source verification.
4. **Identity/migration/feature acceptance:** channel/principal setup tokens and memory/vault namespaces, attachments/interim/control delivery, server/channel restart, delegated linkage, retention/GC, supported-platform process/migration behavior. Distinguish implemented invariants from broader advertised capabilities.
5. **Feature matrix:** update original audit rows with concrete checkpoint/acceptance evidence; some original assessments are stale after repairs. Complete remaining feature-family reviews and classify release blockers vs explicit limitations/future work rather than labeling all proposals implemented.
6. **Broader held-out paired A/B:** freeze corpus/margins/sample/stopping before results. Same GLM/model settings, endpoint, account route, resources and tools on immutable baseline and final modernization. Existing four-task pilots both passed 4/4, but do not establish broad nonregression. Preserve all attempts, failures, request accounting and uncertainty. Protocol proposed 80 independent scenarios/3 repeats; size is to be finalized before held-out runs, not changed after unfavorable results.
7. **Release only after gates:** full final CI, CLI/server/TUI acceptance, old-profile migration/update paths and all release protocol checks; update README/release notes/artifacts, bump 6.0.0, merge main and execute release protocol only when authorized conditions actually hold. Keep PR draft until ready. Refresh final output ZIP/patch/manifests last; current output bundles are stale.

Pending creator choices D01 (personal/shared-host semantics) and D02 (delivery vs automatic durable pursuit) remain in `DECISIONS.md`; preserve existing defaults while implementing noncontroversial foundations. For browser imports, an optional preference question was sent September 9; while the creator is away, the stated working assumption is private profiles by default with explicit import retained. Any later answer must be incorporated.

## Per-push checklist

- State exactly what changed and which creator behavior it preserves.
- Record commands/evidence, actual test results, failed/unfinished checks and limits.
- Update this handoff, `IMPLEMENTATION-STATUS.md`, and the focused implementation document.
- Inspect staged diff, run the repository hooks normally, commit and push this branch.
- Record the new commit/CI state in the next handoff update; never claim a pending CI pass.
- Continue the next bounded task; do not stop with a checkpoint-only final response.
