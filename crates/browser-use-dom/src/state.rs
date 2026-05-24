use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::eval::serialize_eval_tree;
use crate::{
    DomElementRef, DomEvalNode, DomPageStats, EMPTY_DOM_TREE_MESSAGE, render_element_line,
    render_element_line_with_attributes,
};

/// Serialized DOM state in the form the agent consumes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SerializedDomState {
    /// Pre-rendered indexed text shown to the model.
    #[serde(default)]
    pub text: String,
    /// Map from browser-use element index to the element reference.
    #[serde(default)]
    pub selector_map: BTreeMap<u32, DomElementRef>,
    /// Optional aggregate page-shape counters.
    #[serde(default, skip_serializing_if = "DomPageStats::is_empty")]
    pub page_stats: DomPageStats,
    /// Optional tree representation used by evaluation/judge flows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_root: Option<DomEvalNode>,
}

impl SerializedDomState {
    /// Builds a state from indexed elements and renders the default prompt text.
    #[must_use]
    pub fn from_elements(elements: Vec<DomElementRef>) -> Self {
        let mut selector_map = BTreeMap::new();
        let mut lines = Vec::new();

        for element in elements {
            lines.push(render_element_line(&element));
            selector_map.insert(element.index, element);
        }

        Self {
            text: lines.join("\n"),
            selector_map,
            page_stats: DomPageStats::default(),
            eval_root: None,
        }
    }

    /// Attaches page statistics to the state.
    #[must_use]
    pub fn with_page_stats(mut self, page_stats: DomPageStats) -> Self {
        self.page_stats = page_stats;
        self
    }

    /// Attaches an evaluation tree root to the state.
    #[must_use]
    pub fn with_eval_root(mut self, eval_root: DomEvalNode) -> Self {
        self.eval_root = Some(eval_root);
        self
    }

    /// Returns the number of indexed elements available for actions.
    #[must_use]
    pub fn element_count(&self) -> usize {
        self.selector_map.len()
    }

    /// Returns prompt text for the LLM, falling back to the empty-DOM message.
    #[must_use]
    pub fn llm_representation(&self) -> &str {
        if self.text.is_empty() && self.selector_map.is_empty() {
            return EMPTY_DOM_TREE_MESSAGE;
        }
        self.text.as_str()
    }

    /// Renders prompt text with an explicit attribute allow-list.
    #[must_use]
    pub fn llm_representation_with_attributes(&self, include_attributes: &[String]) -> String {
        if self.selector_map.is_empty() {
            return EMPTY_DOM_TREE_MESSAGE.to_owned();
        }
        if include_attributes.is_empty() {
            return self.llm_representation().to_owned();
        }

        self.selector_map
            .values()
            .map(|element| render_element_line_with_attributes(element, include_attributes))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Renders the evaluation tree with the default attribute policy.
    #[must_use]
    pub fn eval_representation(&self) -> String {
        self.eval_representation_with_attributes(&[])
    }

    /// Renders the evaluation tree with caller-provided attributes.
    ///
    /// The current serializer keeps an upstream-compatible fixed attribute set,
    /// so this argument is accepted for API symmetry but is not yet used.
    #[must_use]
    pub fn eval_representation_with_attributes(&self, _include_attributes: &[String]) -> String {
        self.eval_root
            .as_ref()
            .map(|root| serialize_eval_tree(root, 0))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| EMPTY_DOM_TREE_MESSAGE.to_owned())
    }
}
