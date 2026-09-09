# Dependency release review — September 9,2026

The final CI job named Security Audit was green because its cargo-audit command was explicitly advisory/non-blocking. The retained f9d30e7 log reports26vulnerability entries and22warnings. Green job status is not a clean dependency audit. This was discovered during final release inspection; publication is paused for compatible patch updates and residual-risk review.

Compatible lockfile updates include aws-lc-rs/sys, crossbeam-epoch, Diesel, h2 0.4, quinn-proto, rustls-webpki 0.103, anyhow, event-listener5, memmap2 and rand families. Updating plist and wayland-scanner replaces their older quick-xml branches with0.41.0. No source-level API or provider routing change is intended. Serenity0.12.5 and xcb1.7.0 have no newer compatible release resolved by Cargo.

Remaining legacy branches require explicit review: Serenity uses tokio-tungstenite0.21/rustls0.22, and optional AWS compatibility dependencies retain hyper0.14/rustls0.21 in the lockfile. quick-xml0.30 comes from xcb’s generator path. Presence in the lockfile is not proof that a vulnerable operation is reachable; absence of an observed exploit is not proof of safety. RSA’s existing advisory reports no fixed upgrade. Do not silently suppress these findings or describe this release as vulnerability-free.

Next: rerun CI and cargo audit on the updated lockfile, record exact remaining entries and feature reachability, verify declared Rust1.91.1, then run a bounded post-patch actual-provider acceptance check. The original30-pair A/B measures15734b7 before these dependency updates and must remain labeled that way. Do not rerun or replace its scores to improve the result. No merge/tag/release yet.

## Compatible-patch result and redundant AWS connector

cb4d4c8 CI audit reports11vulnerability entries and12warnings (down from26/22); MSRV1.91.1 passes. Native agent/default CLI builds pass; both live GLM post-patch cases pass with no failed/cancelled/pending provider calls, and CLI quit/EOF/server SIGTERM pass. These are bounded post-patch checks, not a replacement comparative benchmark.

Source inspection found aws-sdk-s3 defaults enable both its modern default HTTPS client and an additional legacy rustls feature. The latter maps to aws-smithy-runtime/tls-rustls, legacy-rustls-ring and Hyper0.14. Cargo.toml now preserves sigv4a, default-https-client and rt-tokio while omitting only that redundant legacy connector. Resolving the workspace removes rustls0.21, rustls-webpki0.101 and h20.3 from the lockfile. No application code uses the removed connector API. Updated workspace-all-features CI still required.

Expected residual vulnerability entries are four rustls-webpki0.102.8 advisories through Discord/Serenity0.12.5, two quick-xml0.30 XML parsing advisories through xcb’s build generator, and the RSA0.9.10 advisory with no fixed release. RSA was not in the selected macOS workspace-all-features tree; this is not an all-platform reachability claim. The Discord TLS branch is selected in default builds. Do not dismiss all residual findings as build-only or optional. Await exact post-pruning audit and release-risk disposition.
