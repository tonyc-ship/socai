//! Phase-1 parity smoke test: inject the real XHS page_scripts.js bundle and
//! call one of its functions on a freshly opened tab. Output is JSON on
//! stdout (compact form so `jq -S` can canonicalize it for diffing against
//! the matching Python script at scripts/run_xhs_extractor.py).
//!
//! Usage:
//!     cargo run --example eval_xhs_extractor -p socai-browser -- \
//!         <url> [function=pageState] [arg_json=null]
//!
//! Example:
//!     cargo run --example eval_xhs_extractor -p socai-browser -- \
//!         https://www.xiaohongshu.com pageState

use std::env;
use std::fs;
use std::path::Path;

use serde_json::Value;
use socai_browser::{Cdp, PageSession, PageSessionManager};

const SCRIPTS_RELATIVE_PATH: &str = "crates/sites/src/xhs/page_scripts.js";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,chromiumoxide=off".into()),
        )
        .init();

    let url = env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: eval_xhs_extractor <url> [function] [arg_json]"))?;
    let function = env::args().nth(2).unwrap_or_else(|| "pageState".into());
    let arg_json = env::args().nth(3).unwrap_or_else(|| "null".into());

    let scripts = fs::read_to_string(Path::new(SCRIPTS_RELATIVE_PATH))
        .map_err(|e| anyhow::anyhow!("failed to read {SCRIPTS_RELATIVE_PATH}: {e}"))?;

    let cdp = Cdp::new();
    cdp.connect();
    cdp.wait_connected().await?;

    let tasks = PageSessionManager::new(cdp.clone());
    let page = tasks.create_page("about:blank").await?;
    page.navigate_with_timeout(&url, 20.0).await?;

    let value = eval_xhs_function(&page, &scripts, &function, &arg_json).await?;
    println!("{}", serde_json::to_string_pretty(&value)?);

    page.close().await?;
    Ok(())
}

/// Build the same JS string Python's `xhs_page_script_call` produces and
/// hand it to evaluate_json. The IIFE wrapping is done by evaluate_json's
/// return-detection.
async fn eval_xhs_function(
    page: &PageSession,
    scripts: &str,
    function: &str,
    arg_json: &str,
) -> anyhow::Result<Value> {
    let args = if arg_json == "null" {
        String::new()
    } else {
        // pass arg as a literal JSON value, same as Python's
        // json.dumps(arg, ensure_ascii=False)
        arg_json.to_string()
    };
    let expr = format!(
        "{scripts}\n// SOCAI_XHS_CALL: {function}\nreturn SocaiXhsPageScripts.{function}({args});"
    );
    page.evaluate_json(&expr).await
}
