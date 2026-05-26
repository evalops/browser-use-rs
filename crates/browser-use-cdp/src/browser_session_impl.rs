use super::*;

use async_trait::async_trait;
use browser_use_dom::{BrowserStateSummary, TabInfo};

#[async_trait]
impl BrowserSession for CdpBrowserSession {
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::new(self.lifecycle_event_tx.subscribe())
    }

    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        // State capture is also the point where asynchronous policy/watchdog
        // work becomes visible to the agent. Pending URL-policy violations are
        // surfaced before any new DOM is trusted.
        self.enforce_open_tab_url_policy().await?;
        // Waiting before reading URL/title/DOM keeps prompts from describing an
        // intermediate navigation or loading spinner as if it were stable page
        // state.
        self.wait_for_page_load_settle().await;
        let (url, title) = self.page_location().await?;
        let is_pdf_viewer = is_pdf_viewer_url(&url);
        if is_pdf_viewer {
            // Chrome's built-in PDF viewer is visible as a page, but
            // browser-use also wants the underlying PDF artifact when downloads
            // are accepted. The download path is cached so repeated state calls
            // do not re-fetch the same document.
            self.auto_download_pdf_if_needed(&url).await;
        }
        let page_info = self.page_info().await?;
        let dom_state = self.dom_state().await?;
        // Action methods use the cached DOM to resolve the exact element the
        // model saw. Updating the cache immediately after capture keeps the
        // prompt and subsequent indexed actions tied to the same selector map.
        self.set_cached_dom_state(dom_state.clone()).await;
        self.apply_dom_highlights_if_enabled(&dom_state).await;
        let pagination_buttons = detect_pagination_buttons(&dom_state);
        let current_page = self.current_page().await;
        let tabs = page_tabs(&self.connection).await?;
        let (recent_events, closed_popup_messages, browser_errors) = {
            let events = self.security_events.lock().await;
            security_event_state_fields(&events)
        };
        let screenshot = if include_screenshot {
            Some(self.screenshot().await?.base64_png)
        } else {
            None
        };

        // Some connected-browser environments cannot enumerate tabs. Falling
        // back to the current page still gives the agent a switchable tab id
        // while preserving the full CDP target id for executor calls.
        Ok(BrowserStateSummary {
            dom_state,
            url: url.clone(),
            title: title.clone(),
            tabs: if tabs.is_empty() {
                vec![TabInfo {
                    url,
                    title,
                    tab_id: TabInfo::tab_id_for_target(&current_page.target_id),
                    target_id: current_page.target_id,
                    parent_target_id: None,
                }]
            } else {
                tabs
            },
            screenshot,
            page_info: Some(page_info),
            pixels_above: page_info.pixels_above,
            pixels_below: page_info.pixels_below,
            browser_errors,
            is_pdf_viewer,
            recent_events,
            pending_network_requests: vec![],
            pagination_buttons,
            closed_popup_messages,
        })
    }

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
        self.validate_url_policy_before_navigation(url).await?;
        if new_tab {
            let target_id = create_target(&self.connection, url).await?;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_created(
                target_id.clone(),
                url.to_owned(),
            ))
            .await;
            self.record_lifecycle_event(BrowserLifecycleEvent::navigation_started(
                target_id.clone(),
                url.to_owned(),
            ))
            .await;
            let page = match attach_to_target(&self.connection, target_id.clone()).await {
                Ok(page) => page,
                Err(error) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                        target_id.clone(),
                        url.to_owned(),
                        error.to_string(),
                    ))
                    .await;
                    return Err(error);
                }
            };
            self.apply_viewport_emulation(&page).await?;
            let target_id = page.target_id.clone();
            self.set_current_page(page).await;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id.clone()))
                .await;
            self.clear_cached_dom_state().await;
            let result = self.enforce_url_policy_after_navigation_settle().await;
            match &result {
                Ok(()) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_completed(
                        target_id,
                        url.to_owned(),
                    ))
                    .await;
                }
                Err(error) => {
                    self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                        target_id,
                        url.to_owned(),
                        error.to_string(),
                    ))
                    .await;
                }
            }
            return result;
        }

        let page = self.current_page().await;
        self.record_lifecycle_event(BrowserLifecycleEvent::navigation_started(
            page.target_id.clone(),
            url.to_owned(),
        ))
        .await;
        let navigate = self.connection.command(
            "Page.navigate",
            json!({
                "url": url,
            }),
            Some(&page.session_id),
        );
        let navigate_result = if self.navigation_timeout_ms == 0 {
            navigate.await
        } else {
            match tokio::time::timeout(Duration::from_millis(self.navigation_timeout_ms), navigate)
                .await
            {
                Ok(result) => result,
                Err(_) => {
                    let timeout_seconds =
                        format!("{:.3}", self.navigation_timeout_ms as f64 / 1000.0);
                    self.record_lifecycle_event(BrowserLifecycleEvent::network_timeout(
                        page.target_id.clone(),
                        url.to_owned(),
                        timeout_seconds,
                    ))
                    .await;
                    return Err(BrowserError::NavigationFailed(format!(
                        "Page.navigate timed out after {}ms for {url}",
                        self.navigation_timeout_ms
                    )));
                }
            }
        };
        if let Err(error) = navigate_result {
            self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                page.target_id.clone(),
                url.to_owned(),
                error.to_string(),
            ))
            .await;
            return Err(error);
        }
        self.clear_cached_dom_state().await;
        let result = self.enforce_url_policy_after_navigation_settle().await;
        match &result {
            Ok(()) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::navigation_completed(
                    page.target_id,
                    url.to_owned(),
                ))
                .await;
            }
            Err(error) => {
                self.record_lifecycle_event(BrowserLifecycleEvent::navigation_failed(
                    page.target_id,
                    url.to_owned(),
                    error.to_string(),
                ))
                .await;
            }
        }
        result
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let history = self
            .connection
            .command(
                "Page.getNavigationHistory",
                json!({}),
                Some(&page.session_id),
            )
            .await?;
        let entry_id = previous_navigation_entry_id(&history)?;
        self.connection
            .command(
                "Page.navigateToHistoryEntry",
                json!({ "entryId": entry_id }),
                Some(&page.session_id),
            )
            .await?;
        self.clear_cached_dom_state().await;
        self.enforce_url_policy_after_navigation_settle().await
    }

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        let target_id = resolve_page_target_id(&self.connection, target_id).await?;
        let page = attach_to_target(&self.connection, target_id).await?;
        self.apply_viewport_emulation(&page).await?;
        let target_id = page.target_id.clone();
        self.set_current_page(page).await;
        self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
            .await;
        self.clear_cached_dom_state().await;
        self.enforce_open_tab_url_policy().await
    }

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        let target_id = resolve_page_target_id(&self.connection, target_id).await?;
        self.connection
            .command(
                "Target.closeTarget",
                json!({ "targetId": &target_id }),
                None,
            )
            .await?;
        self.record_lifecycle_event(BrowserLifecycleEvent::target_closed(target_id.clone()))
            .await;

        if self.current_page().await.target_id == target_id {
            let page = attach_or_create_page(&self.connection).await?;
            self.apply_viewport_emulation(&page).await?;
            let target_id = page.target_id.clone();
            self.set_current_page(page).await;
            self.record_lifecycle_event(BrowserLifecycleEvent::target_switched(target_id))
                .await;
        }
        self.clear_cached_dom_state().await;

        self.enforce_open_tab_url_policy().await
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            self.highlight_element_if_enabled(&cached.element).await;
            match self
                .call_element_function(
                    &cached.element,
                    element_action_function_js(CLICK_ELEMENT_ACTION_JS),
                )
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_navigation_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, click_element_js(fallback_index))
            .await?;
        self.enforce_url_policy_after_navigation_settle().await
    }

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        self.highlight_coordinates_if_enabled(x, y).await;
        for event_type in ["mousePressed", "mouseReleased"] {
            self.connection
                .command(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": event_type,
                        "x": x,
                        "y": y,
                        "button": "left",
                        "clickCount": 1,
                    }),
                    Some(&page.session_id),
                )
                .await?;
        }
        self.enforce_url_policy_after_navigation_settle().await
    }

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError> {
        let text_json = serde_json::to_string(text)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let action = if clear {
            format!(
                "el.focus(); el.value = {text_json}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }}));"
            )
        } else {
            format!(
                "el.focus(); el.value = (el.value || '') + {text_json}; el.dispatchEvent(new Event('input', {{ bubbles: true }})); el.dispatchEvent(new Event('change', {{ bubbles: true }}));"
            )
        };
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            self.highlight_element_if_enabled(&cached.element).await;
            match self
                .call_element_function(&cached.element, element_action_function_js(&action))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, element_action_js(fallback_index, &action))
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError> {
        let direction = if down { 1.0 } else { -1.0 };
        if let Some(index) = index {
            let action = format!(
                "el.scrollBy(0, (el.clientHeight || window.innerHeight) * {});",
                pages * direction
            );
            let cached_element = self.cached_element(index).await;
            if let Some(cached) = cached_element.as_ref() {
                match self
                    .call_element_function(&cached.element, element_action_function_js(&action))
                    .await
                {
                    Ok(()) => return self.enforce_url_policy_after_settle().await,
                    Err(error) if should_fallback_to_index_traversal(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            let page = self
                .page_for_index_fallback(cached_element.as_ref())
                .await?;
            let fallback_index = cached_element
                .as_ref()
                .map(|cached| cached.target_local_index)
                .unwrap_or(index);
            self.evaluate_effect_for_page(&page, element_action_js(fallback_index, &action))
                .await?;
            return self.enforce_url_policy_after_settle().await;
        }
        self.evaluate_effect(format!(
            "window.scrollBy(0, window.innerHeight * {}); true;",
            pages * direction
        ))
        .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
        let found = self
            .evaluate_json(&scroll_to_text_js(text)?)
            .await?
            .as_bool()
            .ok_or_else(|| BrowserError::MissingResponseData("scroll-to-text result".to_owned()))?;
        self.enforce_url_policy_after_settle().await?;
        Ok(found)
    }

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
        let page = self.current_page().await;
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": code,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        let rendered = render_runtime_evaluate_result(&result)?;
        self.enforce_url_policy_after_settle().await?;
        Ok(rendered)
    }

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function_value(
                    &cached.element,
                    element_function_js(DROPDOWN_OPTIONS_BODY_JS),
                )
                .await
            {
                Ok(value) => return parse_dropdown_options_value(value),
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        let value = self
            .evaluate_json_for_page(&page, &dropdown_options_js(fallback_index), false)
            .await?;
        parse_dropdown_options_value(value)
    }

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
        let body = select_dropdown_option_body_js(text)?;
        let cached_element = self.cached_element(index).await;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(&body))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = self
            .page_for_index_fallback(cached_element.as_ref())
            .await?;
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        self.evaluate_effect_for_page(&page, select_dropdown_option_js(fallback_index, text)?)
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn page_text(&self) -> Result<String, BrowserError> {
        let page = self.current_page().await;
        let mut texts = Vec::new();
        let root_text = self.page_text_for_page(&page).await?;
        if !root_text.trim().is_empty() {
            texts.push(root_text);
        }
        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        for child_page in child_pages {
            let Ok(text) = self.page_text_for_page(&child_page.page).await else {
                continue;
            };
            if !text.trim().is_empty() {
                texts.push(text);
            }
        }
        Ok(texts.join("\n"))
    }

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        let page = self.current_page().await;
        let mut elements = self
            .find_elements_for_page(&page, selector, attributes, max_results, include_text)
            .await?;
        if elements.len() >= max_results {
            return Ok(elements);
        }

        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        for child_page in child_pages {
            let remaining = max_results.saturating_sub(elements.len());
            if remaining == 0 {
                break;
            }
            let Ok(mut child_elements) = self
                .find_elements_for_page(
                    &child_page.page,
                    selector,
                    attributes,
                    remaining,
                    include_text,
                )
                .await
            else {
                continue;
            };
            elements.append(&mut child_elements);
        }

        Ok(elements)
    }

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        let normalized_keys = normalize_send_keys(keys);
        if normalized_keys.contains('+') {
            let parts = normalized_keys
                .split('+')
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if let Some((main_key, modifiers)) = parts.split_last() {
                let modifier_value = modifier_mask(modifiers);
                for modifier in modifiers {
                    self.connection
                        .command(
                            "Input.dispatchKeyEvent",
                            key_event_params("keyDown", modifier, 0),
                            Some(&page.session_id),
                        )
                        .await?;
                }
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        key_event_params("keyDown", main_key, modifier_value),
                        Some(&page.session_id),
                    )
                    .await?;
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        key_event_params("keyUp", main_key, modifier_value),
                        Some(&page.session_id),
                    )
                    .await?;
                for modifier in modifiers.iter().rev() {
                    self.connection
                        .command(
                            "Input.dispatchKeyEvent",
                            key_event_params("keyUp", modifier, 0),
                            Some(&page.session_id),
                        )
                        .await?;
                }
            }
            return self.enforce_url_policy_after_settle().await;
        }

        if is_special_key(&normalized_keys) {
            self.connection
                .command(
                    "Input.dispatchKeyEvent",
                    key_event_params("keyDown", &normalized_keys, 0),
                    Some(&page.session_id),
                )
                .await?;
            if normalized_keys == "Enter" {
                self.connection
                    .command(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": "char",
                            "text": "\r",
                            "key": "Enter",
                        }),
                        Some(&page.session_id),
                    )
                    .await?;
            }
            self.connection
                .command(
                    "Input.dispatchKeyEvent",
                    key_event_params("keyUp", &normalized_keys, 0),
                    Some(&page.session_id),
                )
                .await?;
            return self.enforce_url_policy_after_settle().await;
        }

        self.connection
            .command(
                "Input.insertText",
                json!({
                    "text": normalized_keys,
                }),
                Some(&page.session_id),
            )
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError> {
        let canonical_path = std::fs::canonicalize(path).map_err(|error| {
            BrowserError::ActionFailed(format!(
                "failed to resolve upload file '{}': {error}",
                path.display()
            ))
        })?;
        if !canonical_path.is_file() {
            return Err(BrowserError::ActionFailed(format!(
                "upload path is not a file: {}",
                canonical_path.display()
            )));
        }
        let path_string = canonical_path.to_str().ok_or_else(|| {
            BrowserError::ActionFailed(format!(
                "upload path is not valid UTF-8: {}",
                canonical_path.display()
            ))
        })?;

        let token = format!(
            "browser-use-rs-upload-{}",
            self.connection.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let token_json = serde_json::to_string(&token)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let mark_upload_body = format!(
            r#"
  if (el.tagName.toLowerCase() !== 'input' || el.type !== 'file') {{
    throw new Error('Element is not a file input');
  }}
  el.setAttribute('data-browser-use-rs-upload-token', {token_json});
  return true;
"#
        );
        let cached_element = self.cached_element(index).await;
        let mut marked_cached_element = None;
        if let Some(cached) = cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(&mark_upload_body))
                .await
            {
                Ok(()) => marked_cached_element = Some(cached.clone()),
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let page = if let Some(cached) = marked_cached_element.as_ref() {
            self.page_for_element(&cached.element).await?
        } else {
            self.page_for_index_fallback(cached_element.as_ref())
                .await?
        };
        let fallback_index = cached_element
            .as_ref()
            .map(|cached| cached.target_local_index)
            .unwrap_or(index);
        if marked_cached_element.is_none() {
            self.evaluate_effect_for_page(
                &page,
                element_eval_js(fallback_index, &mark_upload_body),
            )
            .await?;
        }

        let document = self
            .connection
            .command(
                "DOM.getDocument",
                json!({ "depth": -1, "pierce": true }),
                Some(&page.session_id),
            )
            .await?;
        let root_node_id = document
            .get("root")
            .and_then(|root| u32_field(root, "nodeId"))
            .ok_or_else(|| {
                BrowserError::MissingResponseData("DOM.getDocument root nodeId".to_owned())
            })?;
        let selector = format!(r#"[data-browser-use-rs-upload-token="{token}"]"#);
        let query_result = self
            .connection
            .command(
                "DOM.querySelector",
                json!({
                    "nodeId": root_node_id,
                    "selector": selector,
                }),
                Some(&page.session_id),
            )
            .await?;
        let node_id = u32_field(&query_result, "nodeId")
            .filter(|node_id| *node_id != 0)
            .ok_or_else(|| {
                BrowserError::MissingResponseData("DOM.querySelector nodeId".to_owned())
            })?;

        self.connection
            .command(
                "DOM.setFileInputFiles",
                json!({
                    "nodeId": node_id,
                    "files": [path_string],
                }),
                Some(&page.session_id),
            )
            .await?;

        let finish_upload_body = r#"
  el.removeAttribute('data-browser-use-rs-upload-token');
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
"#;
        if let Some(cached) = marked_cached_element.as_ref() {
            match self
                .call_element_function(&cached.element, element_function_js(finish_upload_body))
                .await
            {
                Ok(()) => return self.enforce_url_policy_after_settle().await,
                Err(error) if should_fallback_to_index_traversal(&error) => {}
                Err(error) => return Err(error),
            }
        }
        self.evaluate_effect_for_page(&page, element_eval_js(fallback_index, finish_upload_body))
            .await?;
        self.enforce_url_policy_after_settle().await
    }

    async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        let page = self.current_page().await;
        let result = self
            .connection
            .command(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "fromSurface": true,
                }),
                Some(&page.session_id),
            )
            .await?;

        let base64_png = result
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                BrowserError::MissingResponseData("Page.captureScreenshot data".to_owned())
            })?;

        Ok(Screenshot { base64_png })
    }

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError> {
        let page = self.current_page().await;
        let (paper_width, paper_height) = paper_size_inches(paper_format);
        let result = self
            .connection
            .command(
                "Page.printToPDF",
                json!({
                    "printBackground": print_background,
                    "landscape": landscape,
                    "scale": scale,
                    "paperWidth": paper_width,
                    "paperHeight": paper_height,
                }),
                Some(&page.session_id),
            )
            .await?;

        let base64_pdf = result
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("Page.printToPDF data".to_owned()))?;

        Ok(Pdf { base64_pdf })
    }
}
