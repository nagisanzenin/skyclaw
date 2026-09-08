# Executable updates and persistent container profiles

Status: implementation and local HTTP fixtures; Docker acceptance runs remotely in CI. No real installation or user container was modified during testing.

## Findings and behavior

The former `temm1e update` treated any Git working directory as Temm1e source: it fetched, stashed, pulled and built in the caller's checkout. That risks unrelated work and substantial Cargo disk growth. Its alternative binary downloader neither verified checksums nor bounded its body. Earlier installer checksum tests did **not** establish safety of this CLI path.

`src/updater.rs` now updates only `current_exe()`. It never invokes Git or Cargo. It reads stable release metadata from the official repository, rejects downgrades/prereleases, selects the running platform's desktop/server class, and requires the selected binary plus `checksums-sha256.txt` in the release. Download destinations are constructed from the fixed official repository and parsed version rather than metadata-provided external URLs.

Metadata is capped at 1 MiB, the manifest at 64 KiB and the streamed binary at 128 MiB. An install-directory lock serializes updates. Downloading requires 384 MiB free initially and retains a 256 MiB reserve. Temporary output is private until verified, lives beside the executable and is removed on failure. SHA-256 must match exactly one selected manifest entry. Before replacement, a bounded five-second `--version` probe must exit successfully and report the expected version. Replacement is atomic; the previous executable is retained on validation failure. Directory sync follows Unix replacement.

Checksums protect against corruption; this is not an independently signed-release verification scheme. Metadata/checksums share GitHub's trust boundary. Unsupported platforms receive a source-install message; the updater does not silently build. Source development continues through explicit maintainer Git/build commands. Running processes continue their old image until restarted. An update cannot circumvent directory permissions. Future signed provenance/rollback policy remains separate work.

OAuth `auth login --output` now uses the same private atomic writer as the primary token store, replacing the former default-permission file write.

## Container persistence and migration

The image advertised `TEMM1E_HOME=/data`, which the application did not read. The root-run process defaulted to `/root/.temm1e`; old Compose mounted two other paths. New image and Compose agree on `TEMM1E_DATA_DIR=/var/lib/temm1e`. This correction needs an **explicit existing-container migration**, documented in `docs/ops/deployment.md`, because silently recreating a container against an empty volume could hide its old profile. Preserve the old stopped container until its actual complete profile has been copied and verified. No automatic merge of populated profiles is attempted.

OAuth setup now mounts a writable directory, not a token-file bind mount. Atomic rename and sibling locks require the directory. Kubernetes Secret projections must bootstrap a persistent writable store once, not overwrite refreshed tokens every start. Fixed claims about refresh-token lifetime were removed; provider metadata/reconnect errors govern behavior.

Docker documentation now describes the actual Debian image, root user, Chromium and dynamic libraries. CI loads its first build for both size reporting and an isolated no-network smoke test, eliminating the former second image build. The smoke test checks actual application database creation in the mount, graceful shutdown, complete container replacement with the same volume and private atomic file replacement. It does not claim live OAuth refresh, messaging or cross-host shared token ownership.

## Validation

- Three updater test functions exercise valid local HTTP replacement, corrupt digest, wrong executable version, missing checksum asset, downgrade refusal, exact checksum selection and preservation of a neighboring fake checkout/uncommitted file. No production updater endpoint override exists.
- Initial no-default-features build exposed an unconditional Telegram constructor. The constructor is now feature-gated, with an explicit configuration error when Telegram is requested from a build without it.
- Full workspace/all-feature/all-target lint and final root tests are recorded in the implementation status after completion; pending checks are not passes.
