//! Update the installed executable, never the caller's Git checkout.
//! Download once into a bounded temporary file; verify hash and executable
//! version before an atomic replacement. Failed validation leaves the old file.
use anyhow::{bail, Context, Result};
use futures::StreamExt;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path, time::Duration};
use tokio::io::AsyncReadExt;

const MAX_BINARY_BYTES: u64 = 128 * 1024 * 1024;
const DISK_RESERVE_BYTES: u64 = 256 * 1024 * 1024;
const CHECKSUM_ASSET: &str = "checksums-sha256.txt";

struct Source {
    release_api: String,
    download_root: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Current,
    Installed(Version),
}

pub async fn run() -> Result<()> {
    let executable =
        std::env::current_exe().context("Cannot locate the running Temm1e executable")?;
    let candidates =
        crate::update_assets::asset_candidates(std::env::consts::OS, std::env::consts::ARCH)
            .context(
                "This platform has no published updater binary; use the source-build instructions",
            )?;
    // Preserve the running Linux binary's desktop/server class rather than
    // downloading a glibc desktop binary onto a musl-only machine.
    let desktop = cfg!(all(feature = "desktop-control", not(target_env = "musl")));
    let asset = if desktop {
        candidates[0]
    } else {
        candidates[candidates.len() - 1]
    };
    let client = reqwest::Client::builder()
        .user_agent("temm1e-updater")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()?;
    let source = Source {
        release_api: "https://api.github.com/repos/temm1e-labs/temm1e/releases/latest".into(),
        download_root: "https://github.com/temm1e-labs/temm1e/releases/download".into(),
    };
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    println!("Checking the official release for {asset} (current v{current})...");
    match update(&client, &source, &executable, asset, &current).await? {
        Outcome::Current => println!("Already on this release or a newer version."),
        Outcome::Installed(version) => {
            println!("Updated v{current} → v{version}: {}", executable.display());
            println!("Restart Temm1e to use the new executable. Source checkouts and profile data were not modified.");
        }
    }
    Ok(())
}

async fn small_body(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .context("Release request failed")?;
    if !response.status().is_success() {
        bail!("Release server returned {}", response.status());
    }
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        bail!("Release metadata exceeds size limit");
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Release metadata download interrupted")?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            bail!("Release metadata exceeds size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn checksum(manifest: &[u8], asset: &str) -> Result<[u8; 32]> {
    let manifest = std::str::from_utf8(manifest).context("Checksum manifest is not UTF-8")?;
    let mut selected = None;
    for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2 {
            bail!("Malformed checksum manifest");
        }
        if fields[1].trim_start_matches('*') != asset {
            continue;
        }
        if selected.is_some() {
            bail!("Duplicate checksum entry for the selected binary");
        }
        let bytes = hex::decode(fields[0]).context("Invalid SHA-256 digest")?;
        selected = Some(
            bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("SHA-256 must contain 32 bytes"))?,
        );
    }
    selected.context("Checksum manifest does not contain the selected binary")
}

async fn update(
    client: &reqwest::Client,
    source: &Source,
    executable: &Path,
    asset: &str,
    current: &Version,
) -> Result<Outcome> {
    let parent = executable
        .parent()
        .context("Executable has no parent directory")?;
    let lock_path = parent.join(".temm1e-update.lock");
    let _lock = temm1e_core::private_file::PrivateFileLock::try_exclusive(&lock_path)?
        .context("Another Temm1e update is running in this install directory")?;
    let release: Release =
        serde_json::from_slice(&small_body(client, &source.release_api, 1024 * 1024).await?)
            .context("Malformed release metadata")?;
    if release.draft || release.prerelease {
        bail!("Automatic updater only accepts stable published releases");
    }
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .context("Invalid release version")?;
    if !version.pre.is_empty() {
        bail!("Automatic updater does not install prereleases");
    }
    if !version.cmp_precedence(current).is_gt() {
        return Ok(Outcome::Current);
    }
    for required in [asset, CHECKSUM_ASSET] {
        if release
            .assets
            .iter()
            .filter(|item| item.name == required)
            .count()
            != 1
        {
            bail!("Release must contain exactly one {required}; current executable was preserved");
        }
    }
    let base = format!(
        "{}/{}",
        source.download_root.trim_end_matches('/'),
        release.tag_name
    );
    let expected = checksum(
        &small_body(client, &format!("{base}/{CHECKSUM_ASSET}"), 64 * 1024).await?,
        asset,
    )?;
    if fs2::available_space(parent)? < DISK_RESERVE_BYTES + MAX_BINARY_BYTES {
        bail!("Update requires at least 384 MiB free; no build or installation was started");
    }
    let response = client
        .get(format!("{base}/{asset}"))
        .send()
        .await
        .context("Binary download failed")?;
    if !response.status().is_success() {
        bail!("Binary download returned {}", response.status());
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_BINARY_BYTES)
    {
        bail!("Release binary exceeds the 128 MiB limit");
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".temm1e-download-")
        .tempfile_in(parent)?;
    let mut stream = response.bytes_stream();
    let mut hash = Sha256::new();
    let mut downloaded = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.context("Binary download interrupted; current executable was preserved")?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MAX_BINARY_BYTES {
            bail!("Release binary exceeds the 128 MiB limit");
        }
        if fs2::available_space(parent)? < DISK_RESERVE_BYTES + chunk.len() as u64 {
            bail!("Download stopped to preserve 256 MiB of free disk space");
        }
        hash.update(&chunk);
        temporary.write_all(&chunk)?;
    }
    let actual: [u8; 32] = hash.finalize().into();
    if downloaded == 0 || actual != expected {
        bail!("Release checksum mismatch; current executable was preserved");
    }
    temporary.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o755))?;
    }
    // Close the writable descriptor before executing (required by Linux ETXTBSY).
    let temporary = temporary.into_temp_path();
    verify_executable(&temporary, &version).await?;
    temporary
        .persist(executable)
        .map_err(|error| error.error)
        .context("Validated binary could not replace the installed executable")?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(Outcome::Installed(version))
}

async fn verify_executable(path: &Path, expected: &Version) -> Result<()> {
    let mut child = tokio::process::Command::new(path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Downloaded binary cannot run on this platform")?;
    let stdout = child
        .stdout
        .take()
        .context("Version probe stdout unavailable")?;
    let mut output = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        stdout.take(4097).read_to_end(&mut output).await?;
        child.wait().await
    })
    .await;
    let status = match result {
        Ok(status) => status?,
        Err(_) => {
            let _ = child.kill().await;
            bail!("Downloaded binary version check timed out; current executable was preserved");
        }
    };
    if !status.success() || output.len() > 4096 {
        bail!("Downloaded binary failed its version check");
    }
    let text = std::str::from_utf8(&output).context("Invalid version output")?;
    let fields: Vec<_> = text.split_whitespace().collect();
    if fields.len() < 2
        || fields[0] != "temm1e"
        || Version::parse(fields[1]).ok().as_ref() != Some(expected)
    {
        bail!("Downloaded binary reports an unexpected version; current executable was preserved");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn checksum_requires_one_exact_asset_entry() {
        let digest = "ab".repeat(32);
        assert_eq!(
            checksum(format!("{digest}  temm1e-test\n").as_bytes(), "temm1e-test").unwrap(),
            [0xab; 32]
        );
        for manifest in [
            format!("{digest}  other\n"),
            format!("{digest}  temm1e-test\n{digest}  temm1e-test\n"),
            "bad  temm1e-test\n".into(),
        ] {
            assert!(checksum(manifest.as_bytes(), "temm1e-test").is_err());
        }
    }

    async fn fixture(bodies: Vec<Vec<u8>>) -> (Source, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let source = Source {
            release_api: format!("http://{address}/release"),
            download_root: format!("http://{address}/downloads"),
        };
        let server = tokio::spawn(async move {
            let paths = [
                "/release",
                "/downloads/v6.0.0/checksums-sha256.txt",
                "/downloads/v6.0.0/temm1e-test",
            ];
            for (index, body) in bodies.into_iter().enumerate() {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut headers = Vec::new();
                loop {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).await.unwrap();
                    headers.push(byte[0]);
                    assert!(headers.len() < 16384);
                    if headers.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(String::from_utf8(headers)
                    .unwrap()
                    .starts_with(&format!("GET {} HTTP/1.1\r\n", paths[index])));
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        (source, server)
    }
    fn release(tag: &str, include_checksum: bool) -> Vec<u8> {
        let mut assets = vec![serde_json::json!({"name":"temm1e-test"})];
        if include_checksum {
            assets.push(serde_json::json!({"name":CHECKSUM_ASSET}));
        }
        serde_json::to_vec(
            &serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false,"assets":assets}),
        )
        .unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn update_checks_hash_and_runnable_version_without_touching_checkout() {
        for (valid_hash, version) in [(true, "6.0.0"), (false, "6.0.0"), (true, "7.0.0")] {
            let root = tempfile::tempdir().unwrap();
            let executable = root.path().join("temm1e");
            std::fs::write(&executable, b"original executable").unwrap();
            std::fs::create_dir(root.path().join(".git")).unwrap();
            std::fs::write(
                root.path().join(".git/HEAD"),
                b"ref: refs/heads/user-work\n",
            )
            .unwrap();
            std::fs::write(root.path().join("user-work.txt"), b"uncommitted work").unwrap();
            let binary = format!("#!/bin/sh\nprintf 'temm1e {version}\\n'\n").into_bytes();
            let digest = if valid_hash {
                hex::encode(Sha256::digest(&binary))
            } else {
                "00".repeat(32)
            };
            let (source, server) = fixture(vec![
                release("v6.0.0", true),
                format!("{digest}  temm1e-test\n").into_bytes(),
                binary.clone(),
            ])
            .await;
            let result = update(
                &reqwest::Client::new(),
                &source,
                &executable,
                "temm1e-test",
                &Version::parse("5.8.1").unwrap(),
            )
            .await;
            server.await.unwrap();
            if valid_hash && version == "6.0.0" {
                assert_eq!(
                    result.unwrap(),
                    Outcome::Installed(Version::parse("6.0.0").unwrap())
                );
                assert_eq!(std::fs::read(&executable).unwrap(), binary);
            } else {
                assert!(result.is_err());
                assert_eq!(std::fs::read(&executable).unwrap(), b"original executable");
            }
            assert_eq!(
                std::fs::read(root.path().join(".git/HEAD")).unwrap(),
                b"ref: refs/heads/user-work\n"
            );
            assert_eq!(
                std::fs::read(root.path().join("user-work.txt")).unwrap(),
                b"uncommitted work"
            );
            assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".temm1e-download-")));
        }
    }

    #[tokio::test]
    async fn missing_checksums_and_older_releases_never_download_a_binary() {
        for (tag, with_checksum) in [("v6.0.0", false), ("v5.7.0", true)] {
            let root = tempfile::tempdir().unwrap();
            let executable = root.path().join("temm1e");
            std::fs::write(&executable, b"original executable").unwrap();
            let (source, server) = fixture(vec![release(tag, with_checksum)]).await;
            let result = update(
                &reqwest::Client::new(),
                &source,
                &executable,
                "temm1e-test",
                &Version::parse("5.8.1").unwrap(),
            )
            .await;
            server.await.unwrap();
            if with_checksum {
                assert_eq!(result.unwrap(), Outcome::Current);
            } else {
                assert!(result.is_err());
            }
            assert_eq!(std::fs::read(&executable).unwrap(), b"original executable");
        }
    }
}
