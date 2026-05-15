//! Agent-callable tool wrappers around [`XhsSiteRuntime`].
//!
//! Each wrapper owns an `Arc<PageSession>` — the same tab is reused across
//! tool calls so the agent's actions accumulate state (search results
//! visible, note modal open, etc.). The caller is responsible for creating
//! the page and closing it after `run_agent` returns.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use socai_agent::{Backend as LlmProvider, Tool, ToolContext, ToolResult};
use socai_browser::PageSession;
use socai_media::{timing_delta, MediaProcessor, TimingSnapshot};

use crate::xhs::{ReadNoteOptions, XhsNoteCard, XhsSiteRuntime};

/// All XHS tools constructed against the same page. Convenience helper for
/// the CLI / agent host — just register everything.
pub fn xhs_tools(page: Arc<PageSession>) -> Vec<Arc<dyn Tool>> {
    xhs_tools_with_llm_provider(page, None)
}

pub fn xhs_tools_with_llm_provider(
    page: Arc<PageSession>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(SearchNotesTool { page: page.clone() }) as Arc<dyn Tool>,
        Arc::new(ExtractSearchCardsTool { page: page.clone() }),
        Arc::new(ListSearchTabsTool { page: page.clone() }),
        Arc::new(ClickSearchTabTool { page: page.clone() }),
        Arc::new(OpenNoteTool { page: page.clone() }),
        Arc::new(CloseNoteTool { page: page.clone() }),
        Arc::new(ReadNoteTool {
            page: page.clone(),
            llm_provider: llm_provider.clone(),
        }),
        Arc::new(ExtractNoteTool {
            page: page.clone(),
            llm_provider: llm_provider.clone(),
        }),
        Arc::new(ExtractCommentsTool { page: page.clone() }),
        Arc::new(ScrollInNoteTool { page: page.clone() }),
        Arc::new(CollectCarouselImagesTool { page: page.clone() }),
        Arc::new(ExtractProfileTool { page: page.clone() }),
        Arc::new(TopicScanTool {
            page: page.clone(),
            llm_provider,
        }),
        Arc::new(PageStateTool { page }),
    ]
}

fn json_result(value: &Value) -> ToolResult {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    ToolResult::text(text)
}

fn get_f64(input: &Value, key: &str, default: f64) -> f64 {
    input.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn get_i64(input: &Value, key: &str, default: i64) -> i64 {
    input.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn get_str<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

fn get_bool(input: &Value, key: &str, default: bool) -> bool {
    input.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn read_note_options(input: &Value) -> ReadNoteOptions {
    ReadNoteOptions {
        level: get_str(input, "level").unwrap_or("lite").to_string(),
        include_media: get_bool(input, "include_media", false),
        max_images: get_i64(input, "max_images", 12).max(1) as usize,
        max_video_frames: get_i64(input, "max_video_frames", 4).max(1) as usize,
    }
}

fn media_for(
    ctx: &ToolContext,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    include_media: bool,
) -> anyhow::Result<Option<MediaProcessor>> {
    if include_media {
        Ok(Some(MediaProcessor::for_run_dir(
            &ctx.run_dir,
            llm_provider,
        )?))
    } else {
        Ok(None)
    }
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

/// read_note(note_id?, index?, wait_seconds?, include_media?) -> full XhsNote
pub struct ReadNoteTool {
    page: Arc<PageSession>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
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
                "wait_seconds": { "type": "number", "default": 6.0 },
                "level": { "type": "string", "enum": ["card", "lite", "deep"], "default": "lite" },
                "include_media": { "type": "boolean", "default": false },
                "max_images": { "type": "integer", "default": 12, "minimum": 1 },
                "max_video_frames": { "type": "integer", "default": 4, "minimum": 1 }
            }
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let note_id = get_str(&input, "note_id").map(str::to_string);
        let index = input
            .get("index")
            .and_then(Value::as_i64)
            .and_then(|i| usize::try_from(i).ok());
        let wait_seconds = get_f64(&input, "wait_seconds", 6.0);
        let options = read_note_options(&input);
        let xhs = XhsSiteRuntime::new_with_media(
            &self.page,
            media_for(ctx, self.llm_provider.clone(), options.include_media)?,
        );
        let value = xhs
            .read_note_with_options(
                note_id.as_deref().unwrap_or(""),
                index,
                wait_seconds,
                options,
            )
            .await?;
        Ok(json_result(&value))
    }
}

/// extract_note(wait_seconds?) -> XhsNote
pub struct ExtractNoteTool {
    page: Arc<PageSession>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
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
                "wait_seconds": { "type": "number", "default": 8.0 },
                "level": { "type": "string", "enum": ["card", "lite", "deep"], "default": "lite" },
                "include_media": { "type": "boolean", "default": false },
                "max_images": { "type": "integer", "default": 12, "minimum": 1 },
                "max_video_frames": { "type": "integer", "default": 4, "minimum": 1 }
            }
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let wait_seconds = get_f64(&input, "wait_seconds", 8.0);
        let options = read_note_options(&input);
        let xhs = XhsSiteRuntime::new_with_media(
            &self.page,
            media_for(ctx, self.llm_provider.clone(), options.include_media)?,
        );
        let note = xhs.extract_note_with_options(wait_seconds, options).await?;
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

/// extract_search_cards() -> [card] — read-only; just returns the cards
/// currently visible in the search results without re-running the search.
pub struct ExtractSearchCardsTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ExtractSearchCardsTool {
    fn name(&self) -> &str {
        "extract_search_cards"
    }

    fn description(&self) -> &str {
        "Return the note cards currently visible on the search results page \
         (without re-running the search). Useful after `click_search_tab` to \
         re-read the filtered card list."
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn call(&self, _input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let xhs = XhsSiteRuntime::new(&self.page);
        let cards = xhs.extract_search_cards().await?;
        let value = serde_json::to_value(&cards)?;
        Ok(json_result(&value))
    }
}

/// list_search_tabs() -> [tab] — list filter tabs on a search results page.
pub struct ListSearchTabsTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ListSearchTabsTool {
    fn name(&self) -> &str {
        "list_search_tabs"
    }

    fn description(&self) -> &str {
        "List the search-filter tabs visible on the current search page \
         (e.g. 全部 / 图文 / 视频 / 用户). Each entry has a label and an \
         `active` flag."
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn call(&self, _input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let xhs = XhsSiteRuntime::new(&self.page);
        let tabs = xhs.list_search_tabs().await?;
        Ok(json_result(&Value::Array(tabs)))
    }
}

/// click_search_tab(label, wait_seconds?) -> {ok, label, active_filter, tabs}
pub struct ClickSearchTabTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ClickSearchTabTool {
    fn name(&self) -> &str {
        "click_search_tab"
    }

    fn description(&self) -> &str {
        "Click the named search-filter tab to narrow results. Tab labels: \
         全部 / 图文 / 视频 / 用户 (or any active tab returned by \
         list_search_tabs). After clicking, follow up with \
         extract_search_cards to see the filtered cards."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "label": { "type": "string", "description": "Tab label (e.g. 全部, 图文, 视频, 用户)" },
                "wait_seconds": { "type": "number", "default": 1.5 }
            },
            "required": ["label"]
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let label = get_str(&input, "label")
            .ok_or_else(|| anyhow::anyhow!("missing label"))?
            .to_string();
        let wait_seconds = get_f64(&input, "wait_seconds", 1.5);
        let xhs = XhsSiteRuntime::new(&self.page);
        let value = xhs.click_search_tab(&label, wait_seconds).await?;
        Ok(json_result(&value))
    }
}

/// scroll_in_note(pixels?) -> {ok, scroll_top, ...}
pub struct ScrollInNoteTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ScrollInNoteTool {
    fn name(&self) -> &str {
        "scroll_in_note"
    }

    fn description(&self) -> &str {
        "Scroll the currently open note's detail panel by `pixels` (positive \
         = down). Use this to bring more comments or note body into view \
         before re-extracting."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pixels": { "type": "integer", "default": 400 }
            }
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let pixels = get_i64(&input, "pixels", 400);
        let xhs = XhsSiteRuntime::new(&self.page);
        let value = xhs.scroll_in_note(pixels).await?;
        Ok(json_result(&value))
    }
}

/// collect_carousel_images(max_images?) -> [url]
pub struct CollectCarouselImagesTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for CollectCarouselImagesTool {
    fn name(&self) -> &str {
        "collect_carousel_images"
    }

    fn description(&self) -> &str {
        "Collect image URLs from the carousel of the currently open note. \
         Requires the note detail modal to be open (use open_note first)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "max_images": { "type": "integer", "default": 12, "minimum": 1 }
            }
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let max_images = get_i64(&input, "max_images", 12);
        let xhs = XhsSiteRuntime::new(&self.page);
        let urls = xhs.collect_carousel_images(max_images).await?;
        Ok(json_result(&serde_json::to_value(&urls)?))
    }
}

/// extract_profile(max_notes?, scroll_rounds?) -> profile entity
pub struct ExtractProfileTool {
    page: Arc<PageSession>,
}

#[async_trait]
impl Tool for ExtractProfileTool {
    fn name(&self) -> &str {
        "extract_profile"
    }

    fn description(&self) -> &str {
        "Extract the currently visible Xiaohongshu profile page (author \
         display_name, xhs_id, bio, followers/following counts, and a paginated \
         list of note cards by scrolling the page). Caller must have navigated \
         to a profile URL first; this errors otherwise."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "max_notes": { "type": "integer", "default": 20, "minimum": 1 },
                "scroll_rounds": { "type": "integer", "default": 6, "minimum": 1 }
            }
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let max_notes = get_i64(&input, "max_notes", 20);
        let scroll_rounds = get_i64(&input, "scroll_rounds", 6);
        let xhs = XhsSiteRuntime::new(&self.page);
        let profile = xhs.extract_profile(max_notes, scroll_rounds).await?;
        Ok(json_result(&profile.to_value()))
    }
}

/// topic_scan(query, tab_label?, depth?) -> aggregated topic bundle
///
/// Composite macro: search → optional tab switch → sample N visible cards in
/// page order → read each (deep or lite depending on depth profile) →
/// bundle into one artifact. Prefer this for any "research a topic on XHS"
/// task — it returns search results plus the note bodies plus comments in
/// one tool call, so the agent doesn't have to chain 10+ tools by hand.
pub struct TopicScanTool {
    page: Arc<PageSession>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
}

struct ScanProfile {
    deep: usize,
    lite: usize,
    deep_comments: i64,
    lite_comments: i64,
    include_media: bool,
}

fn scan_profile_for(depth: &str) -> ScanProfile {
    // Mirrors `_SCAN_PROFILES` from socai/sites/xhs/tools.py.
    match depth.to_ascii_lowercase().as_str() {
        "quick" => ScanProfile {
            deep: 2,
            lite: 4,
            deep_comments: 6,
            lite_comments: 0,
            include_media: false,
        },
        "deep" => ScanProfile {
            deep: 6,
            lite: 4,
            deep_comments: 20,
            lite_comments: 4,
            include_media: true,
        },
        _ => ScanProfile {
            deep: 3,
            lite: 5,
            deep_comments: 12,
            lite_comments: 0,
            include_media: false,
        },
    }
}

#[async_trait]
impl Tool for TopicScanTool {
    fn name(&self) -> &str {
        "topic_scan"
    }

    fn description(&self) -> &str {
        "Xiaohongshu topic research macro: search → optional tab switch → \
         sample top visible cards in page order → read deep/lite notes → \
         return one compact bundle (search results + selected cards + note \
         bodies + comments). Prefer this for XHS topic research. Do not \
         repeat the same scan at a deeper depth unless the previous scan \
         was clearly insufficient."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "tab_label": {
                    "type": "string",
                    "enum": ["全部", "图文", "视频", "用户"]
                },
                "depth": {
                    "type": "string",
                    "enum": ["quick", "standard", "deep"],
                    "default": "standard"
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let query = get_str(&input, "query")
            .ok_or_else(|| anyhow::anyhow!("missing query"))?
            .to_string();
        let depth = get_str(&input, "depth").unwrap_or("standard").to_string();
        let tab_label = get_str(&input, "tab_label").unwrap_or("").to_string();
        let profile = scan_profile_for(&depth);

        let media = media_for(ctx, self.llm_provider.clone(), profile.include_media)?;
        let media_baseline: Option<TimingSnapshot> = media.as_ref().map(|m| m.timing().snapshot());
        let xhs = XhsSiteRuntime::new_with_media(&self.page, media.clone());
        let search = xhs.search_notes(&query, 2.0).await?;

        // Optional tab switch + re-extract cards.
        let mut tab_result = Value::Object(serde_json::Map::new());
        let cards: Vec<XhsNoteCard> = if !tab_label.is_empty() {
            tab_result = xhs.click_search_tab(&tab_label, 1.5).await?;
            xhs.extract_search_cards().await?
        } else {
            search
                .get("cards")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| serde_json::from_value::<XhsNoteCard>(v).ok())
                .collect()
        };

        let total_limit = profile.deep + profile.lite;
        let selected: Vec<&XhsNoteCard> = cards.iter().take(total_limit).collect();
        let sampled_ids: Vec<String> = selected
            .iter()
            .filter(|c| !c.note_id.is_empty())
            .map(|c| c.note_id.clone())
            .collect();
        ctx.add_topic_scan_note_ids(&sampled_ids);

        let mut notes: Vec<Value> = Vec::new();
        for (idx, card) in selected.iter().enumerate() {
            let level = if idx < profile.deep { "deep" } else { "lite" };
            let comment_count = if level == "deep" {
                profile.deep_comments
            } else {
                profile.lite_comments
            };

            // Dedup: skip notes already processed at this level or deeper
            // within the same run.
            let requested_media = profile.include_media && level == "deep";
            if !card.note_id.is_empty()
                && ctx.has_processed_note(&card.note_id, level, requested_media)
            {
                notes.push(json!({
                    "scan_level": level,
                    "source_position": card.position,
                    "skipped": {"reason": "already_processed"},
                    "entity": card,
                }));
                continue;
            }
            let read_result = xhs
                .read_note_with_options(
                    &card.note_id,
                    None,
                    6.0,
                    ReadNoteOptions {
                        level: level.to_string(),
                        include_media: requested_media,
                        max_images: 12,
                        max_video_frames: 4,
                    },
                )
                .await;
            let mut entry = match read_result {
                Ok(payload) => {
                    let entity = payload.get("entity").cloned().unwrap_or(Value::Null);
                    json!({
                        "scan_level": level,
                        "source_position": card.position,
                        "ok": payload.get("ok").and_then(Value::as_bool).unwrap_or(false),
                        "entity": entity,
                    })
                }
                Err(e) => json!({
                    "scan_level": level,
                    "source_position": card.position,
                    "ok": false,
                    "entity": card,
                    "error": format!("{e:#}"),
                }),
            };

            // Pull comments separately for deep notes.
            if level == "deep" && comment_count > 0 {
                if let Ok(comments) = xhs.extract_comments(comment_count).await {
                    let entity_map = entry.get_mut("entity").and_then(|v| v.as_object_mut());
                    if let Some(map) = entity_map {
                        map.insert("top_comments".into(), Value::Array(comments));
                    }
                }
            }

            // Mark processed.
            if !card.note_id.is_empty() {
                ctx.mark_processed_note(&card.note_id, level, requested_media);
            }

            notes.push(entry);
            let _ = xhs.close_note(0.6).await;
        }

        let media_timing = match (&media, &media_baseline) {
            (Some(media), Some(before)) => timing_delta(before, &media.timing().snapshot()),
            _ => json!({}),
        };

        let payload = json!({
            "ok": search.get("ok").and_then(Value::as_bool).unwrap_or(false),
            "query": query,
            "depth": depth,
            "tab": tab_result,
            "search": search,
            "selected_cards": serde_json::to_value(
                selected.iter().map(|c| (*c).clone()).collect::<Vec<_>>()
            )?,
            "notes": notes,
            "sampling": {
                "max_deep_notes": profile.deep,
                "max_lite_notes": profile.lite,
                "depth": depth,
                "include_media": profile.include_media,
            },
            "timing": {
                "media": media_timing,
            }
        });

        // Persist as artifact so it shows up in the run dir + working memory.
        let _ = ctx.write_json_artifact(
            &format!("xhs_topic_scan_{}", sanitize_for_filename(&query)),
            &payload,
            "artifacts",
            "topic_scan",
            "json",
            &format!(
                "Topic scan: {query} ({} notes)",
                payload
                    .get("notes")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            ),
            json!({"site": "xhs", "category": "topic_scan"}),
        );

        Ok(json_result(&payload))
    }
}

fn sanitize_for_filename(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .chars()
        .take(48)
        .collect()
}

/// Hint that callers can include in `extra_instructions` when registering
/// XHS tools. Mirrors the relevant bits of Python's XHS system prompt.
pub const XHS_AGENT_HINT: &str = "You are operating Xiaohongshu (https://www.xiaohongshu.com — also called XHS or 小红书). \
Tools available: search_notes, extract_search_cards, list_search_tabs, click_search_tab, \
open_note, close_note, read_note, extract_note, extract_comments, scroll_in_note, \
collect_carousel_images, extract_profile, topic_scan, page_state. \
Prefer `topic_scan` for any 'research a topic' task — it bundles search + sample + read in one call. \
Workflow for one-off lookups: page_state → search_notes → read_note (or open_note + extract_note + extract_comments). \
Close any open note modal before searching again. \
Default site URL: https://www.xiaohongshu.com/explore";
