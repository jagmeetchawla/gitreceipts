//! End-to-end: a synthetic session against a real (temporary) git repo,
//! through the full pipeline — ingest → causal order → extract → reconcile.
//!
//! The scenario: the agent writes two files, deletes one before committing,
//! and commits the survivor plus a file it never claimed. The equation
//! must show one landed claim, one never-landed claim with a diagnosis,
//! and one residue file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gitreceipts::reconcile::Landing;
use gitreceipts::{causal, extract, ingest, reconcile};

struct TempRepo {
    root: PathBuf,
}

impl TempRepo {
    fn new(name: &str) -> TempRepo {
        let root =
            std::env::temp_dir().join(format!("gitreceipts-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let repo = TempRepo {
            root: root.canonicalize().unwrap(),
        };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo
    }

    fn git(&self, args: &[&str]) {
        self.git_at(args, None);
    }

    fn git_at(&self, args: &[&str], date: Option<&str>) {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.root).args(args);
        if let Some(d) = date {
            cmd.env("GIT_AUTHOR_DATE", d).env("GIT_COMMITTER_DATE", d);
        }
        let status = cmd.status().unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(&self, rel: &str, content: &str) {
        fs::write(self.root.join(rel), content).unwrap();
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_session(path: &Path, repo_root: &str) {
    let cwd = repo_root;
    let e = |json: String| json;
    let lines = [
        e(format!(
            r#"{{"type":"user","uuid":"u1","timestamp":"2026-01-01T10:00:00Z","cwd":"{cwd}","message":{{"content":"build it"}}}}"#
        )),
        // claim 1: src.txt — will land in the commit
        e(format!(
            r#"{{"type":"assistant","uuid":"a1","parentUuid":"u1","timestamp":"2026-01-01T10:00:10Z","cwd":"{cwd}","message":{{"content":[{{"type":"tool_use","id":"t1","name":"Write","input":{{"file_path":"{cwd}/src.txt","content":"hello"}}}}]}}}}"#
        )),
        e(format!(
            r#"{{"type":"user","uuid":"u2","parentUuid":"a1","timestamp":"2026-01-01T10:00:11Z","cwd":"{cwd}","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","is_error":false,"content":"ok"}}]}}}}"#
        )),
        // claim 2: scratch.txt — deleted before the commit, never lands
        e(format!(
            r#"{{"type":"assistant","uuid":"a2","parentUuid":"u2","timestamp":"2026-01-01T10:00:20Z","cwd":"{cwd}","message":{{"content":[{{"type":"tool_use","id":"t2","name":"Write","input":{{"file_path":"{cwd}/scratch.txt","content":"probe"}}}}]}}}}"#
        )),
        e(format!(
            r#"{{"type":"user","uuid":"u3","parentUuid":"a2","timestamp":"2026-01-01T10:00:21Z","cwd":"{cwd}","message":{{"content":[{{"type":"tool_result","tool_use_id":"t2","is_error":false,"content":"ok"}}]}}}}"#
        )),
        // the commit command
        e(format!(
            r#"{{"type":"assistant","uuid":"a3","parentUuid":"u3","timestamp":"2026-01-01T10:00:30Z","cwd":"{cwd}","message":{{"content":[{{"type":"tool_use","id":"t3","name":"Bash","input":{{"command":"git add -A && git commit -m first"}}}}]}}}}"#
        )),
        e(format!(
            r#"{{"type":"user","uuid":"u4","parentUuid":"a3","timestamp":"2026-01-01T10:00:32Z","cwd":"{cwd}","message":{{"content":[{{"type":"tool_result","tool_use_id":"t3","is_error":false,"content":"[main abc] first"}}]}}}}"#
        )),
        // bookkeeping noise the ingester must skip, plus one garbage line
        e(r#"{"type":"queue-operation","uuid":"q1"}"#.to_string()),
        e("{not json at all".to_string()),
    ];
    fs::write(path, lines.join("\n")).unwrap();
}

#[test]
fn full_pipeline_balances_the_interval_equation() {
    let repo = TempRepo::new("pipeline");
    let root = repo.root.display().to_string();

    // what actually happened on disk: src.txt landed, scratch.txt was
    // deleted before the commit, extra.txt was created by "a command"
    // and committed without ever being claimed
    repo.write("src.txt", "hello");
    repo.write("extra.txt", "fallout");
    repo.git(&["add", "-A"]);
    repo.git_at(
        &["commit", "-q", "-m", "first"],
        Some("2026-01-01T10:00:31Z"),
    );

    let session_path = repo.root.join("session.jsonl");
    write_session(&session_path, &root);

    let (records, stats) = ingest::ingest(&session_path).unwrap();
    assert_eq!(stats.kept, 7);
    assert_eq!(stats.skipped_types, 1);
    assert_eq!(stats.unparseable, 1);

    let ordered = causal::order(records);
    let session = extract::extract(&ordered);
    assert_eq!(session.claims.len(), 3); // two writes + one command

    let audit = reconcile::reconcile(&repo.root, &session).unwrap();

    assert_eq!(audit.intervals.len(), 1, "one commit, one interval");
    let interval = &audit.intervals[0];
    assert!(interval.agent_committed, "the Bash claim covers the commit");
    assert!(
        !interval.balanced(),
        "a red interval: one lost claim + residue"
    );

    let landed: Vec<&str> = interval
        .ledger
        .iter()
        .filter(|l| l.landing == Landing::OnTime)
        .map(|l| l.path.as_str())
        .collect();
    assert_eq!(landed, vec!["src.txt"]);

    let never: Vec<&gitreceipts::reconcile::LedgerLine> = interval.never_landed().collect();
    assert_eq!(never.len(), 1);
    assert_eq!(never[0].path, "scratch.txt");
    assert!(
        never[0]
            .diagnosis
            .unwrap()
            .starts_with("deleted before any commit"),
        "diagnosis was: {:?}",
        never[0].diagnosis
    );

    let residue: Vec<&str> = interval.residue.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(residue, vec!["extra.txt"]);
    assert_eq!(interval.effectful_commands, 1);

    assert_eq!(audit.grades.exact, 2);
    assert_eq!(
        audit.grades.receipted, 1,
        "the commit command is corroborated"
    );
}

#[test]
fn session_jsonl_itself_stays_out_of_the_equation() {
    // The session file lives inside the temp repo in these tests but is
    // never claimed and never committed — it must not appear anywhere.
    let repo = TempRepo::new("noleak");
    let root = repo.root.display().to_string();
    repo.write("src.txt", "hello");
    repo.git(&["add", "src.txt"]);
    repo.git_at(
        &["commit", "-q", "-m", "only"],
        Some("2026-01-01T10:00:31Z"),
    );

    let session_path = repo.root.join("session.jsonl");
    write_session(&session_path, &root);

    let (records, _) = ingest::ingest(&session_path).unwrap();
    let session = extract::extract(&causal::order(records));
    let audit = reconcile::reconcile(&repo.root, &session).unwrap();
    let interval = &audit.intervals[0];
    assert!(
        !interval.residue.iter().any(|c| c.path == "session.jsonl"),
        "uncommitted files are not residue"
    );
}
