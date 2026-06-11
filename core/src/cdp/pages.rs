use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chromiumoxide::cdp::browser_protocol::target::CloseTargetParams;
use chromiumoxide::Browser;

use crate::cdp::connection::{Cdp, CdpState};
use crate::cdp::session::PageSession;

const PAGE_CREATE_TIMEOUT: Duration = Duration::from_secs(300);

/// Thin page factory over one CDP connection. Higher-level runtime code can
/// decide whether a page belongs to a tool session, an agent run, or a debug
/// command.
pub struct PageSessionManager {
    cdp: Cdp,
}

impl PageSessionManager {
    pub fn new(cdp: Cdp) -> Self {
        Self { cdp }
    }

    /// Open a new tab navigated to `start_url`. Errors if the CDP connection
    /// is not in `Connected` state.
    pub async fn create_page(&self, start_url: &str) -> anyhow::Result<PageSession> {
        let browser = self.browser().await?;
        let page = tokio::time::timeout(PAGE_CREATE_TIMEOUT, browser.new_page(start_url))
            .await
            .context("create browser page timed out")?
            .context("create browser page")?;
        Ok(PageSession::new(page))
    }

    /// Close a page target by target id. This is stronger than consuming a
    /// `PageSession`: cancellation paths may only have a task snapshot and an
    /// id, or the page may still be held by tool `Arc`s.
    pub async fn close_target(&self, target_id: &str) -> anyhow::Result<bool> {
        let target_id = target_id.trim();
        if target_id.is_empty() {
            return Ok(false);
        }
        let browser = self.browser().await?;
        browser
            .execute(CloseTargetParams::new(target_id.to_string()))
            .await?;
        Ok(true)
    }

    async fn browser(&self) -> anyhow::Result<Arc<Browser>> {
        let state = self.cdp.state();
        let guard = state.lock().await;
        match &*guard {
            CdpState::Connected { browser, .. } => Ok(Arc::clone(browser)),
            _ => Err(anyhow::anyhow!("CDP not connected")),
        }
    }
}
