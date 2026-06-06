use crate::daemon::{self, ExistingDaemonStatus};

use anyhow::{Context, Result};
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use serde_json::Value;
use socai_core::cdp::discover_existing_chrome_endpoint;
use std::cmp::Ordering;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout as tokio_timeout;

const RELEASE_API_URL: &str = "https://api.github.com/repos/tonyc-ship/socai/releases/latest";
const RELEASE_HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const CDP_WS_REACHABILITY_TIMEOUT: Duration = Duration::from_secs(2);
// Managed CLI release asset. The repo also cuts app-only releases whose tag is
// newer than the CLI but that ship no CLI tarball; those must not be reported as
// a CLI update.
const CLI_RELEASE_ASSET: &str = "socai-cli-macos-universal.tar.gz";

pub(crate) async fn run() -> Result<bool> {
    let mut report = DoctorReport::default();

    report.add(DiagnosticRow::ok(
        "platform",
        format!(
            "{}/{}/{}",
            env::consts::OS,
            env::consts::ARCH,
            env::consts::FAMILY
        ),
    ));

    let home_dir = home_dir_from_env();
    let mut install_mode = InstallMode::Unknown;
    match env::current_exe() {
        Ok(path) => {
            install_mode = detect_install_mode(&path, home_dir.as_deref());
            report.add(DiagnosticRow::ok("executable", path.display().to_string()));
        }
        Err(err) => report.add(DiagnosticRow::error(
            "executable",
            format!("could not resolve current executable: {err:#}"),
        )),
    }

    report.add(DiagnosticRow::ok("cli", cli_version_summary()));
    report.add(DiagnosticRow::new(
        "install mode",
        if install_mode == InstallMode::Unknown {
            RowStatus::Warn
        } else {
            RowStatus::Ok
        },
        format!("{}; {}", install_mode.as_str(), update_hint(install_mode)),
    ));

    add_daemon_rows(&mut report).await;
    add_cdp_row(&mut report).await;
    add_release_rows(&mut report, install_mode).await;

    report.print();
    Ok(report.is_healthy())
}

async fn add_daemon_rows(report: &mut DoctorReport) {
    match daemon::inspect_existing_daemon().await {
        Ok(inspection) => {
            report.add(DiagnosticRow::ok(
                "socai home",
                inspection.paths.home.display().to_string(),
            ));
            report.add(DiagnosticRow::ok(
                "daemon socket",
                path_state(&inspection.paths.socket),
            ));
            report.add(DiagnosticRow::ok(
                "daemon pid",
                path_state(&inspection.paths.pid),
            ));
            report.add(DiagnosticRow::ok(
                "daemon log",
                path_state(&inspection.paths.log),
            ));

            match inspection.status {
                ExistingDaemonStatus::Compatible => report.add(DiagnosticRow::ok(
                    "daemon",
                    format!(
                        "running; {}",
                        daemon_metadata_summary(inspection.ping.as_ref())
                    ),
                )),
                ExistingDaemonStatus::Missing { reason } => report.add(DiagnosticRow::ok(
                    "daemon",
                    format!("not running ({reason}); socai commands start it on demand"),
                )),
                ExistingDaemonStatus::Unreachable { reason } => report.add(DiagnosticRow::warn(
                    "daemon",
                    format!(
                        "stale socket ({reason}); the next socai command clears it and spawns a fresh daemon (no action needed)"
                    ),
                )),
                ExistingDaemonStatus::Incompatible { reason } => report.add(DiagnosticRow::error(
                    "daemon",
                    format!(
                        "unhealthy ({reason}); {}; try `socai stop` then retry if it persists",
                        daemon_metadata_summary(inspection.ping.as_ref())
                    ),
                )),
            }
        }
        Err(err) => report.add(DiagnosticRow::error(
            "socai home/daemon",
            format!("could not resolve daemon paths: {err:#}"),
        )),
    }
}

async fn add_cdp_row(report: &mut DoctorReport) {
    match discover_existing_chrome_endpoint().await {
        Ok(Some(endpoint)) => {
            let needs_ws_reachability_check =
                endpoint.http_version_url.is_none() && endpoint.version.is_none();
            let mut parts = vec![
                format!("source {}", endpoint.source),
                format!("ws {}", endpoint.browser_ws_url),
            ];
            if let Some(version_url) = endpoint.http_version_url {
                parts.push(format!("version url {version_url}"));
            }
            if let Some(version) = endpoint.version {
                if let Some(browser) = version.browser {
                    parts.push(format!("browser {browser}"));
                }
                if let Some(protocol) = version.protocol_version {
                    parts.push(format!("protocol {protocol}"));
                }
            }

            if needs_ws_reachability_check {
                match validate_websocket_tcp_reachable(&endpoint.browser_ws_url).await {
                    Ok(()) => parts.push("tcp reachable".to_string()),
                    Err(err) => {
                        parts.push(format!("unreachable ({err:#})"));
                        report.add(DiagnosticRow::error(
                            "browser/cdp",
                            format!(
                                "{}; launch Chrome with --remote-debugging-port=9222 or set SOCAI_CDP_WS/SOCAI_CDP_URL",
                                parts.join("; ")
                            ),
                        ));
                        return;
                    }
                }
            }

            report.add(DiagnosticRow::ok("browser/cdp", parts.join("; ")));
        }
        Ok(None) => report.add(DiagnosticRow::error(
            "browser/cdp",
            "no CDP endpoint found; launch Chrome with --remote-debugging-port=9222 or set SOCAI_CDP_WS/SOCAI_CDP_URL",
        )),
        Err(err) => report.add(DiagnosticRow::error(
            "browser/cdp",
            format!(
                "CDP endpoint probe failed: {err:#}; launch Chrome with --remote-debugging-port=9222 or set SOCAI_CDP_WS/SOCAI_CDP_URL"
            ),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebsocketTcpTarget {
    host: String,
    port: u16,
}

fn websocket_tcp_target(raw_url: &str) -> Result<WebsocketTcpTarget> {
    let url =
        reqwest::Url::parse(raw_url).with_context(|| format!("parse websocket URL {raw_url}"))?;
    let default_port = match url.scheme() {
        "ws" => 80,
        "wss" => 443,
        other => anyhow::bail!("expected ws:// or wss:// CDP websocket URL, got {other}://"),
    };
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .context("websocket URL missing host")?
        .to_string();
    let port = url.port().unwrap_or(default_port);

    Ok(WebsocketTcpTarget { host, port })
}

async fn validate_websocket_tcp_reachable(raw_url: &str) -> Result<()> {
    let target = websocket_tcp_target(raw_url)?;
    let address = format!("{}:{}", target.host, target.port);
    tokio_timeout(
        CDP_WS_REACHABILITY_TIMEOUT,
        TcpStream::connect((target.host.as_str(), target.port)),
    )
    .await
    .with_context(|| format!("timed out connecting to {address}"))?
    .with_context(|| format!("connect to {address}"))?;

    Ok(())
}

async fn add_release_rows(report: &mut DoctorReport, install_mode: InstallMode) {
    match fetch_latest_release().await {
        Ok(latest) => {
            let current = env!("CARGO_PKG_VERSION");
            let (status, message) = latest_release_message(&latest, current);
            report.add(DiagnosticRow::new("latest release", status, message));
        }
        Err(err) => report.add(DiagnosticRow::warn(
            "latest release",
            format!("unavailable/offline ({err:#}); update check is non-fatal"),
        )),
    }

    report.add(DiagnosticRow::ok("update hint", update_hint(install_mode)));
}

fn path_state(path: &Path) -> String {
    let state = if path.exists() { "exists" } else { "missing" };
    format!("{} ({state})", path.display())
}

fn cli_version_summary() -> String {
    match build_sha() {
        Some(sha) => format!("version {}; build {sha}", env!("CARGO_PKG_VERSION")),
        None => format!("version {}; build unavailable", env!("CARGO_PKG_VERSION")),
    }
}

fn build_sha() -> Option<String> {
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

fn daemon_metadata_summary(ping: Option<&Value>) -> String {
    let Some(daemon) = ping.and_then(|value| value.get("daemon")) else {
        return "daemon metadata unavailable".to_string();
    };

    let version = daemon
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let protocol = daemon
        .get("protocol_version")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let build = daemon
        .get("build_sha")
        .and_then(Value::as_str)
        .map(|value| format!("; build {value}"))
        .unwrap_or_else(|| "; build unavailable".to_string());

    format!("version {version}; protocol {protocol}{build}")
}

fn home_dir_from_env() -> Option<PathBuf> {
    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }

    #[cfg(windows)]
    {
        if let Some(profile) = env::var_os("USERPROFILE") {
            return Some(PathBuf::from(profile));
        }
        if let (Some(drive), Some(path)) = (env::var_os("HOMEDRIVE"), env::var_os("HOMEPATH")) {
            let mut combined = PathBuf::from(drive);
            combined.push(path);
            return Some(combined);
        }
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallMode {
    ManagedBinary,
    SourceCargo,
    Unknown,
}

impl InstallMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::ManagedBinary => "managed-binary",
            Self::SourceCargo => "source-cargo",
            Self::Unknown => "unknown",
        }
    }
}

pub(crate) fn detect_install_mode(executable: &Path, home: Option<&Path>) -> InstallMode {
    let Some(home) = home else {
        return InstallMode::Unknown;
    };
    let Some(name) = executable.file_name() else {
        return InstallMode::Unknown;
    };
    if name != "socai" && name != "socai.exe" {
        return InstallMode::Unknown;
    }

    if executable == home.join(".socai").join("bin").join(name) {
        return InstallMode::ManagedBinary;
    }
    if executable == home.join(".cargo").join("bin").join(name) {
        return InstallMode::SourceCargo;
    }

    InstallMode::Unknown
}

fn update_hint(mode: InstallMode) -> &'static str {
    match mode {
        InstallMode::ManagedBinary => {
            "managed-binary update: rerun the socai installer or replace ~/.socai/bin/socai with the latest GitHub release binary"
        }
        InstallMode::SourceCargo => {
            "source-cargo update: in your socai checkout run `git pull --ff-only` then `cargo install --path cli --force`"
        }
        InstallMode::Unknown => {
            "update using the installer or package manager that placed this executable; socai doctor will not self-update"
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: Option<String>,
    name: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseAsset {
    name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LatestRelease {
    version: String,
    has_cli_asset: bool,
}

async fn fetch_latest_release() -> Result<LatestRelease> {
    let client = reqwest::Client::builder()
        .timeout(RELEASE_HTTP_TIMEOUT)
        .build()
        .context("build GitHub release HTTP client")?;
    let release: GitHubRelease = client
        .get(RELEASE_API_URL)
        .header(USER_AGENT, "socai-doctor")
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .context("request latest GitHub release")?
        .error_for_status()
        .context("latest GitHub release HTTP status")?
        .json()
        .await
        .context("decode latest GitHub release")?;

    let version = release
        .tag_name
        .or(release.name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .context("latest GitHub release did not include tag_name or name")?;
    let has_cli_asset = release.assets.iter().any(|asset| {
        asset
            .name
            .as_deref()
            .is_some_and(|name| name == CLI_RELEASE_ASSET)
    });

    Ok(LatestRelease {
        version,
        has_cli_asset,
    })
}

fn latest_release_message(latest: &LatestRelease, current: &str) -> (RowStatus, String) {
    // App-only releases ship no CLI tarball; comparing their (newer) tag to the
    // CLI version would falsely claim a CLI update is available.
    if !latest.has_cli_asset {
        return (
            RowStatus::Ok,
            format!(
                "latest release {} ships no managed CLI asset ({CLI_RELEASE_ASSET}); CLI update check skipped (non-fatal)",
                latest.version
            ),
        );
    }

    match compare_release_versions(&latest.version, current) {
        Some(Ordering::Greater) => (
            RowStatus::Warn,
            format!("update available: {} (current {current})", latest.version),
        ),
        Some(Ordering::Equal) => (RowStatus::Ok, format!("up to date: {}", latest.version)),
        Some(Ordering::Less) => (
            RowStatus::Ok,
            format!(
                "current {current} is newer than latest release {}",
                latest.version
            ),
        ),
        None => (
            RowStatus::Warn,
            format!(
                "latest release {}; could not compare with current {current}",
                latest.version
            ),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

fn compare_release_versions(latest: &str, current: &str) -> Option<Ordering> {
    Some(parse_release_version(latest)?.cmp(&parse_release_version(current)?))
}

fn parse_release_version(raw: &str) -> Option<ReleaseVersion> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let version_part = trimmed
        .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .next()
        .filter(|part| !part.is_empty())?;
    let mut parts = version_part.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some(ReleaseVersion {
        major,
        minor,
        patch,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowStatus {
    Ok,
    Warn,
    Error,
}

impl RowStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
struct DiagnosticRow {
    label: &'static str,
    status: RowStatus,
    message: String,
}

impl DiagnosticRow {
    fn new(label: &'static str, status: RowStatus, message: impl Into<String>) -> Self {
        Self {
            label,
            status,
            message: message.into(),
        }
    }

    fn ok(label: &'static str, message: impl Into<String>) -> Self {
        Self::new(label, RowStatus::Ok, message)
    }

    fn warn(label: &'static str, message: impl Into<String>) -> Self {
        Self::new(label, RowStatus::Warn, message)
    }

    fn error(label: &'static str, message: impl Into<String>) -> Self {
        Self::new(label, RowStatus::Error, message)
    }

    fn render(&self) -> String {
        format!(
            "{:<20} {:<7} {}",
            self.label,
            self.status.as_str(),
            self.message
        )
    }
}

#[derive(Default)]
struct DoctorReport {
    rows: Vec<DiagnosticRow>,
}

impl DoctorReport {
    fn add(&mut self, row: DiagnosticRow) {
        self.rows.push(row);
    }

    fn is_healthy(&self) -> bool {
        self.rows.iter().all(|row| row.status != RowStatus::Error)
    }

    fn print(&self) {
        println!("socai doctor");
        println!();
        for row in &self.rows {
            println!("{}", row.render());
        }
        println!();
        if self.is_healthy() {
            println!("summary              ok      socai CLI prerequisites look usable");
        } else {
            println!("summary              error   fix error rows before running browser-backed socai commands");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_managed_binary_install_mode() {
        let home = Path::new("/Users/alice");
        let exe = home.join(".socai/bin/socai");

        assert_eq!(
            detect_install_mode(&exe, Some(home)),
            InstallMode::ManagedBinary
        );
    }

    #[test]
    fn detects_source_cargo_install_mode() {
        let home = Path::new("/Users/alice");
        let exe = home.join(".cargo/bin/socai");

        assert_eq!(
            detect_install_mode(&exe, Some(home)),
            InstallMode::SourceCargo
        );
    }

    #[test]
    fn unknown_install_mode_for_other_paths() {
        let home = Path::new("/Users/alice");
        let exe = Path::new("/usr/local/bin/socai");

        assert_eq!(detect_install_mode(exe, Some(home)), InstallMode::Unknown);
    }

    #[test]
    fn update_hints_match_install_mode() {
        assert!(update_hint(InstallMode::ManagedBinary).contains("~/.socai/bin/socai"));
        let source_hint = update_hint(InstallMode::SourceCargo);
        assert!(source_hint.contains("git pull --ff-only"));
        assert!(source_hint.contains("cargo install --path cli --force"));
        assert!(!source_hint.contains("cargo install --git"));
        assert!(update_hint(InstallMode::Unknown).contains("will not self-update"));
    }

    #[test]
    fn parses_websocket_tcp_target_with_explicit_port() {
        assert_eq!(
            websocket_tcp_target("ws://127.0.0.1:9222/devtools/browser/id").ok(),
            Some(WebsocketTcpTarget {
                host: "127.0.0.1".to_string(),
                port: 9222,
            })
        );
    }

    #[test]
    fn parses_websocket_tcp_target_default_ports() {
        assert_eq!(
            websocket_tcp_target("ws://localhost/devtools/browser/id").ok(),
            Some(WebsocketTcpTarget {
                host: "localhost".to_string(),
                port: 80,
            })
        );
        assert_eq!(
            websocket_tcp_target("wss://example.com/devtools/browser/id").ok(),
            Some(WebsocketTcpTarget {
                host: "example.com".to_string(),
                port: 443,
            })
        );
    }

    #[test]
    fn rejects_non_websocket_tcp_target() {
        assert!(websocket_tcp_target("http://127.0.0.1:9222/json/version").is_err());
        assert!(websocket_tcp_target("not a url").is_err());
    }

    #[test]
    fn compares_release_versions_with_v_prefix() {
        assert_eq!(
            compare_release_versions("v0.2.0", "0.1.9"),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_release_versions("0.1.0", "v0.1.0"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_release_versions("v0.1.0", "0.2.0"),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn latest_release_message_warns_when_update_available() {
        let latest = LatestRelease {
            version: "v0.2.0".to_string(),
            has_cli_asset: true,
        };
        let (status, message) = latest_release_message(&latest, "0.1.0");

        assert_eq!(status, RowStatus::Warn);
        assert!(message.contains("update available"));
    }

    #[test]
    fn latest_release_message_skips_update_when_no_cli_asset() {
        // App-only release: newer tag than the CLI, but no CLI tarball.
        let latest = LatestRelease {
            version: "v0.1.5".to_string(),
            has_cli_asset: false,
        };
        let (status, message) = latest_release_message(&latest, "0.1.0");

        assert_eq!(status, RowStatus::Ok);
        assert!(!message.contains("update available"));
        assert!(message.contains("no managed CLI asset"));
        assert!(message.contains(CLI_RELEASE_ASSET));
    }

    #[test]
    fn latest_release_message_reports_update_when_cli_asset_present() {
        let latest = LatestRelease {
            version: "v0.1.0".to_string(),
            has_cli_asset: true,
        };
        let (status, message) = latest_release_message(&latest, "0.1.0");

        assert_eq!(status, RowStatus::Ok);
        assert!(message.contains("up to date"));
    }

    #[test]
    fn diagnostic_row_render_includes_columns() {
        let rendered = DiagnosticRow::ok("platform", "macos/aarch64/unix").render();

        assert!(rendered.starts_with("platform"));
        assert!(rendered.contains("ok"));
        assert!(rendered.ends_with("macos/aarch64/unix"));
    }
}
