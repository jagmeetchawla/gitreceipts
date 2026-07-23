//! Stage 1: stream the JSONL, keep execution records, count everything else.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};

use crate::schema::Record;

/// Record types that are execution events; everything else is bookkeeping.
const EXECUTION_TYPES: [&str; 3] = ["user", "assistant", "system"];

/// Per-line ceiling. Real records top out in the low megabytes (a Write
/// with a large file inlined); session files are untrusted input, so one
/// multi-gigabyte line must not become one multi-gigabyte allocation.
const MAX_LINE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct IngestStats {
    pub lines: usize,
    pub kept: usize,
    pub skipped_types: usize,
    pub unparseable: usize,
}

pub fn ingest(path: &Path) -> Result<(Vec<Record>, IngestStats)> {
    ingest_with_cap(path, MAX_LINE_BYTES)
}

/// `ingest` with an explicit line cap — exposed so tests can exercise the
/// oversized-line path without gigabyte fixtures.
pub fn ingest_with_cap(path: &Path, max_line: u64) -> Result<(Vec<Record>, IngestStats)> {
    let file = File::open(path).with_context(|| format!("open session {}", path.display()))?;
    let mut reader = BufReader::new(file);

    let mut stats = IngestStats::default();
    let mut kept = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let n = reader
            .by_ref()
            .take(max_line)
            .read_until(b'\n', &mut buf)
            .context("read session line")?;
        if n == 0 {
            break;
        }
        // Hit the cap without finding a newline: count the line as
        // unparseable and drain the remainder without keeping it.
        if n as u64 == max_line && !buf.ends_with(b"\n") {
            stats.lines += 1;
            stats.unparseable += 1;
            loop {
                buf.clear();
                let m = reader
                    .by_ref()
                    .take(max_line)
                    .read_until(b'\n', &mut buf)
                    .context("skip oversized line")?;
                if m == 0 || buf.ends_with(b"\n") {
                    break;
                }
            }
            continue;
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        stats.lines += 1;
        match serde_json::from_str::<Record>(line) {
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
