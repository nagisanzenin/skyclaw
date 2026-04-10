//! # Real code-level self-grow proof.
//!
//! This is the proof that Tem can write **real Rust code**, not just markdown
//! skill files. Skill authoring is trivial — anyone can ask an LLM to write
//! a `.md` file. The hard claim of self-grow is that the LLM can write code
//! that compiles, lints clean, passes tests, and integrates with the existing
//! codebase. This test proves it.
//!
//! ## What it does
//!
//! 1. Creates a minimal isolated Cargo crate in a tempdir.
//! 2. Builds a real LLM provider from credentials in env or
//!    ~/.temm1e/credentials.toml.
//! 3. Wraps it in `LlmCodeGenerator` with a specific code task.
//! 4. Runs the generator: the LLM produces a JSON-encoded file change list.
//! 5. The generator parses the response and writes the file(s) into the
//!    tempdir crate.
//! 6. Runs `cargo check` against the generated code.
//! 7. Runs `cargo clippy --all-targets -- -D warnings` against the generated code.
//! 8. Runs `cargo test` against the generated code.
//! 9. If all three pass, the proof is complete: a real LLM wrote real Rust
//!    that compiled, linted, and tested.
//!
//! ## Run with
//!
//! ```sh
//! TEMM1E_CAMBIUM_REAL_CODE_TEST=1 \
//!   cargo test -p temm1e-cambium --test real_code_grow_test \
//!   -- --nocapture --test-threads=1
//! ```
//!
//! ## Production safety
//!
//! - The test crate lives in a tempdir; the production codebase is never
//!   touched.
//! - The test requires an explicit env var; it never runs as part of
//!   `cargo test --workspace`.
//! - If no API key is available, the test prints "SKIPPED" and returns OK.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use temm1e_cambium::llm_generator::LlmCodeGenerator;
use temm1e_cambium::pipeline::CodeGenerator;
use temm1e_cambium::sandbox::Sandbox;
use temm1e_core::traits::Provider;
use temm1e_core::types::cambium::{GrowthKind, GrowthTrigger};
use temm1e_core::types::config::ProviderConfig;
use tokio::process::Command;

fn should_run() -> bool {
    std::env::var("TEMM1E_CAMBIUM_REAL_CODE_TEST").unwrap_or_default() == "1"
}

fn resolve_api_key(env_var: &str, provider_name: &str) -> Option<String> {
    if let Ok(k) = std::env::var(env_var) {
        if !k.is_empty() {
            return Some(k);
        }
    }
    let creds = temm1e_core::config::credentials::load_credentials_file()?;
    let p = creds.providers.iter().find(|p| p.name == provider_name)?;
    p.keys.last().cloned()
}

async fn build_provider(
    name: &str,
    model: &str,
    env_var: &str,
) -> Option<(Arc<dyn Provider>, String)> {
    let api_key = resolve_api_key(env_var, name)?;
    let config = ProviderConfig {
        name: Some(name.to_string()),
        api_key: Some(api_key),
        model: Some(model.to_string()),
        ..Default::default()
    };
    let provider = temm1e_providers::create_provider(&config).ok()?;
    Some((Arc::from(provider), model.to_string()))
}

/// Create a minimal isolated Cargo crate in the given directory.
async fn create_minimal_crate(root: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(root.join("src")).await?;
    tokio::fs::write(
        root.join("Cargo.toml"),
        "[package]\n\
         name = \"cambium-test-crate\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\n\
         [dependencies]\n",
    )
    .await?;
    tokio::fs::write(
        root.join("src/lib.rs"),
        "// Minimal crate seeded by Cambium real-code-grow test.\n\
         // The LLM is expected to add code here.\n\n\
         pub fn marker() -> &'static str {\n    \"seeded\"\n}\n\n\
         #[cfg(test)]\n\
         mod tests {\n    use super::*;\n\n    #[test]\n    fn marker_works() {\n        assert_eq!(marker(), \"seeded\");\n    }\n}\n",
    )
    .await?;
    Ok(())
}

async fn run_cargo(args: &[&str], cwd: &Path) -> (bool, String, String) {
    let output = Command::new("cargo")
        .args(args)
        .current_dir(cwd)
        .output()
        .await;
    match output {
        Ok(o) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
        ),
        Err(e) => (false, String::new(), format!("spawn failed: {e}")),
    }
}

#[allow(dead_code)]
struct ProofResult {
    provider: String,
    model: String,
    files_written: Vec<PathBuf>,
    cargo_check_pass: bool,
    cargo_clippy_pass: bool,
    cargo_test_pass: bool,
    elapsed_ms: u64,
    success: bool,
    file_contents: Vec<(PathBuf, String)>,
    failure_reason: Option<String>,
}

async fn run_proof(label: &str, provider: Arc<dyn Provider>, model: String) -> ProofResult {
    println!("\n{} {label} {}", "=".repeat(20), "=".repeat(20));
    println!("Model: {model}");

    let tmp = tempfile::tempdir().unwrap();
    let crate_root = tmp.path().join("test-crate");
    create_minimal_crate(&crate_root).await.unwrap();

    // Treat the test crate as a fake "sandbox" — Sandbox doesn't actually
    // need git for this test, only the path/write-file primitives.
    let sandbox = Sandbox::new(crate_root.clone(), "local".to_string(), "main".to_string());

    // Read the seeded lib.rs to give the LLM context.
    let lib_content = tokio::fs::read_to_string(crate_root.join("src/lib.rs"))
        .await
        .unwrap();

    let generator = LlmCodeGenerator::new(provider, model.clone())
        .with_context_file("src/lib.rs".to_string(), lib_content)
        .with_max_files(2);

    let trigger = GrowthTrigger::Manual {
        description:
            "Modify src/lib.rs to add a public function `slugify(input: &str) -> String` that \
             converts a title to a URL-safe slug. The function must:\n\
             - Lowercase everything.\n\
             - Strip all characters except ASCII alphanumerics, spaces, and hyphens.\n\
             - Collapse consecutive whitespace and hyphens into a single hyphen.\n\
             - Trim leading and trailing hyphens.\n\
             - Example: \"Hello, World! 2026\" becomes \"hello-world-2026\".\n\
             - Example: \"  Multiple   Spaces  \" becomes \"multiple-spaces\".\n\
             - Example: \"\" becomes \"\".\n\
             \n\
             You must keep the existing `marker()` function and its test exactly as they are. \
             Add the new function and at least 5 #[cfg(test)] tests for it (basic title, \
             leading/trailing whitespace, multiple spaces, special characters, empty string). \
             The complete content of src/lib.rs must contain BOTH the existing marker function \
             AND the new slugify function plus tests."
                .to_string(),
    };

    let started = Instant::now();
    let gen_result = generator
        .generate(&sandbox, &trigger, &GrowthKind::NewTool)
        .await;
    let gen_elapsed = started.elapsed();

    if let Err(e) = &gen_result {
        println!("Generator failed: {e}");
        return ProofResult {
            provider: label.to_string(),
            model,
            files_written: vec![],
            cargo_check_pass: false,
            cargo_clippy_pass: false,
            cargo_test_pass: false,
            elapsed_ms: gen_elapsed.as_millis() as u64,
            success: false,
            file_contents: vec![],
            failure_reason: Some(format!("generator: {e}")),
        };
    }
    println!("Generator finished in {gen_elapsed:?}");

    // Collect what was written
    let mut files_written = Vec::new();
    let mut file_contents = Vec::new();
    if let Ok(content) = tokio::fs::read_to_string(crate_root.join("src/lib.rs")).await {
        files_written.push(PathBuf::from("src/lib.rs"));
        println!("\n--- generated src/lib.rs ---\n{content}\n--- end ---\n");
        file_contents.push((PathBuf::from("src/lib.rs"), content));
    }

    println!("Running cargo check...");
    let (check_ok, _check_out, check_err) = run_cargo(&["check"], &crate_root).await;
    if !check_ok {
        println!("cargo check FAILED:\n{check_err}");
    } else {
        println!("cargo check PASSED");
    }

    println!("Running cargo clippy...");
    let (clippy_ok, _clippy_out, clippy_err) = run_cargo(
        &["clippy", "--all-targets", "--", "-D", "warnings"],
        &crate_root,
    )
    .await;
    if !clippy_ok {
        println!("cargo clippy FAILED:\n{clippy_err}");
    } else {
        println!("cargo clippy PASSED");
    }

    println!("Running cargo test...");
    let (test_ok, test_out, test_err) = run_cargo(&["test"], &crate_root).await;
    if !test_ok {
        println!("cargo test FAILED:\nstdout:\n{test_out}\nstderr:\n{test_err}");
    } else {
        // Extract the test summary line
        let summary = test_out
            .lines()
            .find(|l| l.contains("test result:"))
            .unwrap_or("(no summary)");
        println!("cargo test PASSED — {summary}");
    }

    let total_elapsed = started.elapsed();
    let success = check_ok && clippy_ok && test_ok;

    ProofResult {
        provider: label.to_string(),
        model,
        files_written,
        cargo_check_pass: check_ok,
        cargo_clippy_pass: clippy_ok,
        cargo_test_pass: test_ok,
        elapsed_ms: total_elapsed.as_millis() as u64,
        success,
        file_contents,
        failure_reason: if success {
            None
        } else if !check_ok {
            Some("cargo check failed".to_string())
        } else if !clippy_ok {
            Some("cargo clippy failed".to_string())
        } else {
            Some("cargo test failed".to_string())
        },
    }
}

#[tokio::test]
async fn real_llm_writes_compiles_and_tests_rust_code() {
    if !should_run() {
        println!("SKIPPED: set TEMM1E_CAMBIUM_REAL_CODE_TEST=1 to enable");
        return;
    }

    println!("\n===== REAL CODE-LEVEL CAMBIUM PROOF =====");
    println!("This test proves Tem can write Rust, not just markdown.");
    println!("Cost: < $0.10 total\n");

    let mut results = Vec::new();

    if let Some((provider, model)) =
        build_provider("gemini", "gemini-3-flash-preview", "GEMINI_API_KEY").await
    {
        results.push(run_proof("GEMINI 3 FLASH", provider, model).await);
    } else {
        println!("[GEMINI] SKIPPED (no key)");
    }

    if let Some((provider, model)) =
        build_provider("anthropic", "claude-sonnet-4-6", "ANTHROPIC_API_KEY").await
    {
        results.push(run_proof("SONNET 4.6", provider, model).await);
    } else {
        println!("[SONNET] SKIPPED (no key)");
    }

    if results.is_empty() {
        println!("\nNo API keys available, skipping real-LLM test.");
        return;
    }

    println!("\n\n===== FINAL PROOF REPORT =====");
    println!(
        "{:<20} {:<28} {:>8} {:>8} {:>8} {:>8} {:>15}",
        "Provider", "Model", "Files", "Check", "Clippy", "Test", "Elapsed (ms)"
    );
    println!("{}", "-".repeat(110));
    for r in &results {
        println!(
            "{:<20} {:<28} {:>8} {:>8} {:>8} {:>8} {:>15}",
            r.provider,
            r.model,
            r.files_written.len(),
            if r.cargo_check_pass { "OK" } else { "FAIL" },
            if r.cargo_clippy_pass { "OK" } else { "FAIL" },
            if r.cargo_test_pass { "OK" } else { "FAIL" },
            r.elapsed_ms
        );
    }
    println!();
    for r in &results {
        if let Some(ref reason) = r.failure_reason {
            println!("[{}] FAILED: {reason}", r.provider);
        } else {
            println!(
                "[{}] OK — wrote real Rust that compiled, linted, and tested",
                r.provider
            );
        }
    }

    let any_success = results.iter().any(|r| r.success);
    assert!(
        any_success,
        "No provider successfully wrote, compiled, and tested real Rust code. See output above."
    );

    println!("\n===== PROOF: TEM CAN WRITE REAL RUST CODE =====");
}
