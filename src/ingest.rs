//! Stage 1: stream the JSONL, keep execution records, count everything else.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};

use crate::schema::Record;

/// Record types that are execution events; everything else is bookkeeping.
const EXECUTION_TYPES: [&str; 3] = ["user", "assistant", "system"];

#[derive(Debug, Default)]
pub struct IngestStats {
    pub lines: usize,
    pub kept: usize,
    pub skipped_types: usize,
    pub unparseable: usize,
}

pub fn ingest(path: &Path) -> Result<(Vec<Record>, IngestStats)> {
    let file = File::open(path).with_context(|| format!("open session {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut stats = IngestStats::default();
    let mut kept = Vec::new();
    for line in reader.lines() {
        let line = line.context("read session line")?;
        if line.trim().is_empty() {
            continue;
        }
        stats.lines += 1;
        match serde_json::from_str::<Record>(&line) {
            Ok(rec) if EXECUTION_TYPES.contains(&rec.kind.as_str()) => {
                stats.kept += 1;
                kept.push(rec);
            }
            Ok(_) => stats.skipped_types += 1,
            Err(_) => stats.unparseable += 1,
        }
    }
    Ok((kept, stats))
}
