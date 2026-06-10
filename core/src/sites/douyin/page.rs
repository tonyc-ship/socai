use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::cdp::PageSession;
use crate::sites::douyin::entities::DouyinVideoCard;

pub const DOUYIN_HOME_URL: &str = "https://www.douyin.com/";

const PAGE_SCRIPTS_JS: &str = include_str!("page_scripts.js");
const DOUYIN_PAGE_SCRIPT_FUNCTIONS: &[&str] = &[
    "pageState",
    "searchInput",
    "setSearchInput",
    "searchState",
    "videoCards",
    "scrollFeed",
];

pub struct DouyinPageRuntime<'a> {
    page: &'a PageSession,
}

impl<'a> DouyinPageRuntime<'a> {
    pub fn new(page: &'a PageSession) -> Self {
        Self { page }
    }

    pub async fn run_script(&self, name: &str, arg: Option<&Value>) -> Result<Value> {
        if !DOUYIN_PAGE_SCRIPT_FUNCTIONS.contains(&name) {
            anyhow::bail!("Unknown Douyin page script: {name}");
        }
        let args = match arg {
            None => String::new(),
            Some(v) => serde_json::to_string(v)?,
        };
        let expr = format!(
            "{PAGE_SCRIPTS_JS}\n// SOCAI_DOUYIN_CALL: {name}\nreturn SocaiDouyinPageScripts.{name}({args});"
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

    pub async fn ensure_douyin(&self, navigate_if_needed: bool) -> Result<()> {
        let url = self.current_url().await?;
        if url.contains("douyin.com") {
            return Ok(());
        }
        if navigate_if_needed {
            self.page
                .navigate_with_timeout(DOUYIN_HOME_URL, 60.0)
                .await?;
            return Ok(());
        }
        anyhow::bail!(
            "Current page is not Douyin: {}",
            if url.is_empty() { "unknown" } else { &url }
        );
    }

    pub async fn detect_state(&self) -> Result<Value> {
        self.ensure_douyin(false).await?;
        self.expect_object("pageState", None).await
    }

    pub async fn search_videos(
        &self,
        query: &str,
        wait_seconds: f64,
        num_videos: Option<usize>,
    ) -> Result<Value> {
        let keyword = query.trim();
        if keyword.is_empty() {
            anyhow::bail!("query is required");
        }

        self.ensure_douyin(true).await?;
        let submit = self.submit_search(keyword, wait_seconds).await?;
        let ok = script_ok(&submit);
        let cards = if ok {
            match num_videos {
                Some(target) if target > 0 => self.collect_video_cards(target).await?,
                _ => self.extract_video_cards(80).await?,
            }
        } else {
            Vec::new()
        };
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
        if !script_ok(&loc) {
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
        if !script_ok(&set_result) {
            return Ok(json!({
                "ok": false,
                "strategy": "set_search_input_failed",
                "error": set_result.get("error").and_then(Value::as_str).unwrap_or_default(),
            }));
        }

        self.page.press_key("Enter").await?;
        let state = self.wait_for_search_transition(query, wait_seconds).await?;
        if search_transition_ok(&state, query) {
            return Ok(json!({
                "ok": true,
                "strategy": "press_enter",
                "state": state,
            }));
        }

        if let Some(button) = loc.get("button") {
            self.page
                .click(number(button, "x"), number(button, "y"))
                .await?;
            let state = self.wait_for_search_transition(query, wait_seconds).await?;
            return Ok(json!({
                "ok": search_transition_ok(&state, query),
                "strategy": "click_search_button",
                "state": state,
                "error": if search_transition_ok(&state, query) { "" } else { "search_results_not_detected" },
            }));
        }

        Ok(json!({
            "ok": false,
            "strategy": "press_enter",
            "state": state,
            "error": "search_results_not_detected",
        }))
    }

    pub async fn extract_video_cards(&self, limit: usize) -> Result<Vec<DouyinVideoCard>> {
        let raw = self
            .expect_array("videoCards", Some(&json!({ "limit": limit.max(1) })))
            .await?;
        Ok(parse_cards(&raw))
    }

    pub async fn collect_video_cards(&self, target: usize) -> Result<Vec<DouyinVideoCard>> {
        const MAX_SCROLLS: usize = 16;
        const SETTLE_MS: u64 = 900;

        let target = target.max(1);
        let mut cards = self.extract_video_cards(target.max(80)).await?;
        let mut stagnant = 0usize;
        let mut scrolls = 0usize;
        while cards.len() < target && stagnant < 3 && scrolls < MAX_SCROLLS {
            let before = cards.len();
            self.expect_object("scrollFeed", Some(&json!({ "pixels": 900 })))
                .await?;
            scrolls += 1;
            sleep_ms(SETTLE_MS).await;
            cards = self.extract_video_cards(target.max(80)).await?;
            if cards.len() <= before {
                stagnant += 1;
            } else {
                stagnant = 0;
            }
        }
        if cards.len() > target {
            cards.truncate(target);
        }
        Ok(cards)
    }

    async fn wait_for_search_transition(&self, query: &str, wait_seconds: f64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs_f64(wait_seconds.max(2.0));
        let mut latest = Value::Object(Map::new());
        while Instant::now() < deadline {
            latest = self.expect_object("searchState", None).await?;
            if search_transition_ok(&latest, query) {
                return Ok(latest);
            }
            sleep_ms(250).await;
        }
        if latest.as_object().is_some_and(Map::is_empty) {
            self.expect_object("searchState", None).await
        } else {
            Ok(latest)
        }
    }

    async fn expect_object(&self, name: &str, arg: Option<&Value>) -> Result<Value> {
        let value = self.run_script(name, arg).await?;
        if value.is_object() {
            Ok(value)
        } else {
            anyhow::bail!(
                "Douyin page script {name} returned {}, expected object",
                value_type(&value)
            );
        }
    }

    async fn expect_array(&self, name: &str, arg: Option<&Value>) -> Result<Vec<Value>> {
        let value = self.run_script(name, arg).await?;
        value.as_array().cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "Douyin page script {name} returned {}, expected array",
                value_type(&value)
            )
        })
    }
}

fn parse_cards(raw: &[Value]) -> Vec<DouyinVideoCard> {
    let mut seen = HashSet::new();
    raw.iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let obj = item.as_object()?;
            let video_id = string_field(obj, "video_id");
            let url = string_field(obj, "url");
            let key = if video_id.is_empty() {
                url.clone()
            } else {
                video_id.clone()
            };
            if key.is_empty() || !seen.insert(key) {
                return None;
            }
            Some(DouyinVideoCard {
                video_id,
                url,
                title: string_field(obj, "title"),
                author: string_field(obj, "author"),
                author_url: string_field(obj, "author_url"),
                cover_url: string_field(obj, "cover_url"),
                likes: string_field(obj, "likes"),
                comments: string_field(obj, "comments"),
                shares: string_field(obj, "shares"),
                interaction_text: string_field(obj, "interaction_text"),
                raw_text: string_field(obj, "raw_text"),
                position: obj
                    .get("position")
                    .and_then(Value::as_i64)
                    .unwrap_or(index as i64),
            })
        })
        .collect()
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
    if state.get("card_count").and_then(Value::as_i64).unwrap_or(0) > 0 {
        return true;
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
    if !keyword.is_empty() && (visible_keyword == keyword || url_keyword == keyword) {
        return true;
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

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn script_ok(value: &Value) -> bool {
    value.get("ok").and_then(Value::as_bool).unwrap_or(false)
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
