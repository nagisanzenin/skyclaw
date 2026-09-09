# Dependency release review — September 9,2026

The final CI job named Security Audit was green because its cargo-audit command was explicitly advisory/non-blocking. The retained f9d30e7 log reports26vulnerability entries and22warnings. Green job status is not a clean dependency audit. This was discovered during final release inspection; publication is paused for compatible patch updates and residual-risk review.

Compatible lockfile updates include aws-lc-rs/sys, crossbeam-epoch, Diesel, h2 0.4, quinn-proto, rustls-webpki 0.103, anyhow, event-listener5, memmap2 and rand families. Updating plist and wayland-scanner replaces their older quick-xml branches with0.41.0. No source-level API or provider routing change is intended. Serenity0.12.5 and xcb1.7.0 have no newer compatible release resolved by Cargo.

Remaining legacy branches require explicit review: Serenity uses tokio-tungstenite0.21/rustls0.22, and optional AWS compatibility dependencies retain hyper0.14/rustls0.21 in the lockfile. quick-xml0.30 comes from xcb’s generator path. Presence in the lockfile is not proof that a vulnerable operation is reachable; absence of an observed exploit is not proof of safety. RSA’s existing advisory reports no fixed upgrade. Do not silently suppress these findings or describe this release as vulnerability-free.

Next: rerun CI and cargo audit on the updated lockfile, record exact remaining entries and feature reachability, verify declared Rust1.91.1, then run a bounded post-patch actual-provider acceptance check. The original30-pair A/B measures15734b7 before these dependency updates and must remain labeled that way. Do not rerun or replace its scores to improve the result. No merge/tag/release yet.

## Compatible-patch result and redundant AWS connector

cb4d4c8 CI audit reports11vulnerability entries and12warnings (down from26/22); MSRV1.91.1 passes. Native agent/default CLI builds pass; both live GLM post-patch cases pass with no failed/cancelled/pending provider calls, and CLI quit/EOF/server SIGTERM pass. These are bounded post-patch checks, not a replacement comparative benchmark.

Source inspection found aws-sdk-s3 defaults enable both its modern default HTTPS client and an additional legacy rustls feature. The latter maps to aws-smithy-runtime/tls-rustls, legacy-rustls-ring and Hyper0.14. Cargo.toml now preserves sigv4a, default-https-client and rt-tokio while omitting only that redundant legacy connector. Resolving the workspace removes rustls0.21, rustls-webpki0.101 and h20.3 from the lockfile. No application code uses the removed connector API. Updated workspace-all-features CI still required.

Expected residual vulnerability entries are four rustls-webpki0.102.8 advisories through Discord/Serenity0.12.5, two quick-xml0.30 XML parsing advisories through xcb’s build generator, and the RSA0.9.10 advisory with no fixed release. RSA was not in the selected macOS workspace-all-features tree; this is not an all-platform reachability claim. The Discord TLS branch is selected in default builds. Do not dismiss all residual findings as build-only or optional. Await exact post-pruning audit and release-risk disposition.

## Final residual review (0af6a2b)

CI audit now reports **7vulnerability entries and12warnings**, down from26/22. This is a reduction in affected lockfile/advisory entries, not a count of remotely exploitable product bugs. The same residual versions existed in baseline da503c0.

| Package / advisory | Observed path and remaining risk |
|---|---|
| rustls-webpki0.102.8 / RUSTSEC-2026-0049 | Discord Serenity0.12.5→tokio-tungstenite0.21→rustls0.22. The advisory concerns CRL distribution-point handling. The inspected gateway uses default connect_async_with_config; its TLS builder supplies roots and no client auth, without configuring CRLs. No affected CRL path was identified in this configuration. |
| rustls-webpki0.102.8 / RUSTSEC-2026-0104 | CRL parser panic. RustSec explicitly excludes applications that do not use CRLs; the same inspected default Discord path does not configure them. |
| rustls-webpki0.102.8 / RUSTSEC-2026-0098 and0099 | URI/wildcard certificate name constraints. RustSec says exploitation requires certificate misissuance after successful signature verification. The package is selected in default Discord builds; do not claim the issue unreachable or fixed. |
| quick-xml0.30 / RUSTSEC-2026-0194 and0195 | xcb1.7.0 code-generator dependency on Linux, rather than Tem’s runtime web/XML parser. Build-input trust remains relevant. Other XML branches were upgraded. |
| rsa0.9.10 / RUSTSEC-2023-0071 | No fixed release reported. No RSA dependency appeared in the inspected macOS workspace-all-features tree. No all-platform or all-configuration absence claim is made. |

Primary RustSec descriptions: [CRL scope](https://rustsec.org/advisories/RUSTSEC-2026-0049.html), [CRL parser](https://rustsec.org/advisories/RUSTSEC-2026-0104.html), [URI constraints](https://rustsec.org/advisories/RUSTSEC-2026-0098.html), [wildcard constraints](https://rustsec.org/advisories/RUSTSEC-2026-0099.html). Source review: Serenity gateway/ws.rs uses default connect_async_with_config; tokio-tungstenite0.21 tls.rs builds ClientConfig with_root_certificates and with_no_client_auth. This is configuration-level reasoning, not an exploit proof.

Serenity0.12.5 has no compatible newer release resolved by Cargo. Changing its TLS backend can also alter unified reqwest features, so a last-minute backend substitution needs its own cross-platform/connection acceptance. The two options are to release the patched candidate with these inherited, disclosed residuals, or defer publication for that larger transport migration. Maintainer disposition pending; no merge/tag yet. CI Security Audit remains advisory/non-blocking and must not be represented as vulnerability-free approval.
