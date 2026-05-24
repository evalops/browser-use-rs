//! DOM and accessibility-state serialization contracts.
//!
//! The agent never receives a live browser DOM directly. Instead, the CDP
//! crate distills the current page into the structs in this crate: tab
//! metadata, page metrics, indexed interactive elements, and compact text
//! renderings that can be placed in an LLM prompt. The same structs are also
//! used by history replay to rematch old element indexes against a fresh DOM.
//!
//! ```mermaid
//! flowchart LR
//!     CDP["CDP DOM / AX / runtime snapshots"] --> Ref["DomElementRef selector_map"]
//!     Ref --> Text["llm_representation"]
//!     Ref --> Eval["DomEvalNode eval tree"]
//!     Ref --> Fingerprint["DomInteractedElement fingerprints"]
//!     Text --> Prompt["Agent prompt"]
//!     Fingerprint --> Replay["history replay rematch"]
//!     Eval --> Judge["judge/evaluation prompt"]
//! ```

mod eval;
mod interacted;
mod render;
mod state;
mod types;

pub use interacted::{
    DomInteractedElement, DomInteractedElementMatch, DomInteractedElementMatchFailure,
    DomInteractedElementMatchFailureReason, DomInteractedElementMatchLevel,
    rematch_interacted_element,
};
pub use render::{
    render_element_attributes, render_element_attributes_with_attributes, render_element_line,
    render_element_line_with_attributes, render_element_text,
};
pub use state::SerializedDomState;
pub use types::{
    BackendNodeId, BrowserStateSummary, DomElementRef, DomEvalNode, DomEvalNodeType, DomPageStats,
    EMPTY_DOM_TREE_MESSAGE, ElementBounds, NetworkRequest, NodeId, PageInfo, PaginationButton,
    PaginationButtonType, TabInfo, TargetId,
};

#[cfg(test)]
mod tests;
