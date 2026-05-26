//! Golden fixtures and parity utilities for browser-use-rs.
//!
//! The fixtures in this crate are shared by tests in multiple crates. Keeping
//! them as Rust builders instead of static JSON makes it easier to reuse the
//! same DOM/browser states while still exercising serialization, rendering, and
//! replay behavior through the real public types.

use browser_use_dom::{
    BrowserStateSummary, DomElementRef, DomEvalNode, DomPageStats, ElementBounds, NetworkRequest,
    PageInfo, PaginationButton, PaginationButtonType, SerializedDomState, TabInfo,
};
use std::collections::BTreeMap;

mod fixtures;

pub use fixtures::{
    eval_tree_state, frame_shadow_state, mixed_interactive_state, rich_browser_state_summary,
    simple_interactive_state,
};

#[cfg(test)]
mod tests;
