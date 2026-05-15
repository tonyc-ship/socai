//! Final-report enrichment. After the loop produces its `final_text`,
//! walk the run-state artifact registry and append markdown image links
//! for screenshots so the saved `report.md` is self-contained.
//!
//! Mirrors `_report_with_artifacts` in `socai/agent/loop.py`.

use std::sync::Arc;

use serde_json::Value;

use crate::run_state::{ArtifactRecord, RunState};

const MAX_SCREENSHOTS: usize = 12;

pub fn report_with_artifacts(final_text: &str, run_state: Option<&Arc<RunState>>) -> String {
    let Some(state) = run_state else {
        return final_text.to_string();
    };
    let screenshots: Vec<ArtifactRecord> = state
        .artifact_records()
        .into_iter()
        .filter(is_screenshot)
        .take(MAX_SCREENSHOTS)
        .collect();
    if screenshots.is_empty() {
        return final_text.to_string();
    }
    let mut out = final_text.trim_end().to_string();
    out.push_str("\n\n## Artifacts\n");
    for item in screenshots {
        let summary = if item.summary.trim().is_empty() {
            item.label.clone()
        } else {
            item.summary.clone()
        };
        let alt = summary.replace('[', "(").replace(']', ")");
        let alt = if alt.is_empty() { "screenshot".to_string() } else { alt };
        out.push_str(&format!("- {summary}\n"));
        out.push_str(&format!("  ![{alt}]({})\n", item.path));
    }
    out
}

fn is_screenshot(artifact: &ArtifactRecord) -> bool {
    if !artifact.kind.eq_ignore_ascii_case("image") {
        return false;
    }
    artifact
        .metadata
        .as_object()
        .and_then(|m| m.get("category"))
        .and_then(Value::as_str)
        == Some("screenshot")
}
