use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::{Backend as LlmProvider, Tool, ToolContext, ToolResult};
use crate::cdp::PageSession;
use crate::sites::dy::{DyPageRuntime, DY_HOME_URL};
use crate::sites::registry::{
    required_string, ArgKind, BoxFuture, CommandArg, SiteCommand, SiteSpec, SlowWhen,
};
use crate::sites::runner::{
    get_f64, get_str, json_result, run_tool_command, trimmed_required, PageHook, ToolCommand,
};

pub const DY_KNOWLEDGE: &str = include_str!("knowledge.md");

pub fn dy_tools(page: Arc<PageSession>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(SearchVideosTool { page: page.clone() }) as Arc<dyn Tool>,
        Arc::new(PageStateTool { page }),
    ]
}

pub async fn dy_agent_tools(
    page: Arc<PageSession>,
    _llm_provider: Arc<dyn LlmProvider>,
) -> anyhow::Result<Vec<Arc<dyn Tool>>> {
    DyPageRuntime::new(&page).ensure_dy(true).await.ok();
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
    commands: &[
        SiteCommand {
            name: "search_videos",
            tool_name: "search_videos",
            about: "Search Douyin and print visible video cards as JSON.",
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
                    help: "Auto-scroll search results to collect at least this many video cards.",
                    required: false,
                    kind: ArgKind::Int,
                },
            ],
            slow: SlowWhen::ArgPresent("num_videos"),
            run: run_search_videos,
        },
        SiteCommand {
            name: "page_state",
            tool_name: "page_state",
            about: "Open/reuse Douyin and print a quick page-state snapshot.",
            args: &[],
            slow: SlowWhen::Never,
            run: run_page_state,
        },
    ],
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

fn run_page_state(page: Arc<PageSession>, _args: Value, debug_snapshot: bool) -> BoxFuture<Value> {
    Box::pin(async move {
        run_dy_tool_command(
            page,
            "page_state",
            "page_state",
            json!({}),
            debug_snapshot,
            Some(Box::new(|page| {
                Box::pin(async move { ensure_dy_home(&page).await })
            })),
        )
        .await
    })
}

pub async fn search_videos_command(
    page: Arc<PageSession>,
    query: &str,
    num_videos: Option<i64>,
    debug_snapshot: bool,
) -> anyhow::Result<Value> {
    run_dy_tool_command(
        page,
        "search_videos",
        "search_videos",
        search_videos_input(query, num_videos)?,
        debug_snapshot,
        Some(Box::new(|page| {
            Box::pin(async move { ensure_dy_home(&page).await })
        })),
    )
    .await
}

fn search_videos_input(query: &str, num_videos: Option<i64>) -> anyhow::Result<Value> {
    let mut input = json!({
        "query": trimmed_required(query, "query")?,
        "wait_seconds": 2.0,
    });
    if let Some(n) = num_videos {
        input["num_videos"] = json!(n.max(1));
    }
    Ok(input)
}

async fn run_dy_tool_command(
    page: Arc<PageSession>,
    command_name: &'static str,
    tool_name: &'static str,
    input: Value,
    debug_snapshot: bool,
    before: Option<PageHook>,
) -> anyhow::Result<Value> {
    let tools = dy_tools(page.clone());
    run_tool_command(
        ToolCommand {
            site_id: "dy",
            command_name,
            tool_name,
            before,
            after: None,
        },
        page,
        &tools,
        input,
        debug_snapshot,
    )
    .await
}

async fn ensure_dy_home(page: &PageSession) -> anyhow::Result<()> {
    let runtime = DyPageRuntime::new(page);
    runtime.ensure_dy(true).await
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
        "Search Douyin for videos matching `query` and return result cards \
         (video id, title, author, visible metrics, cover, URL). By default \
         reads the current loaded results; pass `num_videos` to scroll the feed \
         until that many unique videos are collected or results stop growing. \
         It opens/searches Douyin with human-like page interaction and does not \
         call Douyin APIs."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "num_videos": {
                    "type": "integer",
                    "description": "Scroll to collect at least this many video cards.",
                    "minimum": 1
                },
                "wait_seconds": {
                    "type": "number",
                    "description": "Extra seconds to wait for search results to load",
                    "default": 2.0
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let query = get_str(&input, "query")
            .ok_or_else(|| anyhow::anyhow!("missing query"))?
            .to_string();
        let wait_seconds = get_f64(&input, "wait_seconds", 2.0);
        let num_videos = input
            .get("num_videos")
            .and_then(Value::as_i64)
            .filter(|n| *n > 0)
            .map(|n| n as usize);
        let dy = DyPageRuntime::new(&self.page);
        let value = dy.search_videos(&query, wait_seconds, num_videos).await?;
        Ok(json_result(&value))
    }
}

pub struct PageStateTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for PageStateTool {
    fn name(&self) -> &str {
        "page_state"
    }

    fn description(&self) -> &str {
        "Read a quick snapshot of the current Douyin page, including URL, \
         login hint, search input availability, and visible video-card count."
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn call(&self, _input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let dy = DyPageRuntime::new(&self.page);
        let value = dy.detect_state(true).await?;
        Ok(json_result(&value))
    }
}
