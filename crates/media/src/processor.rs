use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{json, Value};
use socai_agent::Backend as LlmProvider;

use crate::common::{ensure_dir, find_in_path, save_bytes, MediaConfig, USER_AGENT};
use crate::timing::TimingRecord;

#[derive(Clone)]
pub struct MediaProcessor {
    pub(crate) config: MediaConfig,
    pub(crate) llm_provider: Option<Arc<dyn LlmProvider>>,
    pub(crate) client: reqwest::Client,
    pub(crate) timing: Arc<TimingRecord>,
}

impl MediaProcessor {
    pub fn new(config: MediaConfig, llm_provider: Option<Arc<dyn LlmProvider>>) -> Result<Self> {
        ensure_dir(&config.base_dir)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_s))
            .user_agent(USER_AGENT)
            .build()?;
        Ok(Self {
            config,
            llm_provider,
            client,
            timing: Arc::new(TimingRecord::default()),
        })
    }

    pub fn for_run_dir(
        run_dir: impl AsRef<Path>,
        llm_provider: Option<Arc<dyn LlmProvider>>,
    ) -> Result<Self> {
        Self::new(
            MediaConfig::new(run_dir.as_ref().join("site_media")),
            llm_provider,
        )
    }

    pub fn timing(&self) -> Arc<TimingRecord> {
        self.timing.clone()
    }

    pub fn timing_summary(&self) -> Value {
        self.timing.summary()
    }

    pub fn reset_timing(&self) {
        self.timing.reset();
    }

    pub async fn download_bytes(&self, url: &str, referer: &str) -> Result<Vec<u8>> {
        let t0 = Instant::now();
        let result = async {
            let target = url.trim();
            if target.is_empty() {
                return Ok(Vec::new());
            }
            let mut request = self.client.get(target);
            if !referer.trim().is_empty() {
                request = request.header("Referer", referer.trim());
            }
            let bytes = request.send().await?.error_for_status()?.bytes().await?;
            Ok::<Vec<u8>, anyhow::Error>(bytes.to_vec())
        }
        .await;
        self.timing.record("download", t0.elapsed());
        result
    }

    pub fn save_bytes(&self, payload: &[u8], label: &str, suffix: &str) -> Result<PathBuf> {
        save_bytes(&self.config.base_dir, payload, label, suffix)
    }

    pub async fn download_file(
        &self,
        url: &str,
        referer: &str,
        label: &str,
        suffix: &str,
    ) -> Result<PathBuf> {
        let payload = self.download_bytes(url, referer).await?;
        self.save_bytes(&payload, label, suffix)
    }

    pub fn diagnostics(&self) -> Value {
        json!({
            "ffmpeg": find_in_path("ffmpeg").map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            "whisper_cli": find_in_path("whisper").map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            "whisper_cpp": find_in_path("whisper-cli")
                .or_else(|| find_in_path("main"))
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            "whisper_cpp_model": std::env::var("SOCAI_WHISPER_MODEL").unwrap_or_default(),
            "mlx_whisper_model": "",
            "rust_ocr": "",
            "agent_vision_llm_provider": self.llm_provider.as_ref().map(|b| b.label()).unwrap_or_default(),
        })
    }
}
