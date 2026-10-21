//! The worker's journal (`journal.jsonl` in the session workspace), one JSON object per line.
//!
//! Before an edit, an import, an open or a job touches the engine, the worker appends
//! `{"t":"intent", "requestId", "operation", "class", "at"}`; when it is decided, it appends
//! `{"t":"settled", "requestId", "outcome", "code"?, "revisionAfter", "at"}`, and only then answers
//! the host. When the project moves between requests (a background index puts in what it indexed),
//! it appends `{"t":"revision", "revisionBefore", "revisionAfter", "cause", "checkpoint", "at"}`
//! once the new revision is written as a checkpoint. Each line is on disk (`sync_data`) before the
//! worker goes on.
//!
//! A worker that exits or is killed between an intent and its settlement leaves the intent open,
//! and its class says what the host settles it as ([`recover`]). A mutation (an edit, an import, an
//! open; [`MUTATION`]) may have changed the project: it is `unknown`, `worker_lost`, and the session
//! retires and forks from its last durable project revision (native-worker-architecture.md §5.7,
//! media-agent-contracts.md §9.1). A job (an export, a background index; [`JOB`]) publishes or puts
//! in nothing before its result is settled: it is `not_applied`, `job_failed`, and nothing fences.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// The journal's name in the session workspace.
pub const FILE_NAME: &str = "journal.jsonl";

/// The code a lost worker's unsettled mutation is settled with.
pub const WORKER_LOST: &str = "worker_lost";

/// The code of a job that did not finish: it failed, or the worker was lost while it ran.
pub const JOB_FAILED: &str = "job_failed";

/// The class of an intent that may change the project: an edit, an import, an open.
pub const MUTATION: &str = "mutation";

/// The class of an intent that changes nothing until its result is settled: an export, an index.
pub const JOB: &str = "job";

/// Milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The journal a worker appends to.
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    /// The journal at `path` (created by the first line).
    pub fn at(path: &Path) -> Journal {
        Journal { path: path.to_path_buf() }
    }

    /// Where it is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append `line` and put it on disk.
    fn append(&self, line: &Value) -> io::Result<()> {
        let mut text = line.to_string();
        text.push('\n');
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        file.write_all(text.as_bytes())?;
        file.sync_data()
    }

    /// `requestId` is about to touch the engine with `operation`, of `class` ([`MUTATION`] or [`JOB`]).
    pub fn intent(&self, request: &str, operation: &str, class: &str) -> io::Result<()> {
        self.append(&json!({"t": "intent", "requestId": request, "operation": operation, "class": class, "at": now_ms()}))
    }

    /// `requestId` is decided: its `outcome`, its `code` (if any) and the project revision after it
    /// (a decimal string; for an export, the revision it rendered).
    pub fn settled(&self, request: &str, outcome: &str, code: Option<&str>, revision_after: &str) -> io::Result<()> {
        let mut line = json!({"t": "settled", "requestId": request, "outcome": outcome, "revisionAfter": revision_after, "at": now_ms()});
        if let Some(code) = code {
            line["code"] = json!(code);
        }
        self.append(&line)
    }

    /// The project moved from revision `before` to `after` between requests, by the jobs of `cause`
    /// (none: other engine work), and `checkpoint` holds the new revision (null when it could not
    /// be written).
    pub fn revision(&self, before: u64, after: u64, cause: &[&str], checkpoint: &Value) -> io::Result<()> {
        let (before, after) = (before.to_string(), after.to_string());
        self.append(&json!({"t": "revision", "revisionBefore": before, "revisionAfter": after, "cause": cause, "checkpoint": checkpoint, "at": now_ms()}))
    }
}

/// What the host settles for the worker that wrote the journal at `path` and is gone: one
/// `{"requestId", "operation", "class", "outcome", "code"}` per intent that has no settlement, in
/// journal order. A mutation (or an intent of no class, as older journals wrote them) is
/// `unknown` / [`WORKER_LOST`]; a job is `not_applied` / [`JOB_FAILED`]. A settlement settles the
/// earliest open intent of its request; a line that does not read (the last one, cut by the kill)
/// is skipped. A missing journal settles nothing.
pub fn recover(path: &Path) -> io::Result<Vec<Value>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut open: Vec<(String, Value, String)> = Vec::new();
    for line in String::from_utf8_lossy(&bytes).lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        let Some(request) = entry.get("requestId").and_then(Value::as_str) else { continue };
        match entry.get("t").and_then(Value::as_str) {
            Some("intent") => {
                let class = entry.get("class").and_then(Value::as_str).unwrap_or(MUTATION).to_string();
                open.push((request.to_string(), entry.get("operation").cloned().unwrap_or(Value::Null), class));
            }
            Some("settled") => {
                if let Some(k) = open.iter().position(|(intent, _, _)| intent == request) {
                    open.remove(k);
                }
            }
            _ => {}
        }
    }
    let lost = |(request, operation, class): (String, Value, String)| {
        let (outcome, code) = if class == JOB { ("not_applied", JOB_FAILED) } else { ("unknown", WORKER_LOST) };
        json!({"requestId": request, "operation": operation, "class": class, "outcome": outcome, "code": code})
    };
    Ok(open.into_iter().map(lost).collect())
}
