use std::collections::HashMap;
use std::time::Duration;

use tracing::{debug, warn};

use crate::cdp::connection::{
    page_list_from, BrowserEvent, Cdp, CdpState, StatusPayload, TargetInfo,
};
use crate::cdp::endpoint::{self, DebugTarget, Endpoint};

const MAX_ATTEMPTS: u8 = 3;
const ATTEMPT_DELAY: Duration = Duration::from_millis(500);
const TARGET_POLL_INTERVAL: Duration = Duration::from_secs(2);
const TARGET_POLL_FAILURES: u8 = 3;

impl Cdp {
    /// Trigger an asynchronous connect attempt. Idempotent: if already
    /// connected or connecting, returns immediately.
    pub fn connect(&self) {
        let cdp = self.clone();
        tokio::spawn(async move {
            run_connect(cdp).await;
        });
    }

    pub async fn disconnect(&self) {
        // Close socai-owned page targets before dropping the passive endpoint
        // state. Page sessions use independent target-scoped websockets, so a
        // browser-status disconnect must explicitly tear them down.
        if let Ok(endpoint) = self.endpoint().await {
            for target_id in self.take_owned_targets().await {
                let _ = endpoint::close_debug_target(&endpoint, &target_id).await;
            }
        }
        transition_unconditional(
            self,
            CdpState::Disconnected {
                reason: "user_disconnected".into(),
            },
        )
        .await;
    }
}

async fn run_connect(cdp: Cdp) {
    {
        let state = cdp.state();
        let guard = state.lock().await;
        if !guard.is_disconnected() {
            return;
        }
    }

    for attempt in 1..=MAX_ATTEMPTS {
        if !transition_if_eligible(&cdp, CdpState::Connecting { attempt }).await {
            return;
        }
        match try_connect_once(&cdp).await {
            Ok(()) => return,
            Err(err) => {
                warn!(attempt, error = %err, "cdp connect attempt failed");
                if attempt == MAX_ATTEMPTS {
                    transition_unconditional(
                        &cdp,
                        CdpState::Disconnected {
                            reason: err.to_string(),
                        },
                    )
                    .await;
                    return;
                }
                tokio::time::sleep(ATTEMPT_DELAY).await;
            }
        }
    }
}

async fn try_connect_once(cdp: &Cdp) -> anyhow::Result<()> {
    let endpoint: Endpoint = endpoint::discover_existing_chrome_endpoint()
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no running chrome with --remote-debugging-port found. \
                 launch chrome with the debug flag, or set SOCAI_CDP_WS."
            )
        })?;

    // Passive target inventory over HTTP. This replaces the previous
    // browser-wide chromiumoxide websocket handler, which discovered and
    // initialized every tab in the user's browser.
    let initial_debug_targets = endpoint::list_debug_targets(&endpoint).await?;
    let targets = targets_map(initial_debug_targets);
    let browser_version = browser_version_label(&endpoint);
    let monitor_task = spawn_target_poll_loop(cdp.clone(), endpoint.clone());

    {
        let state = cdp.state();
        let mut guard = state.lock().await;
        if !guard.is_connecting() {
            monitor_task.abort();
            return Err(anyhow::anyhow!("connect cancelled"));
        }
        *guard = CdpState::Connected {
            endpoint,
            browser_version,
            targets,
            monitor_task,
        };
        let payload: StatusPayload = (&*guard).into();
        cdp.emit(BrowserEvent::StatusChanged(payload));
    }

    // Initial targets emit so subscribers can hydrate. Future updates come from
    // a lightweight `/json/list` poller, not from CDP Target.* events.
    let initial_pages = cdp.pages().await;
    cdp.emit(BrowserEvent::TargetsChanged(initial_pages));

    Ok(())
}

async fn on_connection_lost(cdp: Cdp, reason: String) {
    let state = cdp.state();
    let mut guard = state.lock().await;
    if matches!(*guard, CdpState::Connected { .. }) {
        *guard = CdpState::Disconnected { reason };
        let payload: StatusPayload = (&*guard).into();
        cdp.emit(BrowserEvent::StatusChanged(payload));
        cdp.emit(BrowserEvent::TargetsChanged(Vec::new()));
    }
}

async fn transition_if_eligible(cdp: &Cdp, new: CdpState) -> bool {
    let state = cdp.state();
    let mut guard = state.lock().await;
    let eligible = matches!(
        *guard,
        CdpState::Disconnected { .. } | CdpState::Connecting { .. }
    );
    if !eligible {
        return false;
    }
    *guard = new;
    let payload: StatusPayload = (&*guard).into();
    cdp.emit(BrowserEvent::StatusChanged(payload));
    true
}

async fn transition_unconditional(cdp: &Cdp, new: CdpState) {
    let state = cdp.state();
    let mut guard = state.lock().await;
    let clear_targets = matches!(*guard, CdpState::Connected { .. })
        && matches!(new, CdpState::Disconnected { .. });
    abort_monitor_if_connected(&guard);
    *guard = new;
    let payload: StatusPayload = (&*guard).into();
    cdp.emit(BrowserEvent::StatusChanged(payload));
    if clear_targets {
        cdp.emit(BrowserEvent::TargetsChanged(Vec::new()));
    }
}

fn abort_monitor_if_connected(state: &CdpState) {
    if let CdpState::Connected { monitor_task, .. } = state {
        monitor_task.abort();
    }
}

fn spawn_target_poll_loop(cdp: Cdp, endpoint: Endpoint) -> tokio::task::AbortHandle {
    let join = tokio::spawn(async move {
        let mut failures = 0u8;
        loop {
            tokio::time::sleep(TARGET_POLL_INTERVAL).await;
            match endpoint::list_debug_targets(&endpoint).await {
                Ok(debug_targets) => {
                    failures = 0;
                    if let Some(pages) = replace_targets(&cdp, debug_targets).await {
                        cdp.emit(BrowserEvent::TargetsChanged(pages));
                    }
                }
                Err(err) => {
                    failures = failures.saturating_add(1);
                    debug!(failures, error = %err, "passive target poll failed");
                    if failures >= TARGET_POLL_FAILURES {
                        on_connection_lost(cdp.clone(), format!("connection_lost: {err}")).await;
                        break;
                    }
                }
            }
        }
    });
    join.abort_handle()
}

/// Replace cached targets from passive `/json/list` output. Returns the new
/// visible page list when it changed; `None` means either no visible page
/// change or the connection is no longer active.
async fn replace_targets(cdp: &Cdp, debug_targets: Vec<DebugTarget>) -> Option<Vec<TargetInfo>> {
    let next_targets = targets_map(debug_targets);
    let state = cdp.state();
    let mut guard = state.lock().await;
    let CdpState::Connected { targets, .. } = &mut *guard else {
        return None;
    };
    let before = page_list_from(targets);
    let after = page_list_from(&next_targets);
    *targets = next_targets;
    (before != after).then_some(after)
}

fn targets_map(debug_targets: Vec<DebugTarget>) -> HashMap<String, TargetInfo> {
    debug_targets
        .into_iter()
        .map(|target| {
            let info = target_info_from_debug(target);
            (info.target_id.clone(), info)
        })
        .collect()
}

fn target_info_from_debug(target: DebugTarget) -> TargetInfo {
    TargetInfo {
        target_id: target.target_id,
        r#type: target.r#type,
        title: target.title,
        url: target.url,
    }
}

fn browser_version_label(endpoint: &Endpoint) -> String {
    endpoint
        .version
        .as_ref()
        .and_then(|version| version.browser.clone())
        .or_else(|| {
            endpoint
                .version
                .as_ref()
                .and_then(|version| version.user_agent.clone())
        })
        .unwrap_or_else(|| "unknown browser".into())
}
