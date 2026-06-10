use anyhow::Result;
use serde_json::Value;

use crate::cdp::PageSession;

pub const DY_HOME_URL: &str = "https://www.douyin.com/";

const PAGE_SCRIPTS_JS: &str = include_str!("page_scripts.js");

const DY_PAGE_SCRIPT_FUNCTIONS: &[&str] = &["pageState"];

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
            Some(value) => serde_json::to_string(value)?,
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
            self.page.navigate_with_timeout(DY_HOME_URL, 60.0).await?;
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

    pub async fn open_home_and_detect_state(&self) -> Result<Value> {
        self.ensure_dy(true).await?;
        self.detect_state().await
    }
}
