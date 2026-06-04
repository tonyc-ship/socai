use crate::tracking::Telemetry;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use socai_core::runtime::SocaiRuntime;
use socai_core::sites::xhs::{
    extract_note_command, search_notes_command, topic_scan_command, XHS_HOME_URL,
};
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
    telemetry: Telemetry,
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

    let runtime = SocaiRuntime::new();
    let telemetry = Telemetry::new(&paths.home);
    let state = Arc::new(Mutex::new(DaemonState {
        runtime,
        telemetry,
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
    let command = request.command.clone();
    let args_for_tracking = request.args.clone();
    let result = async {
        if command == "ping" {
            return Ok(json!({ "ok": true }));
        }

        if command == "shutdown" {
            stop.notify_waiters();
            return Ok(json!({ "ok": true }));
        }

        let mut state = state.lock().await;
        state.last_activity = Instant::now();
        state.track_command_started(&id, &command, &args_for_tracking);
        if let Some(query) = args_for_tracking.get("query").and_then(Value::as_str) {
            state.track_search_intent(&id, &command, query, &args_for_tracking);
        }

        let started = Instant::now();
        let result = match command.as_str() {
            "search_notes" => state.search_notes(&id, request.args).await,
            "topic_scan" => state.topic_scan(&id, request.args).await,
            "extract_note" => state.extract_note(&id, request.args).await,
            other => Err(anyhow!("unknown daemon command: {other}")),
        };
        state.track_command_finished(&id, &command, &args_for_tracking, started, &result);
        result
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
    async fn search_notes(&mut self, request_id: &str, args: Value) -> Result<Value> {
        let query = required_string(&args, "query")?;
        let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
        self.track_tool_call(request_id, "search_notes", "search_notes", &args);
        let started = Instant::now();
        let result = search_notes_command(page, &query).await;
        self.track_tool_result(
            request_id,
            "search_notes",
            "search_notes",
            &args,
            started,
            &result,
        );
        result
    }

    async fn topic_scan(&mut self, request_id: &str, args: Value) -> Result<Value> {
        let query = required_string(&args, "query")?;
        let depth = args
            .get("depth")
            .and_then(Value::as_str)
            .unwrap_or("standard");
        let tab_label = args.get("tab_label").and_then(Value::as_str);
        let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
        self.track_tool_call(request_id, "topic_scan", "topic_scan", &args);
        let started = Instant::now();
        let result = topic_scan_command(page, &query, depth, tab_label).await;
        self.track_tool_result(
            request_id,
            "topic_scan",
            "topic_scan",
            &args,
            started,
            &result,
        );
        result
    }

    async fn extract_note(&mut self, request_id: &str, args: Value) -> Result<Value> {
        let note_id = required_string(&args, "note_id")?;
        let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
        self.track_tool_call(request_id, "extract_note", "read_note", &args);
        let started = Instant::now();
        let result = extract_note_command(page, &note_id).await;
        self.track_tool_result(
            request_id,
            "extract_note",
            "read_note",
            &args,
            started,
            &result,
        );
        result
    }

    fn track_command_started(&self, request_id: &str, command: &str, args: &Value) {
        self.telemetry.capture(
            "socai_daemon_command_started",
            props_with_summary(
                &self.telemetry,
                json!({
                    "request_id": request_id,
                    "command": command,
                    "telemetry_enabled": self.telemetry.enabled(),
                    "remote_enabled": self.telemetry.remote_enabled(),
                }),
                args,
            ),
        );
    }

    fn track_search_intent(&self, request_id: &str, command: &str, query: &str, args: &Value) {
        let mut props = base_event_props(request_id, command);
        props.insert("site".into(), json!("xhs"));
        insert_query_props(&mut props, &self.telemetry, query);
        insert_optional_str_prop(&mut props, args, "depth");
        insert_optional_str_prop(&mut props, args, "tab_label");
        self.telemetry
            .capture("socai_daemon_search_intent", Value::Object(props));
    }

    fn track_command_finished(
        &self,
        request_id: &str,
        command: &str,
        args: &Value,
        started: Instant,
        result: &Result<Value>,
    ) {
        let duration_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(value) => {
                let mut props = base_event_props(request_id, command);
                props.insert("duration_ms".into(), json!(duration_ms));
                props.insert("ok".into(), json!(true));
                merge_object(&mut props, command_arg_summary(&self.telemetry, args));
                merge_object(&mut props, result_metrics(value));
                self.telemetry
                    .capture("socai_daemon_command_completed", Value::Object(props));
            }
            Err(err) => {
                let mut props = base_event_props(request_id, command);
                props.insert("duration_ms".into(), json!(duration_ms));
                props.insert("ok".into(), json!(false));
                props.insert("error".into(), json!(error_summary(err)));
                merge_object(&mut props, command_arg_summary(&self.telemetry, args));
                self.telemetry
                    .capture("socai_daemon_command_failed", Value::Object(props));
            }
        }
    }

    fn track_tool_call(&self, request_id: &str, command: &str, tool_name: &str, input: &Value) {
        let mut props = base_event_props(request_id, command);
        props.insert("site".into(), json!("xhs"));
        props.insert("tool_name".into(), json!(tool_name));
        merge_object(&mut props, command_arg_summary(&self.telemetry, input));
        self.telemetry
            .capture("socai_daemon_tool_call", Value::Object(props));
    }

    fn track_tool_result(
        &self,
        request_id: &str,
        command: &str,
        tool_name: &str,
        input: &Value,
        started: Instant,
        result: &Result<Value>,
    ) {
        let duration_ms = started.elapsed().as_millis() as u64;
        let mut props = base_event_props(request_id, command);
        props.insert("site".into(), json!("xhs"));
        props.insert("tool_name".into(), json!(tool_name));
        props.insert("duration_ms".into(), json!(duration_ms));
        merge_object(&mut props, command_arg_summary(&self.telemetry, input));
        match result {
            Ok(value) => {
                props.insert("ok".into(), json!(true));
                merge_object(&mut props, result_metrics(value));
                self.telemetry
                    .capture("socai_daemon_tool_result", Value::Object(props));
            }
            Err(err) => {
                props.insert("ok".into(), json!(false));
                props.insert("error".into(), json!(error_summary(err)));
                self.telemetry
                    .capture("socai_daemon_tool_result", Value::Object(props));
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        let _ = self.runtime.close_site_session("xhs").await;
        self.runtime.disconnect_browser().await;
        Ok(())
    }
}

fn base_event_props(request_id: &str, command: &str) -> Map<String, Value> {
    let mut props = Map::new();
    props.insert("request_id".into(), json!(request_id));
    props.insert("command".into(), json!(command));
    props
}

fn props_with_summary(telemetry: &Telemetry, mut props: Value, args: &Value) -> Value {
    let summary = command_arg_summary(telemetry, args);
    let Some(props_map) = props.as_object_mut() else {
        return summary;
    };
    merge_object(props_map, summary);
    props
}

fn command_arg_summary(telemetry: &Telemetry, args: &Value) -> Value {
    let mut props = Map::new();
    if let Some(query) = args.get("query").and_then(Value::as_str) {
        insert_query_props(&mut props, telemetry, query);
    }
    insert_optional_str_prop(&mut props, args, "depth");
    insert_optional_str_prop(&mut props, args, "tab_label");
    if args.get("note_id").and_then(Value::as_str).is_some() {
        props.insert("note_id_present".into(), json!(true));
    }
    if let Some(object) = args.as_object() {
        props.insert(
            "arg_keys".into(),
            Value::Array(object.keys().map(|key| json!(key)).collect()),
        );
    }
    Value::Object(props)
}

fn insert_query_props(props: &mut Map<String, Value>, telemetry: &Telemetry, query: &str) {
    props.insert("query_len".into(), json!(query.chars().count()));
    if telemetry.include_query_text() {
        props.insert("query_text".into(), json!(query));
    } else {
        props.insert("query_redacted".into(), json!(true));
    }
}

fn insert_optional_str_prop(props: &mut Map<String, Value>, args: &Value, key: &str) {
    let Some(value) = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    props.insert(key.to_string(), json!(value));
}

fn merge_object(target: &mut Map<String, Value>, value: Value) {
    let Value::Object(map) = value else {
        return;
    };
    for (key, value) in map {
        target.insert(key, value);
    }
}

fn result_metrics(value: &Value) -> Value {
    let mut props = Map::new();
    let data = value.get("data").unwrap_or(value);
    if let Some(ok) = data.get("ok").and_then(Value::as_bool) {
        props.insert("result_ok".into(), json!(ok));
    }
    if let Some(cards) = data.get("cards").and_then(Value::as_array) {
        props.insert("cards_count".into(), json!(cards.len()));
    }
    if let Some(cards) = data
        .get("search")
        .and_then(|search| search.get("cards"))
        .and_then(Value::as_array)
    {
        props.insert("search_cards_count".into(), json!(cards.len()));
    }
    if let Some(cards) = data.get("selected_cards").and_then(Value::as_array) {
        props.insert("selected_cards_count".into(), json!(cards.len()));
    }
    if let Some(notes) = data.get("notes").and_then(Value::as_array) {
        props.insert("notes_count".into(), json!(notes.len()));
        let skipped = notes
            .iter()
            .filter(|note| note.get("skipped").is_some())
            .count();
        props.insert("notes_skipped_count".into(), json!(skipped));
    }
    if value.get("run_dir").is_some() {
        props.insert("has_run_dir".into(), json!(true));
    }
    Value::Object(props)
}

fn error_summary(err: &anyhow::Error) -> String {
    let rendered = format!("{err:#}");
    let first = rendered.lines().next().unwrap_or("command failed").trim();
    first.chars().take(240).collect()
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
