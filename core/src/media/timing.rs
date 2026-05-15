use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::media::common::round3;

#[derive(Debug, Default)]
pub struct TimingRecord {
    inner: Mutex<TimingInner>,
}

#[derive(Debug, Default)]
struct TimingInner {
    counts: BTreeMap<String, u64>,
    totals: BTreeMap<String, f64>,
}

impl TimingRecord {
    pub fn record(&self, op: &str, duration: Duration) {
        if op.is_empty() {
            return;
        }
        if let Ok(mut inner) = self.inner.lock() {
            *inner.counts.entry(op.to_string()).or_default() += 1;
            *inner.totals.entry(op.to_string()).or_default() += duration.as_secs_f64();
        }
    }

    pub fn snapshot(&self) -> TimingSnapshot {
        let Ok(inner) = self.inner.lock() else {
            return TimingSnapshot::default();
        };
        TimingSnapshot {
            counts: inner.counts.clone(),
            totals: inner.totals.clone(),
        }
    }

    pub fn summary(&self) -> Value {
        timing_delta(&TimingSnapshot::default(), &self.snapshot())
    }

    pub fn reset(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.counts.clear();
            inner.totals.clear();
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TimingSnapshot {
    counts: BTreeMap<String, u64>,
    totals: BTreeMap<String, f64>,
}

pub fn timing_delta(before: &TimingSnapshot, after: &TimingSnapshot) -> Value {
    let mut out = Map::new();
    for (op, total) in &after.totals {
        let base_total = before.totals.get(op).copied().unwrap_or(0.0);
        let base_count = before.counts.get(op).copied().unwrap_or(0);
        let count = after
            .counts
            .get(op)
            .copied()
            .unwrap_or(0)
            .saturating_sub(base_count);
        let total_s = (total - base_total).max(0.0);
        if count == 0 && total_s <= 0.0 {
            continue;
        }
        out.insert(
            op.clone(),
            json!({
                "count": count,
                "total_s": round3(total_s),
                "avg_s": if count == 0 { 0.0 } else { round3(total_s / count as f64) },
            }),
        );
    }
    Value::Object(out)
}
