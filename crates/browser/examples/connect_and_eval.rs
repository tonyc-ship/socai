//! End-to-end smoke test of the crates/browser API.
//!
//! Usage:
//!     cargo run --example connect_and_eval -- [URL] [JS_EXPR]
//!
//! Defaults to navigating https://example.com and evaluating `document.title`.
//! Requires a Chrome instance with remote debugging open (any of the standard
//! discovery paths from `socai_browser::endpoint`).

use std::env;

use socai_browser::{Cdp, PageSessionManager};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // chromiumoxide=off silences upstream `error!` spam from WS messages whose
    // shape chromiumoxide's PDL doesn't model (common with newer Chrome
    // versions — non-fatal, the actual CDP commands still work). Set
    // RUST_LOG explicitly to re-enable.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,chromiumoxide=off".into()),
        )
        .init();

    let url = env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".into());
    let expr = env::args()
        .nth(2)
        .unwrap_or_else(|| "document.title".into());

    let cdp = Cdp::new();
    cdp.connect();
    cdp.wait_connected().await?;
    println!("connected → {:?}", cdp.status().await);

    let tasks = PageSessionManager::new(cdp.clone());
    let page = tasks.create_page("about:blank").await?;
    println!("opened tab → target_id={}", page.target_id());

    page.navigate(&url).await?;
    let info = page.page_info().await?;
    println!("page_info → {}", serde_json::to_string_pretty(&info)?);

    let value = page.evaluate_json(&expr).await?;
    println!("eval result → {}", serde_json::to_string_pretty(&value)?);

    page.close().await?;
    Ok(())
}
