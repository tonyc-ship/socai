use anyhow::Result;
use serde_json::{json, Map, Value};
use socai_browser::PageSession;

use crate::xhs::entities::{normalize_url, XhsNote};

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

/// Site-aware XHS operations on top of a CDP `PageSession`. MVP scope:
/// `extract_note` only. Search / topic / open-note flows need CDP input
/// primitives that crates/browser doesn't expose yet — those land when
/// the agent loop in phase 3 actually needs them.
pub struct XhsSiteRuntime<'a> {
    page: &'a PageSession,
}

impl<'a> XhsSiteRuntime<'a> {
    pub fn new(page: &'a PageSession) -> Self {
        Self { page }
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

        note.wait_meta = Some(json!({
            "ready": raw.get("ready").and_then(Value::as_bool).unwrap_or(false),
            "reason": raw.get("reason").and_then(Value::as_str).unwrap_or(""),
            "waited_ms": raw.get("waited_ms").and_then(Value::as_i64).unwrap_or(0),
            "attempts": raw.get("attempts").and_then(Value::as_i64).unwrap_or(0),
        }));

        Ok(note)
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
            arr.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(String::from)
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
