//! Baseline probe; no external services or user data.
#[tokio::main]
async fn main() {
 let store = temm1e_perpetuum::Store::new("sqlite::memory:").await.unwrap();
 let now = chrono::Utc::now();
 store.record_activity(now).await.unwrap();
 use chrono::Timelike;
 println!("activity_after_one_record={}", store.activity_probability(now.hour(), 0).await.unwrap());
 let mut panicked = false;
 std::panic::set_hook(Box::new(|_| {}));
 for offset in 0..4 {
  let group = temm1e_perpetuum::log_scanner::ErrorGroup { signature: "probe".into(), message: "x".into(), location: None, count: 1, timestamps: vec![], sample_lines: vec![] };
  let triage = format!("{}{}", "a".repeat(offset), "界".repeat(22000));
  panicked |= std::panic::catch_unwind(|| temm1e_perpetuum::bug_reporter::format_issue_body(&group, &triage, "probe", "test")).is_err();
 }
 println!("unicode_report_panics={panicked}");
}
