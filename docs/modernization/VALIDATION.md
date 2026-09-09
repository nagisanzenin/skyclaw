# Validation and limits

Baseline: commit `da503c09ca5c0f41c308c99e42d4736aa3611f8a`, macOS local environment, Rust locked dependencies, 2026-09-08. Documentation-only changes. No live provider credentials, subscriptions, cloud deployment, native desktop action, user data mutation or external messaging was used by audit probes.

## Executed

`cargo test --workspace --lib --locked`: **2,721 passed, zero failed, nine ignored**, across 24 library suites. The workspace contains 25 crates; watchdog is a binary and is not covered by --lib. These are default resolved feature sets, not every feature combination. Root binary/integration/doc tests and live-service tests were not run by this command.

Earlier focused runs: core235 passed/1 ignored, agent788 passed, and subscription/memory/Perpetuum/Witness suites passed. Counts overlap with workspace tests; do not add them as independent test cases. Full redacted-path logs are under evidence/.

| Library | Passed | Failed | Ignored |
|---|---:|---:|---:|
| temm1e-agent | 788 | 0 | 0 |
| temm1e-anima | 75 | 0 | 0 |
| temm1e-automation | 24 | 0 | 0 |
| temm1e-cambium | 133 | 0 | 0 |
| temm1e-channels | 65 | 0 | 0 |
| temm1e-codex-oauth | 21 | 0 | 0 |
| temm1e-core | 235 | 0 | 1 |
| temm1e-cores | 15 | 0 | 0 |
| temm1e-distill | 149 | 0 | 0 |
| temm1e-filestore | 24 | 0 | 0 |
| temm1e-gateway | 54 | 0 | 0 |
| temm1e-gaze | 44 | 0 | 7 |
| temm1e-hive | 76 | 0 | 0 |
| temm1e-mcp | 49 | 0 | 0 |
| temm1e-memory | 70 | 0 | 1 |
| temm1e-observable | 29 | 0 | 0 |
| temm1e-perpetuum | 77 | 0 | 0 |
| temm1e-providers | 81 | 0 | 0 |
| temm1e-skills | 29 | 0 | 0 |
| temm1e-test-utils | 6 | 0 | 0 |
| temm1e-tools | 501 | 0 | 0 |
| temm1e-tui | 33 | 0 | 0 |
| temm1e-vault | 51 | 0 | 0 |
| temm1e-witness | 92 | 0 | 0 |

## Baseline probes

[runtime_probe.rs](evidence/runtime_probe.rs) uses actual public APIs. It demonstrates dependency grouping, numerical outputs, search cache-key collision and a local shell command that writes after its reported timeout. [perpetuum_probe.rs](evidence/perpetuum_probe.rs) uses an in-memory SQLite store and an entirely synthetic Unicode report. It demonstrates activity probability1 after one record and a caught UTF-8 truncation panic. No issue is submitted.

```text
shell_write_then_read_groups=[[0], [1]]
browser_action_groups=[[0], [1]]
wilson_29_of_30_99pct=0.768206
wilson_30_of_30_99pct=0.818872
sprt_at_cap_below_boundary=AcceptH1
power_sample_n=1052
entropy_missing_categories=1
different_search_sort_same_cache_key=true
shell_timeout_error=true
side_effect_after_timeout=true
activity_after_one_record=1
unicode_report_panics=true
```

Dependency groups here are connected components: separate singleton groups mean independent, not ordered execution batches. The normal agent loop is sequential; this probe demonstrates a latent helper defect. The SPRT probe restores a below-boundary positive log likelihood at sample cap to exercise the explicit truncation rule; it does not estimate real-world false-graduation frequency. Entropy1 is algebraically valid observed-category evenness, but insufficient to prove coverage.

To reproduce in a clean disposable baseline checkout, copy the two evidence files to `crates/temm1e-agent/examples/modernization_probe.rs` and `crates/temm1e-perpetuum/examples/modernization_probe.rs` (create the examples directory if absent), then run:

```sh
cargo run -p temm1e-agent --example modernization_probe --locked
cargo run -p temm1e-perpetuum --example modernization_probe --locked
```

Remove only the two copied probe files afterwards. They are retained canonically under documentation, not production source. The first intentionally creates a delayed marker in a tempfile and waits for it; the second catches the intended panic. These are baseline observations, not regression assertions that a fixed implementation should preserve.

## Release acceptance matrix

| Area | Necessary evidence beyond current tests |
|---|---|
| Durable autonomy | Real entrypoint kill/restart at each tool boundary; no duplicated external effect; lease ownership |
| Truthful completion | Required evidence absent/failing/inconclusive; zero unsupported success transitions |
| Context | Exact wire budget, preserved constraints through three compactions, provider-private state roundtrip |
| Cache/billing | Normalized synthetic usage plus real-account hit/write/cost/latency experiment |
| Subscriptions | Official support status and real login/refresh/revocation/model/quota tests by connection |
| Tools | Process descendants die on cancel; byte bounds; index preserved; uncertain effects reconciled |
| Memory/roles | Cross-principal negative tests in all recall paths and delegation |
| Learning/math | Independent held-out task outcomes, quantiles and calibrated sequential stopping |
| Perpetuum | Timezone/DST/outage recovery, real cleanup effects, all-call resource accounting |
| TUI | Event replay + viewport snapshots + native PTY usability/cleanup on supported platforms |
| Channels/cloud | Real platform reconnect/upload tests, local S3/OTLP/Docker contract tests, auth boundaries |
| Distribution | Every supported release asset, missing-checksum behavior, interrupted-update rollback |

## Interpretation

Source-confirmed defects are not reports of observed production incidents. Passing tests establish their tested conditions only. Historical lab benchmarks were reviewed as claims, not independently rerun. No comprehensive adversarial security audit or proof of mathematical optimality is claimed. The feature/source inventory records scope and review depth so future implementers cannot replace missing evidence with a test count.
