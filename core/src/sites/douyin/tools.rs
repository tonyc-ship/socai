use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::{make_run_dir, Backend as LlmProvider, Tool, ToolContext, ToolResult};
use crate::cdp::{with_snapshot_recording, PageSession};
use crate::sites::douyin::{DouyinPageRuntime, DOUYIN_HOME_URL};
use crate::sites::registry::{
    required_string, ArgKind, BoxFuture, CommandArg, SiteCommand, SiteSpec, SlowWhen,
};

pub const DOUYIN_KNOWLEDGE: &str = include_str!("knowledge.md");

pub fn douyin_tools(page: Arc<PageSession>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(SearchVideosTool { page }) as Arc<dyn Tool>]
}

pub async fn douyin_agent_tools(
    page: Arc<PageSession>,
    _llm_provider: Arc<dyn LlmProvider>,
) -> anyhow::Result<Vec<Arc<dyn Tool>>> {
    DouyinPageRuntime::new(&page)
        .ensure_douyin(false)
        .await
        .ok();
    Ok(douyin_tools(page))
}

pub fn douyin_agent_instructions(extra: &str) -> String {
    let base = DOUYIN_KNOWLEDGE.trim().to_string();
    let extra = extra.trim();
    if extra.is_empty() {
        base
    } else {
        format!("{extra}\n\n{base}")
    }
}

pub static DOUYIN_SITE: SiteSpec = SiteSpec {
    id: "dy",
    about: "Douyin (douyin.com)",
    home_url: DOUYIN_HOME_URL,
    agent_tools: |page, llm| Box::pin(douyin_agent_tools(page, llm)),
    agent_instructions: douyin_agent_instructions,
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
                help: "Auto-scroll the search results to collect at least this many videos. Omit for visible results only.",
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
    let input = search_videos_input(query, num_videos)?;
    let (run_dir, ctx) = command_context("search_videos");
    let invocation = json!({
        "command": "search_videos",
        "tool": "search_videos",
        "input": input.clone(),
    });
    let _ = std::fs::create_dir_all(&ctx.run_dir);
    if let Ok(bytes) = serde_json::to_vec_pretty(&invocation) {
        let _ = std::fs::write(ctx.run_dir.join("command_input.json"), bytes);
    }

    let data = with_snapshot_recording(&page, &ctx.run_dir, debug_snapshot, async {
        ensure_search_ready(&page).await?;
        let data = call_douyin_tool(page.clone(), "search_videos", input, &ctx).await?;
        Ok::<Value, anyhow::Error>(data)
    })
    .await?;

    Ok(json!({
        "command": "search_videos",
        "run_dir": run_dir,
        "input": invocation.get("input").cloned().unwrap_or(Value::Null),
        "data": data,
    }))
}

fn search_videos_input(query: &str, num_videos: Option<i64>) -> anyhow::Result<Value> {
    let mut input = json!({
        "query": trimmed_required(query, "query")?,
        "wait_seconds": 8.0,
    });
    if let Some(n) = num_videos {
        input["num_videos"] = json!(n.max(1));
    }
    Ok(input)
}

async fn ensure_search_ready(page: &PageSession) -> anyhow::Result<()> {
    let runtime = DouyinPageRuntime::new(page);
    let current_url = runtime.current_url().await.unwrap_or_default();
    if !current_url.contains("douyin.com") {
        page.navigate_with_timeout(DOUYIN_HOME_URL, 60.0).await?;
    }
    Ok(())
}

async fn call_douyin_tool(
    page: Arc<PageSession>,
    tool_name: &str,
    input: Value,
    ctx: &ToolContext,
) -> anyhow::Result<Value> {
    let tool = douyin_tools(page)
        .into_iter()
        .find(|tool| tool.name() == tool_name)
        .ok_or_else(|| anyhow::anyhow!("douyin tool not found: {tool_name}"))?;
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

fn json_result(value: &Value) -> ToolResult {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    ToolResult::text(text)
}

fn get_f64(input: &Value, key: &str, default: f64) -> f64 {
    input.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn get_str<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
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
        "Search Douyin for videos matching `query` and return search result cards \
         (video_id, url, title, author, cover_url, visible interaction text). By \
         default reads only the visible results. Pass `num_videos` to auto-scroll \
         the feed until that many unique video cards are collected. Does not open \
         video detail pages or comments."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (Chinese works fine)" },
                "num_videos": {
                    "type": "integer",
                    "description": "Scroll to collect at least this many video cards. Omit for visible results only.",
                    "minimum": 1
                },
                "wait_seconds": {
                    "type": "number",
                    "description": "Seconds to wait for search results to load",
                    "default": 8.0
                }
            },
            "required": ["query"]
        })
    }

    fn defer_until_site(&self) -> &str {
        "dy"
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let query = get_str(&input, "query")
            .ok_or_else(|| anyhow::anyhow!("missing query"))?
            .to_string();
        let wait_seconds = get_f64(&input, "wait_seconds", 8.0);
        let num_videos = input
            .get("num_videos")
            .and_then(Value::as_i64)
            .map(|n| n.max(1) as usize);
        let data = DouyinPageRuntime::new(&self.page)
            .search_videos(&query, wait_seconds, num_videos)
            .await?;
        Ok(json_result(&data))
    }
}
