use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DyPageState {
    pub site: String,
    pub url: String,
    pub title: String,
    pub path: String,
    pub ready_state: String,
    pub signed_in: Option<bool>,
}
