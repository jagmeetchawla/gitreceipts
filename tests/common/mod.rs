//! Shared fixtures for integration tests: a disposable git repo and a
//! builder that writes syntactically real session JSONL.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TempRepo {
    pub root: PathBuf,
}

impl TempRepo {
    pub fn new(name: &str) -> TempRepo {
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

    pub fn git(&self, args: &[&str]) {
        self.git_at(args, None);
    }

    /// Run git with a pinned author+committer date, so tests control the
    /// commit timestamps the spine is built from.
    ///
    /// Tests are hermetic: global and system git config are neutralized
    /// (via `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM=/dev/null`) so a
    /// developer's `commit.gpgsign`, signing agent, or `core.hooksPath`
    /// can't hang or fail a temp-repo commit.
    pub fn git_at(&self, args: &[&str], date: Option<&str>) {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .args(args);
        if let Some(d) = date {
            cmd.env("GIT_AUTHOR_DATE", d).env("GIT_COMMITTER_DATE", d);
        }
        let status = cmd.status().unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    /// Commit as a specific author/committer, with an optional
    /// `Co-Authored-By` trailer — for multi-author / multi-agent fixtures.
    /// Staged content must already be added.
    pub fn commit_as(
        &self,
        author: &str,
        email: &str,
        date: &str,
        msg: &str,
        co_author: Option<&str>,
    ) {
        let body = match co_author {
            Some(co) => format!("{msg}\n\nCo-Authored-By: {co}"),
            None => msg.to_string(),
        };
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", author)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .args(["commit", "-q", "-m", &body])
            .status()
            .unwrap();
        assert!(status.success(), "commit_as {author} failed");
    }

    pub fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    pub fn remove(&self, rel: &str) {
        fs::remove_file(self.root.join(rel)).unwrap();
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Builds a causally chained session log: every event's parentUuid is the
/// previous event's uuid, and every claim gets an ok tool_result, like a
/// healthy real session.
pub struct SessionBuilder {
    cwd: String,
    session_id: String,
    lines: Vec<String>,
    next_id: usize,
    last_uuid: Option<String>,
}

impl SessionBuilder {
    pub fn new(cwd: &str) -> SessionBuilder {
        Self::with_id(cwd, "")
    }

    /// A builder whose uuids carry a session-unique prefix — real event
    /// uuids are globally unique, so two merged fixture sessions must not
    /// collide by accident.
    pub fn with_id(cwd: &str, session_id: &str) -> SessionBuilder {
        SessionBuilder {
            cwd: cwd.to_string(),
            session_id: session_id.to_string(),
            lines: Vec::new(),
            next_id: 0,
            last_uuid: None,
        }
    }

    fn uuid(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{}{prefix}{}", self.session_id, self.next_id)
    }

    fn parent_field(&self) -> String {
        match &self.last_uuid {
            Some(p) => format!(r#""parentUuid":"{p}","#),
            None => String::new(),
        }
    }

    fn push(&mut self, kind: &str, ts: &str, uuid: &str, message: &str) {
        let parent = self.parent_field();
        let cwd = &self.cwd;
        self.lines.push(format!(
            r#"{{"type":"{kind}","uuid":"{uuid}",{parent}"timestamp":"{ts}","cwd":"{cwd}","message":{message}}}"#
        ));
        self.last_uuid = Some(uuid.to_string());
    }

    pub fn user_text(&mut self, ts: &str, text: &str) -> &mut Self {
        let u = self.uuid("u");
        self.push("user", ts, &u, &format!(r#"{{"content":"{text}"}}"#));
        self
    }

    /// Subsequent events record this cwd — a session wandering into a
    /// scratch dir or another repo.
    pub fn set_cwd(&mut self, cwd: &str) -> &mut Self {
        self.cwd = cwd.to_string();
        self
    }

    fn claim(&mut self, ts: &str, result_ts: &str, tool: &str, input: &str) -> &mut Self {
        let a = self.uuid("a");
        let t = self.uuid("t");
        self.push(
            "assistant",
            ts,
            &a.clone(),
            &format!(
                r#"{{"content":[{{"type":"tool_use","id":"{t}","name":"{tool}","input":{input}}}]}}"#
            ),
        );
        let u = self.uuid("u");
        self.push(
            "user",
            result_ts,
            &u.clone(),
            &format!(
                r#"{{"content":[{{"type":"tool_result","tool_use_id":"{t}","is_error":false,"content":"ok"}}]}}"#
            ),
        );
        self
    }

    /// A Write claim for an absolute path under the session cwd. The
    /// default claimed content is a usable probe (>= 12 chars) unique to
    /// the path, so content-verification against a commit is meaningful.
    pub fn write_claim(&mut self, ts: &str, result_ts: &str, rel: &str) -> &mut Self {
        let body = format!("// body of {rel} :: placeholder content");
        self.write_claim_content(ts, result_ts, rel, &body)
    }

    /// The default claimed body for a path — matches `write_claim`, so a
    /// test can write the same bytes to disk / into a commit.
    pub fn default_body(rel: &str) -> String {
        format!("// body of {rel} :: placeholder content")
    }

    /// A Write claim with specific claimed content (the probe).
    pub fn write_claim_content(
        &mut self,
        ts: &str,
        result_ts: &str,
        rel: &str,
        content: &str,
    ) -> &mut Self {
        let path = format!("{}/{}", self.cwd, rel);
        let content = Self::json_escape(content);
        self.claim(
            ts,
            result_ts,
            "Write",
            &format!(r#"{{"file_path":"{path}","content":"{content}"}}"#),
        )
    }

    /// A Write claim for an arbitrary absolute path (scratch dirs etc).
    pub fn write_claim_abs(&mut self, ts: &str, result_ts: &str, abs: &str) -> &mut Self {
        self.claim(
            ts,
            result_ts,
            "Write",
            &format!(r#"{{"file_path":"{abs}","content":"x"}}"#),
        )
    }

    fn json_escape(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\t', "\\t")
    }

    pub fn bash_claim(&mut self, ts: &str, result_ts: &str, command: &str) -> &mut Self {
        let escaped = Self::json_escape(command);
        self.claim(
            ts,
            result_ts,
            "Bash",
            &format!(r#"{{"command":"{escaped}"}}"#),
        )
    }

    /// A Bash claim whose tool_result carries specific captured output.
    pub fn bash_claim_with_output(
        &mut self,
        ts: &str,
        result_ts: &str,
        command: &str,
        output: &str,
    ) -> &mut Self {
        self.bash_claim_result(ts, result_ts, command, output, false)
    }

    /// A Bash claim whose tool_result is an error (is_error: true).
    pub fn bash_claim_failed(
        &mut self,
        ts: &str,
        result_ts: &str,
        command: &str,
        output: &str,
    ) -> &mut Self {
        self.bash_claim_result(ts, result_ts, command, output, true)
    }

    fn bash_claim_result(
        &mut self,
        ts: &str,
        result_ts: &str,
        command: &str,
        output: &str,
        is_error: bool,
    ) -> &mut Self {
        let cmd = Self::json_escape(command);
        let out = Self::json_escape(output);
        let a = self.uuid("a");
        let t = self.uuid("t");
        self.push(
            "assistant",
            ts,
            &a.clone(),
            &format!(
                r#"{{"content":[{{"type":"tool_use","id":"{t}","name":"Bash","input":{{"command":"{cmd}"}}}}]}}"#
            ),
        );
        let u = self.uuid("u");
        self.push(
            "user",
            result_ts,
            &u.clone(),
            &format!(
                r#"{{"content":[{{"type":"tool_result","tool_use_id":"{t}","is_error":{is_error},"content":"{out}"}}]}}"#
            ),
        );
        self
    }

    pub fn raw_line(&mut self, line: &str) -> &mut Self {
        self.lines.push(line.to_string());
        self
    }

    pub fn save(&self, path: &Path) {
        fs::write(path, self.lines.join("\n")).unwrap();
    }
}
