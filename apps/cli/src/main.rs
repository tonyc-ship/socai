use clap::{Parser, Subcommand};
use socai_runtime::SocaiRuntime;
use socai_sites::xhs::XhsSiteRuntime;

#[derive(Debug, Parser)]
#[command(name = "socai")]
#[command(about = "socai CLI — Xiaohongshu research agent")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

/// Subcommands here mirror the Python CLI in `socai/cli/commands.py`.
/// Smoke tests for LLM backends and the agent loop live in the
/// `socai-dev` binary (apps/cli/src/bin/socai-dev.rs) so this surface
/// stays small.
#[derive(Debug, Subcommand)]
enum Command {
    /// Search Xiaohongshu and print the visible note cards as JSON.
    SearchNotes {
        query: String,
        #[arg(long, default_value_t = 2.0)]
        wait_seconds: f64,
    },
    /// Open a Xiaohongshu note URL and print the parsed note body as JSON.
    ExtractNote {
        url: String,
        #[arg(long, default_value_t = 8.0)]
        wait_seconds: f64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,chromiumoxide=off,hyper=off,reqwest=off".into()),
        )
        .init();

    let args = Args::parse();
    let runtime = SocaiRuntime::new();
    runtime.connect_browser();
    runtime.wait_browser_connected().await?;

    match args.command {
        Command::SearchNotes {
            query,
            wait_seconds,
        } => {
            let page = runtime.create_task("about:blank").await?;
            let result = XhsSiteRuntime::new(&page)
                .search_notes(&query, wait_seconds)
                .await;
            let close = page.close().await;
            let result = result?;
            close?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::ExtractNote { url, wait_seconds } => {
            let page = runtime.create_task("about:blank").await?;
            page.navigate_with_timeout(&url, 60.0).await?;
            let note = XhsSiteRuntime::new(&page).extract_note(wait_seconds).await;
            let close = page.close().await;
            let note = note?;
            close?;
            println!("{}", serde_json::to_string_pretty(&note)?);
        }
    }

    Ok(())
}
