use serde::{Deserialize, Serialize};
use serde_json::Value;

/// XHS note — wire-ready. Field order, names, and types match the JSON
/// produced by `socai.sites.xhs.entities.XhsNote.to_dict()` exactly, so
/// `jq -S` diffs between the two implementations stay clean.
///
/// All normalization (strip URL fragment, clip hashtags to 12, image_count
/// fallback to images.len()) is performed during parsing, so serializing
/// directly with serde_json yields the same output Python's to_dict() does.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XhsNote {
    pub note_id: String,
    pub url: String,
    pub r#type: String,
    pub title: String,
    pub author: String,
    pub author_id: String,
    pub author_url: String,
    pub content: String,
    pub content_source: String,
    pub hashtags: Vec<String>,
    pub date: String,
    pub location: String,
    pub ip_location: String,
    pub likes: String,
    pub favorites: String,
    pub comments_count: String,
    pub image_count: i64,
    pub images: Vec<Value>,
    pub video: Value,
    pub extraction_level: String,

    /// Python sets `note.wait_meta` and to_dict() renames it to `"wait"`.
    #[serde(rename = "wait", skip_serializing_if = "Option::is_none", default)]
    pub wait_meta: Option<Value>,

    /// Only present when consecutive extracts return the same note_id —
    /// MVP doesn't implement the cross-call tracking, so this stays None.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stale_warning: Option<String>,
}

impl Default for XhsNote {
    fn default() -> Self {
        Self {
            note_id: String::new(),
            url: String::new(),
            r#type: String::new(),
            title: String::new(),
            author: String::new(),
            author_id: String::new(),
            author_url: String::new(),
            content: String::new(),
            content_source: String::new(),
            hashtags: Vec::new(),
            date: String::new(),
            location: String::new(),
            ip_location: String::new(),
            likes: String::new(),
            favorites: String::new(),
            comments_count: String::new(),
            image_count: 0,
            images: Vec::new(),
            // Python defaults video to {}, not null. Match that so the wire
            // shape stays consistent for video-less notes.
            video: Value::Object(Default::default()),
            extraction_level: "lite".into(),
            wait_meta: None,
            stale_warning: None,
        }
    }
}

/// Drop the URL fragment, keep scheme/netloc/path/query intact. Matches
/// Python's `urlsplit/urlunsplit`-based `normalize_url` for the URL shapes
/// XHS actually emits (which never put '#' in path or query).
pub(crate) fn normalize_url(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match trimmed.find('#') {
        Some(idx) => trimmed[..idx].to_string(),
        None => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_strips_fragment() {
        assert_eq!(
            normalize_url("https://www.xiaohongshu.com/explore/abc?x=1#frag"),
            "https://www.xiaohongshu.com/explore/abc?x=1"
        );
    }

    #[test]
    fn normalize_url_passes_through_clean_urls() {
        assert_eq!(
            normalize_url("https://www.xiaohongshu.com/explore/abc?x=1"),
            "https://www.xiaohongshu.com/explore/abc?x=1"
        );
    }

    #[test]
    fn normalize_url_handles_empty() {
        assert_eq!(normalize_url(""), "");
        assert_eq!(normalize_url("   "), "");
    }
}
