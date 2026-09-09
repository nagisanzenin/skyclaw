//! Bounded bytes actually supplied to an evaluator, bound to the sealed Oath.
use crate::{
    error::WitnessError,
    types::{EvidenceKind, Oath, Verdict},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};
use tokio::io::AsyncReadExt;

pub const MAX_FILE_BYTES: usize = 16 * 1024;
pub const MAX_BUNDLE_BYTES: usize = 32 * 1024;
pub const MAX_REFERENCES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEvidenceSnapshot {
    pub schema_version: u32,
    pub id: String,
    pub root_goal_id: String,
    pub subtask_id: String,
    pub session_id: String,
    pub oath_hash: String,
    pub workspace: PathBuf,
    pub declared_path: PathBuf,
    pub relative_path: PathBuf,
    pub captured_at: chrono::DateTime<chrono::Utc>,
    pub content_sha256: String,
    pub content_bytes: usize,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateEvidence {
    pub predicate_index: usize,
    /// These exact bytes were supplied to the configured verifier. Capturing a
    /// path alone or metadata-only EvidenceProduced rows cannot populate this.
    pub snapshots: Vec<FileEvidenceSnapshot>,
}
#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub verdict: Verdict,
    pub evidence: Vec<PredicateEvidence>,
}
fn unavailable(reason: impl Into<String>) -> WitnessError {
    WitnessError::PredicateCheck(reason.into())
}
fn content_hash(content: &str) -> String {
    hex::encode(Sha256::digest(content.as_bytes()))
}

impl FileEvidenceSnapshot {
    /// Offline validation: the source may have changed or been removed after
    /// capture. Do not reopen it and substitute new bytes during replay.
    pub fn validate(&self, oath: &Oath, workspace: &Path) -> Result<(), WitnessError> {
        let matching: Vec<_> = oath
            .evidence_required
            .iter()
            .filter(|s| s.id == self.id)
            .collect();
        if self.schema_version != 1
            || self.root_goal_id != oath.root_goal_id
            || self.subtask_id != oath.subtask_id
            || self.session_id != oath.session_id
            || self.oath_hash != oath.sealed_hash
            || self.workspace != workspace
            || self.id.is_empty()
            || self.id.len() > 256
            || matching.len() != 1
            || self.relative_path.as_os_str().is_empty()
            || self
                .relative_path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || self.content.len() > MAX_FILE_BYTES
            || self.content_bytes != self.content.len()
            || self.content_sha256 != content_hash(&self.content)
        {
            return Err(unavailable("file snapshot binding or integrity mismatch"));
        }
        match &matching[0].kind {
            EvidenceKind::File { path } if path == &self.declared_path => Ok(()),
            _ => Err(unavailable(
                "snapshot does not match the sealed file source",
            )),
        }
    }
}

/// Capture regular UTF-8 files only. Other kinds require an execution-bound
/// event resolver; never run a command/network request merely to gather proof.
pub async fn capture_files(
    oath: &Oath,
    refs: &[String],
    workspace: &Path,
) -> Result<Vec<FileEvidenceSnapshot>, WitnessError> {
    if refs.is_empty() || refs.len() > MAX_REFERENCES || oath.evidence_required.len() > 128 {
        return Err(unavailable("missing or excessive evidence references"));
    }
    let root = tokio::fs::canonicalize(workspace)
        .await
        .map_err(|_| unavailable("workspace unavailable"))?;
    let mut ids = HashSet::new();
    let mut sources = Vec::new();
    // Validate every declaration before reading any source.
    for id in refs {
        if id.is_empty() || id.len() > 256 || !ids.insert(id) {
            return Err(unavailable(
                "empty, duplicate or oversized evidence reference",
            ));
        }
        let matching: Vec<_> = oath
            .evidence_required
            .iter()
            .filter(|s| &s.id == id)
            .collect();
        if matching.len() != 1 {
            return Err(unavailable(
                "evidence reference is absent or ambiguous in the sealed Oath",
            ));
        }
        let EvidenceKind::File { path } = &matching[0].kind else {
            return Err(unavailable(
                "source needs an execution-bound resolver; only file snapshots are available",
            ));
        };
        if path.as_os_str().len() > 4096 {
            return Err(unavailable("evidence path exceeds bound"));
        }
        sources.push((id, path));
    }
    let mut snapshots = Vec::new();
    let mut total = 0usize;
    for (id, declared_path) in sources {
        let path = if declared_path.is_absolute() {
            declared_path.clone()
        } else {
            root.join(declared_path)
        };
        let path = tokio::fs::canonicalize(path)
            .await
            .map_err(|_| unavailable("evidence file unavailable"))?;
        let relative_path = path
            .strip_prefix(&root)
            .map_err(|_| unavailable("evidence file is outside the workspace"))?
            .to_path_buf();
        let initial = tokio::fs::metadata(&path)
            .await
            .map_err(|_| unavailable("evidence metadata unavailable"))?;
        if !initial.is_file() || initial.len() > MAX_FILE_BYTES as u64 {
            return Err(unavailable("evidence must be a regular file within 16 KiB"));
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let mut file = options
            .open(&path)
            .await
            .map_err(|_| unavailable("evidence file cannot be opened safely"))?;
        let before = file
            .metadata()
            .await
            .map_err(|_| unavailable("evidence metadata unavailable"))?;
        if !before.is_file() || before.len() > MAX_FILE_BYTES as u64 {
            return Err(unavailable("evidence source changed or exceeds bound"));
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| unavailable("evidence read failed"))?;
        let after = file
            .metadata()
            .await
            .map_err(|_| unavailable("evidence metadata unavailable"))?;
        if bytes.len() > MAX_FILE_BYTES
            || before.len() != bytes.len() as u64
            || after.len() != before.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err(unavailable(
                "evidence changed during capture or exceeds bound",
            ));
        }
        total += bytes.len();
        if total > MAX_BUNDLE_BYTES {
            return Err(unavailable("evidence bundle exceeds 32 KiB"));
        }
        let content =
            String::from_utf8(bytes).map_err(|_| unavailable("evidence is not UTF-8 text"))?;
        let snapshot = FileEvidenceSnapshot {
            schema_version: 1,
            id: id.clone(),
            root_goal_id: oath.root_goal_id.clone(),
            subtask_id: oath.subtask_id.clone(),
            session_id: oath.session_id.clone(),
            oath_hash: oath.sealed_hash.clone(),
            workspace: root.clone(),
            declared_path: declared_path.clone(),
            relative_path,
            captured_at: chrono::Utc::now(),
            content_sha256: content_hash(&content),
            content_bytes: content.len(),
            content,
        };
        snapshot.validate(oath, &root)?;
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}
