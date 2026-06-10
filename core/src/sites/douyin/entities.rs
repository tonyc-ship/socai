use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DouyinVideoCard {
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub author: String,
    pub author_url: String,
    pub cover_url: String,
    pub likes: String,
    pub comments: String,
    pub shares: String,
    pub interaction_text: String,
    pub raw_text: String,
    pub position: i64,
}
