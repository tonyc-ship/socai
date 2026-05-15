//! Interactive socai TUI — invoked by `socai tui` or by running `socai` with
//! no arguments. Mirrors `socai/cli/repl.py`: slash completion, inline model
//! picker, command history, Ctrl-C exit, Esc-cancellable sub-menus.

use std::borrow::Cow;
use std::io::{self, IsTerminal};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use inquire::{Select, Text};
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Cmd, CompletionType, Config, Editor, EventHandler, Helper, KeyEvent, Modifiers};
use socai_agent::{
    config_for, configured_default_model_for, load_api_key, resolve_provider, save_api_key,
    save_default_model, AgentEvent, AgentOptions, AnthropicBackend, Backend, OpenAICompatBackend,
    Provider, Tool, PROVIDERS,
};
use socai_runtime::{BrowserStatus, SocaiRuntime};
use socai_sites::xhs::{xhs_tools_with_llm_provider, XhsSiteRuntime, XHS_AGENT_HINT, XHS_HOME_URL};
use tokio::time::{sleep, Instant};

const DEFAULT_MAX_TURNS: u32 = 30;
const DEFAULT_MAX_TOKENS: u32 = 4096;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(90);
const PROVIDER_ORDER: &[Provider] = &[
    Provider::Kimi,
    Provider::Qwen,
    Provider::OpenAI,
    Provider::Anthropic,
];

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("model", "Choose the active LLM model"),
    ("exit", "Exit the TUI"),
];

const TUI_AGENT_INSTRUCTIONS: &str = "You are running inside the Socai TUI. \
The browser is locked to Xiaohongshu (xiaohongshu.com / 小红书); every task is \
an XHS task. Do not navigate to other websites. Use the XHS tools to search, \
scan topics, read notes, and inspect page state. Prefer topic_scan for broad \
research tasks. Reply in the same language as the user's task and ground your \
answer in tool output.";

#[derive(Default)]
struct AppState {
    model: Option<String>,
}

// ---------- rustyline helper: slash completion -----------------------------

struct SocaiHelper;

impl Completer for SocaiHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        _pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        if !line.starts_with('/') {
            return Ok((0, Vec::new()));
        }
        let first = line.split_whitespace().next().unwrap_or("");
        let prefix = first.trim_start_matches('/');
        let mut out = Vec::new();
        for (name, desc) in SLASH_COMMANDS {
            if name.starts_with(prefix) {
                out.push(Pair {
                    display: format!("/{name}  — {desc}"),
                    replacement: format!("/{name}"),
                });
            }
        }
        Ok((0, out))
    }
}

impl Hinter for SocaiHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        // Inline ghost suggestion for slash commands — only when cursor is at
        // the end of a single-line buffer that starts with '/'.
        if pos != line.len() || !line.starts_with('/') || line.contains(' ') {
            return None;
        }
        let prefix = line.trim_start_matches('/');
        for (name, desc) in SLASH_COMMANDS {
            if name.starts_with(prefix) && prefix.len() < name.len() {
                let rest = &name[prefix.len()..];
                return Some(format!("{rest}  — {desc}"));
            }
        }
        None
    }
}
impl Highlighter for SocaiHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Borrowed(prompt)
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // ANSI bright-black (dim) so the ghost suggestion is visible but muted.
        Cow::Owned(format!("\x1b[90m{hint}\x1b[0m"))
    }
}
impl Validator for SocaiHelper {}
impl Helper for SocaiHelper {}

// ---------- entry point ----------------------------------------------------

pub async fn run() -> Result<()> {
    ensure_any_llm_key().await?;

    let runtime = SocaiRuntime::new();
    let mut state = AppState::default();

    let mut editor = build_editor()?;
    print_header(&state)?;

    loop {
        let res = tokio::task::block_in_place(|| editor.readline("socai> "));
        let line = match res {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                println!();
                break;
            }
            Err(err) => {
                eprintln!("[socai] read error: {err}");
                break;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(trimmed);

        if trimmed.starts_with('/') {
            if handle_command(trimmed, &mut state).await? {
                break;
            }
            print_header(&state)?;
            continue;
        }

        if let Err(err) = run_agent_task(&runtime, trimmed, state.model.as_deref()).await {
            eprintln!("[socai] error: {err:#}");
        }
    }

    runtime.disconnect_browser().await;
    Ok(())
}

fn build_editor() -> Result<Editor<SocaiHelper, DefaultHistory>> {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .auto_add_history(false)
        .build();
    let mut editor: Editor<SocaiHelper, DefaultHistory> = Editor::with_config(config)?;
    editor.set_helper(Some(SocaiHelper));
    // Soft-newline bindings for multi-line input. Rustyline can only see
    // what the terminal actually sends, so coverage depends on terminal
    // settings:
    //   • Alt+Enter / Option+Enter — always works (terminals send `Esc \r`).
    //   • Shift+Enter — only if the terminal reports modifiers (WezTerm,
    //     Ghostty, Alacritty by default; iTerm2 with "Report modifiers" on).
    //   • Ctrl+Enter via Ctrl-J — only if the terminal is configured to send
    //     `^J` for Ctrl+Return (e.g. iTerm2 → Profile → Keys mapping).
    for ch in ['\r', '\n'] {
        editor.bind_sequence(
            KeyEvent::new(ch, Modifiers::ALT),
            EventHandler::Simple(Cmd::Newline),
        );
        editor.bind_sequence(
            KeyEvent::new(ch, Modifiers::SHIFT),
            EventHandler::Simple(Cmd::Newline),
        );
    }
    editor.bind_sequence(KeyEvent::ctrl('j'), EventHandler::Simple(Cmd::Newline));
    Ok(editor)
}

fn print_header(state: &AppState) -> Result<()> {
    println!("Current model: {}", active_model(state)?);
    println!(
        "Enter your task description. Type / for commands (Tab to list). \
         Alt+Enter for newline (Shift/Ctrl+Enter if your terminal supports it). \
         Ctrl+C to exit."
    );
    Ok(())
}

// ---------- slash command dispatch -----------------------------------------

async fn handle_command(line: &str, state: &mut AppState) -> Result<bool> {
    let body = line.trim_start_matches('/').trim();
    let (cmd, rest) = body
        .split_once(char::is_whitespace)
        .map(|(c, r)| (c, r.trim()))
        .unwrap_or((body, ""));
    match cmd.to_ascii_lowercase().as_str() {
        "exit" | "quit" => Ok(true),
        "model" => {
            handle_model_command(state, rest).await?;
            Ok(false)
        }
        "" => Ok(false),
        other => {
            eprintln!("[socai] unknown command: /{other}");
            Ok(false)
        }
    }
}

// ---------- model picker ---------------------------------------------------

#[derive(Clone)]
struct ModelOption {
    label: String,
    provider: Provider,
    model: String,
}

fn model_options() -> Vec<ModelOption> {
    PROVIDER_ORDER
        .iter()
        .map(|provider| {
            let cfg = config_for(*provider);
            let model = configured_default_model_for(*provider);
            let key_status = if load_api_key(*provider).is_some() {
                "key"
            } else {
                "no key"
            };
            let label = format!("{} — {} ({})", cfg.display_name, model, key_status);
            ModelOption {
                label,
                provider: *provider,
                model,
            }
        })
        .collect()
}

async fn handle_model_command(state: &mut AppState, rest: &str) -> Result<()> {
    if !rest.trim().is_empty() {
        let provider = resolve_provider(None, Some(rest))?;
        let model = if Provider::from_name(rest).is_some() {
            configured_default_model_for(provider)
        } else {
            rest.trim().to_string()
        };
        set_active_model(state, provider, model).await?;
        return Ok(());
    }

    let current = active_model(state)?;
    let options = model_options();
    let starting_cursor = options.iter().position(|o| o.model == current).unwrap_or(0);
    let labels: Vec<String> = options.iter().map(|o| o.label.clone()).collect();

    let chosen_label = tokio::task::spawn_blocking(move || {
        Select::new("Select model", labels)
            .with_starting_cursor(starting_cursor)
            .with_help_message("↑/↓ to move · enter to confirm · esc to cancel")
            .prompt_skippable()
    })
    .await
    .context("model picker thread panicked")??;

    let Some(chosen_label) = chosen_label else {
        return Ok(()); // Esc — fall back to main prompt
    };

    let chosen = options
        .into_iter()
        .find(|o| o.label == chosen_label)
        .context("model picker lost its row")?;

    set_active_model(state, chosen.provider, chosen.model).await
}

async fn set_active_model(state: &mut AppState, provider: Provider, model: String) -> Result<()> {
    if load_api_key(provider).is_none() && !prompt_save_key(provider).await? {
        println!("[socai] model unchanged.");
        return Ok(());
    }

    let path = save_default_model(provider, &model)?;
    state.model = Some(model.clone());
    println!(
        "[socai] model set to {model}; saved defaults to {}",
        path.display()
    );
    Ok(())
}

// ---------- API key prompts (Esc-cancellable) ------------------------------

async fn prompt_save_key(provider: Provider) -> Result<bool> {
    let cfg = config_for(provider);
    let env_hint = cfg.env_keys.join(" or ");
    println!(
        "[socai] No API key found for {}. Enter one now (esc to cancel; you can also set {}).",
        cfg.display_name, env_hint
    );
    let label = cfg.display_name.to_string();
    let provider_for_save = provider;

    tokio::task::spawn_blocking(move || -> Result<bool> {
        let key = match Text::new(&format!("{label} API key:")).prompt_skippable()? {
            Some(k) => k,
            None => return Ok(false),
        };
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        let path = save_api_key(provider_for_save, trimmed)?;
        println!("[socai] Saved {label} key to {}", path.display());
        Ok(true)
    })
    .await
    .context("API key prompt thread panicked")?
}

async fn ensure_any_llm_key() -> Result<()> {
    if PROVIDERS
        .iter()
        .any(|cfg| load_api_key(cfg.provider).is_some())
    {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        anyhow::bail!(
            "No LLM API key found. Set OPENAI_API_KEY / ANTHROPIC_API_KEY / \
             KIMI_API_KEY / QWEN_API_KEY or run `socai tui` in a terminal to save one."
        );
    }

    println!("No LLM API key found. Choose a provider to configure.");
    let options: Vec<(String, Provider)> = PROVIDER_ORDER
        .iter()
        .map(|p| {
            let cfg = config_for(*p);
            (format!("{} ({})", cfg.display_name, cfg.env_keys[0]), *p)
        })
        .collect();
    let labels: Vec<String> = options.iter().map(|(l, _)| l.clone()).collect();

    let chosen_label = tokio::task::spawn_blocking(move || {
        Select::new("Provider", labels)
            .with_help_message("↑/↓ to move · enter to confirm · esc to cancel")
            .prompt_skippable()
    })
    .await
    .context("provider picker thread panicked")??;

    let Some(chosen_label) = chosen_label else {
        anyhow::bail!("cancelled");
    };
    let provider = options
        .into_iter()
        .find(|(l, _)| *l == chosen_label)
        .map(|(_, p)| p)
        .context("provider picker lost its row")?;

    if !prompt_save_key(provider).await? {
        anyhow::bail!("cancelled");
    }
    Ok(())
}

async fn ensure_model_key(model: &str) -> Result<()> {
    let provider = resolve_provider(None, Some(model))?;
    if load_api_key(provider).is_some() {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        let cfg = config_for(provider);
        anyhow::bail!("missing API key for {}", cfg.display_name);
    }
    if !prompt_save_key(provider).await? {
        anyhow::bail!("cancelled");
    }
    Ok(())
}

// ---------- agent task runner ----------------------------------------------

fn active_model(state: &AppState) -> Result<String> {
    if let Some(model) = &state.model {
        if !model.trim().is_empty() {
            return Ok(model.clone());
        }
    }
    if let Some(model) = env_model() {
        return Ok(model);
    }
    let provider = resolve_provider(None, None)?;
    Ok(configured_default_model_for(provider))
}

async fn run_agent_task(runtime: &SocaiRuntime, task: &str, model: Option<&str>) -> Result<()> {
    let llm_provider = build_backend(model).await?;
    println!();
    println!("[socai] using {}", llm_provider.label());
    println!("[socai] connecting browser...");

    connect_runtime(runtime).await?;
    let page = runtime.create_task("about:blank").await?;
    page.navigate_with_timeout(XHS_HOME_URL, 60.0).await?;
    XhsSiteRuntime::new(&page).ensure_xhs(false).await?;
    let page = Arc::new(page);
    let tools: Vec<Arc<dyn Tool>> =
        xhs_tools_with_llm_provider(page.clone(), Some(llm_provider.clone()));

    let options = AgentOptions {
        max_turns: DEFAULT_MAX_TURNS,
        max_tokens: DEFAULT_MAX_TOKENS,
        extra_instructions: format!("{TUI_AGENT_INSTRUCTIONS}\n\n{XHS_AGENT_HINT}"),
        run_dir: None,
        enabled_sites: vec!["xhs".to_string()],
        keep_recent_messages: 12,
        memory_max_chars: 6000,
    };

    let (tx, mut rx) = tokio::sync::broadcast::channel::<AgentEvent>(256);
    let printer = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            print_agent_event(&event);
        }
    });

    let outcome = socai_agent::run_agent_with_events(task, llm_provider, tools, options, tx).await;
    let _ = printer.await;

    let close_result = match Arc::try_unwrap(page) {
        Ok(page) => page.close().await,
        Err(_) => Ok(()),
    };

    let outcome = outcome?;
    close_result.context("close task tab")?;

    println!();
    println!("{}", outcome.final_text.trim());
    println!();
    println!(
        "[socai] run_id={} turns={} input_tokens={} output_tokens={}",
        outcome.run_id, outcome.turns, outcome.total_input_tokens, outcome.total_output_tokens
    );
    println!("[socai] run_dir={}", outcome.run_dir.display());
    println!();
    Ok(())
}

async fn build_backend(model: Option<&str>) -> Result<Arc<dyn Backend>> {
    let provider = resolve_provider(None, model)?;
    let effective = model
        .map(str::to_string)
        .filter(|m| !m.trim().is_empty())
        .or_else(env_model)
        .unwrap_or_else(|| configured_default_model_for(provider));
    ensure_model_key(&effective).await?;
    let backend: Arc<dyn Backend> = match provider {
        Provider::Anthropic => Arc::new(AnthropicBackend::new(effective)?),
        other => Arc::new(OpenAICompatBackend::new(other, effective)?),
    };
    Ok(backend)
}

fn env_model() -> Option<String> {
    std::env::var("SOCAI_MODEL")
        .ok()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
}

async fn connect_runtime(runtime: &SocaiRuntime) -> Result<()> {
    runtime.connect_browser();
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match runtime.browser_status().await {
            BrowserStatus::Connected { .. } => return Ok(()),
            BrowserStatus::Disconnected { reason } if reason != "not_yet_connected" => {
                return Err(anyhow::anyhow!("CDP disconnected: {reason}"));
            }
            BrowserStatus::Disconnected { .. } | BrowserStatus::Connecting { .. } => {}
        }
        if Instant::now() >= deadline {
            return Err(anyhow::anyhow!(
                "CDP did not connect within {:?}",
                CONNECT_TIMEOUT
            ));
        }
        sleep(Duration::from_millis(250)).await;
    }
}

// ---------- event rendering (matches Python prefix style) ------------------

fn print_agent_event(event: &AgentEvent) {
    match event {
        AgentEvent::Started {
            run_id,
            task,
            model,
        } => {
            println!("\n▸ task: {task}");
            println!("  run {run_id} · model {model}");
        }
        AgentEvent::Turn { turn } => println!("\n── turn {turn} ──"),
        AgentEvent::AssistantText { text, .. } => {
            for line in text.lines() {
                println!("  {line}");
            }
        }
        AgentEvent::Reasoning { text, .. } => {
            for line in text.lines() {
                let trimmed = line.trim_end();
                if !trimmed.is_empty() {
                    println!("  · {trimmed}");
                }
            }
        }
        AgentEvent::ToolCall {
            name,
            input,
            repeat_count,
            ..
        } => {
            let mut preview = serde_json::to_string(input).unwrap_or_else(|_| input.to_string());
            if preview.chars().count() > 180 {
                preview = preview.chars().take(180).collect::<String>() + "...";
            }
            if *repeat_count > 1 {
                println!("  → {name}({preview}) repeat={repeat_count}");
            } else {
                println!("  → {name}({preview})");
            }
        }
        AgentEvent::ToolResult {
            name,
            summary,
            duration_ms,
            error,
            ..
        } => {
            if let Some(error) = error {
                println!("  ✗ {name} ({duration_ms}ms): {error}");
            } else {
                let first = summary.lines().next().unwrap_or("");
                println!("  ← {name} ({duration_ms}ms): {first}");
            }
        }
        AgentEvent::ApiError { turn, message } => {
            println!("  ✗ API error on turn {turn}: {message}");
        }
        AgentEvent::Done { turns, .. } => println!("\n✓ done in {turns} turns"),
    }
}
