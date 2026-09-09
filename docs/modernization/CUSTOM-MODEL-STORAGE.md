# Preserve custom-model configuration during updates

Checkpoint 58 repairs two storage defects in `custom_models.toml`: the save function claimed atomic replacement but wrote directly to the destination, and scoped mutations treated malformed/unreadable existing data as an empty registry. An ordinary add could therefore erase the user's previous configuration.

## Mutation contract

All cooperating writers now take the existing core `PrivateFileLock` on the stable `custom_models.lock` sidecar. Lock contention returns an actionable retry error without waiting indefinitely, altering the file or claiming success. The sidecar is retained; deleting/replacing its inode could defeat synchronization.

`upsert_custom_model` and `remove_custom_model` hold that lock across strict loading, modification and replacement. Only a genuinely missing file is treated as empty. Malformed TOML, unreadable files and invalid UTF-8 reject the mutation. Error messages do not reproduce raw file contents. Provider/name-scoped replacement and removal retain other entries.

`save_custom_models` explicitly replaces a complete caller-supplied snapshot under the same writer lock. It is not a compare-and-swap of an externally read stale snapshot or a field-level merge. Internal scoped mutations call a private writer helper while already holding the lock, avoiding recursive locking.

Writes use the existing `write_private_atomic` helper: a private same-directory temporary file, complete serialization/write and file sync, atomic persistence to the destination, and parent-directory sync on Unix. Newly created parent directories use owner-only permissions on Unix; the replacement file is0600. A symlink at the named registry path is replaced by the new regular file; its target remains unchanged. Users maintaining a symlink-backed dotfiles registry should account for that replacement behavior. The operation does not rewrite unrelated profiles or follow the symlink to mutate its target.

Cooperating unlocked readers see a complete prior or new document. Advisory locks do not constrain external editors/writers that ignore them; hostile parent-path races and a whole configuration transaction are not solved here. A sync error after rename can represent an ambiguous persistence outcome: inspect the file before assuming it stayed unchanged.

The legacy read-only loader still warns and returns registry fallback for malformed/unreadable data. This compatibility path is distinct from mutation permission. Bounded loading, propagating unavailable custom capabilities to all consumers, numeric configuration validation and partial-field concurrency semantics remain separate work. This checkpoint does not silently change model defaults or account entitlements.

## Validation

An isolated child process uses a temporary profile and the actual public storage APIs. Before the repair, attempting an upsert on malformed TOML succeeded and erased it (`implementation-custom-model-storage-before.log`). After the fix, add/remove preserve the malformed bytes, a held independent writer lock rejects add/remove/full replacement without changes, and released locks allow scoped updates/removal while retaining unrelated entries.

An independent reader parses TOML while64cooperating atomic replacements run; it sees complete documents. Unix checks confirm0600mode and preservation of a symlink target. Core282unit tests pass, one existing test remains ignored, and one documentation test passes. The expanded post-review fixture with symlink behavior also passes. Logs: `implementation-custom-model-storage-final-tests.log` and `implementation-custom-model-storage-post-review.log`.

Minimal CLI+TUI build passed51.97s (four existing feature-conditional unused-mut warnings). Actual Unix CLI acceptance, after configuring the saved route through `proxy`, verifies `/addmodel` and `/removemodel`, provider-scoped preservation, contention from a Python-held OS lock, malformed-file preservation and private mode, with zero provider requests (`implementation-custom-model-storage-cli-saved-route.log`). Script: `scripts/custom_model_storage_smoke.py`.

The first CLI runs exposed a separate real entrypoint bug: a working config-only connection is ignored by model commands, which re-read the credentials file and report no active provider. Those failures are retained in `implementation-custom-model-storage-cli.log` and `implementation-custom-model-storage-cli-v2.log`. The script's `--config-only` mode preserves this pending acceptance case. This is not fixed or called a fixture-only failure in58; it is the next task. Full workspace/all-feature/all-target clippy passed1m48s and cleaned2.3GiB (`implementation-custom-model-storage-workspace-clippy.log`); the existing dependency future-compatibility notice remains.
