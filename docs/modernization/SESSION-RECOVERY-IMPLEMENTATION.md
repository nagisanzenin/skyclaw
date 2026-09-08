# Session recovery and legacy history migration

Implementation contract with partial implementation recorded below. CLI/TUI and server workers now use canonical conversation heads; full entrypoint acceptance and the delivery outbox remain unfinished. Do not infer complete recovery coverage from the store tests.

## Compatibility policy

Preserve existing conversations. Legacy `chat_history:cli`, `chat_history:tui`, and server chat keys do not establish the workspace in which their messages were created. Do not assign them to a new workspace by guessing. Provide an explicit import into an empty conversation and leave the source untouched. The creator asked for a plain-language explanation of this issue, not a particular database design; explicit import is the working migration assumption, not a recorded creator vote.

CLI/TUI currently use Tem's profile workspace by default, rather than necessarily the launch directory. Preserve that default while making the actual workspace visible. A scope key alone does not make launch directories independent projects. An explicit workspace selection must consistently reach tools, skills, Witness, history and compaction before advertising project switching.

Keep existing group-chat conversation sharing. Do not silently convert shared channel conversations into isolated personal conversations; personal-host versus isolated-server policy is decision D01. Authorization to access a conversation and permission to invoke a tool are separate checks.

## Storage contract

1. A conversation has a stable UUID, canonical workspace, channel identity, chat identity, an explicit access domain, and an active epoch. Serialize scope fields structurally; do not concatenate ambiguous separators. Local terminal access domains use the local owner; group access domains refer to the channel conversation and current admission policy.
2. Each epoch has an append-only ordered event stream and a compare-and-swap revision. Events include inbound identity, raw messages, tool intent, result or unknown outcome, returned reply, and explicit reset/import boundaries. Store original message roles and content blocks without prose conversion.
3. Tool intent and its history boundary must commit before dispatch. Committing a result and advancing the event head must be one transaction. A crash after an external effect but before a result is an unknown outcome, never an automatic retry instruction.
4. Use immutable, content-addressed payload rows and ordered references. A summary generation refers to exact event/source IDs. Do not duplicate the entire history on every operation or rebuild a saved summary from another saved summary.
5. Enforce an active-session byte limit before admission and bound individual events. Load/present pages without deleting canonical events. An oversized session pauses with an explicit archive/new-conversation choice; do not silently drop instructions to keep the request running.
6. Use a session lease with owner ID, expiry and revision fencing to prevent two writers from independently dispatching the same inbound event. A stale lease holder cannot commit. A provider timeout does not prove an external tool effect failed.
7. Delivery is an outbox record separate from task completion. Persist destination, message identity and delivery state. Reconcile platform idempotency capabilities; when delivery outcome is uncertain and the platform cannot reconcile it, show that uncertainty instead of claiming exactly-once delivery.

## Entrypoint work

- Replace the server's raw 200-message deletion with event-backed loading. Its current in-memory history cannot remain the canonical source if old instructions must survive compaction and restart.
- Replace CLI/TUI global history keys with explicit conversation selection. Use the same loader/compiler as server workers; avoid another entrypoint-specific compaction policy.
- Add `/history-import` to the local CLI and TUI command registries. Present the legacy source name and message count before importing; require an empty destination, reject malformed/oversized data, and persist the imported boundary atomically. The command must never reach the model as an instruction to manipulate storage.
- Preserve the legacy entry. Repeated imports into the same destination are idempotent through an import-source digest. Import does not replay tools or send historical replies.
- Add an explicit new-conversation/reset epoch. Clearing the TUI display remains a display-only action and must not secretly reset or restore agent memory.
- Reopen the same epoch after a clean restart. For an interrupted epoch, restore evidence and surface unfinished work; reconcile unknown effects before continuation. Keep reply-returned separate from goal-achieved, consistent with the existing journal.
- Handoffs must match the restored event prefix and epoch. Never combine a handoff with a truncated or unrelated caller-provided history just because lengths happen to match.

## Required acceptance evidence

Use isolated profiles and fake channels/provider fixtures, plus a live CLI/TUI check after deterministic tests pass:

- More than 200 messages, two compactions, restart, and exact recall of an earliest instruction and a later correction.
- Two canonical workspaces and identical chat labels remain separate; a symlink alias of one workspace resolves consistently. Group sharing remains deliberate.
- Legacy history stays intact after import, cancellation and failure; an existing nonempty destination cannot be overwritten.
- Concurrent writers receive one admission; stale generation/lease updates fail; a duplicate inbound ID does not execute a second tool.
- Kill immediately before dispatch, during a tool, after effect/before result, and after reply persistence/before delivery. Verify unknown effects and delivery uncertainty are visible and never treated as success.
- Reset creates a new epoch; old handoffs do not reappear in the new conversation. Display clear leaves the epoch unchanged.
- Corrupt/missing source payloads fail explicitly. Byte-limit failures preserve all previously committed events.
- All entrypoints use the same compiler, scope, epoch and recovery rules. A unit-tested store with unchanged entrypoint loaders does not satisfy this work.

## Admission checkpoint now implemented

The execution journal now has a transactional inbound-claim table and an index for prior execution lookup. One caller can admit a message in a scope; duplicates are rejected before foreground provider/tool work. The migration checks old execution evidence rather than assuming a new claim table means a message is new. The reconciliation lookup returns all matching legacy records. Leases, takeover, event-backed entrypoint history, import commands and outbox delivery remain unfinished; do not treat this checkpoint as the entire recovery contract.

## Execution checkpoint payloads now deduplicated

New execution checkpoints use ordered references to immutable scoped message payloads. Readers reconstruct native messages and verify their hashes; legacy inline checkpoints still load. The same payload may appear repeatedly in one history without losing those repetitions. A transaction commits references together with the execution boundary. This implements the checkpoint storage portion of the contract, but entrypoint conversation selection and an append-only epoch event stream are still outstanding. Existing legacy rows are not rewritten or vacuumed automatically.

## Canonical CLI/TUI checkpoint

CLI and TUI now acquire an authoritative native history under a stable private OS file lock, use its UUID epoch as `SessionContext.session_id`, and commit an immutable revision boundary before presenting a final response. The head advances with owner/epoch/revision fencing. A failed commit, cancellation of the entrypoint future or process death leaves a durable busy marker. Read corruption fails explicitly before creating that marker. Commits reject shortened or rewritten saved prefixes.

Both local interfaces share the same management command handler. Import requires a digest of the current source, an empty destination, and retains the legacy memory entry. Repeating the same import does not duplicate history. New conversation preserves prior epoch boundaries and prevents old context handoffs from appearing in the new epoch. TUI commands are blocked while a turn is active; display clear remains separate.

Execution admission now links canonical epochs and their revision to exact execution IDs. Recovery reads only executions belonging to the interrupted revision. It previews recorded operation states and unmatched native calls; confirmation is bound to the evidence digest. Unmatched native calls receive explicit unknown-outcome results, even if another journal record exists whose operation ID cannot be mapped to that native call. No provider, tool or delivery is invoked by recovery. Recorded results remain distinct from task completion and delivery success.

For local processes, ownership uses the OS lock lifetime instead of expiry-based takeover: a slow provider must not authorize a competing process while its tool may still be running. The durable marker survives lock release after process death, and recovery/reset require explicit user action. This is an intentional local-host implementation of the exclusion requirement, not a claim of distributed fencing on network filesystems. Multi-host leasing remains separate work.

Still outstanding: server history migration, transactional tool-result/event-head integration, durable delivery outbox and platform reconciliation, complete process-kill/PTY acceptance, archive UI and total-storage retention policy. The existing server's 200-message deletion remains until its entrypoint is converted; this checkpoint does not claim parity.

## Server storage conversion

Server workers share one conversation-store pool and load the head under the same local lock as CLI/TUI. Scope includes the canonical workspace, channel and chat, with a shared admitted-members access domain. Heartbeats select the destination channel's conversation instead of a separate heartbeat history. The previous 200-message deletion and global legacy-key overwrites are removed. Existing admin command authorization gates import/recover/reset; the shared command handler itself does not grant authorization.

Normal replies, Hive result text and fallback replies commit the parent history before final delivery. A failed history commit suppresses that final response and retains the interrupted marker. Panic returns no longer restore an older history over partial evidence. Early-return cleanup clears transient busy flags, while durable recovery state remains intact. The secret-censor channel forwards the underlying channel's role resolution and role management rather than silently substituting trait defaults.

This is storage wiring, not full server acceptance. Actual channel dispatch/restart with a fake transport, cross-channel queue/pending-message isolation, attachment pre-processing evidence, delegated worker event linkage, and delivery crash reconciliation remain required. The local CLI fixture and server health/shutdown fixture do not establish those missing results.
