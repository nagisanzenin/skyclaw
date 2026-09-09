# Upgrading to the modernization branch

This guide describes the6.0candidate. The published installer continues to install the latest published release until6.0is tagged and its artifacts pass release verification.

## Keep your existing data

Stop Tem cleanly and back up the entire selected profile before replacing the binary. The default profile is `~/.temm1e`; a custom `TEMM1E_DATA_DIR` selects another profile. Keep configuration, credentials, databases, skills, workspace and existing history together. Do not delete the old profile to resolve a migration error.

New histories use explicit workspace/channel/chat/access identity and a conversation epoch. The local workspace remains the profile's workspace; starting Tem from another directory does not switch projects. CLI and TUI have separate conversation identities. Existing unscoped histories are preserved. Use `/history-import` to preview and explicitly copy an old history into an empty conversation. Import never replays tools.

`/session-new` starts a new epoch without deleting old evidence. TUI `/clear` only clears the display. Interrupted turns can be inspected with `/session-recover`; recovery retains uncertainty and does not automatically repeat external effects.

## Connections and model selection

Keep API keys and subscription connections distinct. Z.ai Coding Plan uses its coding-plan endpoint, not the general API endpoint. Codex login uses the existing OAuth flow; neither login nor a catalog entry guarantees access to every model. Unknown subscription cost is displayed as unknown, not free.

CLI/server `/model <id>` changes the running Tem instance while keeping tools, memory and the owning budget. It does not save a new startup default or make a paid validation call. The next actual request establishes whether the account can use that model. Restart uses configured defaults. The server's selected model is shared by its running instance; a busy runtime asks you to retry after the active turn. Eigen-Tune local routing rejects reference-model changes without requalification. TUI has its own configuration/replacement flow; do not assume every interface persists model selection identically.

A finite dollar budget cannot safely authorize calls with unknown/subscription prices; choose an appropriate explicit policy instead of assuming a zero price. A local call limit is not the provider's account-wide subscription quota. Usage includes owned auxiliary calls; uncertain failed/cancelled calls retain unknown usage. Global dollar accounting remains a threshold, not atomic per-goal reservation.

## Replies and verification

Final replies are committed before delivery. `/delivery-status` and `/delivery-show` let you inspect them. `/delivery-resume` only resends a never-attempted reply in the current conversation. An interrupted send may already have reached the destination; Tem preserves that ambiguity rather than automatically duplicating it. `/delivery-ack` records your acknowledgment, not proof that the task succeeded.

`/goal-status` and `/goal-assessment` expose recorded execution state and checks. A returned answer is not automatically a completed goal. Model-proposed checks do not prove that the whole objective is covered; a model judgment is not proof that a command ran. Automatic durable pursuit across multiple conversations is not introduced by this release.

## Browser and host boundaries

Browser work uses private Tem-owned profiles. Automatic copying of personal Chrome cookies and deletion of personal Chrome locks are removed. Existing explicit `TEMM1E_BROWSER_IMPORT_FROM` import is bounded; review the selected source. Existing personal Chrome data is preserved.

Tem remains a personal-host agent. Shared deployment requires deliberate access configuration; role names and scoped conversation records do not establish an OS sandbox or complete tenant isolation for every memory/vault/tool backend. Do not expose a privileged personal profile as an isolated multi-tenant service.

## Disk and rollback

The new updater verifies release metadata, checksums and the downloaded executable before atomic replacement; it no longer rebuilds from Cargo in the installed updater. Developers on constrained disks should use `python3 scripts/cargo_guard.py -- <cargo arguments>`; it reserves8GiB and cleans generated outputs. The profile itself does not yet have a global retention quota.

Keep the previous binary and the pre-upgrade profile backup. To roll back, stop the candidate, preserve its current profile separately, then restore the matching old binary and old profile backup. Do not point an older binary at a profile it cannot understand and assume reverse migration is safe. Published update-path verification is recorded only after release assets exist.
