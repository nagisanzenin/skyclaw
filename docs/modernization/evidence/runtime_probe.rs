//! Baseline audit probe, not a regression test: prints observed behavior.
//! Run using the instructions in VALIDATION.md. No network or user credentials.
use temm1e_agent::executor::{detect_dependencies, ToolCall};
use temm1e_core::{Tool, ToolContext, ToolInput};

#[tokio::main]
async fn main() {
    let shell_write = ToolCall {
        id: "a".into(), name: "shell".into(),
        arguments: serde_json::json!({"command": "printf changed > shared.txt"}),
    };
    let read = ToolCall {
        id: "b".into(), name: "file_read".into(),
        arguments: serde_json::json!({"path": "shared.txt"}),
    };
    println!("shell_write_then_read_groups={:?}", detect_dependencies(&[shell_write, read]));
    let browser = |id: &str, action: &str| ToolCall {
        id: id.into(), name: "browser".into(),
        arguments: serde_json::json!({"action": action}),
    };
    println!("browser_action_groups={:?}", detect_dependencies(&[browser("c", "navigate"), browser("d", "click")]));
    let lower = temm1e_distill::stats::wilson::wilson_lower(29, 30, 0.99);
    println!("wilson_29_of_30_99pct={lower:.6}");
    println!("wilson_30_of_30_99pct={:.6}", temm1e_distill::stats::wilson::wilson_lower(30, 30, 0.99));
    let sprt = temm1e_distill::stats::sprt::Sprt::from_state(0.5, 0.7, 0.05, 0.1, 100, 0.1, 100);
    println!("sprt_at_cap_below_boundary={:?}", sprt.decision());
    println!("power_sample_n={}", temm1e_distill::stats::power::min_sample_size(0.5, 0.55, 0.05, 0.8));
    println!("entropy_missing_categories={}", temm1e_distill::stats::entropy::normalized_entropy(&[10,10,0,0]));
    use temm1e_tools::web_search::{cache::CacheKey, types::{SearchRequest, SortOrder, TimeRange}};
    let mut req = SearchRequest { query: "probe".into(), max_results: 10, max_total_chars: 8000, max_snippet_chars: 200, time_range: TimeRange::All, category: None, language: None, region: None, include_domains: vec![], exclude_domains: vec![], sort: SortOrder::Relevance };
    let first = CacheKey::from_request(&req, &None);
    req.sort = SortOrder::Date;
    println!("different_search_sort_same_cache_key={}", first == CacheKey::from_request(&req, &None));
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext {
            workspace_path: dir.path().into(), session_id: "audit".into(),
            chat_id: "audit".into(), read_tracker: None,
        };
        let output = temm1e_tools::ShellTool::new().execute(ToolInput {
            name: "shell".into(),
            arguments: serde_json::json!({"command": "sleep 1.5; printf late > late.txt", "timeout": 1}),
        }, &ctx).await.unwrap();
        println!("shell_timeout_error={}; message={}", output.is_error, output.content);
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        println!("side_effect_after_timeout={}", dir.path().join("late.txt").exists());
    }
}
