use chromiumoxide::Browser;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cdp::endpoint::Endpoint;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetInfo {
    pub target_id: String,
    pub r#type: String,
    pub title: String,
    pub url: String,
}

pub enum CdpState {
    Disconnected {
        reason: String,
    },
    Connecting {
        attempt: u8,
    },
    Connected {
        #[allow(dead_code)] // held to tie the WebSocket lifetime to this variant
        browser: Arc<Browser>,
        endpoint: Endpoint,
        browser_version: String,
        targets: HashMap<String, TargetInfo>,
    },
}

impl CdpState {
    pub fn initial() -> Self {
        Self::Disconnected {
            reason: "not_yet_connected".into(),
        }
    }

    pub fn is_disconnected(&self) -> bool {
        matches!(self, Self::Disconnected { .. })
    }

    pub fn is_connecting(&self) -> bool {
        matches!(self, Self::Connecting { .. })
    }
}

pub type SharedState = Arc<Mutex<CdpState>>;

pub fn init_state() -> SharedState {
    Arc::new(Mutex::new(CdpState::initial()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StatusPayload {
    Disconnected {
        reason: String,
    },
    Connecting {
        attempt: u8,
    },
    Connected {
        endpoint: String,
        browser_version: String,
        page_count: usize,
    },
}

impl From<&CdpState> for StatusPayload {
    fn from(state: &CdpState) -> Self {
        match state {
            CdpState::Disconnected { reason } => Self::Disconnected {
                reason: reason.clone(),
            },
            CdpState::Connecting { attempt } => Self::Connecting { attempt: *attempt },
            CdpState::Connected {
                endpoint,
                browser_version,
                targets,
                ..
            } => Self::Connected {
                endpoint: endpoint.browser_ws_url.clone(),
                browser_version: browser_version.clone(),
                page_count: targets.values().filter(|t| t.r#type == "page").count(),
            },
        }
    }
}
