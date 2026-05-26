use super::*;

impl CdpBrowserSession {
    /// Connects to an existing DevTools endpoint with the default profile.
    pub async fn connect(endpoint: DevToolsEndpoint) -> Result<Self, BrowserError> {
        Self::connect_with_profile(endpoint, &BrowserProfile::default()).await
    }

    /// Connects to an existing DevTools endpoint using profile-specific options.
    ///
    /// This path does not launch or own the browser process. It still applies
    /// permissions, download behavior, viewport emulation, recording hooks, and
    /// lifecycle watchdogs where those settings make sense for an attached
    /// browser.
    pub async fn connect_with_profile(
        endpoint: DevToolsEndpoint,
        profile: &BrowserProfile,
    ) -> Result<Self, BrowserError> {
        let downloads = SessionDownloads::from_profile(profile)?;
        let connection =
            CdpConnection::connect_with_headers(&endpoint, profile.headers.as_ref()).await?;
        let permission_grant_event =
            grant_browser_permissions(&connection, &profile.permissions).await;
        if let Some(downloads_path) = &downloads.path {
            enable_browser_download_events(&connection, downloads_path).await?;
        }
        let page = attach_or_create_page(&connection).await?;
        let initial_page = page.clone();
        let viewport_emulation = ViewportEmulationConfig::from_profile(profile);
        apply_viewport_emulation_for_page(&connection, &page, viewport_emulation).await?;
        let page = Arc::new(Mutex::new(page));
        let last_dom_state = Arc::new(Mutex::new(None));
        let pending_url_policy_error = Arc::new(Mutex::new(None));
        let security_events = Arc::new(Mutex::new(VecDeque::new()));
        let lifecycle_events = Arc::new(Mutex::new(VecDeque::new()));
        let network_activity = Arc::new(Mutex::new(NetworkActivityState::new(Instant::now())));
        let har_recorder = CdpHarRecorder::from_profile(profile);
        let video_recorder = CdpVideoRecorder::from_profile(profile);
        let trace_recorder = CdpTraceRecorder::from_profile(profile);
        let auto_pdf_downloads = Arc::new(Mutex::new(BTreeMap::new()));
        let cdp_auto_pdf_download = CdpAutoPdfDownloadState::from_downloads(
            profile.auto_download_pdfs,
            downloads.path.as_deref(),
            auto_pdf_downloads.clone(),
        );
        let (lifecycle_event_tx, _) = broadcast::channel(256);
        {
            let mut events = lifecycle_events.lock().await;
            push_lifecycle_event_and_publish(
                &mut events,
                &lifecycle_event_tx,
                BrowserLifecycleEvent::browser_connected(endpoint.http_url.clone()),
            );
            if let Some(event) = permission_grant_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
        }
        let lifecycle_watchdog = BrowserLifecycleWatchdog::start(
            connection.clone(),
            lifecycle_events.clone(),
            lifecycle_event_tx.clone(),
            profile.network_request_timeout_ms,
            network_activity.clone(),
            BrowserLifecycleWatchdogRecorders {
                cdp_auto_pdf_download,
                har_recorder: har_recorder.clone(),
                video_recorder: video_recorder.clone(),
            },
        );
        let page_load_wait = PageLoadWaitConfig::from_profile(profile);

        let session = Self {
            connection,
            page,
            last_dom_state,
            pending_url_policy_error,
            security_events,
            lifecycle_events,
            lifecycle_event_tx,
            url_policy: UrlAccessPolicy::from_profile(profile),
            iframe_traversal: IframeTraversalConfig::from_profile(profile),
            paint_order_filtering: profile.paint_order_filtering,
            viewport_emulation,
            page_load_wait,
            interaction_highlight: InteractionHighlightConfig::from_profile(profile),
            dom_highlight: DomHighlightConfig::from_profile(profile),
            network_activity,
            har_recorder,
            video_recorder,
            trace_recorder,
            downloads_path: downloads.path,
            auto_download_pdfs: profile.auto_download_pdfs,
            auto_pdf_downloads,
            storage_state_path: None,
            navigation_timeout_ms: profile.navigation_timeout_ms,
            _lifecycle_watchdog: lifecycle_watchdog,
            _security_watchdog: None,
            _launched_browser: None,
            _downloads_dir: downloads.temp_dir,
        };
        session.start_video_recording_for_page(&initial_page).await;
        Ok(session)
    }

    /// Launches or creates a browser described by `profile` and connects to it.
    ///
    /// Local profiles spawn Chrome and keep a process handle unless
    /// `keep_alive` asks to detach. Cloud profiles create a Browser Use Cloud
    /// session and connect to its returned CDP websocket.
    pub async fn launch(profile: &BrowserProfile) -> Result<Self, BrowserError> {
        let downloads = SessionDownloads::from_profile(profile)?;
        let url_policy = UrlAccessPolicy::from_profile(profile);
        let (endpoint, launched_browser) = if profile.uses_cloud() {
            (profile.create_cloud_devtools_endpoint().await?, None)
        } else {
            let launched_browser = profile.launch_local().await?;
            (launched_browser.endpoint().clone(), Some(launched_browser))
        };
        let launched_browser = launched_browser.and_then(|browser| {
            if profile_keeps_launched_browser_alive(profile) {
                let _ = browser.detach();
                None
            } else {
                Some(browser)
            }
        });
        let connection =
            CdpConnection::connect_with_headers(&endpoint, profile.headers.as_ref()).await?;
        let permission_grant_event =
            grant_browser_permissions(&connection, &profile.permissions).await;
        if let Some(downloads_path) = &downloads.path {
            enable_browser_download_events(&connection, downloads_path).await?;
        }
        let page = attach_or_create_page(&connection).await?;
        let initial_page = page.clone();
        let viewport_emulation = ViewportEmulationConfig::from_profile(profile);
        apply_viewport_emulation_for_page(&connection, &page, viewport_emulation).await?;
        let storage_state_loaded_event = if let Some(storage_state_path) =
            &profile.storage_state_path
        {
            let storage_state = load_browser_storage_state(&connection, storage_state_path).await?;
            apply_origin_storage_state(&connection, &page, &storage_state).await?;
            let (cookies_count, origins_count) = storage_state_counts(&storage_state);
            Some(BrowserLifecycleEvent::storage_state_loaded(
                storage_state_path.display().to_string(),
                cookies_count,
                origins_count,
            ))
        } else {
            None
        };
        let page = Arc::new(Mutex::new(page));
        let last_dom_state = Arc::new(Mutex::new(None));
        let pending_url_policy_error = Arc::new(Mutex::new(None));
        let security_events = Arc::new(Mutex::new(VecDeque::new()));
        let lifecycle_events = Arc::new(Mutex::new(VecDeque::new()));
        let network_activity = Arc::new(Mutex::new(NetworkActivityState::new(Instant::now())));
        let har_recorder = CdpHarRecorder::from_profile(profile);
        let video_recorder = CdpVideoRecorder::from_profile(profile);
        let trace_recorder = CdpTraceRecorder::from_profile(profile);
        let auto_pdf_downloads = Arc::new(Mutex::new(BTreeMap::new()));
        let cdp_auto_pdf_download = CdpAutoPdfDownloadState::from_downloads(
            profile.auto_download_pdfs,
            downloads.path.as_deref(),
            auto_pdf_downloads.clone(),
        );
        let (lifecycle_event_tx, _) = broadcast::channel(256);
        {
            let mut events = lifecycle_events.lock().await;
            push_lifecycle_event_and_publish(
                &mut events,
                &lifecycle_event_tx,
                BrowserLifecycleEvent::browser_connected(endpoint.http_url.clone()),
            );
            if let Some(event) = permission_grant_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
            if let Some(event) = storage_state_loaded_event {
                push_lifecycle_event_and_publish(&mut events, &lifecycle_event_tx, event);
            }
        }
        let lifecycle_watchdog = BrowserLifecycleWatchdog::start(
            connection.clone(),
            lifecycle_events.clone(),
            lifecycle_event_tx.clone(),
            profile.network_request_timeout_ms,
            network_activity.clone(),
            BrowserLifecycleWatchdogRecorders {
                cdp_auto_pdf_download,
                har_recorder: har_recorder.clone(),
                video_recorder: video_recorder.clone(),
            },
        );
        let security_watchdog = BrowserSecurityWatchdog::start(
            connection.clone(),
            page.clone(),
            last_dom_state.clone(),
            pending_url_policy_error.clone(),
            security_events.clone(),
            LifecycleEventSink {
                events: lifecycle_events.clone(),
                event_tx: lifecycle_event_tx.clone(),
            },
            url_policy.clone(),
        )
        .await?;

        let session = Self {
            connection,
            page,
            last_dom_state,
            pending_url_policy_error,
            security_events,
            lifecycle_events,
            lifecycle_event_tx,
            url_policy,
            iframe_traversal: IframeTraversalConfig::from_profile(profile),
            paint_order_filtering: profile.paint_order_filtering,
            viewport_emulation,
            page_load_wait: PageLoadWaitConfig::from_profile(profile),
            interaction_highlight: InteractionHighlightConfig::from_profile(profile),
            dom_highlight: DomHighlightConfig::from_profile(profile),
            network_activity,
            har_recorder,
            video_recorder,
            trace_recorder,
            downloads_path: downloads.path,
            auto_download_pdfs: profile.auto_download_pdfs,
            auto_pdf_downloads,
            storage_state_path: profile.storage_state_path.clone(),
            navigation_timeout_ms: profile.navigation_timeout_ms,
            _lifecycle_watchdog: lifecycle_watchdog,
            _security_watchdog: security_watchdog,
            _launched_browser: launched_browser,
            _downloads_dir: downloads.temp_dir,
        };
        session.start_video_recording_for_page(&initial_page).await;
        Ok(session)
    }

    /// Closes the browser after flushing configured storage, HAR, video, and trace artifacts.
    pub async fn close_browser(&self) -> Result<(), BrowserError> {
        self.record_lifecycle_event(BrowserLifecycleEvent::browser_close_requested())
            .await;
        if let Some(path) = &self.storage_state_path {
            self.save_storage_state(path).await?;
        }
        if let Some(har_recorder) = &self.har_recorder {
            let _ = har_recorder.write_har().await;
        }
        if let Some(video_recorder) = &self.video_recorder {
            match video_recorder.stop_and_write(&self.connection).await {
                Ok((_path, Some(error))) => {
                    self.record_lifecycle_event(video_recording_failed_event("encode", &error))
                        .await;
                }
                Ok((_path, None)) => {}
                Err(error) => {
                    self.record_lifecycle_event(video_recording_failed_event("stop", &error))
                        .await;
                }
            }
        }
        if let Err(error) = self.write_trace_artifact().await {
            self.record_lifecycle_event(trace_recording_failed_event("write", &error))
                .await;
        }
        self.connection.mark_intentional_stop();
        self.connection
            .command("Browser.close", json!({}), None)
            .await
            .map(|_| ())
    }

    /// Saves cookies and origin storage to a Playwright/browser-use compatible JSON file.
    pub async fn save_storage_state(&self, path: &Path) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let storage_state = browser_storage_state(&self.connection, Some(&page)).await?;
        let (cookies_count, origins_count) = storage_state_counts(&storage_state);
        write_storage_state(path, &storage_state).await?;
        self.record_lifecycle_event(BrowserLifecycleEvent::storage_state_saved(
            path.display().to_string(),
            cookies_count,
            origins_count,
        ))
        .await;
        Ok(())
    }

    /// Loads cookies and origin storage from a storage-state JSON file.
    pub async fn load_storage_state(&self, path: &Path) -> Result<(), BrowserError> {
        let storage_state = load_browser_storage_state(&self.connection, path).await?;
        let page = self.current_page().await;
        apply_origin_storage_state(&self.connection, &page, &storage_state).await?;
        let (cookies_count, origins_count) = storage_state_counts(&storage_state);
        self.record_lifecycle_event(BrowserLifecycleEvent::storage_state_loaded(
            path.display().to_string(),
            cookies_count,
            origins_count,
        ))
        .await;
        Ok(())
    }

    pub(crate) async fn write_trace_artifact(&self) -> Result<Option<PathBuf>, BrowserError> {
        let Some(trace_recorder) = &self.trace_recorder else {
            return Ok(None);
        };
        let generated_at_millis = trace_epoch_millis();
        let current_page = self.page.lock().await.clone();
        let lifecycle_events = self
            .lifecycle_events
            .lock()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let security_events = self
            .security_events
            .lock()
            .await
            .iter()
            .map(trace_security_event_json)
            .collect::<Vec<_>>();
        let last_dom_state = self.last_dom_state.lock().await.clone();
        let artifact = json!({
            "schema_version": TRACE_ARTIFACT_SCHEMA_VERSION,
            "artifact": {
                "kind": TRACE_ARTIFACT_KIND,
                "format": "json",
                "runtime": "direct_cdp",
                "playwright_trace_zip": false,
            },
            "generated_at": trace_timestamp(generated_at_millis),
            "current_page": {
                "target_id": current_page.target_id,
                "session_id": current_page.session_id,
            },
            "lifecycle_events": lifecycle_events,
            "security_events": security_events,
            "last_dom_state": last_dom_state,
        });

        trace_recorder
            .write_trace_artifact(artifact)
            .await
            .map(Some)
    }
}
