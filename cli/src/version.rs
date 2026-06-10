use std::cmp::Ordering;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const LATEST_RELEASE_URL: &str = "https://github.com/socai-io/socai/releases/latest";
const MACOS_INSTALL_SCRIPT_URL: &str =
    "https://github.com/socai-io/socai/releases/latest/download/install.sh";
const WINDOWS_INSTALL_COMMAND: &str = "(Invoke-WebRequest -UseBasicParsing https://github.com/socai-io/socai/releases/latest/download/install.ps1).Content | Invoke-Expression";
const UPDATE_COMMAND: &str = "socai update";
const CHECK_INTERVAL: u64 = 24 * 60 * 60;
const FAILED_CHECK_INTERVAL: u64 = 60 * 60;
const NOTIFY_INTERVAL: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct StrictVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

#[derive(Debug, Clone)]
struct LatestRelease {
    version: String,
    html_url: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct UpdateCache {
    checked_at: Option<u64>,
    latest_version: Option<String>,
    latest_url: Option<String>,
    last_error_at: Option<u64>,
    notified_at: Option<u64>,
    notified_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct VersionReport<'a> {
    current_version: &'a str,
    latest_version: Option<&'a str>,
    status: &'a str,
    update_available: bool,
    release_url: Option<&'a str>,
    upgrade_command: Option<String>,
    error: Option<&'a str>,
}

pub async fn print_version_command(no_check: bool, json: bool) -> Result<()> {
    if no_check {
        print_report(
            VersionReport {
                current_version: CURRENT_VERSION,
                latest_version: None,
                status: "not-checked",
                update_available: false,
                release_url: None,
                upgrade_command: None,
                error: None,
            },
            json,
        )?;
        return Ok(());
    }

    match fetch_latest_release(Duration::from_secs(5)).await {
        Ok(latest) => {
            let update_available = is_newer_than_current(&latest.version);
            print_report(
                VersionReport {
                    current_version: CURRENT_VERSION,
                    latest_version: Some(&latest.version),
                    status: if update_available {
                        "update-available"
                    } else {
                        "up-to-date"
                    },
                    update_available,
                    release_url: latest.html_url.as_deref(),
                    upgrade_command: update_available.then(upgrade_command_for_report).flatten(),
                    error: None,
                },
                json,
            )?;
        }
        Err(error) => {
            let error = error.to_string();
            print_report(
                VersionReport {
                    current_version: CURRENT_VERSION,
                    latest_version: None,
                    status: "unknown",
                    update_available: false,
                    release_url: None,
                    upgrade_command: None,
                    error: Some(&error),
                },
                json,
            )?;
        }
    }

    Ok(())
}

pub async fn run_update_command() -> Result<()> {
    if cfg!(target_os = "windows") {
        anyhow::bail!(
            "socai update does not replace a running Windows socai.exe yet; rerun the installer instead: {WINDOWS_INSTALL_COMMAND}"
        );
    }
    if !managed_update_supported() {
        anyhow::bail!(
            "socai update currently supports macOS release-binary installs only; see https://github.com/socai-io/socai#cli for the source/Cargo fallback"
        );
    }

    let latest = fetch_latest_release(Duration::from_secs(10)).await?;
    if !is_newer_than_current(&latest.version) {
        println!("socai {CURRENT_VERSION} is already up to date");
        return Ok(());
    }

    let install_dir =
        managed_install_dir().context("could not determine the managed socai install directory")?;
    let install_path = managed_install_path(&install_dir);
    ensure_current_exe_is_managed(&install_path)?;

    println!(
        "updating socai from {current} to {latest}",
        current = CURRENT_VERSION,
        latest = latest.version
    );

    let tmp_dir = make_update_temp_dir()?;
    let installer_path = tmp_dir.join(installer_file_name());
    let result: Result<()> = async {
        download_installer(&installer_path).await?;
        run_installer(&installer_path, &install_dir).await?;
        verify_installed_version(&install_path, &latest.version).await?;
        stop_existing_daemon().await;
        Ok(())
    }
    .await;
    let _ = fs::remove_dir_all(&tmp_dir);
    result?;

    let _ = clear_cache();
    Ok(())
}

pub async fn maybe_warn_if_outdated() {
    if update_check_disabled() {
        return;
    }

    let now = unix_time_secs();
    let mut cache = load_cache().unwrap_or_default();

    let latest = if cache_success_is_fresh(&cache, now) {
        cache.latest_version.clone().map(|version| LatestRelease {
            version,
            html_url: cache.latest_url.clone(),
        })
    } else if cache_error_is_fresh(&cache, now) {
        None
    } else {
        match fetch_latest_release(Duration::from_millis(1500)).await {
            Ok(latest) => {
                cache.checked_at = Some(now);
                cache.latest_version = Some(latest.version.clone());
                cache.latest_url = latest.html_url.clone();
                cache.last_error_at = None;
                let _ = save_cache(&cache);
                Some(latest)
            }
            Err(_) => {
                cache.last_error_at = Some(now);
                let _ = save_cache(&cache);
                None
            }
        }
    };

    let Some(latest) = latest else {
        return;
    };

    if !is_newer_than_current(&latest.version) {
        return;
    }

    if already_notified(&cache, &latest.version, now) {
        return;
    }

    warn_yellow(&format!(
        "socai {current} is outdated; latest is {latest}. {instruction}",
        current = CURRENT_VERSION,
        latest = latest.version,
        instruction = upgrade_instruction()
    ));

    cache.notified_at = Some(now);
    cache.notified_version = Some(latest.version);
    let _ = save_cache(&cache);
}

fn print_report(report: VersionReport<'_>, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("socai {}", report.current_version);
    if let Some(latest) = report.latest_version {
        println!("latest {latest}");
    } else {
        println!("latest unknown");
    }
    println!("status {}", report.status);
    if let Some(url) = report.release_url {
        println!("release {url}");
    }
    if let Some(command) = report.upgrade_command.as_deref() {
        println!("upgrade {command}");
    }
    if let Some(error) = report.error {
        eprintln!("could not check latest socai release: {error}");
    }
    Ok(())
}

async fn fetch_latest_release(timeout: Duration) -> Result<LatestRelease> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("socai/{CURRENT_VERSION}"))
        .build()
        .context("failed to create update-check HTTP client")?;

    let response = client
        .head(LATEST_RELEASE_URL)
        .send()
        .await
        .context("failed to request latest GitHub release")?;

    if !response.status().is_redirection() {
        response
            .error_for_status_ref()
            .context("latest GitHub release request failed")?;
    }

    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("latest GitHub release redirect missing Location"))?;
    let tag_name = release_tag_from_location(location)
        .ok_or_else(|| anyhow::anyhow!("latest GitHub release redirect missing tag: {location}"))?;
    let version = normalize_tag_version(tag_name)
        .ok_or_else(|| anyhow::anyhow!("latest release tag is not strict semver: {tag_name}"))?;

    Ok(LatestRelease {
        version,
        html_url: Some(absolute_github_url(location)),
    })
}

fn release_tag_from_location(location: &str) -> Option<&str> {
    location.trim_end_matches('/').rsplit('/').next()
}

fn absolute_github_url(location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        location.to_string()
    } else {
        format!("https://github.com{location}")
    }
}

fn managed_install_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SOCAI_INSTALL_DIR") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".socai").join("bin"))
}

fn managed_install_path(install_dir: &Path) -> PathBuf {
    install_dir.join(managed_binary_name())
}

fn managed_binary_name() -> &'static str {
    if cfg!(windows) {
        "socai.exe"
    } else {
        "socai"
    }
}

fn managed_update_supported() -> bool {
    cfg!(target_os = "macos")
}

fn ensure_current_exe_is_managed(install_path: &Path) -> Result<()> {
    let current_exe =
        std::env::current_exe().context("failed to resolve current socai executable")?;
    let current = fs::canonicalize(&current_exe).unwrap_or(current_exe.clone());
    let expected = fs::canonicalize(install_path).unwrap_or_else(|_| install_path.to_path_buf());
    if current != expected {
        anyhow::bail!(
            "socai update only manages the release-binary install at {}; current executable is {}. For source/Cargo installs, update from the repo with `cargo install --path cli --force`.",
            install_path.display(),
            current_exe.display()
        );
    }
    Ok(())
}

fn make_update_temp_dir() -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "socai-update-{}-{}",
        std::process::id(),
        unix_time_secs()
    ));
    fs::create_dir_all(&path)?;
    Ok(path)
}

async fn download_installer(installer_path: &Path) -> Result<()> {
    let url = installer_url();
    println!("downloading installer: {url}");
    let bytes = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(format!("socai/{CURRENT_VERSION}"))
        .build()?
        .get(url)
        .send()
        .await
        .context("failed to download socai installer")?
        .error_for_status()
        .context("socai installer download failed")?
        .bytes()
        .await
        .context("failed to read socai installer download")?;
    fs::write(installer_path, bytes).context("failed to write socai installer")?;
    Ok(())
}

async fn run_installer(installer_path: &Path, install_dir: &Path) -> Result<()> {
    println!("installing to {}", install_dir.display());
    let mut command = installer_command(installer_path);
    let status = command
        .env("SOCAI_INSTALL_DIR", install_dir)
        .status()
        .await
        .context("failed to run socai installer")?;
    if !status.success() {
        anyhow::bail!("socai installer failed with status {status}");
    }
    Ok(())
}

fn installer_file_name() -> &'static str {
    "install.sh"
}

fn installer_url() -> &'static str {
    MACOS_INSTALL_SCRIPT_URL
}

fn installer_command(installer_path: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("sh");
    command.arg(installer_path);
    command
}

async fn stop_existing_daemon() {
    if matches!(crate::daemon::stop_daemon().await, Ok(true)) {
        println!("stopped existing socai daemon; it will restart on the next command");
    }
}

async fn verify_installed_version(install_path: &Path, latest_version: &str) -> Result<()> {
    let output = tokio::process::Command::new(install_path)
        .arg("--version")
        .output()
        .await
        .with_context(|| format!("failed to run updated socai at {}", install_path.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "updated socai --version failed with status {}",
            output.status
        );
    }
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let expected = format!("socai {latest_version}");
    if actual != expected {
        anyhow::bail!("updated socai version mismatch: expected {expected}, got {actual}");
    }
    println!("updated {actual}");
    Ok(())
}

fn normalize_tag_version(tag: &str) -> Option<String> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    parse_strict_version(version).map(|_| version.to_string())
}

fn is_newer_than_current(latest_version: &str) -> bool {
    compare_versions(CURRENT_VERSION, latest_version) == Some(Ordering::Less)
}

fn compare_versions(current: &str, latest: &str) -> Option<Ordering> {
    let current = parse_strict_version(current)?;
    let latest = parse_strict_version(latest)?;
    Some(current.cmp(&latest))
}

fn parse_strict_version(version: &str) -> Option<StrictVersion> {
    let mut parts = version.split('.');
    let major = parse_version_part(parts.next()?)?;
    let minor = parse_version_part(parts.next()?)?;
    let patch = parse_version_part(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(StrictVersion {
        major,
        minor,
        patch,
    })
}

fn parse_version_part(raw: &str) -> Option<u64> {
    if raw.is_empty() || !raw.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

fn update_check_disabled() -> bool {
    ["SOCAI_SKIP_UPDATE_CHECK", "SOCAI_NO_UPDATE_CHECK"]
        .into_iter()
        .any(|name| {
            std::env::var(name).ok().is_some_and(|value| {
                matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES")
            })
        })
}

fn cache_success_is_fresh(cache: &UpdateCache, now: u64) -> bool {
    cache.latest_version.is_some()
        && cache
            .checked_at
            .is_some_and(|checked_at| now.saturating_sub(checked_at) < CHECK_INTERVAL)
}

fn cache_error_is_fresh(cache: &UpdateCache, now: u64) -> bool {
    cache
        .last_error_at
        .is_some_and(|checked_at| now.saturating_sub(checked_at) < FAILED_CHECK_INTERVAL)
}

fn already_notified(cache: &UpdateCache, latest_version: &str, now: u64) -> bool {
    cache.notified_version.as_deref() == Some(latest_version)
        && cache
            .notified_at
            .is_some_and(|notified_at| now.saturating_sub(notified_at) < NOTIFY_INTERVAL)
}

fn unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_cache() -> Option<UpdateCache> {
    let path = cache_path()?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_cache(cache: &UpdateCache) -> Result<()> {
    let Some(path) = cache_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(cache)?)?;
    Ok(())
}

fn clear_cache() -> Result<()> {
    let Some(path) = cache_path() else {
        return Ok(());
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn cache_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SOCAI_UPDATE_CACHE") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".socai").join("update-check.json"))
}

fn suggested_upgrade_command() -> Option<String> {
    if managed_update_supported()
        && managed_install_dir()
            .map(|dir| ensure_current_exe_is_managed(&managed_install_path(&dir)).is_ok())
            .unwrap_or(false)
    {
        Some(UPDATE_COMMAND.into())
    } else {
        None
    }
}

fn upgrade_command_for_report() -> Option<String> {
    suggested_upgrade_command().or_else(|| {
        if cfg!(target_os = "windows") {
            Some(WINDOWS_INSTALL_COMMAND.into())
        } else {
            None
        }
    })
}

fn upgrade_instruction() -> String {
    if let Some(command) = upgrade_command_for_report() {
        format!("Run `{command}` to upgrade.")
    } else if managed_update_supported() {
        "See https://github.com/socai-io/socai#cli for install/update options.".into()
    } else {
        "See https://github.com/socai-io/socai#cli for the source/Cargo fallback.".into()
    }
}

fn warn_yellow(message: &str) {
    if color_stderr() {
        eprintln!("\x1b[93m{message}\x1b[0m");
    } else {
        eprintln!("{message}");
    }
}

fn color_stderr() -> bool {
    std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map_or(true, |term| term != "dumb")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_semver_parser_accepts_three_numeric_parts() {
        assert_eq!(
            parse_strict_version("1.2.3"),
            Some(StrictVersion {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
    }

    #[test]
    fn strict_semver_parser_rejects_non_strict_versions() {
        assert_eq!(parse_strict_version("1.2"), None);
        assert_eq!(parse_strict_version("1.2.3.4"), None);
        assert_eq!(parse_strict_version("1.2.x"), None);
        assert_eq!(parse_strict_version("1.2.3-beta"), None);
        assert_eq!(parse_strict_version("v1.2.3"), None);
    }

    #[test]
    fn release_tags_normalize_to_strict_versions() {
        assert_eq!(normalize_tag_version("v1.2.3"), Some("1.2.3".into()));
        assert_eq!(normalize_tag_version("1.2.3"), Some("1.2.3".into()));
        assert_eq!(normalize_tag_version("v1.2.3-beta"), None);
    }

    #[test]
    fn release_redirect_location_yields_tag() {
        assert_eq!(
            release_tag_from_location("https://github.com/socai-io/socai/releases/tag/v1.2.3"),
            Some("v1.2.3")
        );
        assert_eq!(
            release_tag_from_location("/socai-io/socai/releases/tag/v1.2.3/"),
            Some("v1.2.3")
        );
    }

    #[test]
    fn version_compare_detects_newer_latest_version() {
        assert_eq!(compare_versions("1.2.3", "1.2.4"), Some(Ordering::Less));
        assert_eq!(compare_versions("1.2.3", "1.2.3"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.2.3", "1.2.2"), Some(Ordering::Greater));
        assert_eq!(compare_versions("1.2", "1.2.3"), None);
    }

    #[test]
    fn notification_is_throttled_per_latest_version() {
        let cache = UpdateCache {
            notified_at: Some(100),
            notified_version: Some("1.2.4".into()),
            ..UpdateCache::default()
        };
        assert!(already_notified(&cache, "1.2.4", 100 + NOTIFY_INTERVAL - 1));
        assert!(!already_notified(&cache, "1.2.4", 100 + NOTIFY_INTERVAL));
        assert!(!already_notified(&cache, "1.2.5", 100 + 1));
    }
}
