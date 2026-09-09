//! Owned, bounded subprocess execution shared by tools and maintenance jobs.
use std::{
    io,
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
};

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("process I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("process timed out")]
    TimedOut,
}

#[derive(Debug)]
pub struct BoundedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}

struct OwnedChild {
    child: Child,
    pid: u32,
    armed: bool,
}

impl OwnedChild {
    fn terminate(&mut self) {
        if !self.armed {
            return;
        }
        #[cfg(unix)]
        {
            // SAFETY: the child was spawned in its own process group (PGID=PID).
            // kill accepts a negative PGID; no borrowed pointers are involved.
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        {
            // Best-effort tree cleanup until Windows Job Object ownership lands.
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &self.pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.start_kill();
        self.armed = false;
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

async fn capture(mut reader: impl AsyncRead + Unpin, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut result = Vec::new();
    let mut scratch = [0u8; 8192];
    let mut truncated = false;
    loop {
        let n = reader.read(&mut scratch).await?;
        if n == 0 {
            break;
        }
        let keep = n.min(limit.saturating_sub(result.len()));
        result.extend_from_slice(&scratch[..keep]);
        truncated |= keep < n;
    }
    Ok((result, truncated))
}

/// Retain at most `per_stream_limit` bytes from each pipe while continuing to
/// drain it. The deadline covers process exit AND pipe completion. On Unix,
/// timeout or future cancellation kills the owned process group. A descendant
/// that deliberately starts a new session is outside this group boundary.
pub async fn run_bounded(
    command: &mut Command,
    timeout: Duration,
    per_stream_limit: usize,
) -> Result<BoundedOutput, ProcessError> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("spawned process has no PID"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing stderr pipe"))?;
    let mut owned = OwnedChild {
        child,
        pid,
        armed: true,
    };
    let result = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            owned.child.wait(),
            capture(stdout, per_stream_limit),
            capture(stderr, per_stream_limit)
        )
    })
    .await;
    match result {
        Ok(Ok((status, (stdout, out_truncated), (stderr, err_truncated)))) => {
            owned.armed = false;
            Ok(BoundedOutput {
                status,
                stdout,
                stderr,
                truncated: out_truncated || err_truncated,
            })
        }
        Ok(Err(error)) => {
            owned.terminate();
            let _ = owned.child.wait().await;
            Err(ProcessError::Io(error))
        }
        Err(_) => {
            owned.terminate();
            let _ = owned.child.wait().await;
            Err(ProcessError::TimedOut)
        }
    }
}

/// Wait for Ctrl+C or the normal Unix service-stop signal.
pub async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn timeout_kills_descendants_before_late_effect() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", "(sleep 0.3; printf late > marker) & wait"])
            .current_dir(dir.path());
        assert!(matches!(
            run_bounded(&mut command, Duration::from_millis(50), 1024).await,
            Err(ProcessError::TimedOut)
        ));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!dir.path().join("marker").exists());
    }
    #[tokio::test]
    async fn future_cancellation_kills_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", "(sleep 0.3; printf late > marker) & wait"])
            .current_dir(dir.path());
        let work =
            tokio::spawn(
                async move { run_bounded(&mut command, Duration::from_secs(10), 1024).await },
            );
        tokio::time::sleep(Duration::from_millis(50)).await;
        work.abort();
        let _ = work.await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!dir.path().join("marker").exists());
    }
    #[tokio::test]
    async fn noisy_output_is_drained_without_unbounded_retention() {
        let mut command = Command::new("sh");
        command.args(["-c", "head -c 1000000 /dev/zero; printf err >&2"]);
        let output = run_bounded(&mut command, Duration::from_secs(5), 1024)
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 1024);
        assert_eq!(output.stderr, b"err");
        assert!(output.truncated);
    }
}
