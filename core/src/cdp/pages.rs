use std::sync::Arc;

use chromiumoxide::Browser;

use crate::cdp::connection::{Cdp, CdpState};
use crate::cdp::session::PageSession;

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
        let page = browser.new_page(start_url).await?;
        Ok(PageSession::new(page))
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
