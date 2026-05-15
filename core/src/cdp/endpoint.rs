use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const INSPECT_URL: &str = "chrome://inspect/#remote-debugging";
const DEFAULT_DEVTOOLS_PORTS: &[u16] = &[9222, 9223];
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
pub struct Endpoint {
    pub source: String,
    pub browser_ws_url: String,
    pub http_version_url: Option<String>,
    pub version: Option<VersionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionInfo {
    #[serde(rename = "Browser", default)]
    pub browser: Option<String>,
    #[serde(rename = "Protocol-Version", default)]
    pub protocol_version: Option<String>,
    #[serde(rename = "User-Agent", default)]
    pub user_agent: Option<String>,
    #[serde(rename = "V8-Version", default)]
    pub v8_version: Option<String>,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    pub web_socket_debugger_url: Option<String>,
}

/// Resolve only explicitly supplied endpoints (args + env vars). Returns None
/// when nothing explicit was set; the caller decides whether to fall back to
/// profile-scan discovery.
pub async fn resolve_explicit_endpoint(
    browser_ws_url: Option<&str>,
    http_url: Option<&str>,
) -> anyhow::Result<Option<Endpoint>> {
    if let Some(url) = non_empty(browser_ws_url) {
        return Ok(Some(Endpoint {
            source: "argument".into(),
            browser_ws_url: url.into(),
            http_version_url: None,
            version: None,
        }));
    }
    if let Some(url) = non_empty(http_url) {
        return Ok(Some(endpoint_from_http_url(url, "argument").await?));
    }
    if let Some(url) = env_var("SOCAI_CDP_WS") {
        return Ok(Some(Endpoint {
            source: "SOCAI_CDP_WS".into(),
            browser_ws_url: url,
            http_version_url: None,
            version: None,
        }));
    }
    if let Some(url) = env_var("SOCAI_CDP_URL") {
        return Ok(Some(endpoint_from_http_url(&url, "SOCAI_CDP_URL").await?));
    }
    Ok(None)
}

/// One-shot discovery: explicit endpoints first, then `DevToolsActivePort` /
/// `/json/version` probes on the standard Chrome profile roots and ports.
pub async fn discover_existing_chrome_endpoint() -> anyhow::Result<Option<Endpoint>> {
    if let Some(endpoint) = resolve_explicit_endpoint(None, None).await? {
        return Ok(Some(endpoint));
    }
    for profile in chrome_profile_roots() {
        if let Some(endpoint) = endpoint_from_active_port(&profile).await {
            return Ok(Some(endpoint));
        }
    }
    for port in DEFAULT_DEVTOOLS_PORTS {
        let url = format!("http://127.0.0.1:{port}");
        if let Ok(endpoint) = endpoint_from_http_url(&url, &format!("port:{port}")).await {
            return Ok(Some(endpoint));
        }
    }
    Ok(None)
}

/// Poll `discover_existing_chrome_endpoint` until it succeeds or `timeout`
/// elapses. Intended for the first-run path where we opened chrome://inspect
/// and now wait for the user to approve remote debugging.
pub async fn wait_for_existing_chrome_endpoint(
    timeout: Duration,
    poll: Duration,
) -> anyhow::Result<Option<Endpoint>> {
    let deadline = Instant::now() + timeout.max(Duration::from_millis(100));
    let step = poll.max(Duration::from_millis(100));
    loop {
        if let Some(endpoint) = discover_existing_chrome_endpoint().await? {
            return Ok(Some(endpoint));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(step).await;
    }
}

/// Open the chrome://inspect remote-debugging page in the user's default
/// Chrome. Best-effort; failures are swallowed so first-run UX still degrades
/// gracefully to a printed instruction.
pub fn open_remote_debugging_page() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-a", "Google Chrome", INSPECT_URL])
            .status();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(INSPECT_URL)
            .status();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", INSPECT_URL])
            .status();
    }
}

async fn endpoint_from_http_url(url: &str, source: &str) -> anyhow::Result<Endpoint> {
    let version_url = format!("{}/json/version", url.trim_end_matches('/'));
    let version: VersionInfo = get_json(&version_url).await?;
    let ws = version
        .web_socket_debugger_url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("/json/version missing webSocketDebuggerUrl"))?;
    Ok(Endpoint {
        source: source.into(),
        browser_ws_url: ws,
        http_version_url: Some(version_url),
        version: Some(version),
    })
}

async fn endpoint_from_active_port(profile: &Path) -> Option<Endpoint> {
    let marker = profile.join("DevToolsActivePort");
    let contents = fs::read_to_string(&marker).ok()?;
    let mut lines = contents.lines();
    let port: u16 = lines.next()?.trim().parse().ok()?;
    let ws_path = lines.next().map(str::trim).unwrap_or("").to_string();

    // Prefer richer info via HTTP /json/version. Fall back to constructing the
    // ws URL ourselves if the HTTP endpoint refuses but DevToolsActivePort
    // gave us the path.
    let http_url = format!("http://127.0.0.1:{port}");
    let source = format!("active_port:{}", marker.display());
    if let Ok(endpoint) = endpoint_from_http_url(&http_url, &source).await {
        return Some(endpoint);
    }
    if ws_path.is_empty() {
        return None;
    }
    Some(Endpoint {
        source,
        browser_ws_url: format!("ws://127.0.0.1:{port}{ws_path}"),
        http_version_url: None,
        version: None,
    })
}

fn chrome_profile_roots() -> Vec<PathBuf> {
    if let Some(override_path) = env_var("SOCAI_CHROME_USER_DATA_DIR") {
        return vec![PathBuf::from(shellexpand(&override_path))];
    }
    let Some(home) = home_dir() else {
        return Vec::new();
    };

    #[cfg(target_os = "macos")]
    let candidates: &[&str] = &[
        "Library/Application Support/Google/Chrome",
        "Library/Application Support/Comet",
        "Library/Application Support/Arc/User Data",
        "Library/Application Support/Microsoft Edge",
        "Library/Application Support/BraveSoftware/Brave-Browser",
    ];

    #[cfg(target_os = "linux")]
    let candidates: &[&str] = &[
        ".config/google-chrome",
        ".config/chromium",
        ".config/microsoft-edge",
        ".config/BraveSoftware/Brave-Browser",
    ];

    #[cfg(target_os = "windows")]
    let candidates: &[&str] = &[
        "AppData/Local/Google/Chrome/User Data",
        "AppData/Local/Microsoft/Edge/User Data",
        "AppData/Local/BraveSoftware/Brave-Browser/User Data",
    ];

    candidates.iter().map(|c| home.join(c)).collect()
}

fn home_dir() -> Option<PathBuf> {
    if let Some(home) = env_var("HOME") {
        return Some(PathBuf::from(home));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(profile) = env_var("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
        if let (Some(drive), Some(path)) = (env_var("HOMEDRIVE"), env_var("HOMEPATH")) {
            return Some(PathBuf::from(format!("{drive}{path}")));
        }
    }
    None
}

fn env_var(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.filter(|v| !v.is_empty())
}

fn shellexpand(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    s.to_string()
}

async fn get_json<T: for<'de> serde::Deserialize<'de>>(url: &str) -> anyhow::Result<T> {
    let resp = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.json().await?)
}
