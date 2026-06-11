use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DyVideoCard {
    pub video_id: String,
    pub title: String,
    pub author: String,
    pub author_url: String,
    pub likes: String,
    pub comments: String,
    pub shares: String,
    pub collects: String,
    pub duration: String,
    pub publish_time: String,
    pub link: String,
    pub cover_url: String,
    pub position: i64,
}
