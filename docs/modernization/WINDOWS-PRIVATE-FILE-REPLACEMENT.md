# Bounded Windows private-file replacement

Checkpoint60, September9,2026.

## Evidence and change

Checkpoint58 Windows CI34299131475 failed in the unchanged concurrent-reader registry acceptance at custom_models.rs:517: `Failed to persist custom models: Access is denied. (os error 5)`. Earlier sequential writes and lock/scoped-preservation assertions passed. The log is preserved outside target as implementation-custom-model-storage-ci-failed.log. Exact filesystem/reader/scanner interleaving was not instrumented; no claim that an antivirus process was identified.

The pinned tempfile implementation persists with SetFileAttributesW then MoveFileExW(MOVEFILE_REPLACE_EXISTING). Windows handles and permissions can prevent replacement. See Microsoft's [MoveFileEx contract](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexa) and [rename handle-sharing explanation](https://devblogs.microsoft.com/oldnewthing/20211022-00/?p=105822).

`private_file::persist_private` retains the same fully written/synced NamedTempFile and retries Windows errors5(access denied),32(sharing violation),33(lock violation). There are at most51attempts with50 ten-millisecond sleeps. Nonmatching errors return immediately; permanent permission/handle errors return after the allowance. Temporary ownership remains RAII-cleaned on error. Unix uses its previous single persist.

Do not remove the destination to make rename succeed, truncate it, re-read/rebuild the model registry between attempts, retry arbitrary I/O errors, or report success after a failed persist. The caller's stable writer lock remains held throughout registry replacement. The shared helper also protects credential writers using it.

## Validation

Local core282unit tests pass, one existing ignored; one doc test passes. This includes the unchanged64replacement concurrent-reader registry test, malformed-file preservation, lock contention, Unix private permissions and symlink target preservation. Scoped all-target/all-feature core lint passed6.40s and cleaned326.1MiB; implementation-windows-private-file-clippy.log.

A Windows-only test deliberately opens the destination without delete sharing. A short50ms reader must allow replacement after release. A held reader must produce an error while preserving old bytes and leaving no temporary file; replacement must work after release. This is a deterministic handle-conflict fixture, not proof of the precise original CI interleaving. Both this test and the original full concurrent-reader test must pass in Windows CI before Windows acceptance is claimed. macOS cannot execute this platform branch.

## Limits

This bounds retry count and requested sleep(500ms), not filesystem-call or scheduler wall time. Synchronous callers can wait for that allowance on Windows. Persistent ACL, scanner or open-handle conflicts still return an actionable I/O error. No removal fallback, general durability/ACL hardening, hostile external-writer exclusion or multi-file atomicity is claimed. Unix parent-sync failures can still occur after publication, as before.
