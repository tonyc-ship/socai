//! Persistent per-run state for the agent loop.
//!
//! Mirrors `socai/agent/run_state.py` — same file layout under
//! `<run_dir>/run_state/`, so a run produced by the Rust agent can be
//! inspected with the same tooling that already understands the Python
//! agent's output.
//!
//! Files written:
//! - `task.json`            — task + model + created_at
//! - `plan.json`            — optional step list
//! - `artifacts.json`       — registry of all saved artifacts (screenshots, JSON dumps)
//! - `evidence.json`        — entity-like extracts surfaced from tool results
//! - `events.jsonl`         — append-only timeline of assistant turns, tool calls, results
//! - `working_memory.md`    — human-readable summary regenerated on every event
//!
//! All methods are sync-friendly: `RunState` owns a `Mutex` internally so
//! it can be shared across async tasks without `await`-holding a lock.

// `expect("poisoned")` is the idiomatic pattern for `Mutex::lock` failures —
// poisoning here would mean a panic in another thread while holding the
// lock, which we treat as fatal anyway.
#![allow(clippy::expect_used)]
// `record_artifact` and `write_json_artifact` legitimately need 8–10 fields
// (path, label, kind, source_tool, turn, summary, metadata, payload). A
// builder would just hide the same parameters behind more code.
#![allow(clippy::too_many_arguments)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::agent::compaction::{compact_value, truncate};

fn utc_now() -> String {
    let now: DateTime<Utc> = Utc::now();
    now.to_rfc3339()
}

fn write_json(path: &Path, payload: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(payload).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

fn append_jsonl(path: &Path, entry: &Value) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut handle = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
    handle.write_all(line.as_bytes())?;
    handle.write_all(b"\n")
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactRecord {
    pub key: String,
    pub path: String,
    pub label: String,
    pub kind: String,
    pub source_tool: String,
    pub turn: Option<u32>,
    pub summary: String,
    pub metadata: Value,
}

#[derive(Debug)]
struct Inner {
    run_dir: PathBuf,
    state_dir: PathBuf,
    task: String,
    plan: Value,
    artifacts: BTreeMap<String, ArtifactRecord>,
    evidence: BTreeMap<String, Value>,
    recent_events: Vec<Value>,
    max_recent_events: usize,
}

#[derive(Debug)]
pub struct RunState {
    inner: Mutex<Inner>,
}

impl RunState {
    pub fn new(
        run_dir: impl AsRef<Path>,
        task: impl Into<String>,
        model: impl Into<String>,
    ) -> std::io::Result<Self> {
        let run_dir = run_dir.as_ref().to_path_buf();
        let state_dir = run_dir.join("run_state");
        std::fs::create_dir_all(&state_dir)?;
        let task = task.into().trim().to_string();
        let model = model.into().trim().to_string();
        let inner = Inner {
            run_dir: run_dir.clone(),
            state_dir: state_dir.clone(),
            task: task.clone(),
            plan: json!({
                "task": task,
                "updated_at": utc_now(),
                "steps": [],
                "notes": [],
            }),
            artifacts: BTreeMap::new(),
            evidence: BTreeMap::new(),
            recent_events: Vec::new(),
            max_recent_events: 80,
        };

        write_json(
            &state_dir.join("task.json"),
            &json!({
                "task": task,
                "model": model,
                "created_at": utc_now(),
                "run_dir": run_dir.to_string_lossy(),
            }),
        )?;
        write_json(&state_dir.join("plan.json"), &inner.plan)?;
        write_json(
            &state_dir.join("artifacts.json"),
            &json!({"count": 0, "items": []}),
        )?;
        write_json(
            &state_dir.join("evidence.json"),
            &json!({"count": 0, "items": []}),
        )?;

        let state = Self {
            inner: Mutex::new(inner),
        };
        state.flush_working_memory();
        Ok(state)
    }

    pub fn state_dir(&self) -> PathBuf {
        self.inner.lock().expect("poisoned").state_dir.clone()
    }

    pub fn run_dir(&self) -> PathBuf {
        self.inner.lock().expect("poisoned").run_dir.clone()
    }

    fn append_event(&self, event: Value) {
        let mut event_obj = match event {
            Value::Object(m) => m,
            other => {
                let mut m = Map::new();
                m.insert("value".into(), other);
                m
            }
        };
        event_obj.insert("timestamp".into(), Value::String(utc_now()));
        let enriched = Value::Object(event_obj);

        let mut guard = self.inner.lock().expect("poisoned");
        let path = guard.state_dir.join("events.jsonl");
        let _ = append_jsonl(&path, &enriched);
        guard.recent_events.push(enriched);
        if guard.recent_events.len() > guard.max_recent_events {
            let overflow = guard.recent_events.len() - guard.max_recent_events;
            guard.recent_events.drain(0..overflow);
        }
    }

    pub fn note_assistant_turn(&self, turn: u32, text: &str, tool_calls: &[Value]) {
        self.append_event(json!({
            "type": "assistant_turn",
            "turn": turn,
            "text": truncate(text, 800),
            "tool_calls": tool_calls,
        }));
        self.flush_working_memory();
    }

    pub fn note_tool_call(&self, turn: u32, tool_name: &str, tool_input: &Value) {
        self.append_event(json!({
            "type": "tool_call",
            "turn": turn,
            "tool": tool_name,
            "input": compact_value(tool_input),
        }));
        self.flush_working_memory();
    }

    pub fn note_tool_result(
        &self,
        turn: u32,
        tool_name: &str,
        tool_input: &Value,
        result_summary: &str,
        duration_s: f64,
    ) {
        self.append_event(json!({
            "type": "tool_result",
            "turn": turn,
            "tool": tool_name,
            "input": compact_value(tool_input),
            "result_summary": truncate(result_summary, 800),
            "duration_s": duration_s,
        }));
        self.flush_working_memory();
    }

    pub fn record_artifact(
        &self,
        relative_path: &str,
        label: &str,
        kind: &str,
        source_tool: &str,
        turn: Option<u32>,
        summary: &str,
        metadata: Value,
        payload: Option<&Value>,
    ) {
        let record = ArtifactRecord {
            key: format!("{:03}_{label}", {
                let g = self.inner.lock().expect("poisoned");
                let next = g.artifacts.len() + 1;
                drop(g);
                next
            }),
            path: relative_path.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            source_tool: source_tool.to_string(),
            turn,
            summary: summary.to_string(),
            metadata: metadata.clone(),
        };
        {
            let mut guard = self.inner.lock().expect("poisoned");
            guard.artifacts.insert(record.key.clone(), record.clone());
            let items: Vec<&ArtifactRecord> = guard.artifacts.values().collect();
            let _ = write_json(
                &guard.state_dir.join("artifacts.json"),
                &json!({"count": items.len(), "items": items}),
            );
        }

        // Best-effort evidence ingest when the artifact carries entity-like data.
        if let Some(payload) = payload {
            self.ingest_evidence(payload, relative_path, turn);
        }

        self.append_event(json!({
            "type": "artifact_recorded",
            "turn": turn,
            "path": relative_path,
            "label": label,
            "kind": kind,
            "source_tool": source_tool,
        }));
        self.flush_working_memory();
    }

    fn ingest_evidence(&self, payload: &Value, artifact_path: &str, turn: Option<u32>) {
        let candidate = match payload {
            Value::Object(map) => Some(map.clone()),
            _ => None,
        };
        let Some(map) = candidate else { return };
        let interesting = [
            "id",
            "entity_id",
            "note_id",
            "url",
            "resolved_url",
            "title",
            "author",
            "content",
            "content_summary",
            "summary",
            "screenshot",
        ];
        let has_signal = interesting.iter().any(|k| {
            map.get(*k)
                .map(|v| {
                    !matches!(v, Value::Null) && !v.is_string()
                        || v.as_str().is_some_and(|s| !s.is_empty())
                })
                .unwrap_or(false)
        });
        if !has_signal {
            return;
        }
        let key = map
            .get("entity_id")
            .or_else(|| map.get("note_id"))
            .or_else(|| map.get("id"))
            .or_else(|| map.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| artifact_path.to_string());

        let mut compact = match compact_value(&Value::Object(map.clone())) {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        compact.insert("key".into(), Value::String(key.clone()));
        compact.insert(
            "artifact_path".into(),
            Value::String(artifact_path.to_string()),
        );
        if let Some(turn) = turn {
            compact.insert("turn".into(), json!(turn));
        }

        let mut guard = self.inner.lock().expect("poisoned");
        guard.evidence.insert(key, Value::Object(compact));
        let items: Vec<&Value> = guard.evidence.values().collect();
        let _ = write_json(
            &guard.state_dir.join("evidence.json"),
            &json!({"count": items.len(), "items": items}),
        );
    }

    pub fn update_plan_steps(&self, steps: Vec<Value>) {
        let mut guard = self.inner.lock().expect("poisoned");
        let plan = guard.plan.as_object_mut().expect("plan is always object");
        plan.insert("steps".into(), Value::Array(steps));
        plan.insert("updated_at".into(), Value::String(utc_now()));
        let snapshot = guard.plan.clone();
        let _ = write_json(&guard.state_dir.join("plan.json"), &snapshot);
        drop(guard);
        self.append_event(json!({"type": "plan_update"}));
        self.flush_working_memory();
    }

    fn flush_working_memory(&self) {
        let text = self.render_working_memory(10, 8);
        let path = {
            let guard = self.inner.lock().expect("poisoned");
            guard.state_dir.join("working_memory.md")
        };
        let _ = std::fs::write(path, text);
    }

    pub fn render_working_memory(&self, max_recent_events: usize, max_evidence: usize) -> String {
        let guard = self.inner.lock().expect("poisoned");
        let mut out = String::new();
        out.push_str("# Task\n");
        if guard.task.is_empty() {
            out.push_str("(empty task)\n");
        } else {
            out.push_str(&guard.task);
            out.push('\n');
        }
        out.push_str("\n# Plan\n");
        let steps = guard
            .plan
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if steps.is_empty() {
            out.push_str("- No explicit plan has been recorded yet.\n");
        } else {
            for step in &steps {
                let status = step
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("pending");
                let title = step
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let details = step
                    .get("details")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if details.is_empty() {
                    out.push_str(&format!("- [{status}] {title}\n"));
                } else {
                    out.push_str(&format!("- [{status}] {title} — {details}\n"));
                }
            }
        }
        out.push_str(&format!(
            "\n# Current State\n- Saved artifacts: {}\n- Evidence records: {}\n",
            guard.artifacts.len(),
            guard.evidence.len()
        ));
        let latest_result = guard
            .recent_events
            .iter()
            .rev()
            .find(|e| e.get("type").and_then(Value::as_str) == Some("tool_result"));
        if let Some(event) = latest_result {
            let turn = event.get("turn").and_then(Value::as_u64).unwrap_or(0);
            let tool = event.get("tool").and_then(Value::as_str).unwrap_or("");
            let summary = event
                .get("result_summary")
                .and_then(Value::as_str)
                .unwrap_or("");
            out.push_str(&format!(
                "- Latest tool result: turn {turn} {tool} — {}\n",
                truncate(summary, 200)
            ));
        }
        out.push_str("\n# Recent Activity\n");
        let recent: Vec<&Value> = guard
            .recent_events
            .iter()
            .rev()
            .take(max_recent_events)
            .collect();
        if recent.is_empty() {
            out.push_str("- No activity has been recorded yet.\n");
        } else {
            for event in recent.iter().rev() {
                let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
                let turn = event.get("turn").and_then(Value::as_u64).unwrap_or(0);
                match kind {
                    "assistant_turn" => {
                        let text = event.get("text").and_then(Value::as_str).unwrap_or("");
                        out.push_str(&format!(
                            "- turn {turn} assistant: {}\n",
                            truncate(text, 160)
                        ));
                    }
                    "tool_call" => {
                        let tool = event.get("tool").and_then(Value::as_str).unwrap_or("");
                        out.push_str(&format!("- turn {turn} tool_call {tool}\n"));
                    }
                    "tool_result" => {
                        let tool = event.get("tool").and_then(Value::as_str).unwrap_or("");
                        let summary = event
                            .get("result_summary")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        out.push_str(&format!(
                            "- turn {turn} tool_result {tool}: {}\n",
                            truncate(summary, 160)
                        ));
                    }
                    "plan_update" => {
                        out.push_str(&format!("- turn {turn} plan updated\n"));
                    }
                    "artifact_recorded" => {
                        let path = event.get("path").and_then(Value::as_str).unwrap_or("");
                        out.push_str(&format!("- turn {turn} saved {path}\n"));
                    }
                    _ => {}
                }
            }
        }
        out.push_str("\n# Key Evidence\n");
        let evidence_items: Vec<&Value> =
            guard.evidence.values().rev().take(max_evidence).collect();
        if evidence_items.is_empty() {
            out.push_str("- No structured evidence has been extracted yet.\n");
        } else {
            for item in evidence_items.iter().rev() {
                let title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("key").and_then(Value::as_str))
                    .unwrap_or("");
                let author = item
                    .get("author")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let summary = item
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let mut line = format!("- {title}");
                if !author.is_empty() {
                    line.push_str(&format!(" — {author}"));
                }
                if !summary.is_empty() {
                    line.push_str(&format!(" | {}", truncate(summary, 180)));
                }
                line.push('\n');
                out.push_str(&line);
            }
        }
        out.push_str(
            "\n# Retrieval\n\
- Use a task-plan tool, if one is registered, to keep a live checklist for complex tasks.\n\
- Use run-state or artifact-reading tools, if registered by the host, to revisit earlier findings.\n",
        );
        out
    }

    pub fn context_block(&self, max_chars: usize) -> String {
        truncate(&self.render_working_memory(10, 8), max_chars)
    }

    pub fn has_structured_state(&self) -> bool {
        let guard = self.inner.lock().expect("poisoned");
        if guard
            .plan
            .get("steps")
            .and_then(Value::as_array)
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        if !guard.evidence.is_empty() {
            return true;
        }
        for artifact in guard.artifacts.values() {
            let kind = artifact.kind.to_ascii_lowercase();
            let metadata = artifact.metadata.as_object();
            let is_screenshot = kind == "image"
                && metadata
                    .and_then(|m| m.get("category"))
                    .and_then(Value::as_str)
                    == Some("screenshot");
            if !is_screenshot {
                return true;
            }
        }
        false
    }

    pub fn artifact_records(&self) -> Vec<ArtifactRecord> {
        let guard = self.inner.lock().expect("poisoned");
        guard.artifacts.values().cloned().collect()
    }
}
