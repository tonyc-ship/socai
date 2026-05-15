use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::target::{
    EventTargetCreated, EventTargetDestroyed, EventTargetInfoChanged, GetTargetsParams,
    SetDiscoverTargetsParams,
};
use chromiumoxide::Browser;
use futures::StreamExt;
use tracing::{debug, warn};

use crate::cdp::connection::{BrowserEvent, Cdp, CdpState, StatusPayload, TargetInfo};
use crate::cdp::endpoint::{self, Endpoint};

const MAX_ATTEMPTS: u8 = 3;
const ATTEMPT_DELAY: Duration = Duration::from_millis(500);

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

    let (browser, mut handler) = Browser::connect(&endpoint.browser_ws_url).await?;

    let cdp_for_pump = cdp.clone();
    let pump = tokio::spawn(async move {
        // Handler stream yields Result<_, CdpError>. Individual decode errors
        // are non-fatal — only stream exhaustion (None) means the WS closed.
        while let Some(event) = handler.next().await {
            if let Err(e) = event {
                debug!(error = ?e, "cdp handler non-fatal error");
            }
        }
        on_connection_dropped(cdp_for_pump).await;
    });
    let handler_task = pump.abort_handle();

    let version = browser.version().await?;
    let browser_version = format!("{} v{}", version.product, version.revision);

    browser
        .execute(SetDiscoverTargetsParams {
            discover: true,
            filter: None,
        })
        .await?;

    let initial = browser.execute(GetTargetsParams::default()).await?;
    let targets: HashMap<String, TargetInfo> = initial
        .result
        .target_infos
        .iter()
        .map(target_info_to_pair)
        .collect();

    let browser = Arc::new(browser);

    {
        let state = cdp.state();
        let mut guard = state.lock().await;
        if !guard.is_connecting() {
            return Err(anyhow::anyhow!("connect cancelled"));
        }
        *guard = CdpState::Connected {
            browser: Arc::clone(&browser),
            handler_task,
            endpoint,
            browser_version,
            targets,
        };
        let payload: StatusPayload = (&*guard).into();
        cdp.emit(BrowserEvent::StatusChanged(payload));
    }

    // initial targets emit so subscribers can hydrate
    let initial_pages = cdp.pages().await;
    cdp.emit(BrowserEvent::TargetsChanged(initial_pages));

    spawn_target_event_loop(Arc::clone(&browser), cdp.clone());

    Ok(())
}

async fn on_connection_dropped(cdp: Cdp) {
    let state = cdp.state();
    let mut guard = state.lock().await;
    if matches!(*guard, CdpState::Connected { .. }) {
        *guard = CdpState::Disconnected {
            reason: "connection_lost".into(),
        };
        let payload: StatusPayload = (&*guard).into();
        cdp.emit(BrowserEvent::StatusChanged(payload));
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
    abort_pump_if_connected(&guard);
    *guard = new;
    let payload: StatusPayload = (&*guard).into();
    cdp.emit(BrowserEvent::StatusChanged(payload));
}

/// On user-initiated disconnect we have to terminate the WS pump task — its
/// `Handler` owns the WebSocket, so dropping the `Arc<Browser>` alone won't
/// close the socket. Aborting causes the task to be dropped, which drops the
/// `Handler`, which closes the WS — only then does Chrome remove the
/// "controlled by automated software" banner.
fn abort_pump_if_connected(state: &CdpState) {
    if let CdpState::Connected { handler_task, .. } = state {
        handler_task.abort();
    }
}

/// Subscribe to Target.* events from chromiumoxide; fold them into the cached
/// targets map and emit `BrowserEvent::TargetsChanged` whenever the visible
/// page list actually changes. Replaces the previous 100ms `Target.getTargets`
/// polling loop.
fn spawn_target_event_loop(browser: Arc<Browser>, cdp: Cdp) {
    tokio::spawn(async move {
        let (created, destroyed, changed) = match try_join_listeners(&browser).await {
            Ok(streams) => streams,
            Err(e) => {
                warn!(error = %e, "failed to subscribe to target events");
                return;
            }
        };
        let mut created = Box::pin(created);
        let mut destroyed = Box::pin(destroyed);
        let mut changed = Box::pin(changed);

        let mut last_emitted = cdp.pages().await;

        loop {
            let dirty = tokio::select! {
                Some(ev) = created.next() => {
                    apply_target_change(&cdp, |targets| {
                        targets.insert(ev.target_info.target_id.inner().clone(), to_target_info(&ev.target_info));
                    }).await
                }
                Some(ev) = destroyed.next() => {
                    apply_target_change(&cdp, |targets| {
                        targets.remove(ev.target_id.inner().as_str());
                    }).await
                }
                Some(ev) = changed.next() => {
                    apply_target_change(&cdp, |targets| {
                        targets.insert(ev.target_info.target_id.inner().clone(), to_target_info(&ev.target_info));
                    }).await
                }
                else => break,
            };
            if !dirty {
                break;
            }
            let pages = cdp.pages().await;
            if pages != last_emitted {
                last_emitted = pages.clone();
                cdp.emit(BrowserEvent::TargetsChanged(pages));
            }
        }
    });
}

async fn try_join_listeners(
    browser: &Browser,
) -> anyhow::Result<(
    impl futures::Stream<Item = Arc<EventTargetCreated>>,
    impl futures::Stream<Item = Arc<EventTargetDestroyed>>,
    impl futures::Stream<Item = Arc<EventTargetInfoChanged>>,
)> {
    let created = browser.event_listener::<EventTargetCreated>().await?;
    let destroyed = browser.event_listener::<EventTargetDestroyed>().await?;
    let changed = browser.event_listener::<EventTargetInfoChanged>().await?;
    Ok((created, destroyed, changed))
}

/// Apply a closure under the state lock. Returns false if state moved out of
/// Connected (loop should exit).
async fn apply_target_change<F>(cdp: &Cdp, f: F) -> bool
where
    F: FnOnce(&mut HashMap<String, TargetInfo>),
{
    let state = cdp.state();
    let mut guard = state.lock().await;
    match &mut *guard {
        CdpState::Connected { targets, .. } => {
            f(targets);
            true
        }
        _ => false,
    }
}

fn target_info_to_pair(
    info: &chromiumoxide::cdp::browser_protocol::target::TargetInfo,
) -> (String, TargetInfo) {
    let ti = to_target_info(info);
    (ti.target_id.clone(), ti)
}

fn to_target_info(info: &chromiumoxide::cdp::browser_protocol::target::TargetInfo) -> TargetInfo {
    TargetInfo {
        target_id: info.target_id.inner().clone(),
        r#type: info.r#type.clone(),
        title: info.title.clone(),
        url: info.url.clone(),
    }
}
