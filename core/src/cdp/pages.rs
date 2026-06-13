use crate::cdp::connection::Cdp;
use crate::cdp::endpoint;
use crate::cdp::session::PageSession;

/// Thin page factory over one remote-debugging endpoint. Higher-level runtime
/// code decides whether a page belongs to a tool session, an agent run, or a
/// debug command.
pub struct PageSessionManager {
    cdp: Cdp,
}

impl PageSessionManager {
    pub fn new(cdp: Cdp) -> Self {
        Self { cdp }
    }

    /// Open a new socai-owned tab navigated to `start_url` and connect directly
    /// to that page target websocket. This deliberately avoids browser-wide CDP
    /// target discovery/auto-attach, so unrelated user tabs are not
    /// instrumented.
    pub async fn create_page(&self, start_url: &str) -> anyhow::Result<PageSession> {
        let endpoint = self.cdp.endpoint().await?;
        let target = endpoint::create_debug_page(&endpoint, start_url).await?;
        let target_id = target.target_id.clone();
        let Some(ws) = target.web_socket_debugger_url else {
            let _ = endpoint::close_debug_target(&endpoint, &target_id).await;
            anyhow::bail!("created target missing webSocketDebuggerUrl: {target_id}");
        };
        let page = match PageSession::connect(target_id.clone(), &ws, self.cdp.clone()).await {
            Ok(page) => page,
            Err(err) => {
                let _ = endpoint::close_debug_target(&endpoint, &target_id).await;
                return Err(err);
            }
        };
        self.cdp.register_owned_target(target_id).await;
        Ok(page)
    }

    /// Close a page target by target id. This is stronger than consuming a
    /// `PageSession`: cancellation paths may only have a task snapshot and an
    /// id, or the page may still be held by tool `Arc`s.
    pub async fn close_target(&self, target_id: &str) -> anyhow::Result<bool> {
        let target_id = target_id.trim();
        if target_id.is_empty() {
            return Ok(false);
        }
        let endpoint = self.cdp.endpoint().await?;
        let result = endpoint::close_debug_target(&endpoint, target_id).await;
        self.cdp.unregister_owned_target(target_id).await;
        result
    }
}
