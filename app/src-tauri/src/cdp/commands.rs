use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::cdp::state::{CdpState, SharedState, StatusPayload, TargetInfo};
use crate::cdp::supervisor;

#[tauri::command]
pub async fn cdp_connect(
    state: State<'_, SharedState>,
    app: AppHandle,
) -> Result<(), String> {
    let arc = (*state).clone();
    {
        let guard = arc.lock().await;
        if !guard.is_disconnected() {
            return Ok(());
        }
    }
    tokio::spawn(supervisor::run_connect(arc, app));
    Ok(())
}

#[tauri::command]
pub async fn cdp_disconnect(
    state: State<'_, SharedState>,
    app: AppHandle,
) -> Result<(), String> {
    supervisor::run_disconnect((*state).clone(), app).await;
    Ok(())
}

#[tauri::command]
pub async fn cdp_status(state: State<'_, SharedState>) -> Result<StatusPayload, String> {
    Ok((&*state.lock().await).into())
}

#[tauri::command]
pub async fn cdp_refresh(
    state: State<'_, SharedState>,
    app: AppHandle,
) -> Result<(), String> {
    let browser_arc = {
        let guard = state.lock().await;
        match &*guard {
            CdpState::Connected { browser, .. } => Arc::clone(browser),
            _ => return Ok(()),
        }
    };

    let result = match browser_arc.execute(GetTargetsParams::default()).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[cdp refresh] connection check failed: {e:?}");
            supervisor::on_connection_dropped((*state).clone(), app).await;
            return Ok(());
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
    let CdpState::Connected { targets, .. } = &mut *guard else {
        return Ok(());
    };
    if *targets == fresh {
        return Ok(());
    }
    *targets = fresh.clone();
    let mut page_list: Vec<TargetInfo> = fresh
        .values()
        .filter(|t| t.r#type == "page")
        .cloned()
        .collect();
    page_list.sort_by(|a, b| a.target_id.cmp(&b.target_id));
    drop(guard);
    let _ = app.emit("cdp:targets_changed", page_list);
    Ok(())
}

#[tauri::command]
pub async fn cdp_test_search(
    state: State<'_, SharedState>,
    query: String,
) -> Result<String, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err("query is empty".into());
    }

    let browser_arc = {
        let guard = state.lock().await;
        match &*guard {
            CdpState::Connected { browser, .. } => Arc::clone(browser),
            _ => return Err("not connected".into()),
        }
    };

    let page = browser_arc
        .new_page("https://www.google.com")
        .await
        .map_err(|e| format!("new_page failed: {e}"))?;

    page.wait_for_navigation()
        .await
        .map_err(|e| format!("wait_for_navigation failed: {e}"))?;

    let _ = page.bring_to_front().await;

    let search = page
        .find_element("textarea[name=q], input[name=q]")
        .await
        .map_err(|e| format!("could not find search box: {e}"))?;

    search
        .click()
        .await
        .map_err(|e| format!("click failed: {e}"))?
        .type_str(&query)
        .await
        .map_err(|e| format!("type failed: {e}"))?
        .press_key("Enter")
        .await
        .map_err(|e| format!("press enter failed: {e}"))?;

    page.wait_for_navigation()
        .await
        .map_err(|e| format!("wait for results failed: {e}"))?;

    let url = page.url().await.ok().flatten().unwrap_or_default();
    Ok(format!("opened results for \"{query}\" — {url}"))
}

#[tauri::command]
pub async fn cdp_list_pages(
    state: State<'_, SharedState>,
) -> Result<Vec<TargetInfo>, String> {
    let guard = state.lock().await;
    match &*guard {
        CdpState::Connected { targets, .. } => {
            let mut pages: Vec<TargetInfo> = targets
                .values()
                .filter(|t| t.r#type == "page")
                .cloned()
                .collect();
            pages.sort_by(|a, b| a.target_id.cmp(&b.target_id));
            Ok(pages)
        }
        _ => Err("not connected".into()),
    }
}
