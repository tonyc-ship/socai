use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;
use tokio::process::Command;

use crate::media::common::{
    ensure_dir, find_in_path, insert_string, insert_value, short, url_suffix, MediaUnavailable,
    USER_AGENT,
};
use crate::media::md5;
use crate::media::processor::MediaProcessor;

impl MediaProcessor {
    pub async fn enrich_video(
        &self,
        video: &Value,
        note_id: &str,
        title: &str,
        referer: &str,
        max_frames: usize,
        run_vision: bool,
    ) -> Value {
        let t_total = Instant::now();
        let mut result = video.clone();
        if !result.is_object() {
            result = crate::media::common::empty_object();
        }
        let source = result
            .get("resolved_url")
            .or_else(|| result.get("url"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let poster_url = result
            .get("poster_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let label = if !note_id.trim().is_empty() {
            note_id
        } else if !title.trim().is_empty() {
            title
        } else {
            "video"
        };

        if !poster_url.is_empty() {
            self.enrich_video_poster(&mut result, &poster_url, referer, label, title, run_vision)
                .await;
        }

        if !source.is_empty() {
            match self.transcribe_audio(&source, referer, "").await {
                Ok(transcript) if !transcript.trim().is_empty() => {
                    insert_string(&mut result, "transcript", transcript.clone());
                    insert_string(&mut result, "transcript_summary", short(&transcript, 1200));
                }
                Ok(_) => {}
                Err(err) => insert_string(&mut result, "transcript_error", format!("{err:#}")),
            }

            self.enrich_video_frames(&mut result, &source, referer, max_frames, title, run_vision)
                .await;
        }

        self.timing.record("video_enrich_total", t_total.elapsed());
        result
    }

    async fn enrich_video_poster(
        &self,
        result: &mut Value,
        poster_url: &str,
        referer: &str,
        label: &str,
        title: &str,
        run_vision: bool,
    ) {
        match self.download_bytes(poster_url, referer).await {
            Ok(poster) if !poster.is_empty() => {
                match self.save_bytes(
                    &poster,
                    &format!("{label}_poster"),
                    &url_suffix(poster_url, ".jpg"),
                ) {
                    Ok(path) => insert_string(result, "poster_local_path", path.to_string_lossy()),
                    Err(err) => insert_string(result, "poster_save_error", format!("{err:#}")),
                }
                if self.config.use_ocr {
                    match self.ocr_image(&poster) {
                        Ok(text) => insert_string(result, "poster_ocr", short(&text, 800)),
                        Err(err) => insert_string(result, "poster_ocr_error", format!("{err:#}")),
                    }
                }
                if run_vision && self.config.use_vision {
                    match self
                        .describe_image(
                            &poster,
                            &format!("Describe the poster image for Xiaohongshu video: {title}"),
                            180,
                        )
                        .await
                    {
                        Ok(text) => insert_string(result, "poster_description", text),
                        Err(err) => {
                            insert_string(result, "poster_vision_error", format!("{err:#}"))
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(err) => insert_string(result, "poster_download_error", format!("{err:#}")),
        }
    }

    async fn enrich_video_frames(
        &self,
        result: &mut Value,
        source: &str,
        referer: &str,
        max_frames: usize,
        title: &str,
        run_vision: bool,
    ) {
        match self.extract_video_frames(source, referer, max_frames).await {
            Ok(frame_paths) => {
                let frame_values: Vec<Value> = frame_paths
                    .iter()
                    .map(|p| Value::String(p.to_string_lossy().to_string()))
                    .collect();
                insert_value(result, "frame_paths", Value::Array(frame_values));
                let mut frame_notes = Vec::new();
                for frame_path in frame_paths {
                    let payload = match tokio::fs::read(&frame_path).await {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    };
                    if run_vision && self.config.use_vision {
                        match self
                            .describe_image(
                                &payload,
                                &format!("Describe this sampled video frame for: {title}"),
                                180,
                            )
                            .await
                        {
                            Ok(text) if !text.trim().is_empty() => frame_notes.push(text),
                            _ => {}
                        }
                    } else if self.config.use_ocr {
                        match self.ocr_image(&payload) {
                            Ok(text) if !text.trim().is_empty() => frame_notes.push(text),
                            _ => {}
                        }
                    }
                }
                if !frame_notes.is_empty() {
                    insert_value(
                        result,
                        "frame_descriptions",
                        Value::Array(frame_notes.iter().cloned().map(Value::String).collect()),
                    );
                    insert_string(
                        result,
                        "visual_summary",
                        short(&frame_notes.join("\n"), 1200),
                    );
                }
            }
            Err(err) => insert_string(result, "frame_error", format!("{err:#}")),
        }
    }

    pub async fn extract_video_frames(
        &self,
        source: &str,
        referer: &str,
        num_frames: usize,
    ) -> Result<Vec<PathBuf>> {
        if !self.config.use_ffmpeg {
            anyhow::bail!(MediaUnavailable(
                "ffmpeg frame extraction is disabled".into()
            ));
        }
        if find_in_path("ffmpeg").is_none() {
            anyhow::bail!(MediaUnavailable(
                "ffmpeg is not installed or not on PATH".into()
            ));
        }
        let t0 = Instant::now();
        let digest = md5::md5_hex(source.as_bytes());
        let frame_dir = ensure_dir(&self.config.base_dir.join("frames").join(&digest[..10]))?;
        let pattern = frame_dir.join("frame_%02d.jpg");
        let safe_frames = num_frames.max(1);
        let safe_seconds = self.config.max_frame_seconds.max(1);
        let interval = (safe_seconds / safe_frames as u64).max(1);
        let mut command = Command::new("ffmpeg");
        command.arg("-hide_banner").arg("-loglevel").arg("error");
        if !referer.trim().is_empty()
            && (source.starts_with("http://") || source.starts_with("https://"))
        {
            command.arg("-headers").arg(format!(
                "Referer: {referer}\r\nUser-Agent: {USER_AGENT}\r\n"
            ));
        }
        command
            .arg("-t")
            .arg(safe_seconds.to_string())
            .arg("-i")
            .arg(source)
            .arg("-vf")
            .arg(format!("fps=1/{interval},scale=min(960\\,iw):-2"))
            .arg("-frames:v")
            .arg(safe_frames.to_string())
            .arg(pattern);
        let result = crate::media::common::run_command(
            &mut command,
            Duration::from_secs(self.config.ffmpeg_timeout_s),
        )
        .await;
        self.timing.record("video_frame_extract", t0.elapsed());
        result?;

        let mut paths = Vec::new();
        let mut entries = tokio::fs::read_dir(frame_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| name.starts_with("frame_") && name.ends_with(".jpg"))
            {
                paths.push(path);
            }
        }
        paths.sort();
        Ok(paths)
    }
}
