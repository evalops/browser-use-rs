use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use browser_use_dom::BrowserStateSummary;

use super::{
    BrowserError, BrowserLifecycleAdapterEventSubscription, BrowserLifecycleEventSubscription,
    FoundElement, Pdf, Screenshot,
};

#[async_trait]
/// Provider-neutral browser-control interface used by the core executor.
///
/// The trait lets tests and future browser backends satisfy the same contract
/// as [`crate::CdpBrowserSession`]. Each method is intentionally action-shaped: the
/// core executor should not need to know whether a backend uses CDP,
/// Playwright, WebDriver, or a mock session.
pub trait BrowserSession: Send + Sync {
    /// Subscribes to fine-grained lifecycle events, or returns a closed stream by default.
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        BrowserLifecycleEventSubscription::closed()
    }

    /// Subscribes to adapter lifecycle events derived from the fine-grained stream.
    fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        BrowserLifecycleAdapterEventSubscription::new(self.subscribe_lifecycle_events())
    }

    /// Captures current browser state for the agent.
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError>;

    /// Navigates to a URL in the current tab or a new tab.
    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError>;

    /// Goes back in the active tab history.
    async fn go_back(&self) -> Result<(), BrowserError>;

    /// Switches focus to a tab by short tab id or full target id.
    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError>;

    /// Closes a tab by short tab id or full target id.
    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError>;

    /// Clicks an indexed DOM element.
    async fn click(&self, index: u32) -> Result<(), BrowserError>;

    /// Clicks explicit viewport coordinates.
    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError>;

    /// Inputs text into an indexed element.
    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError>;

    /// Scrolls the page or an indexed scrollable element.
    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError>;

    /// Finds text on the active page.
    async fn find_text(&self, text: &str) -> Result<bool, BrowserError>;

    /// Evaluates JavaScript on the active page and returns serialized output.
    async fn evaluate(&self, code: &str) -> Result<String, BrowserError>;

    /// Returns options for an indexed dropdown.
    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError>;

    /// Selects an option in an indexed dropdown by visible text.
    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError>;

    /// Returns visible page text used by search/extract actions.
    async fn page_text(&self) -> Result<String, BrowserError>;

    /// Finds elements by CSS selector with optional attributes and text.
    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError>;

    /// Sends keyboard input to the active page.
    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError>;

    /// Uploads a file through an indexed file input element.
    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError>;

    /// Captures a PNG screenshot.
    async fn screenshot(&self) -> Result<Screenshot, BrowserError>;

    /// Prints the active page to PDF.
    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError>;
}

#[async_trait]
impl<T> BrowserSession for Arc<T>
where
    T: BrowserSession + ?Sized,
{
    fn subscribe_lifecycle_events(&self) -> BrowserLifecycleEventSubscription {
        self.as_ref().subscribe_lifecycle_events()
    }

    fn subscribe_lifecycle_adapter_events(&self) -> BrowserLifecycleAdapterEventSubscription {
        self.as_ref().subscribe_lifecycle_adapter_events()
    }

    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError> {
        self.as_ref().state(include_screenshot).await
    }

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError> {
        self.as_ref().navigate(url, new_tab).await
    }

    async fn go_back(&self) -> Result<(), BrowserError> {
        self.as_ref().go_back().await
    }

    async fn switch_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        self.as_ref().switch_tab(target_id).await
    }

    async fn close_tab(&self, target_id: &str) -> Result<(), BrowserError> {
        self.as_ref().close_tab(target_id).await
    }

    async fn click(&self, index: u32) -> Result<(), BrowserError> {
        self.as_ref().click(index).await
    }

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError> {
        self.as_ref().click_coordinates(x, y).await
    }

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError> {
        self.as_ref().input_text(index, text, clear).await
    }

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError> {
        self.as_ref().scroll(index, down, pages).await
    }

    async fn find_text(&self, text: &str) -> Result<bool, BrowserError> {
        self.as_ref().find_text(text).await
    }

    async fn evaluate(&self, code: &str) -> Result<String, BrowserError> {
        self.as_ref().evaluate(code).await
    }

    async fn dropdown_options(&self, index: u32) -> Result<Vec<String>, BrowserError> {
        self.as_ref().dropdown_options(index).await
    }

    async fn select_dropdown_option(&self, index: u32, text: &str) -> Result<(), BrowserError> {
        self.as_ref().select_dropdown_option(index, text).await
    }

    async fn page_text(&self) -> Result<String, BrowserError> {
        self.as_ref().page_text().await
    }

    async fn find_elements(
        &self,
        selector: &str,
        attributes: &[String],
        max_results: usize,
        include_text: bool,
    ) -> Result<Vec<FoundElement>, BrowserError> {
        self.as_ref()
            .find_elements(selector, attributes, max_results, include_text)
            .await
    }

    async fn send_keys(&self, keys: &str) -> Result<(), BrowserError> {
        self.as_ref().send_keys(keys).await
    }

    async fn upload_file(&self, index: u32, path: &Path) -> Result<(), BrowserError> {
        self.as_ref().upload_file(index, path).await
    }

    async fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        self.as_ref().screenshot().await
    }

    async fn save_pdf(
        &self,
        print_background: bool,
        landscape: bool,
        scale: f64,
        paper_format: &str,
    ) -> Result<Pdf, BrowserError> {
        self.as_ref()
            .save_pdf(print_background, landscape, scale, paper_format)
            .await
    }
}
