use std::collections::HashSet;
use std::time::Instant;

use anyhow::Result;
use base64::Engine;
use serde_json::Value;
use socai_agent::{Block, Message, MessageRole, ToolSchema};

use crate::common::{detect_media_type, insert_string, short, url_suffix, MediaUnavailable};
use crate::md5;
use crate::processor::MediaProcessor;

impl MediaProcessor {
    pub fn ocr_image(&self, _payload: &[u8]) -> Result<String> {
        if !self.config.use_ocr {
            anyhow::bail!(MediaUnavailable("OCR is disabled".into()));
        }
        anyhow::bail!(MediaUnavailable(
            "OCR is unavailable in the Rust media processor".into()
        ));
    }

    pub async fn describe_image(
        &self,
        payload: &[u8],
        prompt: &str,
        max_tokens: u32,
    ) -> Result<String> {
        if !self.config.use_vision {
            anyhow::bail!(MediaUnavailable("Vision is disabled".into()));
        }
        let Some(llm_provider) = &self.llm_provider else {
            anyhow::bail!(MediaUnavailable(
                "No agent LLM provider was provided for image vision".into()
            ));
        };
        if payload.is_empty() {
            return Ok(String::new());
        }
        let t0 = Instant::now();
        let media_type = detect_media_type(payload);
        let data = base64::engine::general_purpose::STANDARD.encode(payload);
        let response = llm_provider
            .send(
                "You describe images concisely and only state visible evidence.",
                &[Message {
                    role: MessageRole::User,
                    content: socai_agent::MessageContent::Blocks(vec![
                        Block::Text {
                            text: prompt.to_string(),
                        },
                        Block::Image { data, media_type },
                    ]),
                }],
                &[] as &[ToolSchema],
                max_tokens,
            )
            .await?;
        self.timing.record("vision_image", t0.elapsed());
        Ok(response.text_blocks.join("\n").trim().to_string())
    }

    pub async fn enrich_images(
        &self,
        images: &[Value],
        referer: &str,
        label: &str,
        run_vision: bool,
    ) -> Vec<Value> {
        if images.is_empty() {
            return Vec::new();
        }

        let t_batch = Instant::now();
        let downloads = images
            .iter()
            .map(|image| {
                let url = image
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.safe_download(url, referer.to_string())
            })
            .collect::<Vec<_>>();
        let download_results = futures::future::join_all(downloads).await;
        self.timing
            .record("image_download_batch", t_batch.elapsed());

        let mut seen = HashSet::new();
        let mut deduped: Vec<(Value, Vec<u8>)> = Vec::new();
        for (image, (payload, error)) in images.iter().zip(download_results) {
            let url = image
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if url.is_empty() {
                continue;
            }
            let mut item = image.clone();
            if let Some(error) = error {
                insert_string(&mut item, "download_error", error);
                deduped.push((item, Vec::new()));
                continue;
            }
            if payload.is_empty() {
                continue;
            }
            let digest = md5::md5_hex(&payload);
            if !seen.insert(digest) {
                continue;
            }
            let path = self.save_bytes(
                &payload,
                &format!("{label}_{}", deduped.len() + 1),
                &url_suffix(url, ".jpg"),
            );
            match path {
                Ok(path) => insert_string(&mut item, "local_path", path.to_string_lossy()),
                Err(err) => insert_string(&mut item, "save_error", format!("{err:#}")),
            }
            deduped.push((item, payload));
        }

        let t_enrich = Instant::now();
        let mut out = Vec::with_capacity(deduped.len());
        for (mut item, payload) in deduped {
            if !payload.is_empty() && self.config.use_ocr && item.get("ocr_text").is_none() {
                match self.ocr_image(&payload) {
                    Ok(text) if !text.trim().is_empty() => {
                        insert_string(&mut item, "ocr_text", short(&text, 800));
                    }
                    Ok(_) => {}
                    Err(err) => insert_string(&mut item, "ocr_error", format!("{err:#}")),
                }
            }
            if run_vision
                && !payload.is_empty()
                && self.config.use_vision
                && item.get("vision_description").is_none()
            {
                match self
                    .describe_image(
                        &payload,
                        "Describe this Xiaohongshu image for the note. Focus on concrete visible facts.",
                        180,
                    )
                    .await
                {
                    Ok(text) if !text.trim().is_empty() => {
                        insert_string(&mut item, "vision_description", text);
                    }
                    Ok(_) => {}
                    Err(err) => insert_string(&mut item, "vision_error", format!("{err:#}")),
                }
            }
            out.push(item);
        }
        self.timing.record("image_enrich_batch", t_enrich.elapsed());
        out
    }

    async fn safe_download(&self, url: String, referer: String) -> (Vec<u8>, Option<String>) {
        if url.trim().is_empty() {
            return (Vec::new(), None);
        }
        match self.download_bytes(&url, &referer).await {
            Ok(bytes) => (bytes, None),
            Err(err) => (Vec::new(), Some(format!("{err:#}"))),
        }
    }
}
