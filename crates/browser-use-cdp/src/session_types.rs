use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use browser_use_dom::{DomElementRef, ElementBounds};
use tempfile::TempDir;

use super::{
    AttachedPage, BrowserError, BrowserProfile, dom_highlight_overlay_elements,
    dom_highlight_overlay_script, interaction_coordinate_highlight_script,
    interaction_element_highlight_script,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IframeTraversalConfig {
    pub(crate) cross_origin_iframes: bool,
    pub(crate) max_iframes: usize,
    pub(crate) max_iframe_depth: usize,
}

impl IframeTraversalConfig {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            cross_origin_iframes: profile.cross_origin_iframes,
            max_iframes: profile.max_iframes,
            max_iframe_depth: profile.max_iframe_depth,
        }
    }

    pub(crate) fn max_iframe_depth_for_same_origin(self) -> usize {
        self.max_iframe_depth
    }

    pub(crate) fn remaining_same_origin_depth(self, current_depth: usize) -> usize {
        self.max_iframe_depth.saturating_sub(current_depth)
    }

    pub(crate) fn allows_cross_origin_depth(self, depth: usize) -> bool {
        self.cross_origin_iframes && depth <= self.max_iframe_depth && self.max_iframes > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageLoadWaitConfig {
    pub(crate) minimum_wait: Duration,
    pub(crate) network_idle_wait: Duration,
}

impl PageLoadWaitConfig {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            minimum_wait: Duration::from_secs_f64(profile.minimum_wait_page_load_time),
            network_idle_wait: Duration::from_secs_f64(
                profile.wait_for_network_idle_page_load_time,
            ),
        }
    }

    pub(crate) fn is_disabled(self) -> bool {
        self.minimum_wait.is_zero() && self.network_idle_wait.is_zero()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InteractionHighlightConfig {
    enabled: bool,
    color: String,
    duration_seconds: f64,
}

impl InteractionHighlightConfig {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            enabled: profile.highlight_elements,
            color: profile.interaction_highlight_color.clone(),
            duration_seconds: profile.interaction_highlight_duration,
        }
    }

    pub(crate) fn element_script(&self, bounds: Option<ElementBounds>) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let bounds = bounds?;
        if bounds.width == 0 || bounds.height == 0 {
            return None;
        }
        Some(interaction_element_highlight_script(
            bounds,
            &self.color,
            self.duration_seconds,
        ))
    }

    pub(crate) fn coordinate_script(&self, x: i32, y: i32) -> Option<String> {
        if !self.enabled {
            return None;
        }
        Some(interaction_coordinate_highlight_script(
            x,
            y,
            &self.color,
            self.duration_seconds,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DomHighlightConfig {
    enabled: bool,
    filter_highlight_ids: bool,
}

impl DomHighlightConfig {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Self {
        Self {
            enabled: profile.dom_highlight_elements,
            filter_highlight_ids: profile.filter_highlight_ids,
        }
    }

    pub(crate) fn overlay_script(
        &self,
        selector_map: &BTreeMap<u32, DomElementRef>,
    ) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let elements = dom_highlight_overlay_elements(selector_map, self.filter_highlight_ids);
        Some(dom_highlight_overlay_script(&elements))
    }
}

#[derive(Debug)]
pub(crate) struct NetworkActivityState {
    active_request_ids: BTreeSet<String>,
    last_activity_at: Instant,
}

impl NetworkActivityState {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            active_request_ids: BTreeSet::new(),
            last_activity_at: now,
        }
    }

    pub(crate) fn observe_request_started(&mut self, request_id: &str, now: Instant) {
        self.active_request_ids.insert(request_id.to_owned());
        self.last_activity_at = now;
    }

    pub(crate) fn observe_request_finished(&mut self, request_id: &str, now: Instant) {
        self.active_request_ids.remove(request_id);
        self.last_activity_at = now;
    }

    pub(crate) fn idle_remaining(&self, now: Instant, idle_for: Duration) -> Option<Duration> {
        if !self.active_request_ids.is_empty() {
            return Some(idle_for);
        }
        let elapsed = now.saturating_duration_since(self.last_activity_at);
        if elapsed >= idle_for {
            None
        } else {
            Some(idle_for - elapsed)
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FrameOffset {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameElementInfo {
    pub(crate) url: String,
    pub(crate) offset: FrameOffset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IframeTargetInfo {
    pub(crate) target_id: String,
    pub(crate) offset: FrameOffset,
    pub(crate) depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachedFramePage {
    pub(crate) page: AttachedPage,
    pub(crate) offset: FrameOffset,
    pub(crate) depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CachedDomElementRef {
    pub(crate) element: DomElementRef,
    pub(crate) target_local_index: u32,
}

pub(crate) struct SessionDownloads {
    pub(crate) path: Option<PathBuf>,
    pub(crate) temp_dir: Option<TempDir>,
}

impl SessionDownloads {
    pub(crate) fn from_profile(profile: &BrowserProfile) -> Result<Self, BrowserError> {
        if !profile.accept_downloads {
            return Ok(Self {
                path: None,
                temp_dir: None,
            });
        }
        if let Some(downloads_path) = &profile.downloads_path {
            return Ok(Self {
                path: Some(downloads_path.clone()),
                temp_dir: None,
            });
        }
        let temp_dir = tempfile::Builder::new()
            .prefix("browser-use-downloads-")
            .tempdir()
            .map_err(|error| BrowserError::StateUnavailable(error.to_string()))?;
        Ok(Self {
            path: Some(temp_dir.path().to_path_buf()),
            temp_dir: Some(temp_dir),
        })
    }
}
