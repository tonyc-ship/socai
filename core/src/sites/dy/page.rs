use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::cdp::PageSession;
use crate::sites::dy::entities::{normalize_url, DyVideoCard};

pub const DY_HOME_URL: &str = "https://www.douyin.com/";

const PAGE_SCRIPTS_JS: &str = include_str!("page_scripts.js");
const SEARCH_TRANSITION_TIMEOUT_S: f64 = 12.0;
const DY_PAGE_SCRIPT_FUNCTIONS: &[&str] = &[
    "pageState",
    "searchInput",
    "setSearchInput",
    "searchState",
    "searchTabs",
    "clickSearchTab",
    "videoCards",
    "scrollFeed",
];

pub struct DyPageRuntime<'a> {
    page: &'a PageSession,
}

impl<'a> DyPageRuntime<'a> {
    pub fn new(page: &'a PageSession) -> Self {
        Self { page }
    }

    pub async fn run_script(&self, name: &str, arg: Option<&Value>) -> Result<Value> {
        if !DY_PAGE_SCRIPT_FUNCTIONS.contains(&name) {
            anyhow::bail!("Unknown Douyin page script: {name}");
        }
        let args = match arg {
            None => String::new(),
            Some(v) => serde_json::to_string(v)?,
        };
        let expr =
            format!("{PAGE_SCRIPTS_JS}\n// SOCAI_DY_CALL: {name}\nreturn SocaiDyPageScripts.{name}({args});");
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

    pub async fn ensure_dy(&self, navigate_if_needed: bool) -> Result<()> {
        let url = self.current_url().await?;
        if url.contains("douyin.com") {
            return Ok(());
        }
        if navigate_if_needed {
            self.page.navigate_with_timeout(DY_HOME_URL, 60.0).await?;
            return Ok(());
        }
        anyhow::bail!(
            "Current page is not Douyin: {}",
            if url.is_empty() { "unknown" } else { &url }
        );
    }

    pub async fn detect_state(&self, navigate_if_needed: bool) -> Result<Value> {
        self.ensure_dy(navigate_if_needed).await?;
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

        self.ensure_dy(true).await?;
        let submit = self.submit_search(keyword, wait_seconds).await?;
        let ok = script_ok(&submit);
        let tab = if ok {
            self.click_search_tab("视频", wait_seconds).await?
        } else {
            Value::Object(Map::new())
        };
        let cards = if ok {
            match num_videos {
                Some(target) if target > 0 => self.collect_video_cards(target).await?,
                _ => self.extract_video_cards().await?,
            }
        } else {
            Vec::new()
        };
        Ok(json!({
            "ok": ok,
            "query": keyword,
            "submit": submit,
            "tab": tab,
            "url": self.current_url().await?,
            "count": cards.len(),
            "videos": cards,
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
                "state": set_result,
                "error": "Search input did not accept the requested keyword",
            }));
        }

        sleep_ms(150).await;
        self.page.press_key("Enter").await?;
        let state = self
            .wait_for_search_transition(query, wait_seconds.max(SEARCH_TRANSITION_TIMEOUT_S))
            .await?;
        if search_transition_ok(&state) {
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
                    .wait_for_search_transition(
                        query,
                        wait_seconds.max(SEARCH_TRANSITION_TIMEOUT_S),
                    )
                    .await?;
                if search_transition_ok(&state) {
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
                "Search did not transition to a valid Douyin result page"
            },
        }))
    }

    pub async fn wait_for_search_transition(&self, _query: &str, timeout_s: f64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs_f64(timeout_s.max(0.2));
        let mut latest = Value::Object(Map::new());
        while Instant::now() < deadline {
            latest = self.expect_object("searchState", None).await?;
            if search_transition_ok(&latest) {
                return Ok(latest);
            }
            sleep_ms(200).await;
        }
        if latest.as_object().is_some_and(Map::is_empty) {
            self.expect_object("searchState", None).await
        } else {
            Ok(latest)
        }
    }

    pub async fn extract_video_cards(&self) -> Result<Vec<DyVideoCard>> {
        self.ensure_dy(false).await?;
        let raw = self.expect_array("videoCards", None).await?;
        Ok(parse_cards(&raw))
    }

    pub async fn click_search_tab(&self, label: &str, wait_seconds: f64) -> Result<Value> {
        self.ensure_dy(false).await?;
        let target = self
            .expect_object("clickSearchTab", Some(&Value::String(label.to_string())))
            .await?;
        if !script_ok(&target) {
            return Ok(target);
        }
        if target
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(json!({ "ok": true, "strategy": "already_active", "target": target }));
        }
        self.page
            .click(number(&target, "x"), number(&target, "y"))
            .await?;
        let state = self
            .wait_for_video_tab(wait_seconds.max(SEARCH_TRANSITION_TIMEOUT_S))
            .await?;
        Ok(json!({
            "ok": search_video_tab_ok(&state),
            "strategy": "click_search_tab",
            "target": target,
            "state": state,
            "url": self.current_url().await?,
        }))
    }

    async fn wait_for_video_tab(&self, timeout_s: f64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs_f64(timeout_s.max(0.2));
        let mut latest = Value::Object(Map::new());
        while Instant::now() < deadline {
            latest = self.expect_object("searchState", None).await?;
            if search_video_tab_ok(&latest) {
                return Ok(latest);
            }
            sleep_ms(200).await;
        }
        Ok(latest)
    }

    pub async fn scroll_feed(&self, nudge_up: bool) -> Result<Value> {
        self.ensure_dy(false).await?;
        self.expect_object("scrollFeed", Some(&json!({ "nudge_up": nudge_up })))
            .await
    }

    async fn wait_for_card_growth(
        &self,
        baseline: usize,
        timeout: Duration,
    ) -> Result<Vec<DyVideoCard>> {
        const POLL: Duration = Duration::from_millis(500);
        let deadline = Instant::now() + timeout;
        loop {
            sleep_ms(POLL.as_millis() as u64).await;
            let cards = self.extract_video_cards().await?;
            if cards.len() > baseline || Instant::now() >= deadline {
                return Ok(cards);
            }
        }
    }

    async fn collect_video_cards(&self, target: usize) -> Result<Vec<DyVideoCard>> {
        const PRE_SCROLL_DELAY: Duration = Duration::from_millis(1200);
        const SETTLE_TIMEOUT: Duration = Duration::from_millis(5000);
        const MAX_STALLS: usize = 3;

        let mut cards = self.extract_video_cards().await?;
        let mut stalls = 0usize;
        while cards.len() < target {
            let before = cards.len();
            sleep_ms(PRE_SCROLL_DELAY.as_millis() as u64).await;
            self.scroll_feed(false).await?;
            cards = self.wait_for_card_growth(before, SETTLE_TIMEOUT).await?;
            if cards.len() <= before {
                self.scroll_feed(true).await?;
                cards = self.wait_for_card_growth(before, SETTLE_TIMEOUT).await?;
            }
            if cards.len() <= before {
                stalls += 1;
                if stalls >= MAX_STALLS {
                    break;
                }
            } else {
                stalls = 0;
            }
        }
        cards.truncate(target);
        Ok(cards)
    }

    async fn expect_object(&self, name: &str, arg: Option<&Value>) -> Result<Value> {
        let value = self.run_script(name, arg).await?;
        if value.as_object().is_none() {
            anyhow::bail!(
                "Douyin script {name} returned {}, expected object",
                value_type(&value)
            );
        }
        Ok(value)
    }

    async fn expect_array(&self, name: &str, arg: Option<&Value>) -> Result<Value> {
        let value = self.run_script(name, arg).await?;
        if value.as_array().is_none() {
            anyhow::bail!(
                "Douyin script {name} returned {}, expected array",
                value_type(&value)
            );
        }
        Ok(value)
    }
}

fn parse_cards(raw: &Value) -> Vec<DyVideoCard> {
    raw.as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| serde_json::from_value::<DyVideoCard>(item.clone()).ok())
        .map(|mut card| {
            card.url = normalize_url(&card.url);
            card.author_url = normalize_url(&card.author_url);
            card.cover_url = normalize_url(&card.cover_url);
            card
        })
        .collect()
}

fn search_transition_ok(state: &Value) -> bool {
    let on_search_page = state
        .get("page_state")
        .and_then(Value::as_str)
        .is_some_and(|state| state == "search_results");
    (on_search_page
        && state
            .get("card_count")
            .and_then(Value::as_i64)
            .is_some_and(|count| count > 0))
        || state
            .get("has_no_results")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn search_video_tab_ok(state: &Value) -> bool {
    state
        .get("tab_type")
        .and_then(Value::as_str)
        .is_some_and(|tab| tab == "video")
        && state
            .get("card_count")
            .and_then(Value::as_i64)
            .is_some_and(|count| count > 0)
}

fn script_ok(value: &Value) -> bool {
    value.get("ok").and_then(Value::as_bool).unwrap_or(false)
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
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
