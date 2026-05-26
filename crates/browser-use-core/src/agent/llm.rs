use super::{Agent, AgentRunError};
use browser_use_cdp::BrowserSession;
use browser_use_llm::{ChatCompletion, ChatModel, ChatRequest, LlmError};
use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

#[derive(Debug)]
pub(super) enum AgentLlmCallError {
    TimedOut { seconds: u64 },
    Provider(LlmError),
}

fn is_fallback_eligible_llm_error(error: &LlmError) -> bool {
    matches!(error, LlmError::Provider(_) | LlmError::RateLimited(_))
}

pub(super) fn agent_llm_call_error_to_run_error(error: AgentLlmCallError) -> AgentRunError {
    match error {
        AgentLlmCallError::TimedOut { seconds } => AgentRunError::LlmTimedOut { seconds },
        AgentLlmCallError::Provider(error) => AgentRunError::Llm(error),
    }
}

impl<M, S> Agent<M, S>
where
    M: ChatModel,
    S: BrowserSession + Send + Sync,
{
    pub(super) async fn invoke_json_with_fallback(
        &mut self,
        request: ChatRequest,
    ) -> Result<ChatCompletion<Value>, AgentLlmCallError> {
        let seconds = self.settings.llm_timeout_seconds;
        let first = Self::invoke_json_once(&self.llm, seconds, request.clone()).await;
        match first {
            Err(AgentLlmCallError::Provider(error)) if self.try_switch_to_fallback_llm(&error) => {
                let completion = Self::invoke_json_once(&self.llm, seconds, request).await?;
                self.record_completion_usage(&completion).await;
                Ok(completion)
            }
            Ok(completion) => {
                self.record_completion_usage(&completion).await;
                Ok(completion)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn invoke_json_once(
        llm: &M,
        seconds: u64,
        request: ChatRequest,
    ) -> Result<ChatCompletion<Value>, AgentLlmCallError> {
        timeout(Duration::from_secs(seconds), llm.invoke_json(request))
            .await
            .map_err(|_| AgentLlmCallError::TimedOut { seconds })?
            .map_err(AgentLlmCallError::Provider)
    }

    fn try_switch_to_fallback_llm(&mut self, error: &LlmError) -> bool {
        if self.using_fallback_llm || !is_fallback_eligible_llm_error(error) {
            return false;
        }
        let Some(fallback_llm) = self.fallback_llm.take() else {
            return false;
        };
        self.llm = fallback_llm;
        self.using_fallback_llm = true;
        true
    }

    pub(super) async fn record_completion_usage(&mut self, completion: &ChatCompletion<Value>) {
        let provider = self.llm.provider().to_owned();
        self.token_usage
            .add_completion_with_provider(&provider, completion);
        self.refresh_usage_summary().await;
    }

    pub(super) async fn refresh_usage_summary(&mut self) {
        self.history.usage = Some(self.token_usage.summary().await);
    }
}
