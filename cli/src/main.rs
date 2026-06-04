mod daemon;
mod tracking;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::Value;

#[derive(Parser, Debug)]
#[command(name = "socai")]
#[command(about = "socai — XHS-savvy browser agent")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Search Xiaohongshu and print visible note cards as JSON.
    #[command(name = "search_notes")]
    SearchNotes {
        query: String,
        #[arg(long)]
        pretty: bool,
    },
    /// Run the default-depth Xiaohongshu topic scan.
    #[command(name = "topic_scan")]
    TopicScan {
        query: String,
        #[arg(long)]
        tab: Option<String>,
        #[arg(long)]
        pretty: bool,
    },
    /// Open a note from the current search/topic page and print the parsed note.
    #[command(name = "extract_note")]
    ExtractNote {
        #[arg(long = "note-id")]
        note_id: String,
        #[arg(long)]
        pretty: bool,
    },
    /// Stop the background socai rust daemon.
    Stop,
    #[command(name = "__daemon", hide = true)]
    Daemon,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,chromiumoxide=off,hyper=off,reqwest=off".into()),
        )
        .init();

    let args = Args::parse();
    let Some(command) = args.command else {
        tui::run().await?;
        return Ok(());
    };
    match command {
        Command::SearchNotes { query, pretty } => {
            let result = daemon::send_or_spawn(
                "search_notes",
                serde_json::json!({ "query": query }),
                daemon::DEFAULT_COMMAND_TIMEOUT,
            )
            .await?;
            print_command_result(&result, pretty)?;
        }
        Command::TopicScan { query, tab, pretty } => {
            let mut input = serde_json::json!({
                "query": query,
                "depth": "standard"
            });
            if let Some(tab) = tab {
                input["tab_label"] = Value::String(tab);
            }

            let result =
                daemon::send_or_spawn("topic_scan", input, daemon::LONG_COMMAND_TIMEOUT).await?;
            print_command_result(&result, pretty)?;
        }
        Command::ExtractNote { note_id, pretty } => {
            let result = daemon::send_or_spawn(
                "extract_note",
                serde_json::json!({ "note_id": note_id }),
                daemon::DEFAULT_COMMAND_TIMEOUT,
            )
            .await?;
            print_command_result(&result, pretty)?;
        }
        Command::Stop => {
            if daemon::stop_daemon().await? {
                eprintln!("socai rust daemon stopped");
            } else {
                eprintln!("socai rust daemon is not running");
            }
        }
        Command::Daemon => daemon::run_daemon().await?,
    }

    Ok(())
}

fn print_command_result(result: &Value, pretty: bool) -> Result<()> {
    if let Some(run_dir) = result.get("run_dir").and_then(Value::as_str) {
        eprintln!("run_dir: {run_dir}");
    }

    let data = result.get("data").unwrap_or(result);
    if pretty {
        println!("{}", serde_json::to_string_pretty(data)?);
    } else {
        println!("{}", serde_json::to_string(data)?);
    }
    Ok(())
}
