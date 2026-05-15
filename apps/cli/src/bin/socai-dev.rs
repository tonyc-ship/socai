//! Internal smoke-test runner. Not exposed to end users — used to verify
//! provider connectivity, agent-loop dispatch, and browser state during
//! development. Kept out of the user-facing `socai` CLI so that surface
//! mirrors the Python entry points exactly.
//!
//!     cargo run --bin socai-dev -- providers
//!     cargo run --bin socai-dev -- llm --provider kimi "hi"
//!     cargo run --bin socai-dev -- agent --provider anthropic --no-browser "echo hi"
//!     cargo run --bin socai-dev -- browser-status
//!     cargo run --bin socai-dev -- page-state <url>

use std::sync::Arc;

use clap::{Parser, Subcommand};
use socai_agent::{
    run_agent_with_events, AgentEvent, AgentOptions, AnthropicBackend, Backend, EchoTool,
    OpenAICompatBackend, Provider, Tool,
};
use socai_runtime::SocaiRuntime;
use socai_sites::xhs::{xhs_tools_with_llm_provider, XhsSiteRuntime, XHS_AGENT_HINT, XHS_HOME_URL};

#[derive(Debug, Parser)]
#[command(name = "socai-dev")]
#[command(about = "socai internal smoke tests")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List which LLM providers have a usable API key on this machine.
    Providers,
    /// Send a one-shot prompt to a provider — no tools, no agent loop.
    Llm {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        prompt: String,
        #[arg(long, default_value_t = 1024)]
        max_tokens: u32,
    },
    /// Run the agent loop. With `--no-browser`, only the echo tool is
    /// registered (handy for verifying tool dispatch without opening Chrome).
    Agent {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value_t = 12)]
        max_turns: u32,
        #[arg(long, default_value_t = false)]
        no_browser: bool,
        task: String,
    },
    /// Print the current Cdp status (connected, endpoint, browser version).
    BrowserStatus,
    /// Open `url` and print the XHS page-state snapshot as JSON.
    PageState {
        #[arg(default_value = XHS_HOME_URL)]
        url: String,
    },
    /// Save an API key to ~/.socai/auth.json (chmod 600).
    Auth {
        #[arg(long)]
        provider: String,
        /// The key to save. Reads from stdin when omitted.
        #[arg(long)]
        key: Option<String>,
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
    match args.command {
        Command::Providers => print_providers(),
        Command::Llm {
            provider,
            model,
            prompt,
            max_tokens,
        } => run_llm_smoke(provider, model, prompt, max_tokens).await,
        Command::Agent {
            provider,
            model,
            max_turns,
            no_browser,
            task,
        } => run_agent_cmd(provider, model, max_turns, no_browser, task).await,
        Command::BrowserStatus => run_browser_status().await,
        Command::PageState { url } => run_page_state(url).await,
        Command::Auth { provider, key } => run_auth(provider, key),
    }
}

fn run_auth(provider: String, key: Option<String>) -> anyhow::Result<()> {
    let provider_enum = Provider::from_name(&provider)
        .ok_or_else(|| anyhow::anyhow!("unknown provider: {provider}"))?;
    let secret = match key {
        Some(value) => value,
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf.trim().to_string()
        }
    };
    let path = socai_agent::save_api_key(provider_enum, &secret)?;
    println!("saved {} key to {}", provider, path.display());
    Ok(())
}

fn print_providers() -> anyhow::Result<()> {
    let available = socai_agent::list_available_providers();
    for cfg in socai_agent::PROVIDERS {
        let has = available.contains(&cfg.provider);
        println!(
            "{:<11} {} (default: {})",
            cfg.provider.as_str(),
            if has { "✓ key found" } else { "✗ no key" },
            cfg.default_model
        );
    }
    Ok(())
}

fn build_backend(provider: Option<&str>, model: Option<&str>) -> anyhow::Result<Arc<dyn Backend>> {
    let resolved = socai_agent::resolve_provider(provider, model)?;
    let model_str = model.unwrap_or("").to_string();
    let backend: Arc<dyn Backend> = match resolved {
        Provider::Anthropic => Arc::new(AnthropicBackend::new(model_str)?),
        other => Arc::new(OpenAICompatBackend::new(other, model_str)?),
    };
    Ok(backend)
}

async fn run_llm_smoke(
    provider: Option<String>,
    model: Option<String>,
    prompt: String,
    max_tokens: u32,
) -> anyhow::Result<()> {
    let backend = build_backend(provider.as_deref(), model.as_deref())?;
    println!("// using {}", backend.label());
    let messages = [socai_agent::Message::user(prompt)];
    let response = backend
        .send(
            "You are a terse assistant. Answer in one sentence.",
            &messages,
            &[],
            max_tokens,
        )
        .await?;
    for text in response.text_blocks {
        println!("{text}");
    }
    eprintln!(
        "// usage: input={} output={}",
        response.input_tokens, response.output_tokens
    );
    Ok(())
}

async fn run_agent_cmd(
    provider: Option<String>,
    model: Option<String>,
    max_turns: u32,
    no_browser: bool,
    task: String,
) -> anyhow::Result<()> {
    let llm_provider = build_backend(provider.as_deref(), model.as_deref())?;
    println!("// using {}", llm_provider.label());

    let runtime = SocaiRuntime::new();
    let (tools, page_to_close, extra_instructions) = if no_browser {
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(EchoTool)];
        (tools, None, String::new())
    } else {
        runtime.connect_browser();
        runtime.wait_browser_connected().await?;
        let page = runtime.create_task("about:blank").await?;
        let _ = page.navigate_with_timeout(XHS_HOME_URL, 60.0).await;
        let _ = XhsSiteRuntime::new(&page).ensure_xhs(false).await;
        let page = Arc::new(page);
        let tools = xhs_tools_with_llm_provider(page.clone(), Some(llm_provider.clone()));
        (tools, Some(page), XHS_AGENT_HINT.to_string())
    };

    let options = AgentOptions {
        max_turns,
        max_tokens: 4096,
        extra_instructions,
        run_dir: None,
        enabled_sites: if no_browser {
            Vec::new()
        } else {
            vec!["xhs".into()]
        },
        keep_recent_messages: 12,
        memory_max_chars: 6000,
    };

    let (tx, mut rx) = tokio::sync::broadcast::channel::<AgentEvent>(256);
    let printer = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            print_event(&event);
        }
    });

    let outcome = run_agent_with_events(&task, llm_provider, tools, options, tx).await?;
    let _ = printer.await;

    if let Some(page) = page_to_close {
        if let Ok(page) = Arc::try_unwrap(page) {
            let _ = page.close().await;
        }
    }

    eprintln!(
        "\n// run_id={} turns={} input_tokens={} output_tokens={}",
        outcome.run_id, outcome.turns, outcome.total_input_tokens, outcome.total_output_tokens
    );
    eprintln!("// run_dir={}", outcome.run_dir.display());
    Ok(())
}

fn print_event(event: &AgentEvent) {
    match event {
        AgentEvent::Started {
            run_id,
            task,
            model,
        } => {
            println!("┌─ run {run_id} ({model})");
            println!("│ task: {task}");
        }
        AgentEvent::Turn { turn } => {
            println!("├─ turn {turn}");
        }
        AgentEvent::AssistantText { text, .. } => {
            for line in text.lines() {
                println!("│  {line}");
            }
        }
        AgentEvent::Reasoning { text, .. } => {
            let preview = text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>();
            if !preview.is_empty() {
                println!("│  ⤷ reasoning: {preview}…");
            }
        }
        AgentEvent::ToolCall { name, input, .. } => {
            let preview = serde_json::to_string(input).unwrap_or_else(|_| input.to_string());
            let preview = if preview.len() > 200 {
                format!("{}…", &preview[..200])
            } else {
                preview
            };
            println!("│  ▸ {name}({preview})");
        }
        AgentEvent::ToolResult {
            name,
            summary,
            duration_ms,
            error,
            ..
        } => {
            if let Some(err) = error {
                println!("│  ✗ {name} ({duration_ms}ms): {err}");
            } else {
                let first = summary.lines().next().unwrap_or("");
                println!("│  ✓ {name} ({duration_ms}ms): {first}");
            }
        }
        AgentEvent::ApiError { turn, message } => {
            println!("│  ! turn {turn} api error: {message}");
        }
        AgentEvent::Done { turns, .. } => {
            println!("└─ done in {turns} turns");
        }
    }
}

async fn run_browser_status() -> anyhow::Result<()> {
    let runtime = SocaiRuntime::new();
    runtime.connect_browser();
    runtime.wait_browser_connected().await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&runtime.browser_status().await)?
    );
    Ok(())
}

async fn run_page_state(url: String) -> anyhow::Result<()> {
    let runtime = SocaiRuntime::new();
    runtime.connect_browser();
    runtime.wait_browser_connected().await?;
    let page = runtime.create_task("about:blank").await?;
    page.navigate(&url).await?;
    let result = XhsSiteRuntime::new(&page).detect_state().await;
    let close = page.close().await;
    let result = result?;
    close?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
