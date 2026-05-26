use super::*;

impl CdpBrowserSession {
    pub(crate) async fn cached_element(&self, index: u32) -> Option<CachedDomElementRef> {
        let state = self.last_dom_state.lock().await;
        let state = state.as_ref()?;
        let element = state.selector_map.get(&index)?.clone();
        let target_local_index =
            target_local_index_for_global_index(&state.selector_map, index, &element.target_id);

        Some(CachedDomElementRef {
            element,
            target_local_index,
        })
    }

    pub(crate) async fn evaluate_json(&self, expression: &str) -> Result<Value, BrowserError> {
        self.evaluate_json_with_options(expression, false).await
    }

    async fn evaluate_json_with_options(
        &self,
        expression: &str,
        include_command_line_api: bool,
    ) -> Result<Value, BrowserError> {
        let page = self.current_page().await;
        self.evaluate_json_for_page(&page, expression, include_command_line_api)
            .await
    }

    pub(crate) async fn evaluate_json_for_page(
        &self,
        page: &AttachedPage,
        expression: &str,
        include_command_line_api: bool,
    ) -> Result<Value, BrowserError> {
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                runtime_evaluate_params(expression, include_command_line_api),
                Some(&page.session_id),
            )
            .await?;

        runtime_evaluate_value(result)
    }

    pub(crate) async fn evaluate_effect(&self, expression: String) -> Result<(), BrowserError> {
        let page = self.current_page().await;
        self.evaluate_effect_for_page(&page, expression).await
    }

    pub(crate) async fn evaluate_effect_for_page(
        &self,
        page: &AttachedPage,
        expression: String,
    ) -> Result<(), BrowserError> {
        let result = self
            .connection
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        let _ = runtime_evaluate_value(result)?;
        Ok(())
    }

    pub(crate) async fn highlight_element_if_enabled(&self, element: &DomElementRef) {
        let Some(script) = self.interaction_highlight.element_script(element.bounds) else {
            return;
        };
        let Ok(page) = self.page_for_element(element).await else {
            return;
        };
        let _ = self.evaluate_effect_for_page(&page, script).await;
    }

    pub(crate) async fn highlight_coordinates_if_enabled(&self, x: i32, y: i32) {
        let Some(script) = self.interaction_highlight.coordinate_script(x, y) else {
            return;
        };
        let page = self.current_page().await;
        let _ = self.evaluate_effect_for_page(&page, script).await;
    }

    pub(crate) async fn apply_dom_highlights_if_enabled(&self, dom_state: &SerializedDomState) {
        let Some(script) = self.dom_highlight.overlay_script(&dom_state.selector_map) else {
            return;
        };
        let page = self.current_page().await;
        let _ = self.evaluate_effect_for_page(&page, script).await;
    }

    pub(crate) async fn page_for_element(
        &self,
        element: &DomElementRef,
    ) -> Result<AttachedPage, BrowserError> {
        let page = self.current_page().await;
        if element.target_id == page.target_id {
            return Ok(page);
        }

        attach_to_target(&self.connection, element.target_id.clone()).await
    }

    pub(crate) async fn page_for_index_fallback(
        &self,
        cached_element: Option<&CachedDomElementRef>,
    ) -> Result<AttachedPage, BrowserError> {
        let page = self.current_page().await;
        let target_id = index_fallback_target_id(&page, cached_element).to_owned();
        if target_id == page.target_id {
            return Ok(page);
        }

        attach_to_target(&self.connection, target_id).await
    }

    async fn resolve_element_object_id(
        &self,
        page: &AttachedPage,
        element: &DomElementRef,
    ) -> Result<String, BrowserError> {
        let params = if element.backend_node_id != 0 {
            json!({ "backendNodeId": element.backend_node_id })
        } else if let Some(node_id) = element.node_id {
            json!({ "nodeId": node_id })
        } else {
            return Err(BrowserError::MissingResponseData(
                "cached element node id".to_owned(),
            ));
        };

        self.connection
            .command("DOM.resolveNode", params, Some(&page.session_id))
            .await?
            .get("object")
            .and_then(|object| object.get("objectId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("DOM.resolveNode objectId".to_owned()))
    }

    pub(crate) async fn call_element_function(
        &self,
        element: &DomElementRef,
        function_declaration: String,
    ) -> Result<(), BrowserError> {
        let _ = self
            .call_element_function_value(element, function_declaration)
            .await?;
        Ok(())
    }

    pub(crate) async fn call_element_function_value(
        &self,
        element: &DomElementRef,
        function_declaration: String,
    ) -> Result<Value, BrowserError> {
        let page = self.page_for_element(element).await?;
        let object_id = self.resolve_element_object_id(&page, element).await?;
        let result = self
            .connection
            .command(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": function_declaration,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                Some(&page.session_id),
            )
            .await?;
        runtime_command_value(result, "Runtime.callFunctionOn")
    }

    pub(crate) async fn page_location(&self) -> Result<(String, String), BrowserError> {
        let value = self
            .evaluate_json("JSON.stringify({ url: location.href, title: document.title })")
            .await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("Runtime.evaluate string value".to_owned())
        })?;
        let page: Value = serde_json::from_str(encoded)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        Ok((
            page.get("url")
                .and_then(Value::as_str)
                .unwrap_or("about:blank")
                .to_owned(),
            page.get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        ))
    }

    pub(crate) async fn page_info(&self) -> Result<PageInfo, BrowserError> {
        let value = self.evaluate_json(PAGE_INFO_JS).await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("Runtime.evaluate page info".to_owned())
        })?;
        let page_info: Value = serde_json::from_str(encoded)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        page_info_from_value(&page_info)
            .ok_or_else(|| BrowserError::MissingResponseData("page info fields".to_owned()))
    }

    pub(crate) async fn dom_state(&self) -> Result<SerializedDomState, BrowserError> {
        let page = self.current_page().await;
        let root_interactive_js =
            interactive_elements_js(self.iframe_traversal, self.paint_order_filtering);
        let value = self
            .evaluate_json_for_page(&page, &root_interactive_js, true)
            .await?;
        let accessibility = self
            .accessibility_enrichment(&page)
            .await
            .unwrap_or_default();
        let _ = self
            .evaluate_effect_for_page(&page, CLEANUP_AX_REFS_JS.to_owned())
            .await;
        let root_state = dom_state_from_interactive_value(&page.target_id, &value, &accessibility)?;
        let frame_infos = self.frame_element_infos(&page).await.unwrap_or_default();
        let child_pages = self
            .iframe_target_pages(&page, &frame_infos)
            .await
            .unwrap_or_default();
        let mut child_states = Vec::new();

        for child_page in child_pages {
            let child_interactive_js = interactive_elements_js(
                IframeTraversalConfig {
                    max_iframe_depth: self
                        .iframe_traversal
                        .remaining_same_origin_depth(child_page.depth),
                    ..self.iframe_traversal
                },
                self.paint_order_filtering,
            );
            let Ok(value) = self
                .evaluate_json_for_page(&child_page.page, &child_interactive_js, true)
                .await
            else {
                continue;
            };
            let accessibility = self
                .accessibility_enrichment(&child_page.page)
                .await
                .unwrap_or_default();
            let _ = self
                .evaluate_effect_for_page(&child_page.page, CLEANUP_AX_REFS_JS.to_owned())
                .await;
            let Ok(mut child_state) = dom_state_from_interactive_value(
                &child_page.page.target_id,
                &value,
                &accessibility,
            ) else {
                continue;
            };
            offset_dom_state_bounds(&mut child_state, child_page.offset);
            child_states.push(child_state);
        }

        Ok(merge_dom_states(root_state, child_states))
    }

    async fn accessibility_enrichment(
        &self,
        page: &AttachedPage,
    ) -> Result<BTreeMap<String, AccessibilityNodeInfo>, BrowserError> {
        let snapshot = self
            .connection
            .command(
                "DOMSnapshot.captureSnapshot",
                json!({ "computedStyles": [] }),
                Some(&page.session_id),
            )
            .await?;
        let backend_by_ref = snapshot_backend_ids_by_ax_ref(&snapshot);
        if backend_by_ref.is_empty() {
            return Ok(BTreeMap::new());
        }
        let backend_node_ids = backend_by_ref.values().copied().collect::<Vec<_>>();
        let node_ids_by_backend = self
            .node_ids_by_backend_ids(page, &backend_node_ids)
            .await
            .unwrap_or_default();

        let ax_by_backend = self
            .connection
            .command(
                "Accessibility.getFullAXTree",
                json!({}),
                Some(&page.session_id),
            )
            .await
            .map(|tree| accessibility_nodes_by_backend_id(&tree))
            .unwrap_or_default();

        Ok(backend_by_ref
            .into_iter()
            .map(|(ax_ref, backend_node_id)| {
                let mut info = ax_by_backend
                    .get(&backend_node_id)
                    .cloned()
                    .unwrap_or_default();
                info.backend_node_id = backend_node_id;
                info.node_id = node_ids_by_backend.get(&backend_node_id).copied();
                (ax_ref, info)
            })
            .collect())
    }

    pub(crate) async fn frame_element_infos(
        &self,
        page: &AttachedPage,
    ) -> Result<Vec<FrameElementInfo>, BrowserError> {
        let value = self
            .evaluate_json_for_page(page, FRAME_ELEMENTS_JS, false)
            .await?;
        frame_element_infos_from_value(&value)
    }

    pub(crate) async fn iframe_target_pages(
        &self,
        page: &AttachedPage,
        frame_infos: &[FrameElementInfo],
    ) -> Result<Vec<AttachedFramePage>, BrowserError> {
        if !self.iframe_traversal.allows_cross_origin_depth(1) {
            return Ok(Vec::new());
        }
        let targets = self
            .connection
            .command("Target.getTargets", json!({}), None)
            .await?;
        let target_infos = iframe_target_infos_from_targets(
            &targets,
            &page.target_id,
            frame_infos,
            self.iframe_traversal,
        );
        let mut pages = Vec::new();

        for target_info in target_infos {
            match attach_to_target(&self.connection, target_info.target_id).await {
                Ok(page) => pages.push(AttachedFramePage {
                    page,
                    offset: target_info.offset,
                    depth: target_info.depth,
                }),
                Err(error) if is_missing_target_error(&error) => {}
                Err(error) => return Err(error),
            }
        }

        Ok(pages)
    }

    pub(crate) async fn page_text_for_page(
        &self,
        page: &AttachedPage,
    ) -> Result<String, BrowserError> {
        let value = self
            .evaluate_json_for_page(
                page,
                "(document.body ? document.body.innerText : document.documentElement.innerText || '')",
                false,
            )
            .await?;
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| BrowserError::MissingResponseData("page text".to_owned()))
    }

    pub(crate) async fn find_elements_for_page(
        &self,
        page: &AttachedPage,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        let selector_json = serde_json::to_string(selector)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let attributes_json = serde_json::to_string(attributes)
            .map_err(|error| BrowserError::Transport(error.to_string()))?;
        let value = self
            .evaluate_json_for_page(
                page,
                &format!(
                    r#"
JSON.stringify((() => {{
  const selector = {selector_json};
  const attributeNames = {attributes_json};
  return Array.from(document.querySelectorAll(selector)).slice(0, {max_results}).map((el) => {{
    const attrs = {{}};
    for (const name of attributeNames) {{
      const value = el.getAttribute(name);
      if (value !== null && value !== '') attrs[name] = value;
    }}
    return {{
      tag_name: el.tagName.toLowerCase(),
      text: {text_expr},
      attributes: attrs
    }};
  }});
}})())
"#,
                    selector_json = selector_json,
                    attributes_json = attributes_json,
                    max_results = max_results,
                    text_expr = if include_text {
                        "(el.innerText || el.value || '').trim().slice(0, 500)"
                    } else {
                        "null"
                    }
                ),
                false,
            )
            .await?;
        let encoded = value.as_str().ok_or_else(|| {
            BrowserError::MissingResponseData("find elements result string".to_owned())
        })?;
        serde_json::from_str(encoded).map_err(|error| BrowserError::Transport(error.to_string()))
    }

    async fn node_ids_by_backend_ids(
        &self,
        page: &AttachedPage,
        backend_node_ids: &[u64],
    ) -> Result<BTreeMap<u64, u64>, BrowserError> {
        if backend_node_ids.is_empty() {
            return Ok(BTreeMap::new());
        }

        let _ = self
            .connection
            .command(
                "DOM.getDocument",
                json!({ "depth": -1, "pierce": true }),
                Some(&page.session_id),
            )
            .await;
        let result = self
            .connection
            .command(
                "DOM.pushNodesByBackendIdsToFrontend",
                json!({ "backendNodeIds": backend_node_ids }),
                Some(&page.session_id),
            )
            .await?;
        let node_ids = result
            .get("nodeIds")
            .and_then(Value::as_array)
            .ok_or_else(|| BrowserError::MissingResponseData("DOM nodeIds".to_owned()))?;

        Ok(backend_node_ids
            .iter()
            .zip(node_ids)
            .filter_map(|(backend_node_id, node_id)| {
                let node_id = node_id.as_u64()?;
                (node_id != 0).then_some((*backend_node_id, node_id))
            })
            .collect())
    }
}
