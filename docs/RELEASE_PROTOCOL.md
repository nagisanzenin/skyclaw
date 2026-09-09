# TEMM1E Release Protocol

## Local disk budget (modernization requirement)

On storage-constrained development machines, run build/test/check/clippy through `python3 scripts/cargo_guard.py -- <build|check|test|clippy> [arguments]`. It reserves 8 GiB of free space, caps total repository `target/` size at 8 GiB, and uses a disposable `target/guarded` directory. It stops a running command when a sampled limit is crossed (exit 75, **not a passing validation**) and removes guarded outputs afterward. Sampling is once per second; this is a practical guard, not a filesystem quota. Unmanaged Cargo invocations are outside its process lock and must not run alongside it.

Use package/feature batches rather than retaining many build variants. Keep test logs and benchmark evidence outside `target/`. The dev/test profiles disable debug symbols and incremental caching by default; opt into debugging only when needed and clean that build afterward.

For a local release build, use `--keep-cache` only long enough to copy and verify the intended release binary; then run `cargo clean --target-dir target/guarded`. Retain final distributable artifacts and evidence, not complete historical target directories. Before archiving or deleting anything outside generated build outputs, identify it explicitly; do not delete user profiles, credentials, source checkouts or benchmark records as a disk workaround.

The installer and update smoke already use temporary directories with exit cleanup. A September 2026 local audit found 20.6 GiB of accumulated Rust build outputs, compared with about 705 MB of remaining operation files after cleanup. Do not confuse build-cache growth with installed-binary size. Heavy all-feature checks may exceed the local budget; split them or run on a suitable CI runner, and record unfinished checks honestly.


**MANDATORY checklist before pushing any release to `main`.** The release owner MUST execute every step and verify results before committing.

## Pre-Release Verification

### 1. Compilation Gates (ALL must pass)

```bash
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --workspace
```

Record the test count from the output. Every `test result: ok` line's passed count must be summed.

### 2. Collect evidence without rebuilding for a count

Retain the complete successful test log from step 1, including its command, commit, toolchain and feature set. Sum passed counts from that log only; record ignored tests separately. A pipeline that filters output must preserve the Cargo exit status. Do not rerun the entire suite merely to refresh a badge. Counts from overlapping focused reruns must not be added together as distinct tests.

Use `rg --files -g '*.rs' -g '!target/**'` for source inventory and workspace metadata for crate counts. Counts describe code size and executed tests, not correctness or product quality. Publish validation in the release report rather than adding implementation statistics to the README hero.

### 3. Version bump

Update `[workspace.package].version` in `Cargo.toml`, refresh the generated lockfile, and verify that `temm1e --version` reports the intended release and source revision. Keep old benchmark versions and observed outputs unchanged. Verify CI using the declared `rust-version`; stable-only CI does not validate the minimum toolchain. The locked AWS dependencies require Rust 1.91.1, superseding the inherited 1.82 claim.

### 4. README and user documentation

The creator requests a full illustrated feature tour, like the original README but with clearer explanations and current artwork. Preserve feature depth, architecture, setup and commands. Review semantic sections rather than historical line numbers:

- Version badge and any release-specific examples agree with Cargo metadata.
- Install/upgrade commands match actual published assets and supported platforms.
- Provider, coding-plan and login claims distinguish tested compatibility from official support; unknown subscription cost is not zero.
- Links lead to feature, setup, CLI and architecture documentation. Keep useful feature explanations in the README and link to detailed acceptance evidence and limitations.
- Artwork follows the approved Tem visual brief and is uniform. Do not regenerate unchanged assets on each release.
- Release notes summarize user-visible changes, migration behavior and known limitations, with links to the validation report and A/B results.

Preserve the original README's feature breadth without restoring unverified source-line, tool-count or benchmark marketing claims.

### 5. CLAUDE.md — Update Stale References

| Location | What |
|----------|------|
| Line ~7 | Crate count ("X crates plus a root binary") |
| Workspace structure | Must list all crates in `crates/` |

### 6. src/main.rs — Check Version-Sensitive Code

| What | How to verify |
|------|---------------|
| `default_model()` | All providers have entries, new providers added |
| System prompt provider list | All providers listed with models |
| `auth status` output | Recommended model is correct |

### 7. Interactive Interface Parity Gate — MANDATORY

**Every interactive interface must be fully wired before release.** TEMM1E has
independent initialization paths for:

| Interface | Code path | Notes |
|---|---|---|
| **TUI** (`temm1e tui`) | `crates/temm1e-tui/src/agent_bridge.rs :: spawn_agent()` | Primary install method per README — new users hit this first |
| **CLI chat** (`temm1e chat`) | `src/main.rs :: Commands::Chat` | Primary self-test vehicle |
| **Server/messengers** (`temm1e start`) | `src/main.rs :: Commands::Start` | Routes Telegram/Discord/WhatsApp/Slack through the shared agent init |

Each path maintains its own tool-list assembly, Hive init (or lack of),
agent construction, and background-service wiring. **Wiring a feature into
one path does NOT wire it into the others.** This has silently drifted
multiple times — v5.4.0 shipped with JIT `spawn_swarm` registered only in
server; TUI has been missing a dozen subsystems for multiple releases.

#### Parity matrix (update every release)

Before pushing a release, confirm every shipped feature is wired in every
interactive interface. Historical pre-modernization snapshot follows; verify the final implementation against `docs/modernization/IMPLEMENTATION-STATUS.md` and record a new evidence matrix at release. These old checkmarks are not current acceptance results:

| Feature | Server | CLI chat | TUI |
|---|:---:|:---:|:---:|
| Hive + JIT `spawn_swarm` | ✓ | ✓ (v5.4.0) | **must wire** |
| Consciousness observer | ✓ | ✓ | **must wire** |
| Social intelligence / user profile | ✓ | ✓ | **must wire** |
| Personality config (`.with_personality`) | ✓ | ✓ | **must wire** |
| Perpetuum (`.with_perpetuum_temporal`) | ✓ | ✓ | **must wire** |
| MCP servers | ✓ | ✓ | **must wire** |
| Custom tools + `SelfCreateTool` | ✓ | ✓ | **must wire** |
| TemDOS cores + `invoke_core` | ✓ | ✓ | **must wire** |
| Eigen-Tune engine | ✓ | ✓ | **must wire** |
| Witness / Cambium trust / auto-oath | ✓ | — | — (opt-in, OK to defer) |
| Shared memory strategy (`/memory lambda`) | ✓ | ✓ | **must wire** |
| Vault + skill_registry wiring | ✓ | ✓ | **must wire** |

#### Per-interface verification steps

For **every** interface above, run a smoke test and confirm the feature's
startup log appears. CLI chat and `start` can be smoke-tested with
pipe/redirect; **TUI cannot** (ratatui needs a real TTY and fails with
`Device not configured` when stdin/stdout are redirected). Use the
dedicated headless harness for TUI:

```bash
# 1) CLI chat parity
./target/release/temm1e chat <<<'hi' 2>&1 | grep -E "JIT spawn_swarm tool registered \(CLI|Many Tems initialized \(CLI|Tem Conscious.*initialized|Social intelligence initialized"

# 2) Server parity
timeout 10 ./target/release/temm1e start > /tmp/parity_start.log 2>&1 &
sleep 12
grep -E "JIT spawn_swarm tool registered|Many Tems initialized|Tem Conscious.*initialized" /tmp/parity_start.log

# 3) TUI parity — MANDATORY use the headless example harness
#    (ratatui crashes with "Device not configured" under pipe/redirect)
cargo build --release --example tui_smoke -p temm1e-tui
./target/release/examples/tui_smoke 2>&1 | grep -E "JIT spawn_swarm tool registered \(TUI|Many Tems initialized \(TUI|JIT spawn_swarm context wired \(TUI|Tem Conscious initialized \(TUI|Social intelligence initialized \(TUI"
```

The **tui_smoke example** (`crates/temm1e-tui/examples/tui_smoke.rs`)
calls the exact `spawn_agent()` function that `launch_tui` calls, but
skips ratatui's terminal init. It sets up a tracing subscriber that
exercises bridge initialization and supports actual prompt/stream/expected-response checks before owned shutdown. It is a headless integration check, not proof of rendered terminal behavior. Run it on release alongside a PTY interaction check.

For every feature listed in the release: include a greppable registration
log message, run ALL THREE smoke tests against the release binary, and
paste the greps into the release report. A missing log = a missing
wiring = blocker for release unless the release notes EXPLICITLY declare
non-parity for that interface.

#### Parity evidence requirements

Use isolated profiles and bounded owned processes. Retain configuration hashes, startup registration logs, exercised actions and shutdown results for CLI/server/TUI. Do not infer functional parity from a startup grep alone. `benchmarks/modernization/lifecycle.py` checks process lifecycle; `conversation_restart.py` exercises actual CLI history/restart and interrupted final output with a fake provider. Neither proves external-channel conversation dispatch or rendered TUI behavior.

The current `tui_smoke` supports a prompt, expected text and `--require-stream`; it exercises the real bridge, waits for completion and checks foreground shutdown plus persisted final delivery. Optional background drainage is reported separately. Run a real PTY test for keyboard/rendering behavior. Do not use the old unowned `sleep; kill $!` shell snippets as an acceptance harness, and do not write test credentials into the real user's profile.

#### Rules

1. **Never declare a feature "shipped" based on one interface's logs.**
   CLI chat passing ≠ TUI passing ≠ server passing. Each must be checked.
2. **"Feature wasn't triggered" must be distinguished from "feature wasn't
   registered."** Grep for the registration log first; then grep for the
   execution log. Skipping step one confuses a wiring bug with a
   behavioural outcome.
3. **When adding a feature, add its registration log alongside the code.**
   Future wiring checks depend on this anchor.
4. **Opt-in features** (Witness, Cambium, auto_planner_oath) may legitimately
   be absent from interactive interfaces, but the release notes must call
   that out.
5. **Non-interactive paths** (MCP client only, tool servers, background
   cron) are out of scope for the parity gate but still need their own
   smoke tests.

### 8. Final Verification

After code or version changes, run the applicable compilation/test gates on the final release commit and record complete logs with exit status. Pure documentation edits do not require rebuilding unchanged code. Confirm that published validation counts identify the tested commit and configuration; a count alone cannot establish a passing run.

### 9. Commit and Push — PR-based flow

`main` is branch-protected: direct pushes are rejected, and merging a PR
requires **1 approving review** (per `gh api repos/temm1e-labs/temm1e/branches/main/protection`).
The release workflow is therefore:

```bash
# 1. Commit on a release branch (not main)
git checkout -b release/vX.Y.Z   # or use whatever feature branch is active
git add <only files actually changed by the release>
git commit -m "vX.Y.Z: <one-line summary>"
git push -u origin release/vX.Y.Z

# 2. Open the PR
gh pr create --title "vX.Y.Z: <one-line summary>" --body "<details>"

# 3. Wait for CI checks to pass on the PR
gh pr checks <PR_NUMBER>    # all rows must say "pass"
```

#### Merging the PR

Once CI is green:

- **If a second reviewer is available**: have them `gh pr review <N> --approve`,
  then `gh pr merge <N> --squash --subject "vX.Y.Z: ... (#<N>)" --body "..."`.
- **Solo maintainer (no second reviewer)**: admin override is the only path
  because `enforce_admins: false` on this repo. Authorized usage:

  ```bash
  gh pr merge <PR_NUMBER> --squash --admin \
      --subject "vX.Y.Z: <summary> (#<PR_NUMBER>)" \
      --body "<details>"
  ```

  `--admin` bypasses the required-review gate. **Only use when**:
  1. All CI checks on the PR have passed (verified via `gh pr checks`).
  2. You are the only maintainer with merge rights for this release.
  3. The release commit was self-reviewed end-to-end (compilation gates,
     test count, README/CLAUDE.md updates, interactive parity gate).

  Document the admin merge in the release notes for traceability. Established
  v5.6.1 (2026-05-15) as the first explicit-admin-bypass release on record.

#### After merge

```bash
git checkout main
git pull --ff-only      # main now contains the squashed release commit
```

### 10. Tag and Release

**CRITICAL — this triggers the GitHub release pipeline.**
Without the tag, no binaries are built and no GitHub release is created.

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

After pushing the tag:
1. GitHub Actions `release.yml` triggers automatically
2. CI runs checks (cargo check, test, clippy, fmt)
3. Builds6primary binaries (Linux x86_64 and ARM64, each server/musl and desktop/glibc; macOS Intel and Apple Silicon), plus4legacy updater aliases
4. Creates GitHub Release with binaries + checksums + auto release notes
5. **Verify the release**: `gh run list --limit 1` and check the Actions tab

Do NOT declare the release done until the workflow completes successfully
and the GitHub Release page shows all6primary binaries,4legacy aliases and the checksum manifest.

### 10.5 Update-Path Smoke — MANDATORY (added in v5.5.2)

After the release workflow publishes, verify that `temm1e update` from the
previous release actually lands on the new one. This step exists because
v5.5.1 shipped a broken updater across **four platforms** for multiple
releases — an asset-naming drift between `release.yml` and the in-binary
updater went undetected because the protocol never exercised the update
path post-publish. The symptom (`Error: No binary found for <target> in
release v...`) was invisible to anyone not actively upgrading.

Run on each maintainer's machine (covers at least one OS/arch naturally):

```bash
scripts/release_update_smoke.sh <PREV_TAG> <NEW_TAG>
# e.g. scripts/release_update_smoke.sh v5.5.1 v5.5.2
```

The script:
1. Downloads the previous release's binary for the current platform
2. Sanity-checks it reports the previous version
3. Runs its `update` subcommand
4. Asserts the binary now reports the new version

If this fails, the release is not shippable to existing users — roll
forward a hotfix rather than advancing the tag. An online compile-time
gate (`update_assets::every_updater_asset_is_published_by_release_yml`
in `src/update_assets.rs`) also runs on every PR and fails the build if
`release.yml`'s artifact matrix ever drifts from the updater's expected
asset list — so drift should be caught at `cargo test` time, not at
release time. This smoke remains the final empirical check.

## Files That Do NOT Need Updating

- **`docs/benchmarks/BENCHMARK_REPORT.md`** — Version in title reflects when benchmark was taken. Only update if benchmarks are re-run.
- **`crates/temm1e-skills/src/lib.rs`** — Test fixtures use hardcoded version strings. These are test data, not release metadata.
- **`Cargo.lock`** — Auto-generated from Cargo.toml changes.
- **Release Timeline old entries** — Historical entries are frozen. Never modify past versions.

## Common Mistakes

| Mistake | Consequence |
|---------|-------------|
| Bump README but not Cargo.toml | `temm1e -V` shows old version |
| Bump Cargo.toml but not README badges | GitHub page shows old version |
| Forget `temm1e update` example version | Users see wrong version in help output |
| Forget test count in dev section | `cargo test` comment says wrong number |
| Forget architecture tree | New crate invisible in docs |
| Forget CLAUDE.md crate count | Claude starts sessions with wrong context |
| Forget `default_model()` for new provider | Omitting model in config crashes with wrong default |
| Push without running tests | Broken code on main |
| Push without tagging | **No GitHub release created, no binaries built, users stuck on old version** |
| Tag before pushing commit | Tag points to wrong commit |
