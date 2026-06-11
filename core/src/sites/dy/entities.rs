use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DyVideoCard {
    pub video_id: String,
    pub title: String,
    pub author: String,
    pub author_url: String,
    pub url: String,
    pub cover_url: String,
    pub likes: String,
    pub comments: String,
    pub shares: String,
    pub duration: String,
    pub raw_text: String,
    pub position: i64,
}

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
