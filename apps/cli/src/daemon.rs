use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use socai_agent::{make_run_dir, ToolContext};
use socai_browser::PageSession;
use socai_runtime::{BrowserStatus, SocaiRuntime};
use socai_sites::xhs::{xhs_tools, XhsSiteRuntime, XHS_HOME_URL};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, Notify};
use tokio::time::{sleep, timeout, Instant};

pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(600);
pub const LONG_COMMAND_TIMEOUT: Duration = Duration::from_secs(1_200);

const SOCKET_NAME: &str = "rust-daemon.sock";
const PID_NAME: &str = "rust-daemon.pid";
const LOG_NAME: &str = "rust-daemon.log";
const IDLE_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Serialize, Deserialize)]
struct DaemonRequest {
    id: String,
    command: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct DaemonResponse {
    id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

struct DaemonPaths {
    home: PathBuf,
    socket: PathBuf,
    pid: PathBuf,
    log: PathBuf,
}

struct DaemonState {
    runtime: SocaiRuntime,
    page: Option<Arc<PageSession>>,
    last_activity: Instant,
}

pub async fn run_daemon() -> Result<()> {
    let paths = daemon_paths()?;
    fs::create_dir_all(&paths.home).await?;
    if paths.socket.exists() {
        fs::remove_file(&paths.socket)
            .await
            .with_context(|| format!("remove stale socket {}", paths.socket.display()))?;
    }

    let listener = UnixListener::bind(&paths.socket)
        .with_context(|| format!("bind daemon socket {}", paths.socket.display()))?;
    fs::write(&paths.pid, std::process::id().to_string()).await?;

    let state = Arc::new(Mutex::new(DaemonState {
        runtime: SocaiRuntime::new(),
        page: None,
        last_activity: Instant::now(),
    }));
    let stop = Arc::new(Notify::new());
    let mut idle_check = tokio::time::interval(Duration::from_secs(60));

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.context("accept daemon client")?;
                let state = state.clone();
                let stop = stop.clone();
                tokio::spawn(async move {
                    if let Err(err) = serve_client(stream, state, stop).await {
                        eprintln!("daemon client error: {err:#}");
                    }
                });
            }
            _ = idle_check.tick() => {
                if state.lock().await.last_activity.elapsed() > IDLE_TIMEOUT {
                    break;
                }
            }
            _ = stop.notified() => break,
        }
    }

    state.lock().await.shutdown().await?;
    let _ = fs::remove_file(&paths.socket).await;
    let _ = fs::remove_file(&paths.pid).await;
    Ok(())
}

pub async fn send_or_spawn(command: &str, args: Value, command_timeout: Duration) -> Result<Value> {
    match send_request(command, args.clone(), command_timeout).await {
        Ok(result) => Ok(result),
        Err(_) => {
            spawn_daemon().await?;
            send_request(command, args, command_timeout).await
        }
    }
}

pub async fn stop_daemon() -> Result<bool> {
    match send_request("shutdown", json!({}), Duration::from_secs(10)).await {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

async fn serve_client(
    stream: UnixStream,
    state: Arc<Mutex<DaemonState>>,
    stop: Arc<Notify>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    while reader.read_line(&mut line).await? != 0 {
        let request: DaemonRequest = serde_json::from_str(line.trim_end())?;
        let response = handle_request(request, state.clone(), stop.clone()).await;
        writer
            .write_all(serde_json::to_string(&response)?.as_bytes())
            .await?;
        writer.write_all(b"\n").await?;
        line.clear();
    }

    Ok(())
}

async fn handle_request(
    request: DaemonRequest,
    state: Arc<Mutex<DaemonState>>,
    stop: Arc<Notify>,
) -> DaemonResponse {
    let id = request.id.clone();
    let result = async {
        if request.command == "ping" {
            return Ok(json!({ "ok": true }));
        }

        if request.command == "shutdown" {
            stop.notify_waiters();
            return Ok(json!({ "ok": true }));
        }

        let mut state = state.lock().await;
        state.last_activity = Instant::now();
        match request.command.as_str() {
            "search_notes" => state.search_notes(request.args).await,
            "topic_scan" => state.topic_scan(request.args).await,
            "extract_note" => state.extract_note(request.args).await,
            other => Err(anyhow!("unknown daemon command: {other}")),
        }
    }
    .await;

    match result {
        Ok(result) => DaemonResponse {
            id,
            ok: true,
            result: Some(result),
            error: None,
        },
        Err(err) => DaemonResponse {
            id,
            ok: false,
            result: None,
            error: Some(format!("{err:#}")),
        },
    }
}

impl DaemonState {
    async fn search_notes(&mut self, args: Value) -> Result<Value> {
        let query = required_string(&args, "query")?;
        let page = self.ensure_page().await?;
        ensure_search_ready(&page).await?;
        let (run_dir, ctx) = command_context("search_notes")?;
        let data = call_xhs_tool(
            page,
            "search_notes",
            json!({ "query": query, "wait_seconds": 2.0 }),
            &ctx,
        )
        .await?;

        Ok(json!({
            "command": "search_notes",
            "run_dir": run_dir,
            "data": data
        }))
    }

    async fn topic_scan(&mut self, args: Value) -> Result<Value> {
        let query = required_string(&args, "query")?;
        let page = self.ensure_page().await?;
        ensure_search_ready(&page).await?;
        let (run_dir, ctx) = command_context("topic_scan")?;
        let mut input = json!({
            "query": query,
            "depth": args.get("depth").and_then(Value::as_str).unwrap_or("standard")
        });
        if let Some(tab_label) = args.get("tab_label").and_then(Value::as_str) {
            input["tab_label"] = Value::String(tab_label.to_string());
        }

        let data = call_xhs_tool(page, "topic_scan", input, &ctx).await?;
        Ok(json!({
            "command": "topic_scan",
            "run_dir": run_dir,
            "data": data
        }))
    }

    async fn extract_note(&mut self, args: Value) -> Result<Value> {
        let note_id = required_string(&args, "note_id")?;
        let page = self.ensure_page().await?;
        close_open_note(&page).await;
        let (run_dir, ctx) = command_context("extract_note")?;
        let data = call_xhs_tool(
            page.clone(),
            "read_note",
            json!({ "note_id": note_id, "wait_seconds": 6.0 }),
            &ctx,
        )
        .await?;
        close_open_note(&page).await;

        Ok(json!({
            "command": "extract_note",
            "run_dir": run_dir,
            "data": data
        }))
    }

    async fn ensure_page(&mut self) -> Result<Arc<PageSession>> {
        if let Some(page) = &self.page {
            if page.page_info().await.is_ok() {
                return Ok(page.clone());
            }
            self.page = None;
        }

        connect_runtime(&self.runtime).await?;
        let page = Arc::new(self.runtime.create_task("about:blank").await?);
        page.navigate_with_timeout(XHS_HOME_URL, 60.0).await?;
        self.page = Some(page.clone());
        Ok(page)
    }

    async fn shutdown(&mut self) -> Result<()> {
        if let Some(page) = self.page.take() {
            if let Ok(page) = Arc::try_unwrap(page) {
                let _ = page.close().await;
            }
        }
        self.runtime.disconnect_browser().await;
        Ok(())
    }
}

async fn connect_runtime(runtime: &SocaiRuntime) -> Result<()> {
    runtime.connect_browser();
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        match runtime.browser_status().await {
            BrowserStatus::Connected { .. } => return Ok(()),
            BrowserStatus::Disconnected { reason } if reason != "not_yet_connected" => {
                return Err(anyhow!("CDP disconnected: {reason}"));
            }
            BrowserStatus::Disconnected { .. } | BrowserStatus::Connecting { .. } => {}
        }
        if Instant::now() >= deadline {
            return Err(anyhow!("CDP did not connect within {:?}", STARTUP_TIMEOUT));
        }
        sleep(Duration::from_millis(250)).await;
    }
}

async fn call_xhs_tool(
    page: Arc<PageSession>,
    tool_name: &str,
    input: Value,
    ctx: &ToolContext,
) -> Result<Value> {
    let tool = xhs_tools(page)
        .into_iter()
        .find(|tool| tool.name() == tool_name)
        .ok_or_else(|| anyhow!("xhs tool not found: {tool_name}"))?;
    let result = tool.call(input, ctx).await?;
    let text = result.flat_text();
    serde_json::from_str(text.trim()).or_else(|_| Ok(json!({ "raw_reply": text })))
}

fn command_context(label: &str) -> Result<(String, ToolContext)> {
    let run_dir = make_run_dir(label);
    let run_id = run_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(label)
        .to_string();
    let ctx = ToolContext::new(run_id, run_dir.clone());
    ctx.enable_site("xhs");
    Ok((run_dir.display().to_string(), ctx))
}

async fn ensure_search_ready(page: &PageSession) -> Result<()> {
    close_open_note(page).await;
    let runtime = XhsSiteRuntime::new(page);
    let state = runtime.detect_state().await.ok();
    let state_name = state
        .as_ref()
        .and_then(|state| state.get("state"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let current_url = runtime.current_url().await.unwrap_or_default();
    if !current_url.contains("xiaohongshu.com") || state_name == "note_detail" {
        page.navigate_with_timeout(XHS_HOME_URL, 60.0).await?;
    }
    Ok(())
}

async fn close_open_note(page: &PageSession) {
    let runtime = XhsSiteRuntime::new(page);
    let state = runtime.detect_state().await.ok();
    let note_open = state
        .as_ref()
        .and_then(|state| state.get("note_open"))
        .and_then(|open| open.get("open"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let state_name = state
        .as_ref()
        .and_then(|state| state.get("state"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    if note_open || state_name == "note_detail" {
        let _ = runtime.close_note(0.8).await;
    }
}

async fn send_request(command: &str, args: Value, request_timeout: Duration) -> Result<Value> {
    let paths = daemon_paths()?;
    let request = DaemonRequest {
        id: request_id(),
        command: command.to_string(),
        args,
    };

    let stream = UnixStream::connect(&paths.socket)
        .await
        .with_context(|| format!("connect daemon socket {}", paths.socket.display()))?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    timeout(request_timeout, async {
        writer
            .write_all(serde_json::to_string(&request)?.as_bytes())
            .await?;
        writer.write_all(b"\n").await?;
        reader.read_line(&mut line).await?;
        if line.trim().is_empty() {
            return Err(anyhow!("empty daemon response"));
        }
        let response: DaemonResponse = serde_json::from_str(line.trim_end())?;
        if !response.ok {
            return Err(anyhow!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "daemon command failed".to_string())
            ));
        }
        response
            .result
            .ok_or_else(|| anyhow!("daemon response missing result"))
    })
    .await
    .map_err(|_| anyhow!("daemon request timed out after {:?}", request_timeout))?
}

async fn spawn_daemon() -> Result<()> {
    let paths = daemon_paths()?;
    fs::create_dir_all(&paths.home).await?;
    if paths.socket.exists() {
        let _ = fs::remove_file(&paths.socket).await;
    }

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
        .with_context(|| format!("open daemon log {}", paths.log.display()))?;
    let stderr = log.try_clone()?;

    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().context("spawn socai rust daemon")?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if send_request("ping", json!({}), Duration::from_secs(2))
            .await
            .is_ok()
        {
            return Ok(());
        }
        sleep(Duration::from_millis(250)).await;
    }

    Err(anyhow!(
        "socai rust daemon did not become ready; see {}",
        paths.log.display()
    ))
}

fn required_string(args: &Value, key: &str) -> Result<String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("missing required argument: {key}"))?;
    Ok(value.to_string())
}

fn daemon_paths() -> Result<DaemonPaths> {
    let home = match std::env::var_os("SOCAI_HOME") {
        Some(path) => PathBuf::from(path),
        None => {
            Path::new(&std::env::var("HOME").context("HOME is not set; cannot locate ~/.socai")?)
                .join(".socai")
        }
    };

    Ok(DaemonPaths {
        socket: home.join(SOCKET_NAME),
        pid: home.join(PID_NAME),
        log: home.join(LOG_NAME),
        home,
    })
}

fn request_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{millis}", std::process::id())
}
