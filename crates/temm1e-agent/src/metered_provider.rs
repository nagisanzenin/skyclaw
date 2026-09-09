//! Logical-call accounting for auxiliary consumers that otherwise discard Usage.
//! Do not wrap a caller that already records the same completion in this budget.
use crate::budget::{get_pricing_with_custom, BudgetTracker, CostEstimate, ModelPricing};
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use std::sync::Arc;
use temm1e_core::{
    types::{error::Temm1eError, message::*},
    Provider,
};

pub struct MeteredProvider {
    inner: Arc<dyn Provider>,
    budget: Arc<BudgetTracker>,
    pricing: Option<(String, ModelPricing)>,
}
impl MeteredProvider {
    pub fn new(inner: Arc<dyn Provider>, budget: Arc<BudgetTracker>) -> Self {
        Self {
            inner,
            budget,
            pricing: None,
        }
    }
    /// Bind an existing immutable model/pricing snapshot instead of re-reading
    /// the registry. A request for another model must obtain another binding.
    pub fn with_pricing(
        inner: Arc<dyn Provider>,
        budget: Arc<BudgetTracker>,
        model: String,
        pricing: ModelPricing,
    ) -> Self {
        Self {
            inner,
            budget,
            pricing: Some((model, pricing)),
        }
    }
    fn begin(&self, model: &str) -> Result<Attempt, Temm1eError> {
        let pricing = match &self.pricing {
            Some((bound, pricing)) if bound == model => *pricing,
            Some(_) => {
                return Err(Temm1eError::Provider(
                    "Metered request model differs from pricing binding".into(),
                ))
            }
            None => get_pricing_with_custom(self.inner.name(), model),
        };
        self.budget
            .check_model_budget(&pricing)
            .map_err(Temm1eError::Provider)?;
        Ok(Attempt {
            budget: self.budget.clone(),
            pricing,
            finished: false,
            partial: None,
        })
    }
}
struct Attempt {
    budget: Arc<BudgetTracker>,
    pricing: ModelPricing,
    finished: bool,
    partial: Option<Usage>,
}
impl Attempt {
    fn finish(&mut self, usage: &Usage) {
        self.budget.record_model_usage(usage, &self.pricing);
        self.finished = true;
    }
}
impl Attempt {
    fn unknown(&mut self) {
        if !self.finished {
            let usage = self.partial.take().unwrap_or_default();
            self.budget.record_estimate(
                usage.input_tokens,
                usage.output_tokens,
                &CostEstimate::Unavailable,
            );
            self.finished = true;
        }
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        self.unknown();
    }
}
#[async_trait]
impl Provider for MeteredProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        let mut attempt = self.begin(&request.model)?;
        let response = self.inner.complete(request).await?;
        attempt.finish(&response.usage);
        Ok(response)
    }
    async fn complete_with_observer(
        &self,
        request: CompletionRequest,
        observer: temm1e_core::streaming::TextObserver,
    ) -> Result<CompletionResponse, Temm1eError> {
        let mut attempt = self.begin(&request.model)?;
        let response = self.inner.complete_with_observer(request, observer).await?;
        attempt.finish(&response.usage);
        Ok(response)
    }
    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
        let attempt = self.begin(&request.model)?;
        let stream = self.inner.stream(request).await?;
        Ok(Box::pin(futures::stream::unfold(
            (stream, attempt, false, false),
            |(mut stream, mut attempt, mut terminal, mut ended)| async move {
                if ended {
                    return None;
                }
                match stream.next().await {
                    Some(Ok(chunk)) => {
                        if let Some(usage) = &chunk.usage {
                            attempt.partial = Some(usage.clone());
                        }
                        terminal |= chunk.stop_reason.is_some();
                        Some((Ok(chunk), (stream, attempt, terminal, ended)))
                    }
                    Some(Err(error)) => {
                        attempt.unknown();
                        ended = true;
                        Some((Err(error), (stream, attempt, terminal, ended)))
                    }
                    None => {
                        if terminal {
                            let usage = attempt.partial.clone().unwrap_or_else(|| Usage {
                                totals_reported: Some(false),
                                ..Usage::default()
                            });
                            attempt.finish(&usage);
                        }
                        None
                    }
                }
            },
        )))
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
    #[tokio::test]
    async fn fixed_pricing_binding_rejects_another_model_without_a_call() {
        let raw = std::sync::Arc::new(temm1e_test_utils::MockProvider::with_text("unused"));
        let owner = std::sync::Arc::new(crate::budget::BudgetTracker::new(0.0));
        let metered = super::MeteredProvider::with_pricing(
            raw.clone(),
            owner.clone(),
            "fixed-model".into(),
            crate::budget::ModelPricing::custom(1.0, 2.0),
        );
        assert!(temm1e_core::Provider::complete(&metered, request())
            .await
            .is_err());
        assert_eq!(raw.calls().await, 0);
        assert_eq!(owner.snapshot().recorded_calls, 0);
    }

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Fixture {
        calls: AtomicUsize,
        wait: bool,
        started: tokio::sync::Semaphore,
        name: &'static str,
    }
    fn request() -> CompletionRequest {
        CompletionRequest {
            model: "gpt-4.1".into(),
            messages: vec![],
            tools: vec![],
            max_tokens: Some(64),
            temperature: None,
            system: None,
            system_volatile: None,
        }
    }
    fn measured(input: u32) -> Usage {
        Usage {
            totals_reported: Some(true),
            input_tokens: input,
            output_tokens: 2,
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            cost_usd: 0.0,
        }
    }
    #[async_trait]
    impl Provider for Fixture {
        fn name(&self) -> &str {
            self.name
        }
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, Temm1eError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.add_permits(1);
            if self.wait {
                futures::future::pending::<()>().await;
            }
            Ok(CompletionResponse {
                id: "r".into(),
                content: vec![ContentPart::Text {
                    text: "result".into(),
                }],
                stop_reason: Some("stop".into()),
                usage: measured(20),
            })
        }
        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(futures::stream::iter([
                Ok(StreamChunk {
                    delta: Some("result".into()),
                    tool_use: None,
                    provider_state: None,
                    response_id: Some("r".into()),
                    stop_reason: Some("stop".into()),
                    usage: Some(measured(10)),
                }),
                Ok(StreamChunk {
                    delta: None,
                    tool_use: None,
                    provider_state: None,
                    response_id: Some("r".into()),
                    stop_reason: None,
                    usage: Some(measured(20)),
                }),
            ])))
        }
        async fn health_check(&self) -> Result<bool, Temm1eError> {
            Ok(true)
        }
        async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
            Ok(vec![])
        }
    }
    fn fixture(wait: bool, name: &'static str) -> Arc<Fixture> {
        Arc::new(Fixture {
            calls: AtomicUsize::new(0),
            wait,
            started: tokio::sync::Semaphore::new(0),
            name,
        })
    }
    #[tokio::test]
    async fn limited_unknown_route_is_denied_before_provider_and_dropped_call_is_unknown() {
        let inner = fixture(false, "unpriced-fixture");
        let budget = Arc::new(BudgetTracker::new(1.0));
        let provider = MeteredProvider::new(inner.clone(), budget.clone());
        assert!(provider.complete(request()).await.is_err());
        assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(budget.snapshot().unpriced_calls, 0);
        let inner = fixture(true, "openai");
        let budget = Arc::new(BudgetTracker::new(0.0));
        let provider = MeteredProvider::new(inner.clone(), budget.clone());
        let task = tokio::spawn(async move { provider.complete(request()).await });
        let permit =
            tokio::time::timeout(std::time::Duration::from_secs(5), inner.started.acquire())
                .await
                .unwrap()
                .unwrap();
        permit.forget();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(budget.snapshot().unpriced_calls, 1);
    }
    #[tokio::test]
    async fn late_cumulative_stream_usage_is_accounted_once() {
        let budget = Arc::new(BudgetTracker::new(0.0));
        let provider = MeteredProvider::new(fixture(false, "openai"), budget.clone());
        let response = temm1e_core::streaming::collect_completion(
            provider.stream(request()).await.unwrap(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
        assert_eq!(response.usage.input_tokens, 20);
        assert_eq!(budget.snapshot().input_tokens, 20);
        assert_eq!(budget.snapshot().output_tokens, 2);
        assert_eq!(budget.snapshot().unpriced_calls, 0);
        assert!(budget.snapshot().cost_usd > 0.0);
    }
    #[tokio::test]
    async fn subscriptions_remain_subscription_when_auxiliary_text_consumer_discards_usage() {
        let parent = Arc::new(BudgetTracker::new(0.0));
        let child = Arc::new(BudgetTracker::child(parent.clone()));
        let provider = MeteredProvider::new(fixture(false, "openai-codex"), child.clone());
        let _text = provider.complete(request()).await.unwrap().content;
        assert_eq!(child.snapshot().subscription_calls, 1);
        assert_eq!(parent.snapshot().subscription_calls, 1);
        assert_eq!(parent.snapshot().input_tokens, 20);
        assert_eq!(parent.snapshot().cost_usd, 0.0);
    }
}
