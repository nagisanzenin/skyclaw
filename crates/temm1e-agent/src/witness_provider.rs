//! Turn-local verification admission. Call limits are not dollars or account quota.
use async_trait::async_trait;
use futures::stream::BoxStream;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use temm1e_core::{
    types::{error::Temm1eError, message::*},
    Provider,
};

pub(crate) struct WitnessProvider {
    inner: Arc<dyn Provider>,
    model: String,
    input_limit: usize,
    model_window: usize,
    model_output: usize,
    remaining: AtomicU32,
}
impl WitnessProvider {
    pub(crate) fn new(
        inner: Arc<dyn Provider>,
        model: String,
        input_limit: usize,
        calls: u32,
    ) -> Self {
        let (model_window, model_output) =
            temm1e_core::types::model_registry::model_limits_with_custom(inner.name(), &model);
        Self {
            inner,
            model,
            input_limit,
            model_window,
            model_output,
            remaining: AtomicU32::new(calls.min(8)),
        }
    }
}
#[async_trait]
impl Provider for WitnessProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }
    async fn complete(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        if request.model != self.model
            || !request.tools.is_empty()
            || !request
                .max_tokens
                .is_some_and(|limit| (1..=4096).contains(&limit))
        {
            return Err(Temm1eError::Provider(
                "invalid Witness provider request".into(),
            ));
        }
        // Capture limits once per turn, and reserve room for evidence even if
        // a model advertises output equal to its whole context window.
        let output = (request.max_tokens.unwrap_or(0) as usize)
            .min(self.model_output)
            .min(self.model_window / 2);
        if output == 0 {
            return Err(Temm1eError::Provider(
                "Witness model has no usable output allowance".into(),
            ));
        }
        request.max_tokens = Some(output as u32); // Already bounded above by4096.
        crate::context::check_context_fit(&request, self.input_limit, self.model_window)?;
        self.remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(1)
            })
            .map_err(|_| {
                Temm1eError::Provider("Witness model verification call allowance exhausted".into())
            })?;
        // MeteredProvider is inside this timeout: dropping an in-flight attempt
        // records unavailable usage once; rejected admission never calls it.
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.inner.complete(request),
        )
        .await
        .map_err(|_| Temm1eError::Provider("Witness model verification timed out".into()))?
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
        Err(Temm1eError::Provider(
            "Witness expects bounded non-streaming completion".into(),
        ))
    }
    async fn health_check(&self) -> Result<bool, Temm1eError> {
        self.inner.health_check().await
    }
    async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
        self.inner.list_models().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temm1e_test_utils::QueuedMockProvider;
    fn request() -> CompletionRequest {
        CompletionRequest {
            model: "fixture".into(),
            messages: vec![],
            tools: vec![],
            max_tokens: Some(4096),
            temperature: None,
            system: Some("verify".into()),
            system_volatile: None,
        }
    }
    #[tokio::test]
    async fn shared_tier_allowance_is_atomic_and_metered_once() {
        let raw = Arc::new(QueuedMockProvider::with_responses(
            vec![QueuedMockProvider::text_response("bad JSON"); 3],
        ));
        let budget = Arc::new(crate::budget::BudgetTracker::new(0.0));
        let metered = Arc::new(crate::metered_provider::MeteredProvider::new(
            raw.clone(),
            budget.clone(),
        ));
        let provider = WitnessProvider::new(metered, "fixture".into(), 32768, 2);
        let (a, b, c) = tokio::join!(
            provider.complete(request()),
            provider.complete(request()),
            provider.complete(request())
        );
        assert_eq!(
            [a.is_ok(), b.is_ok(), c.is_ok()]
                .into_iter()
                .filter(|ok| *ok)
                .count(),
            2
        );
        assert_eq!(*raw.call_count.lock().await, 2);
        assert_eq!(budget.snapshot().recorded_calls, 2);
        assert_eq!(budget.snapshot().unpriced_calls, 2);
    }
    #[tokio::test]
    async fn wrong_model_oversized_context_and_exhausted_owner_make_no_calls() {
        let raw = Arc::new(QueuedMockProvider::with_responses(vec![]));
        let owner = Arc::new(crate::budget::BudgetTracker::new(0.01));
        let provider = WitnessProvider::new(
            Arc::new(crate::metered_provider::MeteredProvider::new(
                raw.clone(),
                owner.clone(),
            )),
            "fixture".into(),
            100,
            2,
        );
        let mut invalid = request();
        invalid.model = "other".into();
        assert!(provider.complete(invalid).await.is_err());
        let mut large = request();
        large.system = Some("long ".repeat(1000));
        assert!(provider.complete(large).await.is_err());
        // Unknown fixture tariff is rejected by a finite USD owner budget.
        assert!(provider.complete(request()).await.is_err());
        assert_eq!(*raw.call_count.lock().await, 0);
        assert_eq!(owner.snapshot().recorded_calls, 0);
    }
    struct Unfinished {
        started: tokio::sync::Notify,
        fail: bool,
    }
    #[async_trait]
    impl Provider for Unfinished {
        fn name(&self) -> &str {
            "fixture"
        }
        async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, Temm1eError> {
            self.started.notify_one();
            if self.fail {
                return Err(Temm1eError::Provider("fixture failure".into()));
            }
            std::future::pending().await
        }
        async fn stream(
            &self,
            _: CompletionRequest,
        ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
            unreachable!()
        }
        async fn health_check(&self) -> Result<bool, Temm1eError> {
            Ok(true)
        }
        async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
            Ok(vec![])
        }
    }
    #[tokio::test]
    async fn failed_and_dropped_verification_are_metered_once_without_refunding_attempt() {
        for fail in [false, true] {
            let raw = Arc::new(Unfinished {
                started: tokio::sync::Notify::new(),
                fail,
            });
            let budget = Arc::new(crate::budget::BudgetTracker::new(0.0));
            let metered = Arc::new(crate::metered_provider::MeteredProvider::new(
                raw.clone(),
                budget.clone(),
            ));
            let provider = WitnessProvider::new(metered, "fixture".into(), 32768, 1);
            if fail {
                assert!(provider.complete(request()).await.is_err());
            } else {
                tokio::select! {
                    _ = provider.complete(request()) => panic!("pending fixture completed"),
                    _ = raw.started.notified() => {},
                }
            }
            assert_eq!(budget.snapshot().recorded_calls, 1);
            assert_eq!(budget.snapshot().unpriced_calls, 1);
            assert!(provider.complete(request()).await.is_err());
            assert_eq!(budget.snapshot().recorded_calls, 1);
        }
    }
    #[tokio::test]
    async fn custom_limits_are_captured_and_output_respects_selected_model() {
        const CHILD: &str = "TEMM1E_WITNESS_LIMITS_FIXTURE";
        if std::env::var_os(CHILD).is_none() {
            let directory = tempfile::tempdir().unwrap();
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "witness_provider::tests::custom_limits_are_captured_and_output_respects_selected_model", "--nocapture"])
                .env(CHILD, "1").env("TEMM1E_DATA_DIR", directory.path()).output().unwrap();
            assert!(
                child.status.success(),
                "isolated limits fixture failed: {} {}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );
            return;
        }
        let path = temm1e_core::config::data_dir().join("custom_models.toml");
        let configure = |window, output| {
            std::fs::write(&path, format!("[[models]]\nprovider = \"queued-mock\"\nname = \"fixture\"\ncontext_window = {window}\nmax_output_tokens = {output}\n")).unwrap()
        };
        configure(8192, 1024);
        let raw = Arc::new(QueuedMockProvider::with_responses(
            vec![QueuedMockProvider::text_response("response"); 3],
        ));
        let old = WitnessProvider::new(raw.clone(), "fixture".into(), 8192, 2);
        old.complete(request()).await.unwrap();
        assert_eq!(raw.captured_requests.lock().await[0].max_tokens, Some(1024));
        configure(256, 128);
        old.complete(request()).await.unwrap();
        let next = WitnessProvider::new(raw.clone(), "fixture".into(), 8192, 1);
        next.complete(request()).await.unwrap();
        let captured = raw.captured_requests.lock().await;
        assert_eq!(
            captured.iter().map(|r| r.max_tokens).collect::<Vec<_>>(),
            vec![Some(1024), Some(1024), Some(128)]
        );
        drop(captured);
        configure(0, 0);
        let invalid = WitnessProvider::new(raw.clone(), "fixture".into(), 8192, 1);
        assert!(invalid.complete(request()).await.is_err());
        assert_eq!(*raw.call_count.lock().await, 3);
    }
}
