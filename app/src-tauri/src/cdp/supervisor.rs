use chromiumoxide::cdp::browser_protocol::target::{GetTargetsParams, SetDiscoverTargetsParams};
use chromiumoxide::Browser;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::cdp::endpoint;
use crate::cdp::events;
use crate::cdp::state::{CdpState, SharedState, StatusPayload, TargetInfo};

const MAX_ATTEMPTS: u8 = 3;
const ATTEMPT_DELAY: Duration = Duration::from_millis(500);

pub async fn run_connect(state: SharedState, app: AppHandle) {
    for attempt in 1..=MAX_ATTEMPTS {
        if !transition_if_eligible_to_connect(&state, &app, CdpState::Connecting { attempt }).await {
            return;
        }

        match try_connect_once(&app, &state).await {
            Ok(()) => return,
            Err(err) => {
                if attempt == MAX_ATTEMPTS {
                    transition_unconditional(
                        &state,
                        &app,
                        CdpState::Disconnected { reason: err },
                    )
                    .await;
                    return;
                }
                tokio::time::sleep(ATTEMPT_DELAY).await;
            }
        }
    }
}

pub async fn run_disconnect(state: SharedState, app: AppHandle) {
    transition_unconditional(
        &state,
        &app,
        CdpState::Disconnected {
            reason: "user_disconnected".into(),
        },
    )
    .await;
}

async fn try_connect_once(app: &AppHandle, state: &SharedState) -> Result<(), String> {
    let endpoint = endpoint::discover()?;

    let (browser, mut handler) = Browser::connect(&endpoint.browser_ws_url)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;

    let app_for_pump = app.clone();
    let state_for_pump = state.clone();
    tokio::spawn(async move {
        // The handler stream yields Result<_, CdpError>. Individual errors
        // (decode failures for unmodeled events, etc.) are NOT fatal — the
        // connection is alive as long as the stream itself keeps producing.
        // Only stream exhaustion (None) means the WebSocket actually closed.
        while let Some(event) = handler.next().await {
            if let Err(e) = event {
                eprintln!("[cdp pump] non-fatal: {e:?}");
            }
        }
        eprintln!("[cdp pump] stream ended — connection lost");
        on_connection_dropped(state_for_pump, app_for_pump).await;
    });

    let version = browser
        .version()
        .await
        .map_err(|e| format!("version query failed: {e}"))?;
    let browser_version = format!("{} v{}", version.product, version.revision);

    browser
        .execute(SetDiscoverTargetsParams {
            discover: true,
            filter: None,
        })
        .await
        .map_err(|e| format!("setDiscoverTargets failed: {e}"))?;

    let raw = browser
        .execute(GetTargetsParams::default())
        .await
        .map_err(|e| format!("getTargets failed: {e}"))?;
    let targets: HashMap<String, TargetInfo> = raw
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

    let browser = Arc::new(browser);

    let mut guard = state.lock().await;
    if !guard.is_connecting() {
        return Err("connect cancelled".into());
    }
    *guard = CdpState::Connected {
        browser: Arc::clone(&browser),
        endpoint,
        browser_version,
        targets,
    };
    let payload: StatusPayload = (&*guard).into();
    drop(guard);
    let _ = app.emit("cdp:status_changed", payload);

    events::spawn_target_poller(browser, state.clone(), app.clone());

    Ok(())
}

pub async fn on_connection_dropped(state: SharedState, app: AppHandle) {
    let mut guard = state.lock().await;
    if matches!(*guard, CdpState::Connected { .. }) {
        eprintln!("[cdp] transitioning Connected → Disconnected (connection_lost)");
        *guard = CdpState::Disconnected {
            reason: "connection_lost".into(),
        };
        let payload: StatusPayload = (&*guard).into();
        drop(guard);
        let _ = app.emit("cdp:status_changed", payload);
    }
}

async fn transition_if_eligible_to_connect(
    state: &SharedState,
    app: &AppHandle,
    new: CdpState,
) -> bool {
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
    drop(guard);
    let _ = app.emit("cdp:status_changed", payload);
    true
}

async fn transition_unconditional(state: &SharedState, app: &AppHandle, new: CdpState) {
    let mut guard = state.lock().await;
    *guard = new;
    let payload: StatusPayload = (&*guard).into();
    drop(guard);
    let _ = app.emit("cdp:status_changed", payload);
}
