# Durable reply delivery

Implementation contract with a final-reply implementation on the modernization branch. The current channel trait returns `Result<()>` from `send_message`; it provides neither a platform message receipt nor a reconciliation/idempotency capability. A generic transport error therefore cannot establish whether the platform accepted the message.

## State and transaction boundaries

1. Commit a final reply's exact destination and payload in the same transaction as its conversation history revision. A final reply cannot be presented before this transaction succeeds. Bind the destination to the authenticated conversation scope, not model-supplied routing text.
2. Each revision's final reply has a stable UUID and state: `pending` (no attempt admitted), `attempting` (attempt marker committed before transport), `accepted_by_sink` (sink returned success), or `outcome_unknown` (transport failed or timed out after admission). A process dying in `attempting` leaves an unknown outcome. Sink acceptance is not proof of user visibility, task completion or external tool success.
3. Claim `pending` with compare-and-swap before invoking the sink. Do not replay `attempting` or `outcome_unknown` through a generic retry loop. Do not use substring matches on error prose to infer safe redelivery.
4. Keep local conversation ownership across final delivery so another process cannot overtake its response. After process death, an unattempted pending reply can be explicitly resumed without repeating provider/tool work. An ambiguous attempt requires platform reconciliation or an informed separate user action; elapsed time alone does not authorize replay.
5. Do not discard pending or ambiguous delivery evidence when starting a new epoch. Scope all inspection/resume operations to the currently authorized channel/chat/workspace/access domain. Old-epoch replies must be identified as such before resending.
6. Bound reply payloads and inspection pages. Store errors without secrets. A limit or storage failure preserves prior canonical history and blocks the uncommitted final response rather than claiming success.

## Entrypoints

- CLI: persistence before terminal output; stdout acceptance is distinct from observed display.
- TUI: persistence before the final event enters the UI queue; queue acceptance is distinct from rendering. Preserve usage/stream lifecycle behavior.
- Server: use the outbox for final replies and Hive parent results. Platform chunking needs ordered delivery records, not one success flag for several unchecked sends. Early acknowledgements, administrative replies and tool-initiated messages require their own explicit coverage; do not claim them covered by a final-reply implementation.
- Keep return-of-reply, delivery acceptance and Witness/goal completion separate.

## Acceptance evidence

Use fake sinks and actual entrypoint fixtures. Assert atomic history+outbox commit; destination mismatch rejection; one admission under concurrent senders; crash before transport; crash after sink effect before acknowledgement; timeout/error after possible effect; repeated resume without a second send; scope/epoch separation; failure to persist acknowledgement; and UTF-8/payload bounds. An uncertain send must remain inspectable after restart and must not trigger automatic provider, tool or delivery replay.

Network-platform reconciliation, transport receipts and a full multi-part delivery protocol must be verified against each platform's current primary documentation before being claimed implemented.

## Implemented checkpoint and remaining coverage

CLI, TUI and server successful final replies (including Hive parent results) now commit a bounded, integrity-checked outbox payload atomically with canonical history. A stable conversation lock remains owned across delivery. Pending admission uses compare-and-swap; interrupted, failed, panicking and timed-out attempts never enter generic retry. Empty replies commit history without attempting a transport. Errors persist static diagnostic categories rather than raw transport secrets.

Owner commands inspect the latest 100 records, review saved text in 4,096-character pages, explicitly resume never-attempted replies in the current epoch, and record user-reported receipt. Earlier-epoch replies remain reviewable but cannot be resumed into a new conversation. Acknowledgement is separate from platform receipt and goal completion.

The payload cap is 1 MiB. The 30-second timeout is cooperative: blocking native I/O such as a full stdout pipe cannot be preempted by an async timeout. One logical record covers serial channel chunks; partial completion remains uncertain and is not replayed. Per-part receipts and platform reconciliation are **not implemented**. Interim acknowledgements, administrative/error responses and tool-initiated sends are outside this outbox. TUI acceptance means the final event entered its queue, not that the screen rendered it. Shutdown now joins the owned bridge task and separately reports optional background-hook drainage.

Validation includes store failure injection (including acknowledgement-write failure), concurrent resume, cancellation after a possible effect, scope/payload integrity, and archived-epoch separation. The actual CLI fixture imports 240 messages, verifies restart requests, then kills the process during a 256,000-byte reply with partially observed stdout. Restart retains the uncertain attempt, rejects replay without another provider request, and supports bounded review. Fixture evidence: `conversation-delivery-02/evidence.json` in local benchmark outputs. A live isolated GLM-5.3-Flash TUI run returned the saved `maple-7319 / amber` pair with eight stream deltas and an accepted final record after bridge shutdown. This is not a rendered-PTY or external-channel acceptance test.

Shared UTF-8 splitting now preserves every delimiter and indentation byte and always advances, including when a chunk starts with whitespace. Channel tests and root final-delivery tests exercise lossless splitting and no generic retry after uncertainty. Full server fake-transport dispatch/restart and platform-specific reconciliation remain release work.
