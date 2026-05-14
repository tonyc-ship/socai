use socai_browser::{Cdp, StatusPayload, TargetInfo, TaskSessionManager};
use tauri::State;

#[tauri::command]
pub async fn cdp_connect(cdp: State<'_, Cdp>) -> Result<(), String> {
    cdp.connect();
    Ok(())
}

#[tauri::command]
pub async fn cdp_disconnect(cdp: State<'_, Cdp>) -> Result<(), String> {
    cdp.disconnect().await;
    Ok(())
}

#[tauri::command]
pub async fn cdp_status(cdp: State<'_, Cdp>) -> Result<StatusPayload, String> {
    Ok(cdp.status().await)
}

#[tauri::command]
pub async fn cdp_list_pages(cdp: State<'_, Cdp>) -> Result<Vec<TargetInfo>, String> {
    Ok(cdp.pages().await)
}

/// Kept for parity with the existing frontend (the v0 "search" button is a
/// connection smoke test, not real functionality). Replaces the prior
/// chromiumoxide-element-driven flow with a plain pre-encoded URL navigate —
/// no Input primitives required at the crates/browser layer yet.
#[tauri::command]
pub async fn cdp_refresh(_cdp: State<'_, Cdp>) -> Result<(), String> {
    // No-op: target changes are now delivered as BrowserEvent::TargetsChanged
    // via the event bridge. The previous explicit refresh polled
    // Target.getTargets; we no longer need that round-trip.
    Ok(())
}

#[tauri::command]
pub async fn cdp_test_search(cdp: State<'_, Cdp>, query: String) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("query is empty".into());
    }
    let encoded = url_encode_query(query);
    let url = format!("https://www.google.com/search?q={encoded}");

    let tasks = TaskSessionManager::new((*cdp).clone());
    let page = tasks
        .create_task(&url)
        .await
        .map_err(|e| format!("create_task failed: {e}"))?;

    let info = page
        .page_info()
        .await
        .map_err(|e| format!("page_info failed: {e}"))?;
    let final_url = info
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(format!("opened results for \"{query}\" — {final_url}"))
}

fn url_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
