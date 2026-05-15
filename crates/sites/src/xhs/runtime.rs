use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Map, Value};
use socai_browser::PageSession;

use crate::xhs::entities::{normalize_url, XhsNote, XhsNoteCard};

pub const XHS_HOME_URL: &str = "https://www.xiaohongshu.com/explore";

const PAGE_SCRIPTS_JS: &str = include_str!("page_scripts.js");

const XHS_PAGE_SCRIPT_FUNCTIONS: &[&str] = &[
    "note",
    "noteWithWait",
    "pageState",
    "searchCards",
    "searchInput",
    "setSearchInput",
    "searchState",
    "searchTabs",
    "clickSearchTab",
    "clickCard",
    "closeNote",
    "noteOpen",
    "comments",
    "scrollInNote",
    "carouselImages",
    "profileInfo",
    "profileCards",
];

/// Site-aware XHS operations on top of a CDP `PageSession`.
pub struct XhsSiteRuntime<'a> {
    page: &'a PageSession,
    last_extracted_note_id: Mutex<String>,
}

impl<'a> XhsSiteRuntime<'a> {
    pub fn new(page: &'a PageSession) -> Self {
        Self {
            page,
            last_extracted_note_id: Mutex::new(String::new()),
        }
    }

    fn set_last_note_id(&self, value: String) {
        if let Ok(mut guard) = self.last_extracted_note_id.lock() {
            *guard = value;
        }
    }

    fn last_note_id(&self) -> String {
        self.last_extracted_note_id
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Inject `page_scripts.js` (the IIFE that defines `SocaiXhsPageScripts`)
    /// and call one of its functions. Mirrors Python's `xhs_page_script_call`.
    pub async fn run_script(&self, name: &str, arg: Option<&Value>) -> Result<Value> {
        if !XHS_PAGE_SCRIPT_FUNCTIONS.contains(&name) {
            anyhow::bail!("Unknown XHS page script: {name}");
        }
        let args = match arg {
            None => String::new(),
            Some(v) => serde_json::to_string(v)?,
        };
        let expr = format!(
            "{PAGE_SCRIPTS_JS}\n// SOCAI_XHS_CALL: {name}\nreturn SocaiXhsPageScripts.{name}({args});"
        );
        self.page.evaluate_json(&expr).await
    }

    pub async fn current_url(&self) -> Result<String> {
        Ok(self
            .page
            .page_info()
            .await?
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    pub async fn ensure_xhs(&self, navigate_if_needed: bool) -> Result<()> {
        let url = self.current_url().await?;
        if url.contains("xiaohongshu.com") {
            return Ok(());
        }
        if navigate_if_needed {
            self.page.navigate(XHS_HOME_URL).await?;
            return Ok(());
        }
        anyhow::bail!(
            "Current page is not Xiaohongshu: {}",
            if url.is_empty() { "unknown" } else { &url }
        );
    }

    pub async fn detect_state(&self) -> Result<Value> {
        self.ensure_xhs(false).await?;
        self.expect_object("pageState", None).await
    }

    pub async fn search_notes(&self, query: &str, wait_seconds: f64) -> Result<Value> {
        let keyword = query.trim();
        if keyword.is_empty() {
            anyhow::bail!("query is required");
        }

        self.ensure_xhs(true).await?;
        let submit = self.submit_search(keyword, wait_seconds).await?;
        let ok = submit.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let cards = if ok {
            self.extract_search_cards().await?
        } else {
            Vec::new()
        };
        let mut submit = submit;
        if let Some(state) = submit.get_mut("state").and_then(Value::as_object_mut) {
            state.remove("url_keyword");
        }
        Ok(json!({
            "ok": ok,
            "query": keyword,
            "submit": submit,
            "url": self.current_url().await?,
            "count": cards.len(),
            "cards": cards,
            "reason": if ok { "" } else { submit.get("error").and_then(Value::as_str).unwrap_or("search_submit_failed") },
        }))
    }

    pub async fn submit_search(&self, query: &str, wait_seconds: f64) -> Result<Value> {
        let loc = self.expect_object("searchInput", None).await?;
        if !loc.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(json!({
                "ok": false,
                "strategy": "search_input_unavailable",
                "error": loc.get("error").and_then(Value::as_str).unwrap_or_default(),
            }));
        }

        if let Some(input) = loc.get("input") {
            self.page
                .click(number(input, "x"), number(input, "y"))
                .await?;
            sleep_ms(150).await;
        }

        let set_result = self
            .expect_object("setSearchInput", Some(&json!({ "query": query })))
            .await?;
        if !set_result
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(json!({
                "ok": false,
                "strategy": "set_search_input_failed",
                "state": set_result,
                "error": set_result
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Search input did not accept the requested keyword"),
            }));
        }

        sleep_ms(150).await;
        self.page.press_key("Enter").await?;
        let state = self
            .wait_for_search_transition(query, wait_seconds.clamp(0.2, 6.0))
            .await?;
        if search_transition_ok(&state, query) {
            return Ok(json!({
                "ok": true,
                "strategy": "click_input_set_value_enter",
                "state": state,
                "url": self.current_url().await?,
            }));
        }

        if let Some(submit) = loc.get("submit") {
            let x = number(submit, "x");
            let y = number(submit, "y");
            if x > 0.0 && y > 0.0 {
                self.page.click(x, y).await?;
                let state = self
                    .wait_for_search_transition(query, wait_seconds.clamp(0.2, 6.0))
                    .await?;
                if search_transition_ok(&state, query) {
                    return Ok(json!({
                        "ok": true,
                        "strategy": "click_search_button",
                        "state": state,
                        "url": self.current_url().await?,
                    }));
                }
            }
        }

        Ok(json!({
            "ok": false,
            "strategy": "manual_submit_failed",
            "state": state,
            "url": self.current_url().await?,
            "error": if state.get("login_required").and_then(Value::as_bool).unwrap_or(false) {
                "login_required"
            } else {
                "Search did not transition to a valid Xiaohongshu result page"
            },
        }))
    }

    pub async fn wait_for_search_transition(&self, query: &str, timeout_s: f64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs_f64(timeout_s.max(0.2));
        let mut latest = Value::Object(Map::new());
        while Instant::now() < deadline {
            latest = self.expect_object("searchState", None).await?;
            if search_transition_ok(&latest, query) {
                return Ok(latest);
            }
            sleep_ms(150).await;
        }
        if latest.as_object().is_some_and(Map::is_empty) {
            self.expect_object("searchState", None).await
        } else {
            Ok(latest)
        }
    }

    pub async fn extract_search_cards(&self) -> Result<Vec<XhsNoteCard>> {
        self.ensure_xhs(false).await?;
        let raw = self.expect_array("searchCards", None).await?;
        Ok(parse_cards(&raw))
    }

    pub async fn open_note(
        &self,
        note_id: &str,
        index: Option<usize>,
        wait_seconds: f64,
    ) -> Result<Value> {
        let cards = self.extract_search_cards().await?;
        let selected = select_card(&cards, note_id, index).ok_or_else(|| {
            anyhow::anyhow!("Could not resolve note target from current search cards.")
        })?;

        let mut click_arg = Map::new();
        if !selected.note_id.is_empty() {
            click_arg.insert("note_id".into(), Value::String(selected.note_id.clone()));
        }
        if let Some(index) = index {
            click_arg.insert("index".into(), json!(index));
        } else {
            click_arg.insert("index".into(), json!(selected.position));
        }

        let target = self
            .expect_object("clickCard", Some(&Value::Object(click_arg.clone())))
            .await?;
        if !target.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            anyhow::bail!(
                "Could not locate card to click for note {}: {}",
                selected.note_id,
                target
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            );
        }

        self.set_last_note_id(String::new());
        let per_attempt = (wait_seconds / 2.0).max(0.2);
        let click_target_kind = target
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or("cover");

        self.page
            .click(number(&target, "x"), number(&target, "y"))
            .await?;
        let opened = self.wait_for_note_open(per_attempt).await?;
        if note_is_open(&opened) {
            return Ok(json!({
                "ok": true,
                "target": selected,
                "url": self.current_url().await?,
                "state": opened,
                "strategy": format!("{click_target_kind}_click"),
            }));
        }

        let retry = self
            .expect_object("clickCard", Some(&Value::Object(click_arg)))
            .await?;
        if retry.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            self.page
                .click(number(&retry, "x"), number(&retry, "y"))
                .await?;
            let opened = self.wait_for_note_open(per_attempt).await?;
            if note_is_open(&opened) {
                return Ok(json!({
                    "ok": true,
                    "target": selected,
                    "url": self.current_url().await?,
                    "state": opened,
                    "strategy": format!("retry_{}_click", retry.get("target").and_then(Value::as_str).unwrap_or("card")),
                }));
            }
        }

        Ok(json!({
            "ok": false,
            "target": selected,
            "url": self.current_url().await?,
            "state": opened,
            "strategy": "card_click_failed",
            "error": if opened.get("login_required").and_then(Value::as_bool).unwrap_or(false) {
                "login_required"
            } else {
                "Note overlay did not open after card-click attempts; site may be throttling or layout changed"
            },
        }))
    }

    pub async fn close_note(&self, wait_seconds: f64) -> Result<Value> {
        let before = self.expect_object("noteOpen", None).await?;
        if !note_is_open(&before) {
            self.set_last_note_id(String::new());
            return Ok(json!({ "ok": true, "strategy": "already_closed", "state": before }));
        }

        let per_attempt = wait_seconds.max(0.2);
        self.page.press_key("Escape").await?;
        let state = self.wait_for_note_closed(per_attempt).await?;
        if !note_is_open(&state) {
            self.set_last_note_id(String::new());
            return Ok(
                json!({ "ok": true, "strategy": "escape", "state": state, "url": self.current_url().await? }),
            );
        }

        let _ = self
            .page
            .evaluate_json("document.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', code: 'Escape', keyCode: 27, which: 27, bubbles: true}))")
            .await;
        let state = self.wait_for_note_closed(per_attempt).await?;
        if !note_is_open(&state) {
            self.set_last_note_id(String::new());
            return Ok(
                json!({ "ok": true, "strategy": "escape_dispatch", "state": state, "url": self.current_url().await? }),
            );
        }

        let close_btn = self.expect_object("closeNote", None).await?;
        if close_btn
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            self.page
                .click(number(&close_btn, "x"), number(&close_btn, "y"))
                .await?;
            let state = self.wait_for_note_closed(per_attempt).await?;
            if !note_is_open(&state) {
                self.set_last_note_id(String::new());
                return Ok(json!({
                    "ok": true,
                    "strategy": "close_button",
                    "selector": close_btn.get("selector").and_then(Value::as_str).unwrap_or_default(),
                    "state": state,
                    "url": self.current_url().await?,
                }));
            }
        }

        Ok(json!({
            "ok": false,
            "strategy": "close_failed",
            "state": state,
            "url": self.current_url().await?,
            "error": "Note modal did not close after Escape, JS-dispatch Escape, and close-button attempts",
        }))
    }

    pub async fn read_note(
        &self,
        note_id: &str,
        index: Option<usize>,
        wait_seconds: f64,
    ) -> Result<Value> {
        let mut open = None;
        if !note_id.is_empty() || index.is_some() {
            let opened = self.open_note(note_id, index, wait_seconds).await?;
            if !opened.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                return Ok(
                    json!({ "ok": false, "open": opened, "error": opened.get("error").and_then(Value::as_str).unwrap_or("open_failed") }),
                );
            }
            open = Some(opened);
        }
        let note = self.extract_note(wait_seconds).await?;
        if !note_id.is_empty() && !note.note_id.is_empty() && note.note_id != note_id {
            return Ok(json!({
                "ok": false,
                "entity": note,
                "open": open,
                "error": format!("stale_note: expected {note_id}, got {}", note.note_id),
            }));
        }
        Ok(json!({ "ok": true, "entity": note, "open": open }))
    }

    pub async fn extract_comments(&self, max_comments: i64) -> Result<Vec<Value>> {
        self.ensure_xhs(false).await?;
        let raw = self
            .expect_array(
                "comments",
                Some(&json!({ "prefer_hot": true, "max_comments": max_comments })),
            )
            .await?;
        Ok(raw.into_iter().filter(Value::is_object).collect())
    }

    /// Extract the currently open note. Caller is responsible for having
    /// navigated to the note URL (or having opened the note modal); the JS
    /// side polls via `noteWithWait` until content hydrates, so the caller
    /// doesn't need a separate readiness check.
    pub async fn extract_note(&self, wait_seconds: f64) -> Result<XhsNote> {
        let timeout_ms = (wait_seconds.max(0.5) * 1000.0) as i64;
        let raw = self
            .run_script("noteWithWait", Some(&json!({ "timeout_ms": timeout_ms })))
            .await?;

        let body = raw
            .get("note")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Map::new()));

        let mut note = parse_note(&body, "lite");

        // Python falls back to the live page URL when the JS payload didn't
        // populate body.url. Mirror that — one extra evaluate is cheap and
        // keeps parity tests stable across navigation styles.
        if note.url.is_empty() {
            if let Ok(href) = self.page.evaluate_json("location.href").await {
                if let Some(s) = href.as_str() {
                    note.url = normalize_url(s);
                }
            }
        }

        if !note.note_id.is_empty() {
            let prev = self.last_note_id();
            if !prev.is_empty() && prev == note.note_id {
                note.stale_warning = Some(format!(
                    "This note (note_id={}) was already extracted in the previous read. The note modal may not have closed before opening the next card — call xhs_close_note to verify the modal is gone, then re-open the target card.",
                    note.note_id
                ));
            }
            self.set_last_note_id(note.note_id.clone());
        }

        note.wait_meta = Some(json!({
            "ready": raw.get("ready").and_then(Value::as_bool).unwrap_or(false),
            "reason": raw.get("reason").and_then(Value::as_str).unwrap_or(""),
            "waited_ms": raw.get("waited_ms").and_then(Value::as_i64).unwrap_or(0),
            "attempts": raw.get("attempts").and_then(Value::as_i64).unwrap_or(0),
        }));

        Ok(note)
    }

    async fn expect_object(&self, name: &str, arg: Option<&Value>) -> Result<Value> {
        let value = self.run_script(name, arg).await?;
        if value.is_object() {
            Ok(value)
        } else {
            anyhow::bail!(
                "XHS page script {name} returned {}, expected object",
                value_type(&value)
            );
        }
    }

    async fn expect_array(&self, name: &str, arg: Option<&Value>) -> Result<Vec<Value>> {
        let value = self.run_script(name, arg).await?;
        value.as_array().cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "XHS page script {name} returned {}, expected array",
                value_type(&value)
            )
        })
    }

    async fn wait_for_note_open(&self, timeout_s: f64) -> Result<Value> {
        wait_for_note_state(self, timeout_s, true).await
    }

    async fn wait_for_note_closed(&self, timeout_s: f64) -> Result<Value> {
        wait_for_note_state(self, timeout_s, false).await
    }
}

/// Parse the JS-side `body` payload into a wire-ready XhsNote. Performs the
/// same normalization Python's `extract_note` + `XhsNote.to_dict()` do, all
/// front-loaded so serde Serialize alone produces parity-clean output.
fn parse_note(body: &Value, level: &str) -> XhsNote {
    let s = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };

    let hashtags: Vec<String> = body
        .get("hashtags")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .take(12) // Python clips at to_dict() time; we do it here so serde-Serialize matches.
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    let image_urls: Vec<String> = body
        .get("image_urls")
        .and_then(Value::as_array)
        .map(|arr| {
            let mut seen = HashSet::new();
            arr.iter()
                .filter_map(Value::as_str)
                .map(normalize_image_url)
                .filter(|s| !s.is_empty())
                .filter(|s| seen.insert(s.clone()))
                .collect()
        })
        .unwrap_or_default();

    let images: Vec<Value> = image_urls
        .iter()
        .enumerate()
        .map(|(index, url)| json!({ "url": url, "index": index as i64 }))
        .collect();

    let video = body
        .get("video")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Map::new()));

    let image_count = body
        .get("image_count")
        .and_then(Value::as_i64)
        .filter(|&n| n > 0)
        .unwrap_or(images.len() as i64);

    XhsNote {
        note_id: s("note_id"),
        url: normalize_url(&s("url")),
        r#type: s("type"),
        title: s("title"),
        author: s("author"),
        author_id: s("author_id"),
        author_url: normalize_url(&s("author_url")),
        content: s("content"),
        content_source: s("content_source"),
        hashtags,
        date: s("date"),
        location: s("location"),
        ip_location: s("ip_location"),
        likes: s("likes"),
        favorites: s("favorites"),
        comments_count: s("comments_count"),
        image_count,
        images,
        video,
        extraction_level: level.to_string(),
        wait_meta: None,
        stale_warning: None,
    }
}

async fn wait_for_note_state(
    runtime: &XhsSiteRuntime<'_>,
    timeout_s: f64,
    want_open: bool,
) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs_f64(timeout_s.max(0.2));
    let mut latest = Value::Object(Map::new());
    while Instant::now() < deadline {
        latest = runtime.expect_object("noteOpen", None).await?;
        if note_is_open(&latest) == want_open {
            return Ok(latest);
        }
        sleep_ms(150).await;
    }
    if latest.as_object().is_some_and(Map::is_empty) {
        runtime.expect_object("noteOpen", None).await
    } else {
        Ok(latest)
    }
}

fn parse_cards(raw: &[Value]) -> Vec<XhsNoteCard> {
    raw.iter()
        .enumerate()
        .filter_map(|(index, item)| {
            item.as_object().map(|obj| XhsNoteCard {
                note_id: string_field(obj, "note_id"),
                title: string_field(obj, "title"),
                author: string_field(obj, "author"),
                author_id: string_field(obj, "author_id"),
                author_url: string_field(obj, "author_url"),
                likes: string_field(obj, "likes"),
                link: string_field(obj, "link"),
                cover_url: string_field(obj, "cover_url"),
                r#type: string_field(obj, "type"),
                position: obj
                    .get("position")
                    .and_then(Value::as_i64)
                    .unwrap_or(index as i64),
                xsec_token: string_field(obj, "xsec_token"),
            })
        })
        .collect()
}

fn select_card(cards: &[XhsNoteCard], note_id: &str, index: Option<usize>) -> Option<XhsNoteCard> {
    if !note_id.is_empty() {
        if let Some(card) = cards.iter().find(|card| card.note_id == note_id) {
            return Some(card.clone());
        }
    }
    index.and_then(|idx| cards.get(idx).cloned())
}

fn search_transition_ok(state: &Value, query: &str) -> bool {
    if state
        .get("page_state")
        .and_then(Value::as_str)
        .unwrap_or_default()
        != "search_results"
    {
        return false;
    }
    let keyword = normalize_keyword(query);
    let visible_keyword = normalize_keyword(
        state
            .get("input_keyword")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let url_keyword = normalize_keyword(
        state
            .get("url_keyword")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    if !keyword.is_empty() && !visible_keyword.is_empty() && visible_keyword != keyword {
        return false;
    }
    if !keyword.is_empty()
        && visible_keyword.is_empty()
        && !url_keyword.is_empty()
        && url_keyword != keyword
    {
        return false;
    }
    if state.get("card_count").and_then(Value::as_i64).unwrap_or(0) > 0 {
        return true;
    }
    if state
        .get("loading")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    state
        .get("has_no_results")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn normalize_keyword(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalize_image_url(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.replacen("http://", "https://", 1)
}

fn note_is_open(state: &Value) -> bool {
    state
        .get("on_detail_route")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || state
            .get("has_modal")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn string_field(obj: &Map<String, Value>, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_body_yields_defaults() {
        let note = parse_note(&Value::Object(Map::new()), "lite");
        assert_eq!(note.note_id, "");
        assert_eq!(note.image_count, 0);
        assert!(note.images.is_empty());
        assert_eq!(note.extraction_level, "lite");
    }

    #[test]
    fn parse_populates_basic_fields() {
        let body = json!({
            "note_id": "abc123",
            "title": "测试笔记",
            "author": "张三",
            "image_urls": ["https://img.example/1.jpg", "https://img.example/2.jpg"],
            "hashtags": ["#tag1", "#tag2"],
        });
        let note = parse_note(&body, "lite");
        assert_eq!(note.note_id, "abc123");
        assert_eq!(note.title, "测试笔记");
        assert_eq!(note.author, "张三");
        assert_eq!(note.image_count, 2);
        assert_eq!(note.images.len(), 2);
        assert_eq!(note.images[0]["url"], "https://img.example/1.jpg");
        assert_eq!(note.images[0]["index"], 0);
        assert_eq!(note.hashtags, vec!["#tag1", "#tag2"]);
    }

    #[test]
    fn hashtags_clipped_to_12() {
        let tags: Vec<String> = (0..20).map(|i| format!("#t{i}")).collect();
        let body = json!({ "hashtags": tags });
        let note = parse_note(&body, "lite");
        assert_eq!(note.hashtags.len(), 12);
    }

    #[test]
    fn image_count_prefers_explicit_then_images_len() {
        let body = json!({ "image_count": 5, "image_urls": ["a", "b"] });
        assert_eq!(parse_note(&body, "lite").image_count, 5);

        let body = json!({ "image_urls": ["a", "b", "c"] });
        assert_eq!(parse_note(&body, "lite").image_count, 3);

        let body = json!({});
        assert_eq!(parse_note(&body, "lite").image_count, 0);
    }
}
