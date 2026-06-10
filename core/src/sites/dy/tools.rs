use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::agent::{make_run_dir, Backend as LlmProvider, Tool, ToolContext, ToolResult};
use crate::cdp::{with_snapshot_recording, PageSession};
use crate::sites::dy::{DyPageRuntime, DY_HOME_URL};
use crate::sites::registry::{BoxFuture, SiteCommand, SiteSpec, SlowWhen};

pub const DY_KNOWLEDGE: &str = include_str!("knowledge.md");

pub fn dy_tools(page: Arc<PageSession>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(DyPageStateTool { page })]
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
        name: "page_state",
        tool_name: "dy_page_state",
        about: "Open Douyin if needed and print a conservative page-state snapshot as JSON.",
        args: &[],
        slow: SlowWhen::Never,
        run: run_page_state,
    }],
};

fn run_page_state(page: Arc<PageSession>, _args: Value, debug_snapshot: bool) -> BoxFuture<Value> {
    Box::pin(async move { page_state_command(page, debug_snapshot).await })
}

pub async fn page_state_command(
    page: Arc<PageSession>,
    debug_snapshot: bool,
) -> anyhow::Result<Value> {
    let (run_dir, ctx) = command_context("dy_page_state");
    let invocation = json!({
        "command": "page_state",
        "tool": "dy_page_state",
        "input": {},
    });
    let _ = std::fs::create_dir_all(&ctx.run_dir);
    if let Ok(bytes) = serde_json::to_vec_pretty(&invocation) {
        let _ = std::fs::write(ctx.run_dir.join("command_input.json"), bytes);
    }

    let data = with_snapshot_recording(&page, &ctx.run_dir, debug_snapshot, async {
        let runtime = DyPageRuntime::new(&page);
        runtime.open_home_and_detect_state().await
    })
    .await?;

    Ok(json!({
        "command": "page_state",
        "run_dir": run_dir,
        "input": invocation.get("input").cloned().unwrap_or(Value::Null),
        "data": data,
    }))
}

struct DyPageStateTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for DyPageStateTool {
    fn name(&self) -> &str {
        "dy_page_state"
    }

    fn description(&self) -> &str {
        "Open Douyin if needed, then return URL, title, readiness, viewport, \
         and conservative login-state hints. Use this before adding or running \
         more specific Douyin workflows."
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn defer_until_site(&self) -> &str {
        "dy"
    }

    async fn call(&self, _input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let value = DyPageRuntime::new(&self.page)
            .open_home_and_detect_state()
            .await?;
        Ok(json_result(&value))
    }
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

fn json_result(value: &Value) -> ToolResult {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    ToolResult::text(text)
}
