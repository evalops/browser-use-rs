use super::Agent;
use super::artifacts::result_requests_screenshot;
use crate::StepMetadata;
use browser_use_cdp::BrowserSession;
use browser_use_llm::ChatModel;

impl<M, S> Agent<M, S>
where
    M: ChatModel,
    S: BrowserSession + Send + Sync,
{
    pub(super) fn step_metadata(&self, step_start_time: f64, step_end_time: f64) -> StepMetadata {
        let step_interval = self
            .history
            .items
            .last()
            .and_then(|item| item.metadata.as_ref())
            .map(|metadata| metadata.duration_seconds().max(0.0));

        StepMetadata {
            step_start_time,
            step_end_time,
            step_number: self.next_step_number(),
            step_interval,
        }
    }

    pub(super) fn next_step_number(&self) -> usize {
        self.history
            .items
            .iter()
            .filter_map(|item| item.metadata.as_ref().map(|metadata| metadata.step_number))
            .max()
            .unwrap_or(0)
            + 1
    }

    pub(super) fn should_include_screenshot(&self) -> bool {
        let action_requested_screenshot = self
            .history
            .items
            .last()
            .is_some_and(|item| item.result.iter().any(result_requests_screenshot));
        self.settings
            .use_vision
            .should_include_screenshot(action_requested_screenshot)
    }
}
