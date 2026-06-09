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
    /// Search Xiaohongshu and print the first results page's note cards as JSON.
    #[command(name = "search_notes")]
    SearchNotes {
        query: String,
        /// Search-result filter as `group=option` (repeatable), e.g.
        /// `--filter publish_time=一天内 --filter note_type=图文`. Groups:
        /// sort, note_type, publish_time, search_scope, distance.
        #[arg(long = "filter", value_name = "GROUP=OPTION")]
        filter: Vec<String>,
        /// Auto-scroll the feed to collect at least this many cards
        /// (titles/likes/covers only — note bodies are not opened). Omit for
        /// the first page only (~19 cards).
        #[arg(long = "num-notes")]
        num_notes: Option<i64>,
        #[arg(long)]
        pretty: bool,
        /// Record DOM + a11y tree + screenshot bundles to <run_dir>/snapshots/
        /// at every page change between tool operations.
        #[arg(long = "debug-snapshot")]
        debug_snapshot: bool,
    },
    /// Run a Xiaohongshu topic scan (note body + top comments per note).
    #[command(name = "topic_scan")]
    TopicScan {
        query: String,
        #[arg(long)]
        tab: Option<String>,
        /// Search-result filter as `group=option` (repeatable), e.g.
        /// `--filter publish_time=一天内 --filter note_type=图文`. Groups:
        /// sort, note_type, publish_time, search_scope, distance.
        #[arg(long = "filter", value_name = "GROUP=OPTION")]
        filter: Vec<String>,
        /// Number of notes to read; scrolls the feed only if the first page
        /// holds fewer.
        #[arg(long = "num-notes")]
        num_notes: Option<i64>,
        #[arg(long)]
        pretty: bool,
        /// Record DOM + a11y tree + screenshot bundles to <run_dir>/snapshots/
        /// at every page change between tool operations.
        #[arg(long = "debug-snapshot")]
        debug_snapshot: bool,
    },
    /// Open a note from the current search/topic page and print the parsed note.
    #[command(name = "extract_note")]
    ExtractNote {
        #[arg(long = "note-id")]
        note_id: String,
        #[arg(long)]
        pretty: bool,
        /// Record DOM + a11y tree + screenshot bundles to <run_dir>/snapshots/
        /// at every page change between tool operations.
        #[arg(long = "debug-snapshot")]
        debug_snapshot: bool,
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
        Command::SearchNotes {
            query,
            filter,
            num_notes,
            pretty,
            debug_snapshot,
        } => {
            let mut input =
                serde_json::json!({ "query": query, "debug_snapshot": debug_snapshot });
            if !filter.is_empty() {
                input["filters"] = Value::Object(parse_filters(&filter)?);
            }
            if let Some(n) = num_notes {
                input["num_notes"] = serde_json::json!(n.max(1));
            }
            // Scrolling for a large `num_notes` can take a while; give it the
            // longer budget rather than the default single-action timeout.
            let timeout = if num_notes.is_some() {
                daemon::LONG_COMMAND_TIMEOUT
            } else {
                daemon::DEFAULT_COMMAND_TIMEOUT
            };
            let result = daemon::send_or_spawn("search_notes", input, timeout).await?;
            print_command_result(&result, pretty)?;
        }
        Command::TopicScan {
            query,
            tab,
            filter,
            num_notes,
            pretty,
            debug_snapshot,
        } => {
            let mut input = serde_json::json!({ "query": query, "debug_snapshot": debug_snapshot });
            if let Some(tab) = tab {
                input["tab_label"] = Value::String(tab);
            }
            if !filter.is_empty() {
                input["filters"] = Value::Object(parse_filters(&filter)?);
            }
            if let Some(n) = num_notes {
                input["num_notes"] = serde_json::json!(n.max(1));
            }

            let result =
                daemon::send_or_spawn("topic_scan", input, daemon::LONG_COMMAND_TIMEOUT).await?;
            print_command_result(&result, pretty)?;
        }
        Command::ExtractNote {
            note_id,
            pretty,
            debug_snapshot,
        } => {
            let result = daemon::send_or_spawn(
                "extract_note",
                serde_json::json!({ "note_id": note_id, "debug_snapshot": debug_snapshot }),
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

/// Parse repeated `--filter group=option` args into a `{group: option}` object.
/// Group/option validity is enforced by the core tool, so this only splits on
/// the first `=` and rejects a missing one.
fn parse_filters(filters: &[String]) -> Result<serde_json::Map<String, Value>> {
    let mut map = serde_json::Map::new();
    for raw in filters {
        let (group, option) = raw
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--filter expects group=option, got: {raw}"))?;
        map.insert(
            group.trim().to_string(),
            Value::String(option.trim().to_string()),
        );
    }
    Ok(map)
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
