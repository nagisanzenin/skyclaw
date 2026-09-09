# Closeout validation — in progress

Production source: `15734b7` (version 5.8.1 candidate). Documentation/benchmark-only commits do not change these binaries. This report is evidence for the candidate, not a published 6.0 release certificate.

## Compilation and platform evidence

[CI 34305585814](https://github.com/temm1e-labs/temm1e/actions/runs/34305585814) passed all eight jobs: formatting/all-target/all-feature lint, Linux tests, Windows tests, minimum Rust 1.91.1 all-feature check, security audit, musl server, glibc desktop and Docker build.

Complete job logs were retained locally. Summing `test result: ok` lines separately gives Linux **3,138 passed / 0 failed / 21 ignored**, and Windows **3,109 passed / 0 failed / 20 ignored**. These overlap and must not be added into a unique-test total. Ignored external/platform checks are not passing acceptance. Local default CLI build passed in 2m04s; the actual TUI bridge example built in 28.10s. Local guarded `check --workspace` passed in54.98s and removed4.0GiB generated cache. The existing proc-macro-error2 future-compatibility notice remains.

## Actual interface and persistence checks

All use isolated temporary profiles and a local synthetic provider; no real user profiles or messaging accounts are touched.

| Boundary | Observed result | Limit |
|---|---|---|
| TUI keyboard/rendering lifecycle | PTY connection/model switching, onboarding, history restart, Unicode/resize and exact terminal-attribute restoration pass | Unix PTY; native Windows rendering not certified |
| Headless real TUI bridge | One streamed request, exact fixture answer, persisted accepted UI delivery and owned shutdown pass | Functional bridge check, separate from PTY rendering; optional services initialized, not every feature invoked |
| Gateway onboarding | Health/readiness before and after setup and clean shutdown pass | Local HTTP lifecycle, not actual external-channel dispatch |
| CLI/server lifecycle | CLI quit/EOF and server SIGTERM exit cleanly | Not crash-durable platform intake/reconnect proof |
| Conversation restart | 244 native messages preserved, legacy source retained, commands bypass provider, context survives restart, new epoch excludes old context | Five local-provider requests; not semantic retention across arbitrary workloads |
| Interrupted final delivery | Process killed after partial output; ambiguous send not replayed; saved reply review bounded | Final replies only; interim/control/tool sends remain distinct |
| Connections and budgets | Current connection, foreground/auxiliary accounting, runtime policy and finite-owner rejection pass | Local owner logical-call accounting, not global subscription quota or per-goal USD reservations |
| Engram | Configuration, caller identity, persistence/reopen and scoped mutation checks pass | EMA/cadence/provenance and all shared-host namespaces remain separate |
| Runtime resources | Actual TemDOS endpoint/model/owner replacement passes | Does not certify every legacy factory reconstruction |
| Goals and Witness | CLI/TUI status, original criteria, exact saved evidence, current-model bounded verifier and restart checks pass | Coverage stays unverified; model judgment is not execution proof |
| Model registry / observer | Storage integrity and no-HTTP model selection; six observer binding/scope/budget/output controls pass | TUI persistence semantics differ; observer modes/intervention semantics remain documented limits |

The final batch of 13 script groups passed without failures. These are acceptance scenarios, not extra unique Rust tests. Logs are `final-acceptance/*`, `final-tui-pty.log`, `final-tui-bridge.*.log`, `final-gateway.log`, `final-lifecycle/*` and `final-conversation-restart/*` in the operation's private work area. Test inputs contain synthetic credentials only.

## Documentation and artwork

README now provides installation, connection choices, concise capability summaries and direct feature/upgrade links. The command reference corrects the obsolete Cargo-rebuild updater description and explains instance-only CLI/server model selection. The upgrade guide covers preserved legacy histories, explicit import, uncertain delivery, personal-host boundaries and rollback.

The approved banner plus 17 feature images are retained uniformly. All 18 match their committed SHA-256 manifest (17,651,355 bytes total). Local links in README, feature guide, command reference and upgrade guide resolve. These images are illustrations, not screenshots or evidence of feature success. No unnecessary regeneration was performed in this closeout.

## Outstanding gates

The frozen 30-pair GLM coding-plan A/B is running; no aggregate verdict yet. See [fixed design](CLOSEOUT-DESIGN.md). Its core harness does not replace product-path acceptance above. Final source-affecting changes require affected validation. Published old-to-new update smoke, version 6.0 metadata, merge, tag and four downloadable verified artifacts remain pending. Do not mark release complete based on CI or this report alone.

### A/B environment note

The initial A/B pairs overlapped local build/check work on the same machine. Serial provider runs prevent pair-to-pair load overlap but do not remove that local CPU/I/O contention. Foreground/drained latency is therefore descriptive and confounded, especially for the initial pairs; do not publish a clean performance-speedup claim. No corpus, resource cap or result was changed because of this observation.
