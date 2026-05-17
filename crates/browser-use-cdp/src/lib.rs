//! Chrome DevTools Protocol browser-session layer.

use std::path::PathBuf;

use async_trait::async_trait;
use browser_use_dom::BrowserStateSummary;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserViewport {
    pub width: u32,
    pub height: u32,
}

impl Default for BrowserViewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxySettings {
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    #[serde(default = "default_headless")]
    pub headless: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub prohibited_domains: Vec<String>,
    #[serde(default)]
    pub viewport: BrowserViewport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxySettings>,
}

impl Default for BrowserProfile {
    fn default() -> Self {
        Self {
            cdp_url: None,
            executable_path: None,
            headless: default_headless(),
            user_data_dir: None,
            args: Vec::new(),
            allowed_domains: Vec::new(),
            prohibited_domains: Vec::new(),
            viewport: BrowserViewport::default(),
            proxy: None,
        }
    }
}

fn default_headless() -> bool {
    true
}

impl BrowserProfile {
    #[must_use]
    pub fn launch_plan(&self) -> BrowserLaunchPlan {
        let mut args = vec![
            "--remote-debugging-port=0".to_owned(),
            "--no-first-run".to_owned(),
            "--no-default-browser-check".to_owned(),
            format!(
                "--window-size={},{}",
                self.viewport.width, self.viewport.height
            ),
        ];

        if self.headless {
            args.push("--headless=new".to_owned());
        }

        if let Some(user_data_dir) = &self.user_data_dir {
            args.push(format!("--user-data-dir={}", user_data_dir.display()));
        }

        if let Some(proxy) = &self.proxy {
            args.push(format!("--proxy-server={}", proxy.server));
        }

        args.extend(self.args.iter().cloned());

        BrowserLaunchPlan {
            executable_path: self.executable_path.clone(),
            args,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserLaunchPlan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<PathBuf>,
    pub args: Vec<String>,
}

#[async_trait]
pub trait BrowserSession: Send + Sync {
    async fn state(&self, include_screenshot: bool) -> Result<BrowserStateSummary, BrowserError>;

    async fn navigate(&self, url: &str, new_tab: bool) -> Result<(), BrowserError>;

    async fn click(&self, index: u32) -> Result<(), BrowserError>;

    async fn click_coordinates(&self, x: i32, y: i32) -> Result<(), BrowserError>;

    async fn input_text(&self, index: u32, text: &str, clear: bool) -> Result<(), BrowserError>;

    async fn scroll(&self, index: Option<u32>, down: bool, pages: f64) -> Result<(), BrowserError>;

    async fn screenshot(&self) -> Result<Screenshot, BrowserError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_uses_headless_chrome_args() {
        let plan = BrowserProfile::default().launch_plan();

        assert!(plan.args.contains(&"--headless=new".to_owned()));
        assert!(plan.args.contains(&"--remote-debugging-port=0".to_owned()));
        assert!(plan.args.contains(&"--window-size=1280,720".to_owned()));
    }

    #[test]
    fn launch_plan_preserves_profile_and_custom_args_order() {
        let profile = BrowserProfile {
            headless: false,
            user_data_dir: Some(PathBuf::from("/tmp/browser-use-rs-profile")),
            args: vec!["--disable-gpu".to_owned()],
            proxy: Some(ProxySettings {
                server: "http://127.0.0.1:8080".to_owned(),
                username: None,
                password: None,
            }),
            ..BrowserProfile::default()
        };

        let plan = profile.launch_plan();

        assert!(!plan.args.contains(&"--headless=new".to_owned()));
        assert!(
            plan.args
                .contains(&"--user-data-dir=/tmp/browser-use-rs-profile".to_owned())
        );
        assert!(
            plan.args
                .contains(&"--proxy-server=http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(plan.args.last(), Some(&"--disable-gpu".to_owned()));
    }
}
