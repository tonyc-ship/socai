//! Agent-callable tool wrappers around [`XhsSiteRuntime`].
//!
//! Each wrapper owns an `Arc<PageSession>` — the same tab is reused across
//! tool calls so the agent's actions accumulate state (search results
//! visible, note modal open, etc.). The caller is responsible for creating
//! the page and closing it after `run_agent` returns.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use socai_browser::PageSession;
use socai_agent::{Tool, ToolContext, ToolResult};

use crate::xhs::XhsSiteRuntime;

/// All XHS tools constructed against the same page. Convenience helper for
/// the CLI / agent host — just register everything.
pub fn xhs_tools(page: Arc<PageSession>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(SearchNotesTool {
            page: page.clone(),
        }) as Arc<dyn Tool>,
        Arc::new(OpenNoteTool {
            page: page.clone(),
        }),
        Arc::new(CloseNoteTool {
            page: page.clone(),
        }),
        Arc::new(ReadNoteTool {
            page: page.clone(),
        }),
        Arc::new(ExtractNoteTool {
            page: page.clone(),
        }),
        Arc::new(ExtractCommentsTool {
            page: page.clone(),
        }),
        Arc::new(PageStateTool { page }),
    ]
}

fn json_result(value: &Value) -> ToolResult {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    ToolResult::text(text)
}

fn get_f64(input: &Value, key: &str, default: f64) -> f64 {
    input
        .get(key)
        .and_then(Value::as_f64)
        .unwrap_or(default)
}

fn get_i64(input: &Value, key: &str, default: i64) -> i64 {
    input.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn get_str<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

/// search_notes(query, wait_seconds) -> {query, cards: [...]}
pub struct SearchNotesTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for SearchNotesTool {
    fn name(&self) -> &str {
        "search_notes"
    }

    fn description(&self) -> &str {
        "Search Xiaohongshu for notes matching `query`. Returns the cards \
         visible on the results page (id, title, author, image). Use this \
         before `open_note` to pick which note to read."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (Chinese works fine)" },
                "wait_seconds": {
                    "type": "number",
                    "description": "Extra seconds to wait for cards to load",
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
        let xhs = XhsSiteRuntime::new(&self.page);
        let value = xhs.search_notes(&query, wait_seconds).await?;
        Ok(json_result(&value))
    }
}

/// open_note(note_id?, index?, wait_seconds?) -> {ok, ...}
pub struct OpenNoteTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for OpenNoteTool {
    fn name(&self) -> &str {
        "open_note"
    }

    fn description(&self) -> &str {
        "Open a note's detail modal on the current search results page. \
         Specify either `note_id` (from a card returned by search_notes) or \
         a 0-based `index` into the visible card list."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "note_id": { "type": "string", "description": "Note id from a search card" },
                "index": { "type": "integer", "description": "0-based index into the search results", "minimum": 0 },
                "wait_seconds": { "type": "number", "default": 4.0 }
            }
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let note_id = get_str(&input, "note_id").map(str::to_string);
        let index = input
            .get("index")
            .and_then(Value::as_i64)
            .and_then(|i| usize::try_from(i).ok());
        let wait_seconds = get_f64(&input, "wait_seconds", 4.0);
        let xhs = XhsSiteRuntime::new(&self.page);
        let value = xhs
            .open_note(note_id.as_deref().unwrap_or(""), index, wait_seconds)
            .await?;
        Ok(json_result(&value))
    }
}

/// close_note(wait_seconds?) -> {ok}
pub struct CloseNoteTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for CloseNoteTool {
    fn name(&self) -> &str {
        "close_note"
    }

    fn description(&self) -> &str {
        "Close the currently open note detail modal so search results are \
         visible again."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "wait_seconds": { "type": "number", "default": 1.0 }
            }
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let wait_seconds = get_f64(&input, "wait_seconds", 1.0);
        let xhs = XhsSiteRuntime::new(&self.page);
        let value = xhs.close_note(wait_seconds).await?;
        Ok(json_result(&value))
    }
}

/// read_note(note_id?, index?, wait_seconds?) -> full XhsNote
pub struct ReadNoteTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ReadNoteTool {
    fn name(&self) -> &str {
        "read_note"
    }

    fn description(&self) -> &str {
        "Open a note from the current search results and return its full \
         body (title, author, content, images, location, like/collect/comment \
         counts). Closes the modal when done. Prefer this over open_note + \
         extract_note when you only need the body."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "note_id": { "type": "string" },
                "index": { "type": "integer", "minimum": 0 },
                "wait_seconds": { "type": "number", "default": 6.0 }
            }
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let note_id = get_str(&input, "note_id").map(str::to_string);
        let index = input
            .get("index")
            .and_then(Value::as_i64)
            .and_then(|i| usize::try_from(i).ok());
        let wait_seconds = get_f64(&input, "wait_seconds", 6.0);
        let xhs = XhsSiteRuntime::new(&self.page);
        let value = xhs
            .read_note(note_id.as_deref().unwrap_or(""), index, wait_seconds)
            .await?;
        Ok(json_result(&value))
    }
}

/// extract_note(wait_seconds?) -> XhsNote
pub struct ExtractNoteTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ExtractNoteTool {
    fn name(&self) -> &str {
        "extract_note"
    }

    fn description(&self) -> &str {
        "Extract the currently visible note body from the page. Assumes the \
         user already navigated to a note URL or has the detail modal open."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "wait_seconds": { "type": "number", "default": 8.0 }
            }
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let wait_seconds = get_f64(&input, "wait_seconds", 8.0);
        let xhs = XhsSiteRuntime::new(&self.page);
        let note = xhs.extract_note(wait_seconds).await?;
        let value = serde_json::to_value(&note)?;
        Ok(json_result(&value))
    }
}

/// extract_comments(max_comments?) -> [comment]
pub struct ExtractCommentsTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ExtractCommentsTool {
    fn name(&self) -> &str {
        "extract_comments"
    }

    fn description(&self) -> &str {
        "Extract visible comments on the currently open note. Requires a \
         note detail modal to be open (use open_note first)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "max_comments": { "type": "integer", "default": 20, "minimum": 1 }
            }
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let max = get_i64(&input, "max_comments", 20);
        let xhs = XhsSiteRuntime::new(&self.page);
        let value = xhs.extract_comments(max).await?;
        Ok(json_result(&Value::Array(value)))
    }
}

/// page_state() -> {site, location, signed_in, modal_open, ...}
pub struct PageStateTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for PageStateTool {
    fn name(&self) -> &str {
        "page_state"
    }

    fn description(&self) -> &str {
        "Read a quick snapshot of the current page (site, signed-in state, \
         whether a note modal is open, current URL). Use this to verify what \
         step the agent is on."
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn call(&self, _input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let xhs = XhsSiteRuntime::new(&self.page);
        // ensure we're on XHS first, but don't navigate if we're not — just
        // report whatever the current page is.
        let value = xhs.detect_state().await?;
        Ok(json_result(&value))
    }
}

/// Hint that callers can include in `extra_instructions` when registering
/// XHS tools. Mirrors the relevant bits of Python's XHS system prompt.
pub const XHS_AGENT_HINT: &str = "You are operating Xiaohongshu (https://www.xiaohongshu.com — also called XHS or 小红书). \
Tools available: search_notes, open_note, close_note, read_note, extract_note, extract_comments, page_state. \
Workflow: page_state → search_notes → read_note (or open_note + extract_note + extract_comments). \
Close any open note modal before searching again. \
Default site URL: https://www.xiaohongshu.com/explore";
