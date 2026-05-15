//! Phase-2 demo: navigate to an XHS note URL and extract structured data.
//!
//! Usage:
//!     cargo run --example extract_note -p socai-sites -- <note_url>
//!
//! Output is the wire JSON that should diff byte-equal (after `jq -S`)
//! against `scripts/run_xhs_extract_note.py` for stable fields.

use std::env;

use socai_browser::{Cdp, PageSessionManager};
use socai_sites::xhs::XhsPageRuntime;

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
        .ok_or_else(|| anyhow::anyhow!("usage: extract_note <note_url>"))?;

    let cdp = Cdp::new();
    cdp.connect();
    cdp.wait_connected().await?;

    let tasks = PageSessionManager::new(cdp.clone());
    let page = tasks.create_page("about:blank").await?;
    page.navigate_with_timeout(&url, 60.0).await?;

    let runtime = XhsPageRuntime::new(&page);
    let note = runtime.extract_note(8.0).await?;

    println!("{}", serde_json::to_string_pretty(&note)?);

    page.close().await?;
    Ok(())
}
