//! Integration tests for the deploy module against real fake binaries.
//!
//! These tests use shell scripts as "binaries" and exercise the full
//! Deployer.swap() protocol against an isolated tempdir. No real temm1e
//! binary is touched.
//!
//! **Unix-only.** The fake binaries are `#!/bin/sh` shell scripts that
//! Windows `CreateProcess` cannot execute without a `.bat`/`.cmd` extension
//! and a compatible interpreter. The matching production codepaths
//! (kill/taskkill split, validate, swap) are covered by cross-platform
//! unit tests inside `src/deploy.rs`, so no Windows coverage is lost.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use temm1e_cambium::deploy::{DeployConfig, DeployOutcome, Deployer};
use tempfile::tempdir;
use tokio::process::Command;

/// Write a fake binary to `path`, setting it executable on Unix.
///
/// Uses **synchronous** `std::fs` + explicit `sync_all` to defeat a Linux
/// ETXTBSY race: `tokio::fs::write` runs the underlying `close(2)` on a
/// blocking-pool thread, and on Linux the subsequent `exec(2)` from the
/// test can briefly observe an open write fd (errno 26, "Text file busy").
/// Doing the open/write/sync/close fully on the current thread before
/// returning eliminates the cross-thread window.
fn write_executable_script(path: &Path, script: &str) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    f.sync_all().unwrap();
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }
}

/// Write a small shell-script binary that responds to --version, then
/// sleeps for `sleep_secs` if invoked with "start".
fn write_fake_temm1e(path: &Path, version: &str, sleep_secs: u64) {
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then\n\
           echo 'temm1e {version}'\n\
           exit 0\n\
         fi\n\
         if [ \"$1\" = \"start\" ]; then\n\
           exec sleep {sleep_secs}\n\
         fi\n\
         exit 0\n"
    );
    write_executable_script(path, &script);
}

/// Write a binary that crashes immediately on `start`.
fn write_crashing_temm1e(path: &Path) {
    let script = "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then\n\
           echo 'temm1e crash 1.0.0'\n\
           exit 0\n\
         fi\n\
         if [ \"$1\" = \"start\" ]; then\n\
           exit 1\n\
         fi\n\
         exit 0\n";
    write_executable_script(path, script);
}

/// Write a binary that fails the --version check.
fn write_broken_version_temm1e(path: &Path) {
    let script = "#!/bin/sh\n\
         if [ \"$1\" = \"--version\" ]; then\n\
           echo 'crash on version' >&2\n\
           exit 1\n\
         fi\n\
         exit 0\n";
    write_executable_script(path, script);
}

// Uses POSIX shell scripts with `#!/bin/sh` shebang as fake binaries.
// Windows `CreateProcess` cannot execute shebang-only files and has no
// direct PowerShell equivalent that exercises the Deployer's spawn path the
// same way. The matching production codepaths (kill/taskkill, validate,
// swap) are covered by cross-platform unit tests in src/deploy.rs.
#[cfg(unix)]
#[tokio::test]
async fn deploy_swap_replaces_old_binary_with_new() {
    let tmp = tempdir().unwrap();
    let installed = tmp.path().join("install/temm1e");
    let new = tmp.path().join("staging/temm1e");
    let rollback_dir = tmp.path().join("rollback");
    let staging_dir = tmp.path().join("staging");

    tokio::fs::create_dir_all(installed.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::create_dir_all(new.parent().unwrap())
        .await
        .unwrap();

    write_fake_temm1e(&installed, "1.0.0", 60);
    write_fake_temm1e(&new, "2.0.0", 60);

    let config = DeployConfig {
        installed_binary: installed.clone(),
        rollback_dir: rollback_dir.clone(),
        staging_dir,
        pid_file: None, // No process management for this test.
        post_start_health_check: None,
    };

    let deployer = Deployer::new(config);
    let report = deployer.swap(&new).await;

    println!("Outcome: {:?}", report.outcome);
    println!("Timings: {:?}", report.timings);

    match report.outcome {
        DeployOutcome::Success { .. } => {}
        other => panic!("expected Success, got {other:?}"),
    }

    // The installed binary should now respond as 2.0.0
    let output = Command::new(&installed)
        .arg("--version")
        .output()
        .await
        .unwrap();
    let version_text = String::from_utf8_lossy(&output.stdout);
    assert!(version_text.contains("2.0.0"), "version is: {version_text}");

    // A backup should exist
    let backups: Vec<_> = std::fs::read_dir(&rollback_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(backups.len(), 1);
    println!("Backup file: {}", backups[0].file_name().to_string_lossy());
}

#[cfg(unix)]
#[tokio::test]
async fn deploy_swap_aborts_on_broken_version() {
    let tmp = tempdir().unwrap();
    let installed = tmp.path().join("install/temm1e");
    let new = tmp.path().join("staging/temm1e");
    tokio::fs::create_dir_all(installed.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::create_dir_all(new.parent().unwrap())
        .await
        .unwrap();

    write_fake_temm1e(&installed, "1.0.0", 60);
    write_broken_version_temm1e(&new);

    let config = DeployConfig {
        installed_binary: installed.clone(),
        rollback_dir: tmp.path().join("rollback"),
        staging_dir: tmp.path().join("staging"),
        pid_file: None,
        post_start_health_check: None,
    };

    let deployer = Deployer::new(config);
    let report = deployer.swap(&new).await;

    match report.outcome {
        DeployOutcome::AbortedBeforeSwap { stage, .. } => {
            assert_eq!(stage, "validate");
        }
        other => panic!("expected AbortedBeforeSwap(validate), got {other:?}"),
    }

    // The installed binary should still report 1.0.0 — untouched.
    let output = Command::new(&installed)
        .arg("--version")
        .output()
        .await
        .unwrap();
    let version_text = String::from_utf8_lossy(&output.stdout);
    assert!(version_text.contains("1.0.0"));
}

#[cfg(unix)]
#[tokio::test]
async fn deploy_swap_with_pid_file_starts_new_process() {
    let tmp = tempdir().unwrap();
    let installed = tmp.path().join("install/temm1e");
    let new = tmp.path().join("staging/temm1e");
    let pid_file = tmp.path().join("temm1e.pid");
    tokio::fs::create_dir_all(installed.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::create_dir_all(new.parent().unwrap())
        .await
        .unwrap();

    // Write old binary that sleeps long enough for the test
    write_fake_temm1e(&installed, "1.0.0", 30);
    write_fake_temm1e(&new, "2.0.0", 30);

    // Pretend the old binary is "running" — start it ourselves and write its PID.
    let child = std::process::Command::new(&installed)
        .arg("start")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let old_pid = child.id();
    std::fs::write(&pid_file, old_pid.to_string()).unwrap();
    std::mem::forget(child);

    // Give the process a moment to actually start
    tokio::time::sleep(Duration::from_millis(200)).await;

    let config = DeployConfig {
        installed_binary: installed.clone(),
        rollback_dir: tmp.path().join("rollback"),
        staging_dir: tmp.path().join("staging"),
        pid_file: Some(pid_file.clone()),
        post_start_health_check: None,
    };

    let deployer = Deployer::new(config);
    let report = deployer.swap(&new).await;

    println!("Outcome: {:?}", report.outcome);
    println!("Timings: total={}ms", report.timings.total_ms);

    let new_pid = match report.outcome {
        DeployOutcome::Success { new_pid, .. } => new_pid,
        other => panic!("expected Success, got {other:?}"),
    };

    let new_pid = new_pid.expect("new_pid should be set when pid_file is configured");
    assert_ne!(new_pid, old_pid, "should have spawned a new process");

    // Cleanup: kill the new process.
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &new_pid.to_string()])
        .status();
    let _ = std::fs::remove_file(&pid_file);
}

#[cfg(unix)]
#[tokio::test]
async fn deploy_swap_rollback_on_failed_start() {
    let tmp = tempdir().unwrap();
    let installed = tmp.path().join("install/temm1e");
    let new = tmp.path().join("staging/temm1e");
    let pid_file = tmp.path().join("temm1e.pid");
    tokio::fs::create_dir_all(installed.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::create_dir_all(new.parent().unwrap())
        .await
        .unwrap();

    write_fake_temm1e(&installed, "1.0.0", 30);
    // New binary passes --version but crashes on start.
    write_crashing_temm1e(&new);

    // No running process for this test — we want to focus on the
    // start-then-crash detection.
    let config = DeployConfig {
        installed_binary: installed.clone(),
        rollback_dir: tmp.path().join("rollback"),
        staging_dir: tmp.path().join("staging"),
        pid_file: Some(pid_file),
        post_start_health_check: None,
    };

    let deployer = Deployer::new(config);
    let report = deployer.swap(&new).await;

    println!("Outcome: {:?}", report.outcome);

    match report.outcome {
        DeployOutcome::RolledBack {
            stage,
            backup_restored,
            ..
        } => {
            // The crash is detected during health_check (the 2-second wait
            // catches the immediate exit).
            assert!(
                stage == "health_check" || stage == "start",
                "unexpected rollback stage: {stage}"
            );
            assert!(backup_restored, "expected backup to be restored");
        }
        other => panic!("expected RolledBack, got {other:?}"),
    }

    // The installed binary should be the old 1.0.0 again
    let output = Command::new(&installed)
        .arg("--version")
        .output()
        .await
        .unwrap();
    let version_text = String::from_utf8_lossy(&output.stdout);
    assert!(
        version_text.contains("1.0.0"),
        "rollback failed, version is: {version_text}"
    );
}

#[tokio::test]
#[ignore = "requires a prebuilt target/release/temm1e binary; run explicitly with --ignored"]
async fn deploy_validate_finds_real_temm1e() {
    // Explicit smoke test against an actual Rust binary, not a shell fixture.
    let workspace = std::env::current_dir()
        .unwrap()
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    let bin = workspace.join("target/release/temm1e");
    assert!(
        bin.exists(),
        "build target/release/temm1e before running this explicit smoke test"
    );

    let tmp = tempdir().unwrap();
    let config = DeployConfig {
        installed_binary: tmp.path().join("install/temm1e"),
        rollback_dir: tmp.path().join("rollback"),
        staging_dir: tmp.path().join("staging"),
        pid_file: None,
        post_start_health_check: None,
    };
    let deployer = Deployer::new(config);
    let result = deployer.validate_new_binary(&bin).await;
    println!("validate result: {result:?}");
    assert!(
        result.is_ok(),
        "real temm1e --version should pass: {result:?}"
    );
}

#[tokio::test]
async fn deploy_outcome_serializes_to_debug() {
    // Cheap sanity test — Debug formatting works for all outcomes.
    let outcome = DeployOutcome::Success {
        new_binary_path: PathBuf::from("/x"),
        backup_path: PathBuf::from("/y"),
        new_pid: Some(42),
    };
    let s = format!("{outcome:?}");
    assert!(s.contains("Success"));
    assert!(s.contains("42"));
}
