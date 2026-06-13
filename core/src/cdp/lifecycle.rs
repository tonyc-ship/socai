use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::cdp::connection::{
    page_list_from, BrowserEvent, Cdp, CdpState, StatusPayload, TargetInfo,
};
use crate::cdp::endpoint::{self, DebugTarget, Endpoint};
use crate::cdp::raw_client::RawCdpClient;

const MAX_ATTEMPTS: u8 = 3;
const ATTEMPT_DELAY: Duration = Duration::from_millis(500);
const TARGET_POLL_INTERVAL: Duration = Duration::from_secs(2);
const TARGET_POLL_FAILURES: u8 = 3;

#[derive(Clone)]
enum TargetPoller {
    Http(Endpoint),
    BrowserWs(RawCdpClient),
}

struct ConnectInventory {
    targets: HashMap<String, TargetInfo>,
    browser_version: String,
    browser_client: Option<RawCdpClient>,
    poller: TargetPoller,
}

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
        // Close socai-owned page targets before dropping endpoint state. Page
        // sessions may use independent target websockets or a browser websocket
        // session, so browser-status disconnect must explicitly tear them down.
        let endpoint = self.endpoint().await.ok();
        let browser_client = self.browser_client().await;
        for target_id in self.take_owned_targets().await {
            if let Some(client) = browser_client.as_ref() {
                let _ = close_target_via_browser_ws(client, &target_id).await;
            } else if let Some(endpoint) = endpoint.as_ref() {
                let _ = endpoint::close_debug_target(endpoint, &target_id).await;
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

    let inventory = connect_inventory(&endpoint).await?;
    let monitor_task = spawn_target_poll_loop(cdp.clone(), inventory.poller.clone());

    {
        let state = cdp.state();
        let mut guard = state.lock().await;
        if !guard.is_connecting() {
            monitor_task.abort();
            return Err(anyhow::anyhow!("connect cancelled"));
        }
        *guard = CdpState::Connected {
            endpoint,
            browser_client: inventory.browser_client,
            browser_version: inventory.browser_version,
            targets: inventory.targets,
            monitor_task,
        };
        let payload: StatusPayload = (&*guard).into();
        cdp.emit(BrowserEvent::StatusChanged(payload));
    }

    // Initial targets emit so subscribers can hydrate. Future updates come from
    // lightweight polling: HTTP `/json/list` where available, otherwise raw
    // `Target.getTargets` over the browser websocket. Neither path enables
    // Target discovery nor attaches to user-owned tabs.
    let initial_pages = cdp.pages().await;
    cdp.emit(BrowserEvent::TargetsChanged(initial_pages));

    Ok(())
}

async fn connect_inventory(endpoint: &Endpoint) -> anyhow::Result<ConnectInventory> {
    if let Ok(debug_targets) = endpoint::list_debug_targets(endpoint).await {
        return Ok(ConnectInventory {
            targets: targets_map(debug_targets),
            browser_version: browser_version_label(endpoint),
            browser_client: None,
            poller: TargetPoller::Http(endpoint.clone()),
        });
    }

    let client = RawCdpClient::connect(&endpoint.browser_ws_url).await?;
    let targets = browser_ws_targets(&client).await?;
    let browser_version = browser_ws_version(&client)
        .await
        .unwrap_or_else(|_| browser_version_label(endpoint));
    Ok(ConnectInventory {
        targets,
        browser_version,
        browser_client: Some(client.clone()),
        poller: TargetPoller::BrowserWs(client),
    })
}

async fn on_connection_lost(cdp: Cdp, reason: String) {
    let _ = cdp.take_owned_targets().await;
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

fn spawn_target_poll_loop(cdp: Cdp, poller: TargetPoller) -> tokio::task::AbortHandle {
    let join = tokio::spawn(async move {
        let mut failures = 0u8;
        loop {
            tokio::time::sleep(TARGET_POLL_INTERVAL).await;
            match poll_targets(&poller).await {
                Ok(targets) => {
                    failures = 0;
                    if let Some(pages) = replace_targets(&cdp, targets).await {
                        cdp.emit(BrowserEvent::TargetsChanged(pages));
                    }
                }
                Err(err) => {
                    failures = failures.saturating_add(1);
                    debug!(failures, error = %err, "target poll failed");
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

async fn poll_targets(poller: &TargetPoller) -> anyhow::Result<HashMap<String, TargetInfo>> {
    match poller {
        TargetPoller::Http(endpoint) => {
            let debug_targets = endpoint::list_debug_targets(endpoint).await?;
            Ok(targets_map(debug_targets))
        }
        TargetPoller::BrowserWs(client) => browser_ws_targets(client).await,
    }
}

/// Replace cached targets. Returns the new visible page list when it changed;
/// `None` means either no visible page change or the connection is inactive.
async fn replace_targets(
    cdp: &Cdp,
    next_targets: HashMap<String, TargetInfo>,
) -> Option<Vec<TargetInfo>> {
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

async fn browser_ws_targets(client: &RawCdpClient) -> anyhow::Result<HashMap<String, TargetInfo>> {
    let value = client.execute("Target.getTargets", json!({})).await?;
    let infos = value
        .get("targetInfos")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Target.getTargets missing targetInfos"))?;
    let mut targets = HashMap::new();
    for info in infos {
        let target = target_info_from_protocol(info);
        if !target.target_id.is_empty() {
            targets.insert(target.target_id.clone(), target);
        }
    }
    Ok(targets)
}

fn target_info_from_protocol(info: &Value) -> TargetInfo {
    TargetInfo {
        target_id: info
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        r#type: info
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: info
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        url: info
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

async fn browser_ws_version(client: &RawCdpClient) -> anyhow::Result<String> {
    let value = client.execute("Browser.getVersion", json!({})).await?;
    let product = value
        .get("product")
        .and_then(Value::as_str)
        .unwrap_or("unknown browser");
    let revision = value
        .get("revision")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if revision.is_empty() {
        Ok(product.to_string())
    } else {
        Ok(format!("{product} v{revision}"))
    }
}

async fn close_target_via_browser_ws(client: &RawCdpClient, target_id: &str) -> anyhow::Result<()> {
    client
        .execute("Target.closeTarget", json!({ "targetId": target_id }))
        .await?;
    Ok(())
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
