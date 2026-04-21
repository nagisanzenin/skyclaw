//! Updater asset-name mapping.
//!
//! Single source of truth for the `temm1e update` subcommand's
//! target → release-asset-name resolution. Asset names must match the
//! `matrix.artifact` values in `.github/workflows/release.yml`; a unit
//! test at the bottom of this file pins the two together so future
//! renames in either place break the build loudly instead of silently
//! producing a non-functional updater (see v5.5.2 incident — issue #52
//! follow-up).

/// Return the list of release asset names the updater should try for
/// the given (os, arch) tuple, in preference order.
///
/// Linux returns two candidates: the `-desktop` glibc build first
/// (matches `install.sh`'s preference), then the `-linux` musl server
/// build as a fallback. macOS returns a single candidate per arch.
///
/// Returns `None` for unsupported platforms (Windows users build from
/// source today).
pub fn asset_candidates(os: &str, arch: &str) -> Option<&'static [&'static str]> {
    match (os, arch) {
        ("macos", "aarch64") => Some(&["temm1e-aarch64-macos"]),
        ("macos", "x86_64") => Some(&["temm1e-x86_64-macos"]),
        ("linux", "x86_64") => Some(&["temm1e-x86_64-linux-desktop", "temm1e-x86_64-linux"]),
        ("linux", "aarch64") => Some(&["temm1e-aarch64-linux-desktop", "temm1e-aarch64-linux"]),
        _ => None,
    }
}

/// All asset names the updater could ever request, flattened. Used by
/// the release.yml-parity test below.
#[cfg(test)]
pub fn all_known_asset_names() -> Vec<&'static str> {
    let mut out = Vec::new();
    for (os, arch) in [
        ("macos", "aarch64"),
        ("macos", "x86_64"),
        ("linux", "x86_64"),
        ("linux", "aarch64"),
    ] {
        if let Some(candidates) = asset_candidates(os, arch) {
            out.extend_from_slice(candidates);
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Parse `.github/workflows/release.yml` and extract every
    /// `artifact:` value under the `matrix.include` list. Returns the
    /// set of asset names the release workflow actually uploads.
    fn release_yml_artifacts() -> Vec<String> {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let release_yml = repo_root.join(".github/workflows/release.yml");
        let raw = std::fs::read_to_string(&release_yml).unwrap_or_else(|e| {
            panic!(
                "failed to read {}: {e} — the updater's drift test requires release.yml to exist at this path",
                release_yml.display()
            )
        });

        // Extract `artifact: temm1e-...` lines. The matrix is the only
        // place these appear, and each one is on its own line per YAML
        // convention, so a line-scan is safer than pulling in a full
        // YAML parser.
        let mut artifacts = Vec::new();
        for line in raw.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("artifact:") {
                let name = rest.trim().trim_matches('"').trim_matches('\'');
                if name.starts_with("temm1e-") {
                    artifacts.push(name.to_string());
                }
            }
        }
        artifacts
    }

    #[test]
    fn every_updater_asset_is_published_by_release_yml() {
        let published = release_yml_artifacts();
        assert!(
            !published.is_empty(),
            "release.yml parse returned zero artifacts — is the matrix still under .github/workflows/release.yml?"
        );

        for asset in all_known_asset_names() {
            assert!(
                published.iter().any(|p| p == asset),
                "updater expects release asset '{asset}' but release.yml does not publish it.\n\
                 Either add it to .github/workflows/release.yml's matrix or remove it from \
                 update_assets::asset_candidates.\n\
                 Currently published by release.yml: {published:?}"
            );
        }
    }

    #[test]
    fn macos_has_single_candidate_per_arch() {
        assert_eq!(asset_candidates("macos", "aarch64").unwrap().len(), 1);
        assert_eq!(asset_candidates("macos", "x86_64").unwrap().len(), 1);
    }

    #[test]
    fn linux_prefers_desktop_then_falls_back_to_musl_server() {
        let x86 = asset_candidates("linux", "x86_64").unwrap();
        assert_eq!(x86, &["temm1e-x86_64-linux-desktop", "temm1e-x86_64-linux"]);
        let arm = asset_candidates("linux", "aarch64").unwrap();
        assert_eq!(
            arm,
            &["temm1e-aarch64-linux-desktop", "temm1e-aarch64-linux"]
        );
    }

    #[test]
    fn unsupported_platforms_return_none() {
        assert!(asset_candidates("windows", "x86_64").is_none());
        assert!(asset_candidates("macos", "arm").is_none());
        assert!(asset_candidates("freebsd", "x86_64").is_none());
    }
}
