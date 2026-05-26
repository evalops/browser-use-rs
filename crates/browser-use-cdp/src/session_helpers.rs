use super::*;

impl CdpBrowserSession {
    pub(crate) async fn current_page(&self) -> AttachedPage {
        let page = self.page.lock().await.clone();
        if self
            .connection
            .is_registered_session_stale(&page.session_id)
            .await
        {
            return self
                .reattach_current_page(page.clone())
                .await
                .unwrap_or(page);
        }
        page
    }

    pub(crate) async fn set_current_page(&self, page: AttachedPage) {
        *self.page.lock().await = page.clone();
        self.start_video_recording_for_page(&page).await;
    }

    pub(crate) async fn start_video_recording_for_page(&self, page: &AttachedPage) {
        let Some(video_recorder) = &self.video_recorder else {
            return;
        };
        if let Err(error) = video_recorder
            .start_screencast_for_page(&self.connection, page)
            .await
        {
            self.record_lifecycle_event(video_recording_failed_event("start", &error))
                .await;
        }
    }

    pub(crate) async fn apply_viewport_emulation(
        &self,
        page: &AttachedPage,
    ) -> Result<(), BrowserError> {
        apply_viewport_emulation_for_page(&self.connection, page, self.viewport_emulation).await
    }

    pub(crate) async fn wait_for_page_load_settle(&self) {
        if self.page_load_wait.is_disabled() {
            return;
        }
        if !self.page_load_wait.minimum_wait.is_zero() {
            sleep(self.page_load_wait.minimum_wait).await;
        }
        if !self.page_load_wait.network_idle_wait.is_zero() {
            self.wait_for_network_idle(self.page_load_wait.network_idle_wait)
                .await;
        }
    }

    async fn wait_for_network_idle(&self, idle_for: Duration) {
        let deadline = Instant::now() + idle_for;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            let remaining = {
                self.network_activity
                    .lock()
                    .await
                    .idle_remaining(now, idle_for)
            };
            let Some(remaining) = remaining else {
                return;
            };
            let until_deadline = deadline.saturating_duration_since(now);
            let sleep_for = remaining.min(until_deadline).min(Duration::from_millis(50));
            if sleep_for.is_zero() {
                return;
            }
            sleep(sleep_for).await;
        }
    }

    pub(crate) async fn auto_download_pdf_if_needed(&self, url: &str) {
        if !self.auto_download_pdfs || !is_pdf_viewer_url(url) {
            return;
        }
        let Some(downloads_path) = &self.downloads_path else {
            return;
        };

        match self.auto_download_pdf(url, downloads_path).await {
            Ok(Some(event)) => self.record_lifecycle_event(event).await,
            Ok(None) => {}
            Err(error) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::pdf_auto_download_failed(
                    url,
                    error.to_string(),
                ))
                .await;
            }
        }
    }

    pub(crate) async fn auto_download_pdf(
        &self,
        url: &str,
        downloads_path: &Path,
    ) -> Result<Option<BrowserLifecycleEvent>, BrowserError> {
        if let Some(path) = self.cached_auto_pdf_download(url).await {
            if tokio::fs::metadata(&path).await.is_ok() {
                return Ok(None);
            }
        }

        let response = download_http_client()
            .get(url)
            .send()
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        if !response.status().is_success() {
            return Err(BrowserError::StateUnavailable(format!(
                "PDF download returned HTTP {}",
                response.status()
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        tokio::fs::create_dir_all(downloads_path)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        let file_name = pdf_download_filename_from_url(url);
        let path = unique_download_path(downloads_path, &file_name).await?;
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        self.auto_pdf_downloads
            .lock()
            .await
            .insert(url.to_owned(), path.clone());

        Ok(Some(BrowserLifecycleEvent::pdf_auto_downloaded(
            url,
            path.display().to_string(),
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .unwrap_or(file_name),
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        )))
    }

    async fn cached_auto_pdf_download(&self, url: &str) -> Option<PathBuf> {
        self.auto_pdf_downloads.lock().await.get(url).cloned()
    }

    async fn reattach_current_page(
        &self,
        stale_page: AttachedPage,
    ) -> Result<AttachedPage, BrowserError> {
        let page = match attach_to_target(&self.connection, stale_page.target_id.clone()).await {
            Ok(page) => page,
            Err(error) if is_missing_target_error(&error) => {
                attach_or_create_page(&self.connection).await?
            }
            Err(error) => return Err(error),
        };
        self.apply_viewport_emulation(&page).await?;
        let target_id = page.target_id.clone();
        self.set_current_page(page.clone()).await;
        self.clear_cached_dom_state().await;
        self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
            .await;
        Ok(page)
    }

    pub(crate) async fn set_cached_dom_state(&self, dom_state: SerializedDomState) {
        *self.last_dom_state.lock().await = Some(dom_state);
    }

    pub(crate) async fn clear_cached_dom_state(&self) {
        *self.last_dom_state.lock().await = None;
    }

    pub(crate) async fn take_pending_url_policy_error(&self) -> Result<(), BrowserError> {
        if let Some(error) = self.pending_url_policy_error.lock().await.take() {
            return Err(error);
        }
        Ok(())
    }

    pub(crate) async fn clear_matching_pending_url_policy_errors(
        &self,
        handled: &[(String, String)],
    ) {
        let mut pending = self.pending_url_policy_error.lock().await;
        let Some(BrowserError::NavigationBlocked { url, reason }) = pending.as_ref() else {
            return;
        };
        if handled
            .iter()
            .any(|(handled_url, handled_reason)| handled_url == url && handled_reason == reason)
        {
            *pending = None;
        }
    }

    pub(crate) async fn validate_url_policy_before_navigation(
        &self,
        url: &str,
    ) -> Result<(), BrowserError> {
        match self.url_policy.validate(url) {
            Ok(()) => Ok(()),
            Err(BrowserError::NavigationBlocked { url, reason }) => {
                self.record_security_event(BrowserSecurityEvent::prevented_navigation(
                    url.clone(),
                    reason.clone(),
                ))
                .await;
                Err(BrowserError::NavigationBlocked { url, reason })
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn record_security_event(&self, event: BrowserSecurityEvent) {
        let lifecycle_event = event.lifecycle_event.clone();
        let mut events = self.security_events.lock().await;
        push_security_event(&mut events, event);
        drop(events);
        self.record_lifecycle_event(lifecycle_event).await;
    }

    pub(crate) async fn record_lifecycle_event(&self, event: BrowserLifecycleEvent) {
        let mut events = self.lifecycle_events.lock().await;
        push_lifecycle_event_and_publish(&mut events, &self.lifecycle_event_tx, event);
    }

    /// Returns a snapshot of fine-grained lifecycle events recorded so far.
    pub async fn lifecycle_events(&self) -> Vec<BrowserLifecycleEvent> {
        self.lifecycle_events.lock().await.iter().cloned().collect()
    }

    /// Returns recorded lifecycle events converted to the adapter taxonomy.
    pub async fn lifecycle_adapter_events(&self) -> Vec<BrowserLifecycleAdapterEvent> {
        browser_lifecycle_adapter_events(&self.lifecycle_events().await)
    }

    /// Subscribes to future fine-grained lifecycle events.
    pub fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::new(self.lifecycle_event_tx.subscribe())
    }

    /// Subscribes to future lifecycle events converted to adapter events.
    pub fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        BrowserLifecycleAdapterEventSubscription::new(self.subscribe_lifecycle_events())
    }
}
