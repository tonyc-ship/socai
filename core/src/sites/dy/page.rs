use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};

use crate::cdp::PageSession;
use crate::sites::dy::entities::DyVideoCard;

pub const DY_HOME_URL: &str = "https://www.douyin.com/";

const PAGE_SCRIPTS_JS: &str = include_str!("page_scripts.js");
const DY_OPEN_TIMEOUT_S: f64 = 300.0;
const DY_PAGE_SCRIPT_FUNCTIONS: &[&str] = &[
    "pageState",
    "searchInput",
    "setSearchInput",
    "searchState",
    "videoCards",
    "scrollResults",
];
const SEARCH_TRANSITION_TIMEOUT_S: f64 = 12.0;

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
        let expr = format!(
            "{PAGE_SCRIPTS_JS}\n// SOCAI_DY_CALL: {name}\nreturn SocaiDyPageScripts.{name}({args});"
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

    pub async fn ensure_dy(&self, navigate_if_needed: bool) -> Result<()> {
        let url = self.current_url().await?;
        if url.contains("douyin.com") {
            return Ok(());
        }
        if navigate_if_needed {
            self.page
                .navigate_with_timeout(DY_HOME_URL, DY_OPEN_TIMEOUT_S)
                .await?;
            return Ok(());
        }
        anyhow::bail!(
            "Current page is not Douyin: {}",
            if url.is_empty() { "unknown" } else { &url }
        );
    }

    pub async fn detect_state(&self) -> Result<Value> {
        self.ensure_dy(false).await?;
        self.run_script("pageState", None).await
    }

    pub async fn search_videos(&self, query: &str, num_videos: usize) -> Result<Value> {
        let query = query.trim();
        if query.is_empty() {
            anyhow::bail!("query is required");
        }
        self.ensure_dy(true).await?;
        let submit = self
            .submit_search(query, SEARCH_TRANSITION_TIMEOUT_S)
            .await?;
        let ok = script_ok(&submit);
        let mut videos = if ok {
            self.collect_video_cards(num_videos).await?
        } else {
            Vec::new()
        };
        videos.truncate(num_videos);
        let state = self.detect_state().await?;
        Ok(json!({
            "ok": ok,
            "query": query,
            "requested": num_videos,
            "submit": submit,
            "state": state,
            "url": self.current_url().await?,
            "count": videos.len(),
            "videos": videos,
            "reason": if ok { "" } else { "search_submit_failed" },
        }))
    }

    async fn submit_search(&self, query: &str, wait_seconds: f64) -> Result<Value> {
        let loc = self.run_script("searchInput", None).await?;
        if !script_ok(&loc) {
            return Ok(json!({
                "ok": false,
                "strategy": "search_input_unavailable",
                "state": loc,
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
            .run_script("setSearchInput", Some(&json!({ "query": query })))
            .await?;
        if !script_ok(&set_result) {
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
        let state = self.wait_for_search_transition(wait_seconds).await?;
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
                let state = self.wait_for_search_transition(wait_seconds).await?;
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

    async fn wait_for_search_transition(&self, timeout_s: f64) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs_f64(timeout_s.max(0.2));
        let mut latest = Value::Object(Default::default());
        while Instant::now() < deadline {
            latest = self.run_script("searchState", None).await?;
            if search_transition_ok(&latest) {
                return Ok(latest);
            }
            sleep_ms(200).await;
        }
        Ok(latest)
    }

    async fn extract_video_cards(&self) -> Result<Vec<DyVideoCard>> {
        self.ensure_dy(false).await?;
        let raw = self.run_script("videoCards", None).await?;
        let Some(items) = raw.as_array() else {
            return Ok(Vec::new());
        };
        Ok(items
            .iter()
            .filter_map(|item| serde_json::from_value(item.clone()).ok())
            .filter(|card: &DyVideoCard| !card.video_id.is_empty() || !card.link.is_empty())
            .collect())
    }

    async fn collect_video_cards(&self, target: usize) -> Result<Vec<DyVideoCard>> {
        const POST_SCROLL_DELAY: Duration = Duration::from_millis(1800);
        const MAX_STALLS: usize = 6;

        let mut cards = Vec::new();
        merge_cards(&mut cards, self.extract_video_cards().await?);
        let mut stalls = 0usize;
        while cards.len() < target {
            let before = cards.len();
            self.run_script("scrollResults", Some(&json!({ "nudge_up": false })))
                .await?;
            sleep_ms(POST_SCROLL_DELAY.as_millis() as u64).await;
            merge_cards(&mut cards, self.extract_video_cards().await?);

            if cards.len() <= before {
                self.run_script("scrollResults", Some(&json!({ "nudge_up": true })))
                    .await?;
                sleep_ms(POST_SCROLL_DELAY.as_millis() as u64).await;
                merge_cards(&mut cards, self.extract_video_cards().await?);
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
}

fn merge_cards(into: &mut Vec<DyVideoCard>, batch: Vec<DyVideoCard>) {
    for mut card in batch {
        let key = if !card.video_id.is_empty() {
            card.video_id.as_str()
        } else {
            card.link.as_str()
        };
        if key.is_empty() {
            continue;
        }
        if into
            .iter()
            .any(|existing| existing.video_id == key || existing.link == key)
        {
            continue;
        }
        card.position = into.len() as i64;
        into.push(card);
    }
}

fn script_ok(value: &Value) -> bool {
    value.get("ok").and_then(Value::as_bool).unwrap_or(false)
}

fn search_transition_ok(value: &Value) -> bool {
    let page_state = value
        .get("page_state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let card_count = value
        .get("card_count")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let has_no_results = value
        .get("has_no_results")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    page_state == "search_results" && (card_count > 0 || has_no_results)
}

fn number(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or_default()
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}
