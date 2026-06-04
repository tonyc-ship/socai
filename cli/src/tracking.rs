use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

const EVENT_SCHEMA_VERSION: u32 = 1;
const DEFAULT_TELEMETRY_ENDPOINT: &str = "https://socai.io/v1/events";
const CHANNEL_CAPACITY: usize = 512;
const REMOTE_BATCH_SIZE: usize = 25;
const REMOTE_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Telemetry {
    sender: Option<mpsc::Sender<QueuedEvent>>,
    include_query_text: bool,
    remote_enabled: bool,
}

#[derive(Debug)]
struct QueuedEvent {
    name: String,
    properties: Value,
}

#[derive(Clone)]
struct WorkerConfig {
    install_id: String,
    daemon_session_id: String,
    local_path: PathBuf,
    endpoint: Option<String>,
    include_query_text: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct IdentityFile {
    install_id: String,
}

impl Telemetry {
    pub fn new(home: &Path) -> Self {
        if telemetry_disabled() {
            return Self {
                sender: None,
                include_query_text: false,
                remote_enabled: false,
            };
        }

        let include_query_text = query_text_enabled();
        let install_id = load_or_create_install_id(home);
        let daemon_session_id = new_session_id();
        let local_path = home.join("telemetry/events.jsonl");
        let endpoint = telemetry_endpoint();

        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        let config = WorkerConfig {
            install_id,
            daemon_session_id,
            local_path,
            endpoint,
            include_query_text,
        };
        let remote_enabled = config.endpoint.is_some();
        tokio::spawn(worker_loop(receiver, config));

        let telemetry = Self {
            sender: Some(sender),
            include_query_text,
            remote_enabled,
        };
        telemetry.capture(
            "socai_daemon_started",
            json!({
                "remote_enabled": remote_enabled,
                "query_text_enabled": include_query_text,
            }),
        );
        telemetry
    }

    pub fn enabled(&self) -> bool {
        self.sender.is_some()
    }

    pub fn include_query_text(&self) -> bool {
        self.include_query_text
    }

    pub fn remote_enabled(&self) -> bool {
        self.remote_enabled
    }

    pub fn capture(&self, name: impl Into<String>, properties: Value) {
        let Some(sender) = &self.sender else {
            return;
        };
        let _ = sender.try_send(QueuedEvent {
            name: name.into(),
            properties,
        });
    }
}

async fn worker_loop(mut receiver: mpsc::Receiver<QueuedEvent>, config: WorkerConfig) {
    let client = reqwest::Client::new();
    let mut remote_batch: Vec<Value> = Vec::new();
    let mut flush_tick = tokio::time::interval(REMOTE_FLUSH_INTERVAL);
    flush_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            maybe_event = receiver.recv() => {
                let Some(event) = maybe_event else {
                    break;
                };
                let timestamp_ms = now_ms();
                let properties = enrich_properties(event.properties, &config, timestamp_ms);
                let row = local_row(&event.name, &config.install_id, timestamp_ms, &properties);
                let _ = append_jsonl(&config.local_path, &row).await;

                if config.endpoint.is_some() {
                    remote_batch.push(remote_event(&event.name, &config.install_id, timestamp_ms, &properties));
                    if remote_batch.len() >= REMOTE_BATCH_SIZE {
                        flush_remote(&client, &config, &mut remote_batch).await;
                    }
                }
            }
            _ = flush_tick.tick() => {
                flush_remote(&client, &config, &mut remote_batch).await;
            }
        }
    }

    flush_remote(&client, &config, &mut remote_batch).await;
}

fn enrich_properties(properties: Value, config: &WorkerConfig, timestamp_ms: u64) -> Value {
    let mut map = match properties {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("value".into(), other);
            map
        }
    };
    map.insert("schema_version".into(), json!(EVENT_SCHEMA_VERSION));
    map.insert("app".into(), json!("socai"));
    map.insert("source".into(), json!("cli_daemon"));
    map.insert("app_version".into(), json!(env!("CARGO_PKG_VERSION")));
    map.insert("platform".into(), json!(std::env::consts::OS));
    map.insert("arch".into(), json!(std::env::consts::ARCH));
    map.insert("daemon_session_id".into(), json!(config.daemon_session_id));
    map.insert(
        "query_text_enabled".into(),
        json!(config.include_query_text),
    );
    map.insert("created_at_ms".into(), json!(timestamp_ms));
    Value::Object(map)
}

fn local_row(event_name: &str, install_id: &str, timestamp_ms: u64, properties: &Value) -> Value {
    json!({
        "event": event_name,
        "install_id": install_id,
        "created_at_ms": timestamp_ms,
        "properties": properties,
    })
}

fn remote_event(
    event_name: &str,
    install_id: &str,
    timestamp_ms: u64,
    properties: &Value,
) -> Value {
    let mut map = match properties {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    map.insert("event".into(), json!(event_name));
    map.insert("install_id".into(), json!(install_id));
    map.insert("created_at_ms".into(), json!(timestamp_ms));
    Value::Object(map)
}

async fn flush_remote(client: &reqwest::Client, config: &WorkerConfig, batch: &mut Vec<Value>) {
    let Some(endpoint) = config.endpoint.as_deref() else {
        batch.clear();
        return;
    };
    if batch.is_empty() {
        return;
    }

    let events = std::mem::take(batch);
    let body = json!({ "events": events });
    let _ = client.post(endpoint).json(&body).send().await;
}

async fn append_jsonl(path: &Path, row: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    let line = serde_json::to_string(row).map_err(std::io::Error::other)?;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await
}

fn load_or_create_install_id(home: &Path) -> String {
    let path = home.join("telemetry/identity.json");
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(identity) = serde_json::from_slice::<IdentityFile>(&bytes) {
            let id = identity.install_id.trim();
            if !id.is_empty() {
                return id.to_string();
            }
        }
    }

    let install_id = new_install_id();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_vec_pretty(&IdentityFile {
            install_id: install_id.clone(),
        })
        .unwrap_or_else(|_| b"{}".to_vec()),
    );
    install_id
}

fn new_install_id() -> String {
    format!(
        "socai-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    )
}

fn new_session_id() -> String {
    format!("daemon-{}-{}", std::process::id(), now_ms())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn telemetry_disabled() -> bool {
    env_truthy("DO_NOT_TRACK")
        || env_truthy("SOCAI_DISABLE_TELEMETRY")
        || env_value_is("SOCAI_TELEMETRY", &["0", "false", "off", "disabled", "no"])
}

fn query_text_enabled() -> bool {
    !(env_truthy("SOCAI_TELEMETRY_REDACT_QUERIES")
        || env_value_is(
            "SOCAI_TELEMETRY_QUERY_TEXT",
            &["0", "false", "off", "disabled", "no"],
        ))
}

fn telemetry_endpoint() -> Option<String> {
    env_nonempty("SOCAI_TELEMETRY_ENDPOINT")
        .or_else(|| option_env!("SOCAI_TELEMETRY_ENDPOINT").map(str::to_string))
        .or_else(|| Some(DEFAULT_TELEMETRY_ENDPOINT.to_string()))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| {
            !matches!(
                value.to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "disabled" | "no"
            )
        })
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_truthy(name: &str) -> bool {
    env_value_is(name, &["1", "true", "yes", "on"])
}

fn env_value_is(name: &str, values: &[&str]) -> bool {
    let Ok(value) = std::env::var(name) else {
        return false;
    };
    let value = value.trim().to_ascii_lowercase();
    values.iter().any(|candidate| value == *candidate)
}
