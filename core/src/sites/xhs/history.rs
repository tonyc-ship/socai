//! Cross-run XHS analysis history. Tracks which notes have already been
//! analyzed (and at what level) in a project-local JSON file so the agent
//! can skip repeats across separate runs.
//!
//! - In-run dedup still lives on `ToolContext::processed_notes`; this store
//!   only handles cross-run persistence.
//! - Schema keeps note metadata plus a small local cache of successful note
//!   entities keyed by read level/media settings. Run dirs and artifact-local
//!   paths are dropped — they live in per-run logs and would mostly point at
//!   stale paths anyway.
//! - File at `~/.socai/xhs/history.json` (overridable via `SOCAI_HOME`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedAnalysis {
    #[serde(default)]
    pub cache_key: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub include_media: bool,
    #[serde(default)]
    pub analyzed_at: String,
    #[serde(default)]
    pub entity: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub note_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub url: String,
    /// Deepest level ever recorded: "card" | "lite" | "deep".
    #[serde(default)]
    pub level: String,
    /// True once any past read had media enabled.
    #[serde(default)]
    pub include_media: bool,
    #[serde(default)]
    pub analysis_count: u32,
    #[serde(default)]
    pub first_seen_at: String,
    #[serde(default)]
    pub last_seen_at: String,
    /// Cached successful entities by level/media cache key. Kept out of CLI
    /// skip metadata, but persisted so repeated scans can return deep evidence
    /// without reopening notes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cached_results: BTreeMap<String, CachedAnalysis>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    notes: BTreeMap<String, HistoryEntry>,
}

pub struct XhsHistoryStore {
    path: PathBuf,
    inner: Mutex<HistoryFile>,
}

impl XhsHistoryStore {
    /// `$SOCAI_HOME/xhs/history.json`, else `~/.socai/xhs/history.json`,
    /// else `.socai/xhs/history.json` relative to cwd.
    pub fn default_path() -> PathBuf {
        if let Ok(env) = std::env::var("SOCAI_HOME") {
            return PathBuf::from(env).join("xhs/history.json");
        }
        if let Some(home) = dirs::home_dir() {
            return home.join(".socai/xhs/history.json");
        }
        PathBuf::from(".socai/xhs/history.json")
    }

    pub fn open_default() -> Self {
        Self::open(Self::default_path())
    }

    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let inner = load_file(&path).unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(inner),
        }
    }

    pub fn get(&self, note_id: &str) -> Option<HistoryEntry> {
        let id = note_id.trim();
        if id.is_empty() {
            return None;
        }
        let guard = self.inner.lock().ok()?;
        guard.notes.get(id).cloned()
    }

    pub fn cache_key_for(note_id: &str, level: &str, include_media: bool) -> String {
        format!(
            "xhs:note:{}:level={}:include_media={}",
            note_id.trim(),
            normalize_level(level),
            include_media
        )
    }

    /// Return the best cached entity that covers the requested level/media.
    /// A deeper cached result can satisfy a shallower request, and media is
    /// only required when the caller requested media.
    pub fn cached_result(
        &self,
        note_id: &str,
        level: &str,
        include_media: bool,
    ) -> Option<CachedAnalysis> {
        let id = note_id.trim();
        if id.is_empty() {
            return None;
        }
        let guard = self.inner.lock().ok()?;
        let entry = guard.notes.get(id)?;
        best_cached_analysis(entry, level, include_media)
    }

    /// True when a prior analysis already covers what's being requested:
    /// recorded level is >= requested AND, if media was requested, media
    /// was included previously.
    pub fn is_satisfied_by(&self, note_id: &str, level: &str, include_media: bool) -> bool {
        let Some(prev) = self.get(note_id) else {
            return false;
        };
        if best_cached_analysis(&prev, level, include_media).is_some() {
            return true;
        }
        if level_value(&prev.level) < level_value(level) {
            return false;
        }
        if include_media && !prev.include_media {
            return false;
        }
        true
    }

    /// Add `already_analyzed` / `history_level` / `history_include_media`
    /// flags onto any card whose `note_id` is in the store. Mutates in place.
    pub fn annotate_cards(&self, cards: &mut Value) {
        let Ok(guard) = self.inner.lock() else {
            return;
        };
        annotate_cards_from(&guard.notes, cards);
    }

    /// Take an owned snapshot of all entries currently in the store. Use
    /// this when a tool mutates history during its own call (e.g.
    /// `topic_scan` records every note it reads) but still wants to
    /// annotate output cards based on what was known *before* the call —
    /// otherwise the annotation reflects this run's own writes.
    pub fn snapshot(&self) -> HistorySnapshot {
        let entries = self
            .inner
            .lock()
            .map(|guard| guard.notes.clone())
            .unwrap_or_default();
        HistorySnapshot { entries }
    }

    /// Upsert an entry after a successful read. Never downgrades the
    /// recorded level or media flag — once a note was read deeply, that
    /// stays.
    pub fn record(&self, entity: &Value, level: &str, include_media: bool) {
        let Some(note_id) = entity
            .get("note_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            return;
        };
        let now = Utc::now().to_rfc3339();
        let title = string_field(entity, "title");
        let author = string_field(entity, "author");
        let url = string_field(entity, "url");
        let cache_key = Self::cache_key_for(&note_id, level, include_media);
        let cached_entity = sanitize_cached_entity(entity);

        let snapshot = {
            let Ok(mut guard) = self.inner.lock() else {
                return;
            };
            let entry = guard
                .notes
                .entry(note_id.clone())
                .or_insert_with(|| HistoryEntry {
                    note_id: note_id.clone(),
                    first_seen_at: now.clone(),
                    ..Default::default()
                });
            entry.note_id = note_id.clone();
            if !title.is_empty() {
                entry.title = title;
            }
            if !author.is_empty() {
                entry.author = author;
            }
            if !url.is_empty() {
                entry.url = url;
            }
            if level_value(level) > level_value(&entry.level) {
                entry.level = normalize_level(level);
            }
            if include_media {
                entry.include_media = true;
            }
            entry.cached_results.insert(
                cache_key.clone(),
                CachedAnalysis {
                    cache_key,
                    level: normalize_level(level),
                    include_media,
                    analyzed_at: now.clone(),
                    entity: cached_entity,
                },
            );
            entry.analysis_count = entry.analysis_count.saturating_add(1);
            entry.last_seen_at = now;
            guard.clone()
        };

        // Best-effort write. A failure here just means the next process
        // won't see this entry — agent still works.
        let _ = save_file(&self.path, &snapshot);
    }
}

/// Owned snapshot of the history at a point in time. Cheap to pass around
/// since it's a plain map.
pub struct HistorySnapshot {
    entries: BTreeMap<String, HistoryEntry>,
}

impl HistorySnapshot {
    pub fn annotate_cards(&self, cards: &mut Value) {
        annotate_cards_from(&self.entries, cards);
    }
}

fn annotate_cards_from(entries: &BTreeMap<String, HistoryEntry>, cards: &mut Value) {
    let Some(arr) = cards.as_array_mut() else {
        return;
    };
    for card in arr {
        let Some(map) = card.as_object_mut() else {
            continue;
        };
        let note_id = map
            .get("note_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let Some(note_id) = note_id else { continue };
        if let Some(entry) = entries.get(&note_id) {
            map.insert("already_analyzed".into(), json!(true));
            map.insert("history_level".into(), json!(entry.level));
            map.insert("history_include_media".into(), json!(entry.include_media));
        }
    }
}

fn best_cached_analysis(
    entry: &HistoryEntry,
    level: &str,
    include_media: bool,
) -> Option<CachedAnalysis> {
    let mut best: Option<&CachedAnalysis> = None;
    for cached in entry.cached_results.values() {
        if level_value(&cached.level) < level_value(level) {
            continue;
        }
        if include_media && !cached.include_media {
            continue;
        }
        if best
            .map(|current| cached_analysis_is_better(cached, current))
            .unwrap_or(true)
        {
            best = Some(cached);
        }
    }
    best.cloned()
}

fn cached_analysis_is_better(candidate: &CachedAnalysis, current: &CachedAnalysis) -> bool {
    let candidate_level = level_value(&candidate.level);
    let current_level = level_value(&current.level);
    if candidate_level != current_level {
        return candidate_level > current_level;
    }
    if candidate.include_media != current.include_media {
        return candidate.include_media;
    }
    candidate.analyzed_at > current.analyzed_at
}

fn sanitize_cached_entity(entity: &Value) -> Value {
    let mut value = entity.clone();
    remove_artifact_local_paths(&mut value);
    value
}

fn remove_artifact_local_paths(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("local_path");
            map.remove("poster_local_path");
            map.remove("frame_paths");
            for child in map.values_mut() {
                remove_artifact_local_paths(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                remove_artifact_local_paths(item);
            }
        }
        _ => {}
    }
}

fn normalize_level(level: &str) -> String {
    match level.trim().to_ascii_lowercase().as_str() {
        "deep" => "deep".to_string(),
        "lite" => "lite".to_string(),
        "card" => "card".to_string(),
        other => other.to_string(),
    }
}

fn level_value(level: &str) -> i32 {
    match level.trim().to_ascii_lowercase().as_str() {
        "deep" => 3,
        "lite" => 2,
        "card" => 1,
        _ => 0,
    }
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn load_file(path: &Path) -> Option<HistoryFile> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_file(path: &Path, data: &HistoryFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(data).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn records_and_recalls_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        let store = XhsHistoryStore::open(&path);

        store.record(
            &json!({"note_id": "abc", "title": "T", "author": "A", "url": "u"}),
            "lite",
            false,
        );
        let entry = store.get("abc").expect("entry present");
        assert_eq!(entry.note_id, "abc");
        assert_eq!(entry.level, "lite");
        assert_eq!(entry.analysis_count, 1);
        assert!(!entry.first_seen_at.is_empty());
        assert!(entry
            .cached_results
            .contains_key(&XhsHistoryStore::cache_key_for("abc", "lite", false)));

        // Reopen from disk — entries and cached entities persist.
        let store2 = XhsHistoryStore::open(&path);
        let cached = store2
            .cached_result("abc", "lite", false)
            .expect("cached result present");
        assert_eq!(cached.entity["title"], json!("T"));
    }

    #[test]
    fn level_never_downgrades_but_media_upgrades() {
        let dir = tempdir().unwrap();
        let store = XhsHistoryStore::open(dir.path().join("h.json"));

        store.record(&json!({"note_id": "n1"}), "deep", true);
        store.record(&json!({"note_id": "n1"}), "lite", false);
        let entry = store.get("n1").unwrap();
        assert_eq!(entry.level, "deep");
        assert!(entry.include_media);
        assert_eq!(entry.analysis_count, 2);
    }

    #[test]
    fn satisfied_when_prior_is_deeper_or_equal() {
        let dir = tempdir().unwrap();
        let store = XhsHistoryStore::open(dir.path().join("h.json"));
        store.record(&json!({"note_id": "n1"}), "lite", false);

        assert!(store.is_satisfied_by("n1", "card", false));
        assert!(store.is_satisfied_by("n1", "lite", false));
        assert!(!store.is_satisfied_by("n1", "deep", false));
        assert!(!store.is_satisfied_by("n1", "lite", true));
        assert!(!store.is_satisfied_by("unknown", "card", false));
    }

    #[test]
    fn cached_result_returns_deepest_satisfying_entity() {
        let dir = tempdir().unwrap();
        let store = XhsHistoryStore::open(dir.path().join("h.json"));
        store.record(
            &json!({"note_id": "n1", "title": "lite", "content": "short"}),
            "lite",
            false,
        );
        store.record(
            &json!({"note_id": "n1", "title": "deep", "content": "full", "top_comments": ["c"]}),
            "deep",
            false,
        );

        let cached = store
            .cached_result("n1", "lite", false)
            .expect("deeper cache satisfies lite request");
        assert_eq!(cached.level, "deep");
        assert_eq!(cached.entity["content"], json!("full"));
        assert_eq!(cached.entity["top_comments"], json!(["c"]));
    }

    #[test]
    fn cached_result_respects_media_requirement() {
        let dir = tempdir().unwrap();
        let store = XhsHistoryStore::open(dir.path().join("h.json"));
        store.record(&json!({"note_id": "n1", "title": "deep"}), "deep", false);

        assert!(store.cached_result("n1", "deep", true).is_none());
        assert!(!store.is_satisfied_by("n1", "deep", true));
    }

    #[test]
    fn metadata_only_history_from_old_versions_still_satisfies_requests() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "notes": {
                    "n1": {
                        "note_id": "n1",
                        "level": "deep",
                        "include_media": false,
                        "last_seen_at": "2026-01-01T00:00:00Z"
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let store = XhsHistoryStore::open(&path);
        assert!(store.is_satisfied_by("n1", "deep", false));
        assert!(store.cached_result("n1", "deep", false).is_none());
    }

    #[test]
    fn partial_cache_history_falls_back_to_top_level_metadata() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "notes": {
                    "n1": {
                        "note_id": "n1",
                        "level": "deep",
                        "include_media": false,
                        "last_seen_at": "2026-01-01T00:00:00Z",
                        "cached_results": {
                            "xhs:note:n1:level=lite:include_media=false": {
                                "cache_key": "xhs:note:n1:level=lite:include_media=false",
                                "level": "lite",
                                "include_media": false,
                                "analyzed_at": "2026-01-01T00:00:00Z",
                                "entity": {"note_id": "n1", "content": "lite"}
                            }
                        }
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let store = XhsHistoryStore::open(&path);
        assert!(store.is_satisfied_by("n1", "deep", false));
        assert!(store.cached_result("n1", "deep", false).is_none());
    }

    #[test]
    fn cached_result_strips_artifact_local_paths() {
        let dir = tempdir().unwrap();
        let store = XhsHistoryStore::open(dir.path().join("h.json"));
        store.record(
            &json!({
                "note_id": "n1",
                "images": [{"url": "https://example.test/i.jpg", "local_path": "/tmp/run/i.jpg"}],
                "video": {"local_path": "/tmp/run/v.mp4", "poster_local_path": "/tmp/run/p.jpg", "frame_paths": ["/tmp/run/f.jpg"]}
            }),
            "deep",
            false,
        );

        let cached = store.cached_result("n1", "deep", false).unwrap();
        assert!(cached.entity["images"][0].get("local_path").is_none());
        assert!(cached.entity["video"].get("local_path").is_none());
        assert!(cached.entity["video"].get("poster_local_path").is_none());
        assert!(cached.entity["video"].get("frame_paths").is_none());
    }

    #[test]
    fn snapshot_freezes_pre_call_state() {
        let dir = tempdir().unwrap();
        let store = XhsHistoryStore::open(dir.path().join("h.json"));
        store.record(&json!({"note_id": "old"}), "lite", false);

        let pre = store.snapshot();
        // Writes after the snapshot must not show up when annotating with it.
        store.record(&json!({"note_id": "new_this_run"}), "deep", true);

        let mut cards = json!([
            {"note_id": "old"},
            {"note_id": "new_this_run"},
        ]);
        pre.annotate_cards(&mut cards);
        let arr = cards.as_array().unwrap();
        assert_eq!(arr[0]["already_analyzed"], json!(true));
        assert!(arr[1].get("already_analyzed").is_none());
    }

    #[test]
    fn annotate_cards_marks_known_notes() {
        let dir = tempdir().unwrap();
        let store = XhsHistoryStore::open(dir.path().join("h.json"));
        store.record(&json!({"note_id": "seen", "title": "x"}), "deep", true);

        let mut cards = json!([
            {"note_id": "seen", "title": "x"},
            {"note_id": "fresh", "title": "y"},
        ]);
        store.annotate_cards(&mut cards);
        let arr = cards.as_array().unwrap();
        assert_eq!(arr[0]["already_analyzed"], json!(true));
        assert_eq!(arr[0]["history_level"], json!("deep"));
        assert_eq!(arr[0]["history_include_media"], json!(true));
        assert!(arr[1].get("already_analyzed").is_none());
    }
}
