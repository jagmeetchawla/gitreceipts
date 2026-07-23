//! Stage 2: causal ordering via the parentUuid chain.
//!
//! Records form a tree (forks happen on retries and sidechains). We walk it
//! depth-first from the roots, siblings in timestamp order, then append any
//! orphans (parent not in this file) in timestamp order. Iterative walk — the
//! chain in a long session is thousands of records deep.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::schema::Record;

pub fn order(records: Vec<Record>) -> Vec<Record> {
    // Parse timestamps once for sorting: RFC3339 strings do NOT sort
    // lexicographically when fractional seconds vary ("…00.5Z" < "…00Z").
    let parsed: Vec<DateTime<Utc>> = records
        .iter()
        .map(|r| {
            r.timestamp
                .as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or(DateTime::<Utc>::MIN_UTC)
        })
        .collect();
    let ts_key = |i: usize| parsed[i];

    let mut children: HashMap<String, Vec<usize>> = HashMap::new();
    let uuids: HashSet<&str> = records.iter().filter_map(|r| r.uuid.as_deref()).collect();

    let mut roots: Vec<usize> = Vec::new();
    for (i, r) in records.iter().enumerate() {
        match r.parent_uuid.as_deref() {
            Some(p) if uuids.contains(p) => children.entry(p.to_string()).or_default().push(i),
            _ => roots.push(i),
        }
    }
    for v in children.values_mut() {
        v.sort_by_key(|&i| ts_key(i));
    }
    roots.sort_by_key(|&i| ts_key(i));

    let mut seen = vec![false; records.len()];
    let mut ordered_idx = Vec::with_capacity(records.len());
    // depth-first: push children in reverse so the earliest sibling pops first
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(i) = stack.pop() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        ordered_idx.push(i);
        if let Some(uuid) = records[i].uuid.as_deref()
            && let Some(kids) = children.get(uuid)
        {
            stack.extend(kids.iter().rev());
        }
    }
    // orphans without a uuid (never entered the tree) go last, by timestamp
    let mut leftovers: Vec<usize> = (0..records.len()).filter(|&i| !seen[i]).collect();
    leftovers.sort_by_key(|&i| ts_key(i));
    ordered_idx.extend(leftovers);

    let mut slots: Vec<Option<Record>> = records.into_iter().map(Some).collect();
    ordered_idx
        .into_iter()
        .filter_map(|i| slots[i].take())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::order;
    use crate::schema::Record;

    fn rec(json: &str) -> Record {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn walks_parent_chain_before_timestamps() {
        // b is a's child but has an EARLIER wall clock than orphan c
        let records = vec![
            rec(r#"{"type":"user","uuid":"a","timestamp":"2026-01-01T00:00:00Z"}"#),
            rec(
                r#"{"type":"assistant","uuid":"b","parentUuid":"a","timestamp":"2026-01-01T00:00:01Z"}"#,
            ),
            rec(r#"{"type":"user","uuid":"c","timestamp":"2026-01-01T00:00:00.500Z"}"#),
        ];
        let ordered = order(records);
        let uuids: Vec<&str> = ordered.iter().filter_map(|r| r.uuid.as_deref()).collect();
        assert_eq!(uuids, vec!["a", "b", "c"]);
    }

    #[test]
    fn orphans_with_missing_parents_still_appear() {
        let records = vec![
            rec(
                r#"{"type":"user","uuid":"x","parentUuid":"missing","timestamp":"2026-01-01T00:00:02Z"}"#,
            ),
            rec(r#"{"type":"user","uuid":"y","timestamp":"2026-01-01T00:00:01Z"}"#),
        ];
        let ordered = order(records);
        assert_eq!(ordered.len(), 2);
    }
}
