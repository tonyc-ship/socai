use anyhow::Result;
use serde_json::Value;
use socai_agent::{configured_default_model_for, load_api_key, AgentEvent, Provider};
use socai_runtime::{
    create_llm_provider, ensure_llm_provider_configured, run_agent_task as run_agent_with_tools,
    AgentRunConfig, BrowserStatus, BrowserTargetInfo, RuntimePageSession, SocaiRuntime,
};
use socai_sites::xhs::{
    extract_note_command, search_notes_command, topic_scan_command, xhs_agent_instructions,
    xhs_agent_tools, XHS_HOME_URL,
};
use tauri::{AppHandle, Emitter, State};

const TAURI_AGENT_PREAMBLE: &str = "You are running inside the Socai desktop app.";

// ── CDP connect tests (existing) ───────────────────────────────────────────

#[tauri::command]
pub async fn cdp_connect(runtime: State<'_, SocaiRuntime>) -> Result<(), String> {
    runtime.connect_browser();
    Ok(())
}

#[tauri::command]
pub async fn cdp_disconnect(runtime: State<'_, SocaiRuntime>) -> Result<(), String> {
    // Close the shared XHS site page (if any) BEFORE tearing down the WS,
    // so Chrome doesn't keep a stale "automated software" indicator on
    // the tab we opened. Best-effort: if the page is already gone, ignore.
    let _ = runtime.close_site_session("xhs").await;
    runtime.disconnect_browser().await;
    Ok(())
}

#[tauri::command]
pub async fn cdp_status(runtime: State<'_, SocaiRuntime>) -> Result<BrowserStatus, String> {
    Ok(runtime.browser_status().await)
}

#[tauri::command]
pub async fn cdp_list_pages(
    runtime: State<'_, SocaiRuntime>,
) -> Result<Vec<BrowserTargetInfo>, String> {
    Ok(runtime.browser_pages().await)
}

#[tauri::command]
pub async fn cdp_refresh(_runtime: State<'_, SocaiRuntime>) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub async fn cdp_test_search(
    runtime: State<'_, SocaiRuntime>,
    query: String,
) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("query is empty".into());
    }
    let encoded = url_encode_query(query);
    let url = format!("https://www.google.com/search?q={encoded}");
    let page = runtime
        .create_page(&url)
        .await
        .map_err(|e| format!("create_page failed: {e}"))?;
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

// ── Tool-call tests ─────────────────────────────────────────────────────────
//
// Unlike the CLI daemon, the desktop app manages its own CDP lifecycle via
// the explicit cdp_connect button; tool / agent commands assume the
// connection is already up and refuse to run otherwise. (No `tool_stop`
// here — cdp_disconnect handles tab cleanup.)

#[tauri::command]
pub async fn tool_search_notes(
    runtime: State<'_, SocaiRuntime>,
    query: String,
) -> Result<Value, String> {
    require_connected(&runtime).await?;
    search_notes_command(xhs_page(&runtime).await?, &query)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn tool_topic_scan(
    runtime: State<'_, SocaiRuntime>,
    query: String,
) -> Result<Value, String> {
    require_connected(&runtime).await?;
    topic_scan_command(xhs_page(&runtime).await?, &query, "standard", None)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn tool_extract_note(
    runtime: State<'_, SocaiRuntime>,
    note_id: String,
) -> Result<Value, String> {
    require_connected(&runtime).await?;
    extract_note_command(xhs_page(&runtime).await?, &note_id)
        .await
        .map_err(|e| format!("{e:#}"))
}

async fn require_connected(runtime: &SocaiRuntime) -> Result<(), String> {
    match runtime.browser_status().await {
        BrowserStatus::Connected { .. } => Ok(()),
        _ => Err("chrome not connected — click connect first".into()),
    }
}

// ── Agent run (TUI parity) ─────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
struct AgentEventPayload {
    kind: &'static str,
    text: String,
}

#[derive(serde::Serialize, Clone)]
pub struct AgentRunOutcome {
    run_id: String,
    run_dir: String,
    turns: u32,
    final_text: String,
    input_tokens: u64,
    output_tokens: u64,
}

#[tauri::command]
pub async fn agent_save_api_key(provider: String, api_key: String) -> Result<(), String> {
    let provider_enum = Provider::from_name(provider.trim())
        .ok_or_else(|| format!("unknown provider: {provider}"))?;
    socai_agent::save_api_key(provider_enum, api_key.trim())
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn agent_list_models() -> Result<Vec<Value>, String> {
    use socai_agent::PROVIDERS;
    let mut out = Vec::new();
    for cfg in PROVIDERS {
        out.push(serde_json::json!({
            "provider": cfg.provider.as_str(),
            "display_name": cfg.display_name,
            "default_model": configured_default_model_for(cfg.provider),
            "has_key": load_api_key(cfg.provider).is_some(),
        }));
    }
    Ok(out)
}

#[tauri::command]
pub async fn agent_run(
    app: AppHandle,
    runtime: State<'_, SocaiRuntime>,
    task: String,
    model: Option<String>,
) -> Result<AgentRunOutcome, String> {
    require_connected(&runtime).await?;
    let result = run_agent_task(app, runtime.inner().clone(), &task, model.as_deref()).await;
    result.map_err(|e| format!("{e:#}"))
}

async fn run_agent_task(
    app: AppHandle,
    runtime: SocaiRuntime,
    task: &str,
    model: Option<&str>,
) -> Result<AgentRunOutcome> {
    let task = task.trim();
    if task.is_empty() {
        anyhow::bail!("task is empty");
    }

    ensure_llm_provider_configured(model)?;
    let llm_provider = create_llm_provider(model)?;
    let tools = xhs_agent_tools(
        runtime.ensure_site_page("xhs", XHS_HOME_URL).await?,
        llm_provider.clone(),
    )
    .await?;

    let (tx, rx) = tokio::sync::broadcast::channel::<AgentEvent>(256);
    let pump = pump_agent_events(app, rx);

    let config = AgentRunConfig {
        extra_instructions: xhs_agent_instructions(TAURI_AGENT_PREAMBLE),
        enabled_sites: vec!["xhs".to_string()],
        ..AgentRunConfig::default()
    };
    let outcome = run_agent_with_tools(task, llm_provider, tools, config, tx).await;
    let _ = pump.await;
    let outcome = outcome?;

    Ok(AgentRunOutcome {
        run_id: outcome.run_id,
        run_dir: outcome.run_dir.display().to_string(),
        turns: outcome.turns,
        final_text: outcome.final_text,
        input_tokens: outcome.total_input_tokens,
        output_tokens: outcome.total_output_tokens,
    })
}

async fn xhs_page(runtime: &SocaiRuntime) -> Result<std::sync::Arc<RuntimePageSession>, String> {
    runtime
        .ensure_site_page("xhs", XHS_HOME_URL)
        .await
        .map_err(|e| format!("{e:#}"))
}

fn pump_agent_events(
    app: AppHandle,
    mut rx: tokio::sync::broadcast::Receiver<AgentEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            let _ = app.emit("agent:event", event_to_payload(&event));
        }
    })
}

fn event_to_payload(event: &AgentEvent) -> AgentEventPayload {
    match event {
        AgentEvent::Started {
            run_id,
            task,
            model,
        } => AgentEventPayload {
            kind: "started",
            text: format!("task: {task}\nrun {run_id} · model {model}"),
        },
        AgentEvent::Turn { turn } => AgentEventPayload {
            kind: "turn",
            text: format!("turn {turn}"),
        },
        AgentEvent::AssistantText { text, .. } => AgentEventPayload {
            kind: "assistant",
            text: text.clone(),
        },
        AgentEvent::Reasoning { text, .. } => AgentEventPayload {
            kind: "reasoning",
            text: text.clone(),
        },
        AgentEvent::ToolCall {
            name,
            input,
            repeat_count,
            ..
        } => {
            let preview = serde_json::to_string(input).unwrap_or_else(|_| input.to_string());
            let text = if *repeat_count > 1 {
                format!("{name}({preview}) repeat={repeat_count}")
            } else {
                format!("{name}({preview})")
            };
            AgentEventPayload {
                kind: "tool_call",
                text,
            }
        }
        AgentEvent::ToolResult {
            name,
            summary,
            duration_ms,
            error,
            ..
        } => {
            let first = summary.lines().next().unwrap_or("");
            let text = match error {
                Some(err) => format!("{name} ({duration_ms}ms): {err}"),
                None => format!("{name} ({duration_ms}ms): {first}"),
            };
            AgentEventPayload {
                kind: if error.is_some() {
                    "tool_error"
                } else {
                    "tool_result"
                },
                text,
            }
        }
        AgentEvent::ApiError { turn, message } => AgentEventPayload {
            kind: "api_error",
            text: format!("turn {turn}: {message}"),
        },
        AgentEvent::Done { turns, .. } => AgentEventPayload {
            kind: "done",
            text: format!("done in {turns} turns"),
        },
    }
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
