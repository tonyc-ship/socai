use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
use chromiumoxide::Browser;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::cdp::state::{CdpState, SharedState, TargetInfo};
use crate::cdp::supervisor;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

// v1 strategy: poll Target.getTargets every 100ms while Connected, diff,
// emit cdp:targets_changed when the page list actually changes.
// TODO: replace with chromiumoxide event subscription on Target.targetCreated /
// targetDestroyed / targetInfoChanged once the browser-level event API surface
// is confirmed.
pub fn spawn_target_poller(browser: Arc<Browser>, state: SharedState, app: AppHandle) {
    tokio::spawn(async move {
        let mut last_emitted: Vec<TargetInfo> = current_page_list(&state).await;
        let _ = app.emit("cdp:targets_changed", &last_emitted);

        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut consecutive_failures: u32 = 0;

        loop {
            ticker.tick().await;

            if !state_is_connected(&state).await {
                break;
            }

            let result = match browser.execute(GetTargetsParams::default()).await {
                Ok(r) => {
                    consecutive_failures = 0;
                    r
                }
                Err(e) => {
                    consecutive_failures += 1;
                    eprintln!(
                        "[cdp poll] getTargets failed ({consecutive_failures}/{MAX_CONSECUTIVE_FAILURES}): {e:?}"
                    );
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        eprintln!("[cdp poll] connection appears dead — declaring Disconnected");
                        supervisor::on_connection_dropped(state.clone(), app.clone()).await;
                        break;
                    }
                    continue;
                }
            };

            let fresh: HashMap<String, TargetInfo> = result
                .result
                .target_infos
                .iter()
                .map(|t| {
                    let id = t.target_id.inner().clone();
                    (
                        id.clone(),
                        TargetInfo {
                            target_id: id,
                            r#type: t.r#type.clone(),
                            title: t.title.clone(),
                            url: t.url.clone(),
                        },
                    )
                })
                .collect();

            let mut guard = state.lock().await;
            let pages_changed = if let CdpState::Connected { targets, .. } = &mut *guard {
                let changed = *targets != fresh;
                if changed {
                    *targets = fresh;
                }
                changed
            } else {
                break;
            };

            if !pages_changed {
                continue;
            }

            let page_list = if let CdpState::Connected { targets, .. } = &*guard {
                page_list_from(targets)
            } else {
                break;
            };
            drop(guard);

            if page_list != last_emitted {
                last_emitted = page_list.clone();
                let _ = app.emit("cdp:targets_changed", &page_list);
            }
        }
    });
}

async fn state_is_connected(state: &SharedState) -> bool {
    matches!(*state.lock().await, CdpState::Connected { .. })
}

async fn current_page_list(state: &SharedState) -> Vec<TargetInfo> {
    if let CdpState::Connected { targets, .. } = &*state.lock().await {
        page_list_from(targets)
    } else {
        Vec::new()
    }
}

fn page_list_from(targets: &HashMap<String, TargetInfo>) -> Vec<TargetInfo> {
    let mut pages: Vec<TargetInfo> = targets
        .values()
        .filter(|t| t.r#type == "page")
        .cloned()
        .collect();
    pages.sort_by(|a, b| a.target_id.cmp(&b.target_id));
    pages
}
