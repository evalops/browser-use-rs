//! Chrome DevTools Protocol browser-session layer.

use async_trait::async_trait;
use browser_use_dom::BrowserStateSummary;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("browser is not connected")]
    NotConnected,
    #[error("navigation failed: {0}")]
    NavigationFailed(String),
    #[error("action failed: {0}")]
    ActionFailed(String),
    #[error("browser state unavailable: {0}")]
    StateUnavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub base64_png: String,
}

#[async_trait]
pub trait BrowserSession: Send + Sync {
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError>;

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError>;

    async fn click(&self, index: u32) -> Result<(), BrowserError>;

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError>;

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError>;

    async fn screenshot(&self) -> Result<Screenshot, BrowserError>;
}
