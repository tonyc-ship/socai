//! Tool trait + ToolContext.
//!
//! Each tool advertises a name, description, and JSON Schema for its input,
//! plus an async `call` that returns either text or mixed content
//! (text + images) back to the model. Tools own their own state — the
//! `ToolContext` is for *shared* per-run state (counters, run-state handle,
//! enabled sites for gating, …).

// Same rationale as run_state.rs: lock-poisoned panics are fatal.
#![allow(clippy::expect_used)]
// write_json_artifact takes label + payload + 4 metadata fields — see also
// the matching helper in `run_state.rs`.
#![allow(clippy::too_many_arguments)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::run_state::RunState;

/// Content block returned by a tool. Mirrors the subset of Anthropic's
/// content blocks we actually use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResultBlock {
    Text {
        text: String,
    },
    Image {
        /// Base64-encoded image bytes.
        data: String,
        /// IANA media type, e.g. "image/png".
        media_type: String,
    },
}

impl ToolResultBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image_png(data: impl Into<String>) -> Self {
        Self::Image {
            data: data.into(),
            media_type: "image/png".into(),
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Image { .. } => "[image omitted]".into(),
        }
    }
}

/// What a tool's `call` returns.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub blocks: Vec<ToolResultBlock>,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            blocks: vec![ToolResultBlock::text(text)],
        }
    }

    pub fn blocks(blocks: Vec<ToolResultBlock>) -> Self {
        Self { blocks }
    }

    pub fn flat_text(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.as_text())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn has_image(&self) -> bool {
        self.blocks
            .iter()
            .any(|b| matches!(b, ToolResultBlock::Image { .. }))
    }
}

impl From<String> for ToolResult {
    fn from(value: String) -> Self {
        ToolResult::text(value)
    }
}

impl From<&str> for ToolResult {
    fn from(value: &str) -> Self {
        ToolResult::text(value.to_string())
    }
}

/// Per-run shared context. Counters and the run-state handle live in
/// `Arc<Mutex>` so tools can clone the context and still cooperate.
#[derive(Clone)]
pub struct ToolContext {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub turn: u32,
    pub active_tool_name: String,
    pub run_state: Option<Arc<RunState>>,
    pub enabled_sites: Arc<Mutex<BTreeSet<String>>>,
    counters: Arc<Mutex<Counters>>,
}

#[derive(Default)]
struct Counters {
    screenshot: u32,
    artifact: u32,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("run_id", &self.run_id)
            .field("run_dir", &self.run_dir)
            .field("turn", &self.turn)
            .field("active_tool_name", &self.active_tool_name)
            .field("has_run_state", &self.run_state.is_some())
            .finish()
    }
}

impl ToolContext {
    pub fn new(run_id: impl Into<String>, run_dir: impl AsRef<Path>) -> Self {
        Self {
            run_id: run_id.into(),
            run_dir: run_dir.as_ref().to_path_buf(),
            turn: 0,
            active_tool_name: String::new(),
            run_state: None,
            enabled_sites: Arc::new(Mutex::new(BTreeSet::new())),
            counters: Arc::new(Mutex::new(Counters::default())),
        }
    }

    pub fn with_run_state(mut self, run_state: Arc<RunState>) -> Self {
        self.run_state = Some(run_state);
        self
    }

    pub fn enable_site(&self, site: impl Into<String>) {
        if let Ok(mut guard) = self.enabled_sites.lock() {
            guard.insert(site.into());
        }
    }

    pub fn site_enabled(&self, site: &str) -> bool {
        self.enabled_sites
            .lock()
            .map(|g| g.contains(site))
            .unwrap_or(false)
    }

    /// Next screenshot path: `<run_dir>/NNN_<label>.png`.
    pub fn next_screenshot_path(&self, label: &str) -> PathBuf {
        let mut guard = self.counters.lock().expect("poisoned");
        guard.screenshot += 1;
        let label = sanitize_label(label, "screenshot");
        let path = self
            .run_dir
            .join(format!("{:03}_{label}.png", guard.screenshot));
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        path
    }

    /// Next artifact path under `<run_dir>/<subdir>/NNN_<label><suffix>`.
    pub fn next_artifact_path(&self, label: &str, suffix: &str, subdir: &str) -> PathBuf {
        let mut guard = self.counters.lock().expect("poisoned");
        guard.artifact += 1;
        let label = sanitize_label(label, "artifact");
        let dir = self.run_dir.join(subdir);
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("{:03}_{label}{suffix}", guard.artifact))
    }

    /// Register an existing on-disk artifact with the run-state registry.
    /// Returns the path *relative to* the run directory.
    pub fn register_artifact(
        &self,
        path: &Path,
        label: &str,
        kind: &str,
        summary: &str,
        metadata: Value,
        payload: Option<&Value>,
        source_tool: &str,
    ) -> String {
        let rel = path
            .strip_prefix(&self.run_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string_lossy().to_string());
        if let Some(state) = &self.run_state {
            let source = if source_tool.is_empty() {
                self.active_tool_name.as_str()
            } else {
                source_tool
            };
            state.record_artifact(
                &rel,
                label,
                kind,
                source,
                Some(self.turn),
                summary,
                metadata,
                payload,
            );
        }
        rel
    }

    /// Convenience: write a JSON payload to a new artifact path, then register it.
    pub fn write_json_artifact(
        &self,
        label: &str,
        payload: &Value,
        subdir: &str,
        source_tool: &str,
        artifact_kind: &str,
        summary: &str,
        metadata: Value,
    ) -> std::io::Result<String> {
        let path = self.next_artifact_path(label, ".json", subdir);
        let rendered = serde_json::to_string_pretty(payload).map_err(std::io::Error::other)?;
        std::fs::write(&path, rendered)?;
        Ok(self.register_artifact(
            &path,
            label,
            artifact_kind,
            summary,
            metadata,
            Some(payload),
            source_tool,
        ))
    }
}

fn sanitize_label(label: &str, fallback: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let trimmed: String = cleaned.trim_matches('_').to_string();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed
    }
}

/// A tool the agent can call.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;

    /// Tools that should always be exposed regardless of `enabled_sites`.
    fn always_available(&self) -> bool {
        false
    }

    /// Empty string = no gating; otherwise the tool only appears when the
    /// matching site name has been added to `ctx.enabled_sites`.
    fn defer_until_site(&self) -> &str {
        ""
    }

    fn is_available(&self, ctx: &ToolContext) -> bool {
        if self.always_available() {
            return true;
        }
        let site = self.defer_until_site();
        if site.is_empty() {
            return true;
        }
        ctx.site_enabled(site)
    }

    async fn call(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolResult>;
}

pub type SharedTool = Arc<dyn Tool>;

/// A trivial echo tool — used for testing and as a documentation example.
pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo the input text back verbatim. Useful for verifying tool dispatch."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to echo back" }
            },
            "required": ["text"]
        })
    }

    fn always_available(&self) -> bool {
        true
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> anyhow::Result<ToolResult> {
        let text = input
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(ToolResult::text(text))
    }
}
