use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::task::AbortHandle;

const MAX_CONCURRENT_AGENT_TASKS: usize = 1;
const MAX_EVENT_TEXT_CHARS: usize = 8_000;

#[derive(Clone)]
pub struct AgentTaskRegistry {
    inner: Arc<Mutex<AgentTaskRegistryInner>>,
    runner_permits: Arc<Semaphore>,
}

#[derive(Default)]
struct AgentTaskRegistryInner {
    next_seq: u64,
    tasks: Vec<AgentTaskSnapshot>,
    abort_handles: HashMap<String, AbortHandle>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct AgentTaskSnapshot {
    pub(crate) task_id: String,
    pub(crate) task: String,
    pub(crate) model: Option<String>,
    pub(crate) status: String,
    pub(crate) created_at: u64,
    pub(crate) started_at: Option<u64>,
    pub(crate) finished_at: Option<u64>,
    pub(crate) run_id: Option<String>,
    pub(crate) run_dir: Option<String>,
    pub(crate) target_id: Option<String>,
    // Hydrated from `<run_dir>/report.md` for API responses; not persisted in tasks.json.
    pub(crate) final_text: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) turns: Option<u32>,
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct AgentTaskEventPayload {
    pub(crate) task_id: String,
    pub(crate) kind: String,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) snapshot: Option<AgentTaskSnapshot>,
    #[serde(default)]
    pub(crate) sequence: u64,
    #[serde(default)]
    pub(crate) created_at: u64,
}

impl Default for AgentTaskRegistry {
    fn default() -> Self {
        let mut tasks = load_task_index();
        let interrupted_at = now_ms();
        for task in &mut tasks {
            if matches!(task.status.as_str(), "queued" | "running") {
                task.status = "interrupted".into();
                task.finished_at = Some(interrupted_at);
                task.error = Some("app was closed before this task finished".into());
                task.target_id = None;
            }
        }
        let next_seq = tasks.len() as u64;
        if !tasks.is_empty() {
            persist_task_index(&tasks);
        }
        Self {
            inner: Arc::new(Mutex::new(AgentTaskRegistryInner {
                next_seq,
                tasks,
                abort_handles: HashMap::new(),
            })),
            runner_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_AGENT_TASKS)),
        }
    }
}

impl AgentTaskRegistry {
    pub(crate) async fn create(
        &self,
        task: String,
        model: Option<String>,
        run_dir: String,
    ) -> AgentTaskSnapshot {
        let mut guard = self.inner.lock().await;
        guard.next_seq += 1;
        let task_id = format!("task-{}-{}", now_ms(), guard.next_seq);
        let snapshot = AgentTaskSnapshot {
            task_id,
            task,
            model,
            status: "queued".into(),
            created_at: now_ms(),
            started_at: None,
            finished_at: None,
            run_id: None,
            run_dir: Some(run_dir),
            target_id: None,
            final_text: None,
            error: None,
            turns: None,
            input_tokens: None,
            output_tokens: None,
        };
        guard.tasks.push(snapshot.clone());
        persist_task_index(&guard.tasks);
        snapshot
    }

    pub(crate) async fn acquire_run_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.runner_permits.clone().acquire_owned().await.ok()
    }

    /// Register the task abort handle. Returns the handle back to the caller
    /// if the task is already terminal (for example, cancelled by another
    /// window after task creation but before handle registration).
    pub(crate) async fn set_abort_handle(
        &self,
        task_id: &str,
        handle: AbortHandle,
    ) -> Option<AbortHandle> {
        let mut guard = self.inner.lock().await;
        let active = guard
            .tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .map(|task| matches!(task.status.as_str(), "queued" | "running"))
            .unwrap_or(false);
        if !active {
            return Some(handle);
        }
        if let Some(previous) = guard.abort_handles.insert(task_id.to_string(), handle) {
            previous.abort();
        }
        None
    }

    pub(crate) async fn remove_abort_handle(&self, task_id: &str) -> Option<AbortHandle> {
        self.inner.lock().await.abort_handles.remove(task_id)
    }

    pub(crate) async fn cancel(
        &self,
        task_id: &str,
    ) -> Option<(AgentTaskSnapshot, Option<AbortHandle>, Option<String>, bool)> {
        let mut guard = self.inner.lock().await;
        let pos = guard
            .tasks
            .iter()
            .position(|task| task.task_id == task_id)?;
        let changed = matches!(guard.tasks[pos].status.as_str(), "queued" | "running");
        let handle = if changed {
            guard.abort_handles.remove(task_id)
        } else {
            None
        };
        let target_id = guard.tasks[pos].target_id.clone();
        if changed {
            let task = &mut guard.tasks[pos];
            task.status = "cancelled".into();
            task.finished_at = Some(now_ms());
            task.target_id = None;
            task.error = None;
        }
        let snapshot = hydrate_task_snapshot(guard.tasks[pos].clone());
        persist_task_index(&guard.tasks);
        Some((snapshot, handle, target_id, changed))
    }

    pub(crate) async fn interrupt_missing_targets(
        &self,
        active_targets: &HashSet<String>,
    ) -> Vec<(AgentTaskSnapshot, Option<AbortHandle>)> {
        let mut guard = self.inner.lock().await;
        let mut out = Vec::new();
        let mut task_ids = Vec::new();
        for task in &mut guard.tasks {
            if task.status != "running" {
                continue;
            }
            let Some(target_id) = task.target_id.as_ref() else {
                continue;
            };
            if active_targets.contains(target_id) {
                continue;
            }
            task.status = "interrupted".into();
            task.finished_at = Some(now_ms());
            task.error = Some("chrome tab was closed".into());
            task.target_id = None;
            task_ids.push(task.task_id.clone());
            out.push((task.clone(), None));
        }
        if !task_ids.is_empty() {
            for (idx, task_id) in task_ids.into_iter().enumerate() {
                out[idx].1 = guard.abort_handles.remove(&task_id);
            }
            persist_task_index(&guard.tasks);
        }
        out
    }

    pub(crate) async fn list(&self) -> Vec<AgentTaskSnapshot> {
        self.inner
            .lock()
            .await
            .tasks
            .clone()
            .into_iter()
            .map(hydrate_task_snapshot)
            .collect()
    }

    pub(crate) async fn get(&self, task_id: &str) -> Option<AgentTaskSnapshot> {
        self.inner
            .lock()
            .await
            .tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .cloned()
            .map(hydrate_task_snapshot)
    }

    pub(crate) async fn events(&self, task_id: &str) -> Option<Vec<AgentTaskEventPayload>> {
        let snapshot = self.get(task_id).await?;
        Some(load_task_events(&snapshot))
    }

    pub(crate) async fn update<F>(&self, task_id: &str, f: F) -> Option<AgentTaskSnapshot>
    where
        F: FnOnce(&mut AgentTaskSnapshot),
    {
        let mut guard = self.inner.lock().await;
        let snapshot = {
            let task = guard
                .tasks
                .iter_mut()
                .find(|task| task.task_id == task_id)?;
            f(task);
            task.clone()
        };
        persist_task_index(&guard.tasks);
        Some(hydrate_task_snapshot(snapshot))
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn hydrate_task_snapshot(mut snapshot: AgentTaskSnapshot) -> AgentTaskSnapshot {
    snapshot.final_text = snapshot
        .run_dir
        .as_deref()
        .and_then(final_text_from_run_dir);
    snapshot
}

fn final_text_from_run_dir(run_dir: &str) -> Option<String> {
    let text = std::fs::read_to_string(PathBuf::from(run_dir).join("report.md")).ok()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn app_data_dir() -> PathBuf {
    if let Ok(home) = std::env::var("SOCAI_HOME") {
        return PathBuf::from(home).join("app");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".socai/app");
    }
    PathBuf::from(".socai/app")
}

fn task_index_path() -> PathBuf {
    app_data_dir().join("tasks.json")
}

fn load_task_index() -> Vec<AgentTaskSnapshot> {
    let path = task_index_path();
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut tasks = serde_json::from_str::<Vec<AgentTaskSnapshot>>(&text).unwrap_or_default();
    for task in &mut tasks {
        task.final_text = None;
    }
    tasks
}

fn persist_task_index(tasks: &[AgentTaskSnapshot]) {
    let path = task_index_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Keep tasks.json as an app index only. Task results live under run_dir.
    let records: Vec<Value> = tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "task_id": &task.task_id,
                "task": &task.task,
                "model": &task.model,
                "status": &task.status,
                "created_at": task.created_at,
                "started_at": task.started_at,
                "finished_at": task.finished_at,
                "run_id": &task.run_id,
                "run_dir": &task.run_dir,
                "target_id": &task.target_id,
                "error": &task.error,
                "turns": task.turns,
                "input_tokens": task.input_tokens,
                "output_tokens": task.output_tokens,
            })
        })
        .collect();
    if let Ok(text) = serde_json::to_string_pretty(&records) {
        let _ = std::fs::write(path, text);
    }
}

fn load_task_events(snapshot: &AgentTaskSnapshot) -> Vec<AgentTaskEventPayload> {
    let mut events = replay_task_events(snapshot);
    append_terminal_snapshot_events(snapshot, &mut events);
    events
}

fn replay_task_events(snapshot: &AgentTaskSnapshot) -> Vec<AgentTaskEventPayload> {
    let Some(run_dir) = snapshot
        .run_dir
        .as_ref()
        .filter(|dir| !dir.trim().is_empty())
    else {
        return Vec::new();
    };
    let run_dir = PathBuf::from(run_dir);
    let mut events = replay_reasoning_log(snapshot, &run_dir.join("reasoning_log.jsonl"));
    if events.is_empty() {
        events = replay_run_state_events(snapshot, &run_dir.join("run_state/events.jsonl"));
    }
    if !events.is_empty() {
        ensure_started_event(snapshot, &mut events);
        reindex_replay_events(snapshot, &mut events);
    }
    events
}

fn replay_reasoning_log(snapshot: &AgentTaskSnapshot, path: &Path) -> Vec<AgentTaskEventPayload> {
    let mut events = Vec::new();
    let mut last_turn = None;
    for value in read_jsonl_values(path) {
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        match event_type {
            "task_start" => {
                let task = value.get("task").and_then(Value::as_str);
                let model = value.get("model").and_then(Value::as_str);
                events.push(replay_event(
                    snapshot,
                    "started",
                    started_event_text(snapshot, task, model),
                ));
            }
            "llm_response" => {
                let turn = number_field(&value, "turn");
                push_turn_event(snapshot, &mut events, &mut last_turn, turn);
                let text = value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if !text.is_empty() {
                    events.push(replay_event(snapshot, "assistant", text.to_string()));
                }
            }
            "tool_call_start" => {
                let turn = number_field(&value, "turn");
                push_turn_event(snapshot, &mut events, &mut last_turn, turn);
                let tool = value.get("tool").and_then(Value::as_str).unwrap_or("tool");
                let input = value.get("input").unwrap_or(&Value::Null);
                let repeat_count = number_field(&value, "repeat_count");
                events.push(replay_event(
                    snapshot,
                    "tool_call",
                    format_tool_call_text(tool, input, repeat_count),
                ));
            }
            "tool_result" => {
                let turn = number_field(&value, "turn");
                push_turn_event(snapshot, &mut events, &mut last_turn, turn);
                let tool = value.get("tool").and_then(Value::as_str).unwrap_or("tool");
                let summary = value
                    .get("result_summary")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let error = value.get("error").and_then(Value::as_str).unwrap_or("");
                let duration_ms = duration_ms(&value);
                let (kind, text) = format_tool_result_text(tool, summary, duration_ms, error);
                events.push(replay_event(snapshot, kind, text));
            }
            "api_error" => {
                let turn = number_field(&value, "turn");
                push_turn_event(snapshot, &mut events, &mut last_turn, turn);
                let message = value
                    .get("error")
                    .or_else(|| value.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("api error");
                events.push(replay_event(
                    snapshot,
                    "api_error",
                    format!("turn {turn}: {message}"),
                ));
            }
            "task_end" => {
                let turn = number_field(&value, "turn");
                if turn > 0 {
                    events.push(replay_event(
                        snapshot,
                        "done",
                        format!("done in {turn} turns"),
                    ));
                } else {
                    events.push(replay_event(snapshot, "done", "done".into()));
                }
            }
            _ => {}
        }
    }
    events
}

fn replay_run_state_events(
    snapshot: &AgentTaskSnapshot,
    path: &Path,
) -> Vec<AgentTaskEventPayload> {
    let mut events = Vec::new();
    let mut last_turn = None;
    for value in read_jsonl_values(path) {
        let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        match event_type {
            "assistant_turn" => {
                let turn = number_field(&value, "turn");
                push_turn_event(snapshot, &mut events, &mut last_turn, turn);
                let text = value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if !text.is_empty() {
                    events.push(replay_event(snapshot, "assistant", text.to_string()));
                }
            }
            "tool_call" => {
                let turn = number_field(&value, "turn");
                push_turn_event(snapshot, &mut events, &mut last_turn, turn);
                let tool = value.get("tool").and_then(Value::as_str).unwrap_or("tool");
                let input = value.get("input").unwrap_or(&Value::Null);
                events.push(replay_event(
                    snapshot,
                    "tool_call",
                    format_tool_call_text(tool, input, 0),
                ));
            }
            "tool_result" => {
                let turn = number_field(&value, "turn");
                push_turn_event(snapshot, &mut events, &mut last_turn, turn);
                let tool = value.get("tool").and_then(Value::as_str).unwrap_or("tool");
                let summary = value
                    .get("result_summary")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let duration_ms = duration_ms(&value);
                let (kind, text) = format_tool_result_text(tool, summary, duration_ms, "");
                events.push(replay_event(snapshot, kind, text));
            }
            _ => {}
        }
    }
    events
}

fn append_terminal_snapshot_events(
    snapshot: &AgentTaskSnapshot,
    events: &mut Vec<AgentTaskEventPayload>,
) {
    match snapshot.status.as_str() {
        "completed" => {
            if !events.iter().any(|event| event.kind == "done") {
                let text = snapshot
                    .turns
                    .map(|turns| format!("done in {turns} turns"))
                    .unwrap_or_else(|| "done".into());
                push_snapshot_event(snapshot, events, "done", text);
            }
            if !events.iter().any(|event| event.kind == "completed") {
                push_snapshot_event(snapshot, events, "completed", "task completed".into());
            }
        }
        "failed" => {
            if !events.iter().any(|event| event.kind == "failed") {
                let text = snapshot
                    .error
                    .as_deref()
                    .filter(|error| !error.trim().is_empty())
                    .unwrap_or("task failed")
                    .to_string();
                push_snapshot_event(snapshot, events, "failed", text);
            }
        }
        "cancelled" => {
            if !events.iter().any(|event| event.kind == "cancelled") {
                push_snapshot_event(snapshot, events, "cancelled", "task cancelled".into());
            }
        }
        "interrupted" => {
            if !events.iter().any(|event| event.kind == "interrupted") {
                let text = snapshot
                    .error
                    .as_deref()
                    .filter(|error| !error.trim().is_empty())
                    .unwrap_or("task interrupted")
                    .to_string();
                push_snapshot_event(snapshot, events, "interrupted", text);
            }
        }
        _ => {}
    }
}

fn push_snapshot_event(
    snapshot: &AgentTaskSnapshot,
    events: &mut Vec<AgentTaskEventPayload>,
    kind: &str,
    text: String,
) {
    let sequence = events.iter().map(|event| event.sequence).max().unwrap_or(0) + 1;
    events.push(AgentTaskEventPayload {
        task_id: snapshot.task_id.clone(),
        kind: kind.into(),
        text: truncate_event_text(&text),
        snapshot: None,
        sequence,
        created_at: snapshot.finished_at.unwrap_or_else(now_ms),
    });
}

fn ensure_started_event(snapshot: &AgentTaskSnapshot, events: &mut Vec<AgentTaskEventPayload>) {
    if events.iter().any(|event| event.kind == "started") {
        return;
    }
    events.insert(
        0,
        replay_event(
            snapshot,
            "started",
            started_event_text(snapshot, None, None),
        ),
    );
}

fn reindex_replay_events(snapshot: &AgentTaskSnapshot, events: &mut [AgentTaskEventPayload]) {
    let base = snapshot.started_at.unwrap_or(snapshot.created_at);
    for (idx, event) in events.iter_mut().enumerate() {
        event.sequence = idx as u64 + 1;
        event.created_at = base.saturating_add(idx as u64 + 1);
    }
}

fn replay_event(snapshot: &AgentTaskSnapshot, kind: &str, text: String) -> AgentTaskEventPayload {
    AgentTaskEventPayload {
        task_id: snapshot.task_id.clone(),
        kind: kind.into(),
        text: truncate_event_text(&text),
        snapshot: None,
        sequence: 0,
        created_at: 0,
    }
}

fn push_turn_event(
    snapshot: &AgentTaskSnapshot,
    events: &mut Vec<AgentTaskEventPayload>,
    last_turn: &mut Option<u64>,
    turn: u64,
) {
    if turn == 0 || *last_turn == Some(turn) {
        return;
    }
    *last_turn = Some(turn);
    events.push(replay_event(snapshot, "turn", format!("turn {turn}")));
}

fn started_event_text(
    snapshot: &AgentTaskSnapshot,
    task: Option<&str>,
    model: Option<&str>,
) -> String {
    let task = task.unwrap_or(&snapshot.task);
    let model = model
        .or(snapshot.model.as_deref())
        .unwrap_or("unknown model");
    let run = snapshot
        .run_id
        .as_deref()
        .map(|run_id| format!("run {run_id} · "))
        .unwrap_or_default();
    format!("task: {task}\n{run}model {model}")
}

fn format_tool_call_text(tool: &str, input: &Value, repeat_count: u64) -> String {
    let preview = serde_json::to_string(input).unwrap_or_else(|_| input.to_string());
    if repeat_count > 1 {
        format!("{tool}({preview}) repeat={repeat_count}")
    } else {
        format!("{tool}({preview})")
    }
}

fn format_tool_result_text(
    tool: &str,
    summary: &str,
    duration_ms: u64,
    error: &str,
) -> (&'static str, String) {
    if !error.trim().is_empty() {
        return ("tool_error", format!("{tool} ({duration_ms}ms): {error}"));
    }
    let first = summary.lines().next().unwrap_or("");
    ("tool_result", format!("{tool} ({duration_ms}ms): {first}"))
}

fn duration_ms(value: &Value) -> u64 {
    if let Some(ms) = value.get("duration_ms").and_then(Value::as_u64) {
        return ms;
    }
    value
        .get("duration_s")
        .and_then(Value::as_f64)
        .map(|seconds| (seconds * 1000.0).round().max(0.0) as u64)
        .unwrap_or_default()
}

fn number_field(value: &Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(|number| {
            number
                .as_u64()
                .or_else(|| number.as_f64().map(|n| n as u64))
        })
        .unwrap_or_default()
}

fn read_jsonl_values(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn truncate_event_text(text: &str) -> String {
    if text.chars().count() <= MAX_EVENT_TEXT_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(MAX_EVENT_TEXT_CHARS).collect();
    format!("{kept}\n... [truncated]")
}
