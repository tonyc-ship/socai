use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use socai_runtime::SocaiRuntime;
use socai_sites::xhs::{
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
    let state = Arc::new(Mutex::new(DaemonState {
        runtime,
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
        let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
        search_notes_command(page, &query).await
    }

    async fn topic_scan(&mut self, args: Value) -> Result<Value> {
        let query = required_string(&args, "query")?;
        let depth = args
            .get("depth")
            .and_then(Value::as_str)
            .unwrap_or("standard");
        let tab_label = args.get("tab_label").and_then(Value::as_str);
        let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
        topic_scan_command(page, &query, depth, tab_label).await
    }

    async fn extract_note(&mut self, args: Value) -> Result<Value> {
        let note_id = required_string(&args, "note_id")?;
        let page = self.runtime.ensure_site_page("xhs", XHS_HOME_URL).await?;
        extract_note_command(page, &note_id).await
    }

    async fn shutdown(&mut self) -> Result<()> {
        let _ = self.runtime.close_site_session("xhs").await;
        self.runtime.disconnect_browser().await;
        Ok(())
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
