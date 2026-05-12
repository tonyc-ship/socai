use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Endpoint {
    pub source: String,
    pub browser_ws_url: String,
}

pub fn discover() -> Result<Endpoint, String> {
    if let Ok(url) = env::var("SOCAI_CDP_WS") {
        if !url.is_empty() {
            return Ok(Endpoint {
                source: "SOCAI_CDP_WS".into(),
                browser_ws_url: url,
            });
        }
    }

    for profile in profile_roots() {
        if let Some(endpoint) = endpoint_from_profile(&profile) {
            return Ok(endpoint);
        }
    }

    Err("no running chrome with --remote-debugging-port found. \
         launch chrome with the debug flag, or set SOCAI_CDP_WS."
        .into())
}

fn profile_roots() -> Vec<PathBuf> {
    let Some(home) = home_dir() else { return Vec::new(); };

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
    if let Ok(home) = env::var("HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(profile) = env::var("USERPROFILE") {
            if !profile.is_empty() {
                return Some(PathBuf::from(profile));
            }
        }
        if let (Ok(drive), Ok(path)) = (env::var("HOMEDRIVE"), env::var("HOMEPATH")) {
            if !drive.is_empty() && !path.is_empty() {
                return Some(PathBuf::from(format!("{drive}{path}")));
            }
        }
    }
    None
}

fn endpoint_from_profile(profile: &Path) -> Option<Endpoint> {
    let marker = profile.join("DevToolsActivePort");
    let contents = fs::read_to_string(&marker).ok()?;
    let mut lines = contents.lines();
    let port: u16 = lines.next()?.trim().parse().ok()?;
    let path = lines.next()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(Endpoint {
        source: format!("active_port:{}", marker.display()),
        browser_ws_url: format!("ws://127.0.0.1:{port}{path}"),
    })
}
