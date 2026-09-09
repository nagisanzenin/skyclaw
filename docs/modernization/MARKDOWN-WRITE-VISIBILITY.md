# Markdown store completion means visible bytes

Checkpoint 56 repairs a production write-completion bug exposed by checkpoint54 CI34295726293. After `MarkdownMemory::store(...).await` returned, an independent synchronous reader sometimes saw only the separator newline; a later reader saw the stored entry. This made the Engram unsupported-backend preservation test fail, even though Engram correctly refused to write.

## Cause and implementation

`MarkdownMemory::append_to_file` issued two Tokio `File::write_all` calls, then returned. Tokio's filesystem operations use background blocking work; its filesystem documentation explicitly states that a write can return before that work finishes and that `flush` waits for completion. The locked dependency's `src/fs/mod.rs` and `src/fs/file.rs` were inspected directly.

The append helper now awaits `file.flush()` after writing the entry. `store` acknowledges success only after the queued write completes, and completion errors propagate to its caller. Both daily conversation files and `MEMORY.md` use this same helper. Existing entry formatting, leading separators, routing, paths and file contents are preserved. No migration or real user data edits occur.

This is write visibility and error propagation, not `fsync`/power-loss durability. Concurrent append/delete atomicity, file locking, crash recovery and storage retention remain separate contracts. The unsupported-Engram test is retained unchanged; it must still prove that rejected Engram operations do not alter existing generic Markdown memory.

## Validation

The new regression calls the actual `Memory::store` path and immediately reads from an independent synchronous file handle, without a Tokio read or an intervening yield that could accidentally finish the pending append. It checks32successive entries in each of the long-term and conversation destinations, including Unicode payloads and complete trailing entry bytes.

Before the fix it failed at `visible-LongTerm-1`: store had returned before the complete entry was visible (`implementation-markdown-flush-before.log`). The unchanged regression passes with the flush. All75memory unit tests pass, one existing test remains ignored, and all7memory integration tests pass (`implementation-markdown-flush-memory-tests.log`). The exact Engram test that failed in CI also passes with the browser feature enabled (`implementation-markdown-flush-engram-test.log`).

Scoped memory/tools all-feature/all-target clippy passed33.04s and cleaned1.4GiB (`implementation-markdown-flush-clippy.log`); formatting and diff checks pass. Final remote CI on this revision must still pass; earlier green revisions do not replace that gate. The original checkpoint54 CI failure remains preserved in `implementation-witness-contract-ci-failed.log`.
