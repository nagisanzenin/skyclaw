//! Explicit opt-in smoke test. Reads a private key file, never prints credentials.
//! TEMM1E_ZAI_KEY_FILE=/private/path cargo run -p temm1e-providers --example coding_plan_smoke
use temm1e_core::types::{
    config::ProviderConfig,
    message::{ChatMessage, CompletionRequest, ContentPart, MessageContent, Role},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("TEMM1E_ZAI_KEY_FILE")?;
    let key = std::fs::read_to_string(path)?;
    let config = ProviderConfig {
        name: Some("zai-coding-plan".into()),
        api_key: Some(key.trim().into()),
        keys: vec![],
        model: Some("glm-5.3-flash".into()),
        base_url: None,
        extra_headers: Default::default(),
    };
    let provider = temm1e_providers::create_provider(&config)?;
    let start = std::time::Instant::now();
    let first_delta = std::sync::Arc::new(std::sync::Mutex::new(None));
    let observed = first_delta.clone();
    let observer: temm1e_core::streaming::TextObserver = std::sync::Arc::new(move |_| {
        observed
            .lock()
            .unwrap()
            .get_or_insert(start.elapsed().as_millis());
    });
    let request = CompletionRequest {
        model: "glm-5.3-flash".into(),
        messages: vec![ChatMessage {
            role: Role::User,
            content: MessageContent::Text("What is 2 + 2? Reply with the digit only.".into()),
        }],
        system: Some("You are running a connection smoke test.".into()),
        system_volatile: Some("Do not call external tools.".into()),
        tools: vec![],
        max_tokens: Some(256),
        temperature: Some(0.0),
    };
    let result = if std::env::args().any(|arg| arg == "--stream") {
        provider.complete_with_observer(request, observer).await
    } else {
        provider.complete(request).await
    };
    match result {
        Ok(response) => {
            println!(
                "connection={} model=glm-5.3-flash elapsed_ms={} input_tokens={} output_tokens={}",
                provider.name(),
                start.elapsed().as_millis(),
                response.usage.input_tokens,
                response.usage.output_tokens
            );
            let text: String = response
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            if text.trim() != "4" {
                return Err("Provider returned unexpected smoke-test content".into());
            }
            println!("arithmetic_answer_verified=true first_delta_ms={:?} totals_reported={:?} cache_read_tokens={:?} cache_write_tokens={:?}", *first_delta.lock().unwrap(), response.usage.totals_reported, response.usage.cache_read_tokens, response.usage.cache_write_tokens);
            if std::env::args().any(|arg| arg == "--stream")
                && first_delta.lock().unwrap().is_none()
            {
                return Err("No real streamed text was observed".into());
            }
        }
        Err(_) => {
            eprintln!("Coding-plan request failed; no metered fallback was attempted. Inspect provider status with redacted diagnostics.");
            std::process::exit(1);
        }
    }
    Ok(())
}
