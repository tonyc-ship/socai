use crate::tracking::{query_text_enabled, telemetry_enabled, Telemetry};

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
const PING_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const RESTART_SHUTDOWN_WAIT: Duration = Duration::from_secs(5);
const DAEMON_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct DaemonRequest {
    id: String,
    command: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    telemetry: DaemonTelemetry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonTelemetry {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_true")]
    include_query_text: bool,
}

impl Default for DaemonTelemetry {
    fn default() -> Self {
        Self {
            enabled: true,
            include_query_text: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VersionMetadata {
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_sha: Option<String>,
    protocol_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DaemonCompatibility {
    Compatible,
    Incompatible { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExistingDaemonStatus {
    Compatible,
    Missing { reason: String },
    Incompatible { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DaemonRecoveryAction {
    UseExisting,
    SpawnFresh,
    RestartExisting { reason: String },
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

#[derive(Debug, Clone)]
pub(crate) struct DaemonPathInfo {
    pub(crate) home: PathBuf,
    pub(crate) socket: PathBuf,
    pub(crate) pid: PathBuf,
    pub(crate) log: PathBuf,
}

struct DaemonPaths {
    home: PathBuf,
    socket: PathBuf,
    pid: PathBuf,
    log: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct DaemonInspection {
    pub(crate) paths: DaemonPathInfo,
    pub(crate) status: ExistingDaemonStatus,
    pub(crate) ping: Option<Value>,
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
    ensure_compatible_daemon().await?;

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
        let mut disconnect_probe = String::new();
        tokio::select! {
            response = handle_request(request, state.clone(), stop.clone()) => {
                writer
                    .write_all(serde_json::to_string(&response)?.as_bytes())
                    .await?;
                writer.write_all(b"\n").await?;
            }
            read = reader.read_line(&mut disconnect_probe) => {
                if read? == 0 {
                    return Ok(());
                }
                anyhow::bail!("daemon client sent another request before the previous response");
            }
        }
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
    let telemetry = request.telemetry.clone();
    let result = async {
        if command == "ping" {
            return Ok(ping_response(&request.args));
        }

        if command == "shutdown" {
            stop.notify_waiters();
            return Ok(json!({ "ok": true }));
        }

        let mut state = state.lock().await;
        state.last_activity = Instant::now();
        match command.as_str() {
            "search_notes" => state.search_notes(&id, request.args, &telemetry).await,
            "topic_scan" => state.topic_scan(&id, request.args, &telemetry).await,
            "extract_note" => state.extract_note(&id, request.args, &telemetry).await,
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
    async fn search_notes(
        &mut self,
        request_id: &str,
        args: Value,
        telemetry: &DaemonTelemetry,
    ) -> Result<Value> {
        let started = Instant::now();
        let result = async {
            let query = required_string(&args, "query")?;
            let filters = args.get("filters");
            let debug_snapshot = debug_snapshot_flag(&args);
            let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
            search_notes_command(page, &query, filters, debug_snapshot).await
        }
        .await;
        self.track_tool_trace(
            request_id,
            "search_notes",
            "search_notes",
            &args,
            telemetry,
            started,
            &result,
        );
        result
    }

    async fn topic_scan(
        &mut self,
        request_id: &str,
        args: Value,
        telemetry: &DaemonTelemetry,
    ) -> Result<Value> {
        let started = Instant::now();
        let result = async {
            let query = required_string(&args, "query")?;
            let tab_label = args.get("tab_label").and_then(Value::as_str);
            let filters = args.get("filters");
            let num_notes = args.get("num_notes").and_then(Value::as_i64);
            let debug_snapshot = debug_snapshot_flag(&args);
            let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
            topic_scan_command(page, &query, tab_label, filters, num_notes, debug_snapshot).await
        }
        .await;
        self.track_tool_trace(
            request_id,
            "topic_scan",
            "topic_scan",
            &args,
            telemetry,
            started,
            &result,
        );
        result
    }

    async fn extract_note(
        &mut self,
        request_id: &str,
        args: Value,
        telemetry: &DaemonTelemetry,
    ) -> Result<Value> {
        let started = Instant::now();
        let result = async {
            let note_id = required_string(&args, "note_id")?;
            let debug_snapshot = debug_snapshot_flag(&args);
            let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
            extract_note_command(page, &note_id, debug_snapshot).await
        }
        .await;
        self.track_tool_trace(
            request_id,
            "extract_note",
            "read_note",
            &args,
            telemetry,
            started,
            &result,
        );
        result
    }

    fn track_tool_trace(
        &self,
        request_id: &str,
        command: &str,
        tool_name: &str,
        input: &Value,
        telemetry: &DaemonTelemetry,
        started: Instant,
        result: &Result<Value>,
    ) {
        if !telemetry.enabled {
            return;
        }

        let mut props = base_trace_props(request_id, command, tool_name);
        props.insert(
            "duration_ms".into(),
            json!(started.elapsed().as_millis() as u64),
        );
        merge_object(&mut props, command_arg_summary(telemetry, input));
        match result {
            Ok(value) => {
                props.insert("ok".into(), json!(true));
                merge_object(&mut props, result_metrics(value));
            }
            Err(err) => {
                props.insert("ok".into(), json!(false));
                props.insert("error".into(), json!(error_summary(err)));
            }
        }

        self.telemetry
            .capture("socai_cli_tool_trace", Value::Object(props));
    }

    async fn shutdown(&mut self) -> Result<()> {
        let _ = self.runtime.close_site_session("xhs").await;
        self.runtime.disconnect_browser().await;
        Ok(())
    }
}

fn base_trace_props(request_id: &str, command: &str, tool_name: &str) -> Map<String, Value> {
    let mut props = Map::new();
    props.insert("request_id".into(), json!(request_id));
    props.insert("command".into(), json!(command));
    props.insert("tool_name".into(), json!(tool_name));
    props.insert("site".into(), json!("xhs"));
    props
}

fn command_arg_summary(telemetry: &DaemonTelemetry, args: &Value) -> Value {
    let mut props = Map::new();
    if let Some(query) = args.get("query").and_then(Value::as_str) {
        insert_query_props(&mut props, telemetry, query);
    }
    if let Some(metadata) = explicit_param_metadata(args) {
        props.insert("metadata".into(), Value::Object(metadata));
    }
    if args.get("note_id").and_then(Value::as_str).is_some() {
        props.insert("note_id_present".into(), json!(true));
    }
    Value::Object(props)
}

fn explicit_param_metadata(args: &Value) -> Option<Map<String, Value>> {
    let mut metadata = Map::new();
    insert_optional_str_metadata(&mut metadata, args, "tab_label", "tab");
    insert_optional_i64_metadata(&mut metadata, args, "num_notes", "num_notes");
    insert_true_bool_metadata(&mut metadata, args, "debug_snapshot", "debug_snapshot");
    if metadata.is_empty() {
        None
    } else {
        Some(metadata)
    }
}

fn insert_query_props(props: &mut Map<String, Value>, telemetry: &DaemonTelemetry, query: &str) {
    props.insert("query_len".into(), json!(query.chars().count()));
    props.insert(
        "query_text_enabled".into(),
        json!(telemetry.include_query_text),
    );
    if telemetry.include_query_text {
        props.insert("query_text".into(), json!(query));
    }
}

fn insert_optional_str_metadata(
    metadata: &mut Map<String, Value>,
    args: &Value,
    arg_key: &str,
    metadata_key: &str,
) {
    let Some(value) = args
        .get(arg_key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    metadata.insert(metadata_key.to_string(), json!(value));
}

fn insert_optional_i64_metadata(
    metadata: &mut Map<String, Value>,
    args: &Value,
    arg_key: &str,
    metadata_key: &str,
) {
    let Some(value) = args.get(arg_key).and_then(Value::as_i64) else {
        return;
    };
    metadata.insert(metadata_key.to_string(), json!(value));
}

fn insert_true_bool_metadata(
    metadata: &mut Map<String, Value>,
    args: &Value,
    arg_key: &str,
    metadata_key: &str,
) {
    if args.get(arg_key).and_then(Value::as_bool) == Some(true) {
        metadata.insert(metadata_key.to_string(), json!(true));
    }
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

fn current_version_metadata() -> VersionMetadata {
    VersionMetadata {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_sha: compile_time_build_sha(),
        protocol_version: DAEMON_PROTOCOL_VERSION,
    }
}

fn compile_time_build_sha() -> Option<String> {
    option_env!("SOCAI_BUILD_SHA").and_then(normalize_build_sha)
}

fn normalize_build_sha(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("unknown") {
        None
    } else {
        Some(value.to_string())
    }
}

fn ping_args() -> Value {
    json!({ "client": current_version_metadata() })
}

fn ping_response(args: &Value) -> Value {
    let client = args
        .get("client")
        .cloned()
        .unwrap_or_else(|| json!(current_version_metadata()));

    json!({
        "ok": true,
        "protocol_version": DAEMON_PROTOCOL_VERSION,
        "daemon": current_version_metadata(),
        "client": client,
    })
}

fn daemon_compatibility(ping_result: &Value, client: &VersionMetadata) -> DaemonCompatibility {
    let Some(daemon_value) = ping_result.get("daemon") else {
        return DaemonCompatibility::Incompatible {
            reason: "daemon ping response missing daemon version metadata".to_string(),
        };
    };

    let daemon: VersionMetadata = match serde_json::from_value(daemon_value.clone()) {
        Ok(metadata) => metadata,
        Err(err) => {
            return DaemonCompatibility::Incompatible {
                reason: format!("daemon ping response has invalid version metadata: {err}"),
            };
        }
    };

    if daemon.protocol_version != client.protocol_version {
        return DaemonCompatibility::Incompatible {
            reason: format!(
                "daemon protocol {} does not match client protocol {}",
                daemon.protocol_version, client.protocol_version
            ),
        };
    }

    if daemon.version != client.version {
        return DaemonCompatibility::Incompatible {
            reason: format!(
                "daemon version {} does not match client version {}",
                daemon.version, client.version
            ),
        };
    }

    match (&daemon.build_sha, &client.build_sha) {
        (Some(daemon_sha), Some(client_sha)) if daemon_sha == client_sha => {}
        (None, None) => {}
        (Some(_), Some(_)) => {
            return DaemonCompatibility::Incompatible {
                reason: "daemon build SHA does not match client build SHA".to_string(),
            };
        }
        (None, Some(_)) => {
            return DaemonCompatibility::Incompatible {
                reason: "daemon build SHA is missing".to_string(),
            };
        }
        (Some(_), None) => {
            return DaemonCompatibility::Incompatible {
                reason: "client build SHA is missing".to_string(),
            };
        }
    }

    DaemonCompatibility::Compatible
}

fn daemon_recovery_action(status: ExistingDaemonStatus) -> DaemonRecoveryAction {
    match status {
        ExistingDaemonStatus::Compatible => DaemonRecoveryAction::UseExisting,
        ExistingDaemonStatus::Missing { .. } => DaemonRecoveryAction::SpawnFresh,
        ExistingDaemonStatus::Incompatible { reason } => {
            DaemonRecoveryAction::RestartExisting { reason }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_summary_includes_query_text_by_default() {
        let summary = command_arg_summary(
            &DaemonTelemetry::default(),
            &json!({ "query": "运营爆款思路" }),
        );
        let object = summary.as_object().expect("summary is an object");
        assert_eq!(object.get("query_text"), Some(&json!("运营爆款思路")));
        assert_eq!(object.get("query_text_enabled"), Some(&json!(true)));
        assert_eq!(object.get("query_len"), Some(&json!(6)));
    }

    #[test]
    fn command_summary_redacts_query_text_when_disabled() {
        let telemetry = DaemonTelemetry {
            enabled: true,
            include_query_text: false,
        };
        let summary = command_arg_summary(&telemetry, &json!({ "query": "运营爆款思路" }));
        let object = summary.as_object().expect("summary is an object");
        assert!(!object.contains_key("query_text"));
        assert_eq!(object.get("query_text_enabled"), Some(&json!(false)));
        assert_eq!(object.get("query_len"), Some(&json!(6)));
    }

    #[test]
    fn command_summary_omits_defaulted_optional_params() {
        let summary = command_arg_summary(
            &DaemonTelemetry::default(),
            &json!({ "query": "x", "debug_snapshot": false }),
        );
        let object = summary.as_object().expect("summary is an object");
        assert!(!object.contains_key("metadata"));
    }

    #[test]
    fn command_summary_tracks_explicit_optional_params_as_metadata() {
        let summary = command_arg_summary(
            &DaemonTelemetry::default(),
            &json!({
                "query": "x",
                "tab_label": "latest",
                "num_notes": 12,
                "debug_snapshot": true
            }),
        );
        let object = summary.as_object().expect("summary is an object");
        let metadata = object
            .get("metadata")
            .and_then(Value::as_object)
            .expect("metadata object");
        assert_eq!(metadata.get("tab"), Some(&json!("latest")));
        assert_eq!(metadata.get("num_notes"), Some(&json!(12)));
        assert_eq!(metadata.get("debug_snapshot"), Some(&json!(true)));
        assert!(!object.contains_key("tab_label"));
        assert!(!object.contains_key("num_notes"));
    }

    #[test]
    fn result_metrics_extracts_safe_counts() {
        let metrics = result_metrics(&json!({
            "run_dir": "/tmp/socai-run",
            "data": {
                "ok": true,
                "cards": [{}, {}],
                "search": { "cards": [{}, {}, {}] },
                "selected_cards": [{}],
                "notes": [
                    { "id": "1", "body": "must not be copied" },
                    { "skipped": "missing note" },
                    { "skipped": true, "comments": ["must not be copied"] }
                ]
            }
        }));
        let object = metrics.as_object().expect("metrics is an object");
        assert_eq!(object.get("result_ok"), Some(&json!(true)));
        assert_eq!(object.get("cards_count"), Some(&json!(2)));
        assert_eq!(object.get("search_cards_count"), Some(&json!(3)));
        assert_eq!(object.get("selected_cards_count"), Some(&json!(1)));
        assert_eq!(object.get("notes_count"), Some(&json!(3)));
        assert_eq!(object.get("notes_skipped_count"), Some(&json!(2)));
        assert_eq!(object.get("has_run_dir"), Some(&json!(true)));
        assert!(!object.contains_key("body"));
        assert!(!object.contains_key("comments"));
    }

    #[test]
    fn ping_response_includes_daemon_client_and_protocol_metadata() {
        let client = VersionMetadata {
            version: "0.1.0".to_string(),
            build_sha: Some("abc123".to_string()),
            protocol_version: 1,
        };

        let response = ping_response(&json!({ "client": client }));

        assert_eq!(response.get("protocol_version"), Some(&json!(1)));
        assert_eq!(
            response
                .get("daemon")
                .and_then(|daemon| daemon.get("version")),
            Some(&json!(env!("CARGO_PKG_VERSION")))
        );
        assert_eq!(
            response
                .get("daemon")
                .and_then(|daemon| daemon.get("protocol_version")),
            Some(&json!(DAEMON_PROTOCOL_VERSION))
        );
        assert_eq!(
            response
                .get("client")
                .and_then(|client| client.get("version")),
            Some(&json!("0.1.0"))
        );
        assert_eq!(
            response
                .get("client")
                .and_then(|client| client.get("build_sha")),
            Some(&json!("abc123"))
        );
    }

    #[test]
    fn daemon_compatibility_accepts_matching_metadata() {
        let client = VersionMetadata {
            version: "0.1.0".to_string(),
            build_sha: Some("abc123".to_string()),
            protocol_version: 1,
        };
        let ping = json!({
            "daemon": {
                "version": "0.1.0",
                "build_sha": "abc123",
                "protocol_version": 1
            }
        });

        assert_eq!(
            daemon_compatibility(&ping, &client),
            DaemonCompatibility::Compatible
        );
    }

    #[test]
    fn daemon_compatibility_accepts_when_build_sha_unavailable_on_both_sides() {
        let client = VersionMetadata {
            version: "0.1.0".to_string(),
            build_sha: None,
            protocol_version: 1,
        };
        let ping = json!({
            "daemon": {
                "version": "0.1.0",
                "protocol_version": 1
            }
        });

        assert_eq!(
            daemon_compatibility(&ping, &client),
            DaemonCompatibility::Compatible
        );
    }

    #[test]
    fn daemon_compatibility_rejects_missing_daemon_metadata() {
        let client = VersionMetadata {
            version: "0.1.0".to_string(),
            build_sha: Some("abc123".to_string()),
            protocol_version: 1,
        };

        let decision = daemon_compatibility(&json!({ "ok": true }), &client);

        assert!(matches!(
            decision,
            DaemonCompatibility::Incompatible { reason } if reason.contains("missing daemon version metadata")
        ));
    }

    #[test]
    fn daemon_compatibility_rejects_mismatched_version() {
        let client = VersionMetadata {
            version: "0.2.0".to_string(),
            build_sha: Some("abc123".to_string()),
            protocol_version: 1,
        };
        let ping = json!({
            "daemon": {
                "version": "0.1.0",
                "build_sha": "abc123",
                "protocol_version": 1
            }
        });

        let decision = daemon_compatibility(&ping, &client);

        assert!(matches!(
            decision,
            DaemonCompatibility::Incompatible { reason } if reason.contains("daemon version 0.1.0")
        ));
    }

    #[test]
    fn daemon_compatibility_rejects_mismatched_protocol() {
        let client = VersionMetadata {
            version: "0.1.0".to_string(),
            build_sha: Some("abc123".to_string()),
            protocol_version: 2,
        };
        let ping = json!({
            "daemon": {
                "version": "0.1.0",
                "build_sha": "abc123",
                "protocol_version": 1
            }
        });

        let decision = daemon_compatibility(&ping, &client);

        assert!(matches!(
            decision,
            DaemonCompatibility::Incompatible { reason } if reason.contains("daemon protocol 1")
        ));
    }

    #[test]
    fn daemon_compatibility_rejects_mismatched_build_sha() {
        let client = VersionMetadata {
            version: "0.1.0".to_string(),
            build_sha: Some("new-sha".to_string()),
            protocol_version: 1,
        };
        let ping = json!({
            "daemon": {
                "version": "0.1.0",
                "build_sha": "old-sha",
                "protocol_version": 1
            }
        });

        let decision = daemon_compatibility(&ping, &client);

        assert!(matches!(
            decision,
            DaemonCompatibility::Incompatible { reason } if reason.contains("build SHA")
        ));
    }

    #[test]
    fn daemon_compatibility_rejects_missing_build_sha_when_client_has_one() {
        let client = VersionMetadata {
            version: "0.1.0".to_string(),
            build_sha: Some("abc123".to_string()),
            protocol_version: 1,
        };
        let ping = json!({
            "daemon": {
                "version": "0.1.0",
                "protocol_version": 1
            }
        });

        let decision = daemon_compatibility(&ping, &client);

        assert!(matches!(
            decision,
            DaemonCompatibility::Incompatible { reason } if reason.contains("daemon build SHA is missing")
        ));
    }

    #[test]
    fn daemon_recovery_action_spawns_when_daemon_missing() {
        assert_eq!(
            daemon_recovery_action(ExistingDaemonStatus::Missing {
                reason: "socket not found".to_string(),
            }),
            DaemonRecoveryAction::SpawnFresh
        );
    }

    #[test]
    fn daemon_recovery_action_restarts_when_daemon_incompatible() {
        assert_eq!(
            daemon_recovery_action(ExistingDaemonStatus::Incompatible {
                reason: "stale version".to_string(),
            }),
            DaemonRecoveryAction::RestartExisting {
                reason: "stale version".to_string(),
            }
        );
    }
}

async fn ensure_compatible_daemon() -> Result<()> {
    match daemon_recovery_action(probe_existing_daemon().await?) {
        DaemonRecoveryAction::UseExisting => Ok(()),
        DaemonRecoveryAction::SpawnFresh => spawn_daemon().await,
        DaemonRecoveryAction::RestartExisting { reason } => restart_daemon(&reason).await,
    }
}

async fn probe_existing_daemon() -> Result<ExistingDaemonStatus> {
    Ok(inspect_existing_daemon().await?.status)
}

pub(crate) async fn inspect_existing_daemon() -> Result<DaemonInspection> {
    let paths = daemon_paths()?;
    let path_info = DaemonPathInfo::from(&paths);
    if !paths.socket.exists() {
        return Ok(DaemonInspection {
            paths: path_info,
            status: ExistingDaemonStatus::Missing {
                reason: format!("daemon socket {} does not exist", paths.socket.display()),
            },
            ping: None,
        });
    }

    match ping_daemon().await {
        Ok(result) => {
            let status = match daemon_compatibility(&result, &current_version_metadata()) {
                DaemonCompatibility::Compatible => ExistingDaemonStatus::Compatible,
                DaemonCompatibility::Incompatible { reason } => {
                    ExistingDaemonStatus::Incompatible { reason }
                }
            };
            Ok(DaemonInspection {
                paths: path_info,
                status,
                ping: Some(result),
            })
        }
        Err(err) => Ok(DaemonInspection {
            paths: path_info,
            status: ExistingDaemonStatus::Incompatible {
                reason: format!("daemon ping failed: {err:#}"),
            },
            ping: None,
        }),
    }
}

impl From<&DaemonPaths> for DaemonPathInfo {
    fn from(paths: &DaemonPaths) -> Self {
        Self {
            home: paths.home.clone(),
            socket: paths.socket.clone(),
            pid: paths.pid.clone(),
            log: paths.log.clone(),
        }
    }
}

async fn restart_daemon(reason: &str) -> Result<()> {
    let paths = daemon_paths()?;
    eprintln!("socai rust daemon is stale ({reason}); restarting");

    if let Err(err) = send_request("shutdown", json!({}), SHUTDOWN_TIMEOUT).await {
        eprintln!("socai rust daemon shutdown request failed before restart: {err:#}");
    }

    if !wait_for_daemon_shutdown(&paths.socket, RESTART_SHUTDOWN_WAIT).await {
        return Err(anyhow!(
            "socai rust daemon is stale ({reason}) but did not stop after a shutdown request; {}",
            daemon_recovery_hint(&paths)
        ));
    }

    spawn_daemon().await.with_context(|| {
        format!(
            "restart stale socai rust daemon after compatibility mismatch ({reason}); {}",
            daemon_recovery_hint(&paths)
        )
    })
}

async fn wait_for_daemon_shutdown(socket: &Path, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => {
                drop(stream);
                if Instant::now() >= deadline {
                    return false;
                }
                sleep(Duration::from_millis(100)).await;
            }
            Err(_) => return true,
        }
    }
}

async fn ping_daemon() -> Result<Value> {
    send_request("ping", ping_args(), PING_TIMEOUT).await
}

fn daemon_recovery_hint(paths: &DaemonPaths) -> String {
    format!(
        "see daemon log at {}; try `socai stop` then retry, or run `socai doctor`",
        paths.log.display()
    )
}

async fn send_request(command: &str, args: Value, request_timeout: Duration) -> Result<Value> {
    let paths = daemon_paths()?;
    let request = DaemonRequest {
        id: request_id(),
        command: command.to_string(),
        args,
        telemetry: DaemonTelemetry {
            enabled: telemetry_enabled(),
            include_query_text: query_text_enabled(),
        },
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
        .with_context(|| {
            format!(
                "open daemon log at {}; try `socai stop` then retry, or run `socai doctor`",
                paths.log.display()
            )
        })?;
    let stderr = log.try_clone()?;

    let current_exe = std::env::current_exe().context("resolve current socai executable")?;
    let mut command = std::process::Command::new(&current_exe);
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
    command.spawn().with_context(|| {
        format!(
            "spawn socai rust daemon from {}; {}",
            current_exe.display(),
            daemon_recovery_hint(&paths)
        )
    })?;

    let mut last_error = None;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        match ping_daemon().await {
            Ok(result) => match daemon_compatibility(&result, &current_version_metadata()) {
                DaemonCompatibility::Compatible => return Ok(()),
                DaemonCompatibility::Incompatible { reason } => {
                    last_error = Some(format!("daemon answered incompatible ping: {reason}"));
                }
            },
            Err(err) => {
                last_error = Some(format!("daemon ping failed: {err:#}"));
            }
        }
        sleep(Duration::from_millis(250)).await;
    }

    let detail = last_error.unwrap_or_else(|| "daemon never answered ping".to_string());
    Err(anyhow!(
        "socai rust daemon did not become ready ({detail}); {}",
        daemon_recovery_hint(&paths)
    ))
}

fn debug_snapshot_flag(args: &Value) -> bool {
    args.get("debug_snapshot")
        .and_then(Value::as_bool)
        .unwrap_or(false)
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
