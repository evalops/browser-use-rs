use super::{Agent, AgentCallbackFuture, AgentRunError};
use crate::{AgentHistory, AgentOutput};
use browser_use_cdp::BrowserSession;
use browser_use_dom::BrowserStateSummary;
use browser_use_llm::ChatModel;

impl<M, S> Agent<M, S>
where
    M: ChatModel,
    S: BrowserSession + Send + Sync,
{
    /// Registers a synchronous callback invoked after each accepted model output.
    pub fn register_new_step_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut(&BrowserStateSummary, &AgentOutput, usize) -> Result<(), E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_new_step_callback_async(move |state, output, step| {
            let result = callback(state, output, step).map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    /// Registers an async callback invoked after each accepted model output.
    pub fn register_new_step_callback_async<F>(&mut self, callback: F)
    where
        F: for<'a> FnMut(
                &'a BrowserStateSummary,
                &'a AgentOutput,
                usize,
            ) -> AgentCallbackFuture<'a, ()>
            + Send
            + 'static,
    {
        self.step_callbacks.push(Box::new(callback));
    }

    /// Removes all step callbacks.
    pub fn clear_new_step_callbacks(&mut self) {
        self.step_callbacks.clear();
    }

    /// Registers a synchronous callback invoked after successful completion.
    pub fn register_done_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut(&AgentHistory) -> Result<(), E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_done_callback_async(move |history| {
            let result = callback(history).map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    /// Registers an async callback invoked after successful completion.
    pub fn register_done_callback_async<F>(&mut self, callback: F)
    where
        F: for<'a> FnMut(&'a AgentHistory) -> AgentCallbackFuture<'a, ()> + Send + 'static,
    {
        self.done_callbacks.push(Box::new(callback));
    }

    /// Removes all done callbacks.
    pub fn clear_done_callbacks(&mut self) {
        self.done_callbacks.clear();
    }

    /// Registers a synchronous callback that can request a graceful stop.
    pub fn register_should_stop_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut() -> Result<bool, E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_should_stop_callback_async(move || {
            let result = callback().map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    /// Registers an async callback that can request a graceful stop.
    pub fn register_should_stop_callback_async<F>(&mut self, callback: F)
    where
        F: FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static,
    {
        self.should_stop_callback = Some(Box::new(callback));
    }

    /// Clears the stop callback.
    pub fn clear_should_stop_callback(&mut self) {
        self.should_stop_callback = None;
    }

    /// Registers a synchronous callback that reports external interruption.
    pub fn register_external_agent_status_callback<F, E>(&mut self, mut callback: F)
    where
        F: FnMut() -> Result<bool, E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_external_agent_status_callback_async(move || {
            let result = callback().map_err(|error| error.to_string());
            Box::pin(async move { result })
        });
    }

    /// Registers an async callback that reports external interruption.
    pub fn register_external_agent_status_callback_async<F>(&mut self, callback: F)
    where
        F: FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static,
    {
        self.external_status_callback = Some(Box::new(callback));
    }

    /// Compatibility alias for registering an external status callback.
    pub fn register_external_agent_status_raise_error_callback<F, E>(&mut self, callback: F)
    where
        F: FnMut() -> Result<bool, E> + Send + 'static,
        E: ToString + 'static,
    {
        self.register_external_agent_status_callback(callback);
    }

    /// Async compatibility alias for registering an external status callback.
    pub fn register_external_agent_status_raise_error_callback_async<F>(&mut self, callback: F)
    where
        F: FnMut() -> AgentCallbackFuture<'static, bool> + Send + 'static,
    {
        self.register_external_agent_status_callback_async(callback);
    }

    /// Clears the external status callback.
    pub fn clear_external_agent_status_callback(&mut self) {
        self.external_status_callback = None;
    }

    /// Requests that the agent stop before its next step.
    pub fn stop(&mut self) {
        self.stopped = true;
    }

    /// Returns true when stop has been requested.
    pub fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// Appends a follow-up user request and clears stopped/paused state.
    pub fn add_new_task(&mut self, new_task: impl AsRef<str>) {
        if !self.task.contains("<initial_user_request>") {
            self.task = format!("<initial_user_request>{}</initial_user_request>", self.task);
        }
        self.task.push('\n');
        self.task.push_str(&format!(
            "<follow_up_user_request> {} </follow_up_user_request>",
            new_task.as_ref().trim()
        ));
        self.stopped = false;
        self.paused = false;
    }

    /// Pauses the agent before the next step.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Clears paused state.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Returns true when the agent is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub(super) async fn check_stop_requested(&mut self) -> Result<(), AgentRunError> {
        if let Some(callback) = self.should_stop_callback.as_mut() {
            let should_stop = callback()
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "should_stop",
                    message,
                })?;
            if should_stop {
                self.stopped = true;
                return Err(AgentRunError::Stopped {
                    reason: "should_stop callback requested stop".to_owned(),
                });
            }
        }

        if let Some(callback) = self.external_status_callback.as_mut() {
            let interrupted = callback()
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "external_agent_status",
                    message,
                })?;
            if interrupted {
                return Err(AgentRunError::ExternalStatusInterrupted);
            }
        }

        if self.stopped {
            return Err(AgentRunError::Stopped {
                reason: "stop requested".to_owned(),
            });
        }
        if self.paused {
            return Err(AgentRunError::Paused);
        }

        Ok(())
    }

    pub(super) async fn invoke_step_callbacks(
        &mut self,
        state: &BrowserStateSummary,
        model_output: &AgentOutput,
        step_number: usize,
    ) -> Result<(), AgentRunError> {
        for callback in &mut self.step_callbacks {
            callback(state, model_output, step_number)
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "new_step",
                    message,
                })?;
        }
        Ok(())
    }

    pub(super) async fn invoke_done_callbacks(&mut self) -> Result<(), AgentRunError> {
        for callback in &mut self.done_callbacks {
            callback(&self.history)
                .await
                .map_err(|message| AgentRunError::Callback {
                    callback: "done",
                    message,
                })?;
        }
        Ok(())
    }
}
