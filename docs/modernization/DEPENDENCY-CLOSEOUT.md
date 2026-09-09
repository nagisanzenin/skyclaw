# Dependency release review — September 9,2026

The final CI job named Security Audit was green because its cargo-audit command was explicitly advisory/non-blocking. The retained f9d30e7 log reports26vulnerability entries and22warnings. Green job status is not a clean dependency audit. This was discovered during final release inspection; publication is paused for compatible patch updates and residual-risk review.

Compatible lockfile updates include aws-lc-rs/sys, crossbeam-epoch, Diesel, h2 0.4, quinn-proto, rustls-webpki 0.103, anyhow, event-listener5, memmap2 and rand families. Updating plist and wayland-scanner replaces their older quick-xml branches with0.41.0. No source-level API or provider routing change is intended. Serenity0.12.5 and xcb1.7.0 have no newer compatible release resolved by Cargo.

Remaining legacy branches require explicit review: Serenity uses tokio-tungstenite0.21/rustls0.22, and optional AWS compatibility dependencies retain hyper0.14/rustls0.21 in the lockfile. quick-xml0.30 comes from xcb’s generator path. Presence in the lockfile is not proof that a vulnerable operation is reachable; absence of an observed exploit is not proof of safety. RSA’s existing advisory reports no fixed upgrade. Do not silently suppress these findings or describe this release as vulnerability-free.

Next: rerun CI and cargo audit on the updated lockfile, record exact remaining entries and feature reachability, verify declared Rust1.91.1, then run a bounded post-patch actual-provider acceptance check. The original30-pair A/B measures15734b7 before these dependency updates and must remain labeled that way. Do not rerun or replace its scores to improve the result. No merge/tag/release yet.
