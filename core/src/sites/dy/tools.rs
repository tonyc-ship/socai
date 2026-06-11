use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::{make_run_dir, Backend as LlmProvider, Tool, ToolContext, ToolResult};
use crate::cdp::{with_snapshot_recording, PageSession};
use crate::sites::dy::{DyPageRuntime, DY_HOME_URL};
use crate::sites::registry::{
    required_string, ArgKind, BoxFuture, CommandArg, SiteCommand, SiteSpec, SlowWhen,
};

pub const DY_KNOWLEDGE: &str = include_str!("knowledge.md");
const DY_OPEN_TIMEOUT_S: f64 = 300.0;

pub fn dy_tools(page: Arc<PageSession>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(SearchVideosTool { page }) as Arc<dyn Tool>]
}

pub async fn dy_agent_tools(
    page: Arc<PageSession>,
    _llm_provider: Arc<dyn LlmProvider>,
) -> anyhow::Result<Vec<Arc<dyn Tool>>> {
    DyPageRuntime::new(&page).ensure_dy(false).await.ok();
    Ok(dy_tools(page))
}

pub fn dy_agent_instructions(extra: &str) -> String {
    let base = DY_KNOWLEDGE.trim().to_string();
    let extra = extra.trim();
    if extra.is_empty() {
        base
    } else {
        format!("{extra}\n\n{base}")
    }
}

pub static DY_SITE: SiteSpec = SiteSpec {
    id: "dy",
    about: "Douyin (douyin.com)",
    home_url: DY_HOME_URL,
    agent_tools: |page, llm| Box::pin(dy_agent_tools(page, llm)),
    agent_instructions: dy_agent_instructions,
    commands: &[SiteCommand {
        name: "search_videos",
        tool_name: "search_videos",
        about: "Search Douyin and print video result cards as JSON.",
        args: &[
            CommandArg {
                key: "query",
                long: None,
                value_name: "QUERY",
                help: "Search query",
                required: true,
                kind: ArgKind::Str,
            },
            CommandArg {
                key: "num_videos",
                long: Some("num-videos"),
                value_name: "N",
                help: "Auto-scroll search results to collect at least this many videos.",
                required: false,
                kind: ArgKind::Int,
            },
        ],
        slow: SlowWhen::ArgPresent("num_videos"),
        run: run_search_videos,
    }],
};

fn run_search_videos(
    page: Arc<PageSession>,
    args: Value,
    debug_snapshot: bool,
) -> BoxFuture<Value> {
    Box::pin(async move {
        let query = required_string(&args, "query")?;
        let num_videos = args.get("num_videos").and_then(Value::as_i64);
        search_videos_command(page, &query, num_videos, debug_snapshot).await
    })
}

pub async fn search_videos_command(
    page: Arc<PageSession>,
    query: &str,
    num_videos: Option<i64>,
    debug_snapshot: bool,
) -> anyhow::Result<Value> {
    let input = json!({
        "query": trimmed_required(query, "query")?,
        "num_videos": num_videos.unwrap_or(30).max(1),
    });
    let (run_dir, ctx) = command_context("search_videos");
    let _ = std::fs::create_dir_all(&ctx.run_dir);
    let _ = std::fs::write(
        ctx.run_dir.join("command_input.json"),
        serde_json::to_vec_pretty(&json!({
            "command": "search_videos",
            "tool": "search_videos",
            "input": input.clone(),
        }))?,
    );
    let data = with_snapshot_recording(&page, &ctx.run_dir, debug_snapshot, async {
        ensure_search_ready(&page).await?;
        call_dy_tool(page.clone(), "search_videos", input, &ctx).await
    })
    .await?;
    Ok(json!({
        "command": "search_videos",
        "run_dir": run_dir,
        "data": data,
    }))
}

pub async fn ensure_search_ready(page: &PageSession) -> anyhow::Result<()> {
    let runtime = DyPageRuntime::new(page);
    let current_url = runtime.current_url().await.unwrap_or_default();
    if !current_url.contains("douyin.com") {
        page.navigate_with_timeout(DY_HOME_URL, DY_OPEN_TIMEOUT_S)
            .await?;
    }
    Ok(())
}

async fn call_dy_tool(
    page: Arc<PageSession>,
    tool_name: &str,
    input: Value,
    ctx: &ToolContext,
) -> anyhow::Result<Value> {
    let tool = dy_tools(page)
        .into_iter()
        .find(|tool| tool.name() == tool_name)
        .ok_or_else(|| anyhow::anyhow!("dy tool not found: {tool_name}"))?;
    let result = tool.call(input, ctx).await?;
    let text = result.flat_text();
    serde_json::from_str(text.trim()).or_else(|_| Ok(json!({ "raw_reply": text })))
}

fn command_context(label: &str) -> (String, ToolContext) {
    let run_dir = make_run_dir(label);
    let run_id = run_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(label)
        .to_string();
    let ctx = ToolContext::new(run_id, run_dir.clone());
    ctx.enable_site("dy");
    (run_dir.display().to_string(), ctx)
}

fn trimmed_required(value: &str, label: &str) -> anyhow::Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{label} is empty");
    }
    Ok(trimmed.to_string())
}

fn get_str<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str).map(str::trim)
}

fn get_usize(input: &Value, key: &str, default: usize) -> usize {
    input
        .get(key)
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .map(|n| n as usize)
        .unwrap_or(default)
}

fn json_result(value: &Value) -> ToolResult {
    ToolResult::text(serde_json::to_string(value).unwrap_or_else(|_| "{}".into()))
}

pub struct SearchVideosTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for SearchVideosTool {
    fn name(&self) -> &str {
        "search_videos"
    }

    fn description(&self) -> &str {
        "Search Douyin for videos matching `query` and return result cards. \
         This uses the logged-in browser through CDP, starts from douyin.com, \
         and scrolls the result feed when `num_videos` is requested."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "num_videos": {
                    "type": "integer",
                    "description": "Number of videos to collect",
                    "minimum": 1,
                    "default": 30
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let query = get_str(&input, "query")
            .ok_or_else(|| anyhow::anyhow!("missing query"))?
            .to_string();
        let num_videos = get_usize(&input, "num_videos", 30);
        let dy = DyPageRuntime::new(&self.page);
        let value = dy.search_videos(&query, num_videos).await?;
        Ok(json_result(&value))
    }
}
