//! The worker's jobs (EWP `job`, `cancel`, `progress` and `jobResult`; T-NW-EXP), each journaled as
//! a job ([`journal::JOB`]): `intent` when it starts, `settled` when its result is known.
//!
//! - A media export (`export_media`), rendered by the engine's own exporter into
//!   `outputs/<requestId>/`, a folder only the job writes, on a path the worker builds (the agent and
//!   the host never name one). The job binds the revision and the sequence its caller read
//!   (`expect`, checked in the same call as the export starts), renders that revision whatever the
//!   session does meanwhile, and names it in `jobAccepted`, in its `jobResult` and in its journal
//!   line. It is refused while a stream is still being indexed (`source_loading`: it would render
//!   the stream's head only) and over the session's `maxExportSeconds` (`budget_exceeded`). A job
//!   that fails (`job_failed`) or is cancelled (`job_cancelled`) publishes nothing and leaves nothing
//!   in its folder.
//! - The index of a stream imported in the background (`index`, `<import requestId>/index-<job>`):
//!   the import answers at once with what the stream's head holds, and when the engine has indexed
//!   the whole stream, the worker's poll puts it in. The project's revision then moves between two
//!   requests: the new revision is written as a checkpoint and journaled, a `revision` message says
//!   so, and the index's `jobResult` follows.

use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use filmcraft_engine::export_tools;
use filmcraft_engine::project::{ItemId, MediaRef, Project};
use filmcraft_engine::time::TimeRange;
use serde_json::{Value, json};

use crate::agent_view_policy as policy;
use crate::{Worker, deadline_of, error, expectation, fonts, journal, observe, preconditions, services};

/// The job kind of a media export.
pub(crate) const EXPORT_MEDIA: &str = "export_media";

/// The job kind of the background index of an imported stream.
pub(crate) const INDEX: &str = "index";

/// The engine command whose parameters an `export_media` job takes.
const EXPORT_COMMAND: &str = "file.exportMedia";

/// A media export the worker runs.
#[derive(Debug)]
pub(crate) struct Job {
    request: String,
    /// The engine's job.
    engine: u64,
    /// The job's own folder, `outputs/<requestId>/`.
    folder: PathBuf,
    /// The project revision it renders, and the sequence.
    revision: u64,
    sequence: u64,
    /// The families the rendered project's text substitutes (RD-14).
    substituted: Vec<String>,
    /// Whether the host cancelled it.
    cancelled: bool,
    /// The progress (done, total) last reported.
    reported: Option<(u64, u64)>,
}

/// The background index of an imported stream, a job of the worker.
#[derive(Debug)]
pub(crate) struct Indexing {
    request: String,
    /// The engine's job, the item it indexes for and the file it indexes.
    engine: u64,
    item: u64,
    path: String,
}

/// A refusal of a job before it ran: a registered code and why.
type Refusal = (&'static str, String);

/// The type of a finished output, from its extension.
fn mime(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).as_deref() {
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mxf") => "application/mxf",
        Some("gif") => "image/gif",
        Some("wav") => "audio/wav",
        Some("aiff") => "audio/aiff",
        Some("png") => "image/png",
        Some("tif" | "tiff") => "image/tiff",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// The files a finished job left in its folder, in name order: each with its path in the
/// workspace, its type, size and sha256.
fn outputs(folder: &Path, request: &str) -> std::io::Result<Vec<Value>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    let mut files = Vec::new();
    for name in names {
        let (bytes, sha) = services::file_digest(&folder.join(&name))?;
        let path = format!("{}/{request}/{name}", services::OUTPUTS);
        files.push(json!({"path": path, "mime": mime(&name), "bytes": bytes, "sha256": sha}));
    }
    Ok(files)
}

/// Delete what a failed or cancelled job left in its own folder, then the folder: nothing partial
/// stays.
fn discard(folder: &Path) {
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_file()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir(folder);
}

/// The `jobResult` of a job that did not publish.
fn failed(request: &str, code: &str, why: &str) -> Value {
    json!({"type": "jobResult", "requestId": request, "ok": false, "code": code, "message": why, "outputs": []})
}

/// The seconds of sequence `sequence` an export of `range` renders: the range; with none, the
/// sequence's In to Out when both are set, else the whole sequence (the engine's default range).
fn export_seconds(project: &Project, sequence: ItemId, range: Option<TimeRange>) -> f64 {
    if let Some(range) = range {
        return range.duration.seconds();
    }
    let Some(q) = project.sequence(sequence) else { return 0.0 };
    match (q.mark_in, q.mark_out) {
        (Some(start), Some(out)) if out >= start => (out - start + q.settings.frame_rate.frame_duration()).seconds(),
        _ => q.duration().seconds(),
    }
}

impl Worker {
    /// Answer a `job` message: `jobAccepted` once the export runs, with the revision and sequence
    /// it renders, or its `jobResult` at once when it is refused before it ran (a request id that is
    /// not a plain name or runs already, a job kind other than `export_media`, stale preconditions,
    /// a stream still being indexed, an export over the session's length, parameters outside the
    /// table's media export or naming a path).
    pub(crate) fn job(&mut self, message: &Value) -> Value {
        let request = message.get("requestId").and_then(Value::as_str).unwrap_or_default().to_string();
        let kind = message.get("kind").and_then(Value::as_str).unwrap_or_default();
        if !services::plain(&request) {
            return error(message, "invalid_params", "a job's requestId is a plain name: letters, digits, `.`, `_` and `-`");
        }
        if self.jobs.iter().any(|job| job.request == request) || self.indexing.iter().any(|job| job.request == request) {
            return error(message, "invalid_params", "a job with that requestId runs already");
        }
        let revision = self.session.revision.to_string();
        if let Err(e) = self.journal.intent(&request, kind, journal::JOB) {
            return failed(&request, "workspace_write_failed", &format!("the journal did not take the job: {e}"));
        }
        let expect = message.get("expect").cloned();
        match self.start(&request, kind, message.get("params"), expect.as_ref(), deadline_of(message)) {
            Ok(job) => {
                let accepted = json!({"type": "jobAccepted", "requestId": request, "revision": job.revision.to_string(), "sequence": job.sequence.to_string()});
                self.jobs.push(job);
                accepted
            }
            Err((code, why)) => {
                let journaled = self.journal.settled(&request, "not_applied", Some(code), &revision);
                let mut answer = failed(&request, code, &why);
                answer["revision"] = json!(revision);
                if let Err(e) = journaled {
                    answer["journalError"] = json!(e.to_string());
                }
                answer
            }
        }
    }

    /// Start the export of job `request`, bound to the revision and active sequence of `expect`
    /// (checked as an edit's, with `deadline_ms`): the table's own keys and values of the media
    /// export (checked as the hook checks them, the path aside), settled into the engine's export
    /// settings as `file.exportMedia` settles them, written to `outputs/<request>/export.<ext>`.
    fn start(&mut self, request: &str, kind: &str, params: Option<&Value>, expect: Option<&Value>, deadline_ms: f64) -> Result<Job, Refusal> {
        if kind != EXPORT_MEDIA {
            return Err(("unsupported_operation", format!("no job kind `{kind}`")));
        }
        let observed = observe(&self.session);
        let expect = expectation(expect, deadline_ms, &observed);
        preconditions(&expect, &observed).map_err(|code| (code, "the session is not the one the job was decided on".to_string()))?;
        if let Some(pending) = self.session.background_imports.first() {
            let why = format!("a stream is still being indexed (item {}): export once its index job has its result", pending.item.0);
            return Err(("source_loading", why));
        }
        let params = params.cloned().unwrap_or_else(|| json!({}));
        let Some(keys) = params.as_object() else { return Err(("invalid_params", "a job's `params` is an object".to_string())) };
        if keys.contains_key("path") || keys.contains_key("captionSidecar") {
            return Err(("invalid_params", "the worker builds the output path, and writes no caption sidecar".to_string()));
        }
        let mut checked = keys.clone();
        checked.insert("path".to_string(), json!(format!("{}export", policy::EXPORT_ROOT)));
        match policy::allowed(EXPORT_COMMAND, &Value::Object(checked), policy::Runtime::Browser) {
            Ok(_) | Err("native_only") => {}
            Err(code) => return Err(("invalid_params", format!("the export parameters are outside the table ({code})"))),
        }
        let bad = |e: filmcraft_engine::EngineError| ("invalid_params", e.to_string());
        let session = &mut self.session;
        let sequence = export_tools::sequence_param(session, &params, EXPORT_COMMAND).map_err(bad)?;
        let (_, mut settings) = export_tools::settings_from_params(session, &params, EXPORT_COMMAND).map_err(bad)?;
        let project = session.project.clone();
        settings.range = export_tools::range_param(session, &project, sequence, params.get("range"), &params, EXPORT_COMMAND).map_err(bad)?;
        if let Some(cap) = self.limits.max_export_seconds {
            let seconds = export_seconds(&project, sequence, settings.range);
            if seconds > cap {
                return Err(("budget_exceeded", format!("the export renders {seconds:.2} s, over the session's {cap} s")));
            }
        }
        let folder = self.services.output_folder(request).map_err(|e| ("invalid_params", e.to_string()))?;
        settings.path = folder.join(format!("export.{}", settings.extension())).to_string_lossy().into_owned();
        let substituted = fonts::substituted(&project);
        let label = format!("Export {request}");
        let started = export_tools::spawn_export(&mut self.session, project, sequence, settings, label, false);
        let engine = started.map_err(|e| {
            discard(&folder);
            ("job_failed", e.to_string())
        })?;
        Ok(Job {
            request: request.to_string(),
            engine,
            folder,
            revision: observed.revision,
            sequence: sequence.0,
            substituted,
            cancelled: false,
            reported: None,
        })
    }

    /// Answer a `cancel` message: the export stops at its next batch, and its `jobResult` says
    /// `job_cancelled`.
    pub(crate) fn cancel(&mut self, message: &Value) -> Value {
        let request = message.get("requestId").and_then(Value::as_str).unwrap_or_default();
        let Some(job) = self.jobs.iter_mut().find(|job| job.request == request) else {
            return error(message, "invalid_params", "no export of that requestId runs");
        };
        job.cancelled = true;
        if let Some(engine) = self.session.jobs.iter().find(|engine| engine.id == job.engine) {
            engine.progress.cancel.store(true, Ordering::Relaxed);
        }
        json!({"type": "cancelAccepted", "requestId": request})
    }

    /// Ask every running export to stop.
    pub(crate) fn cancel_jobs(&mut self) {
        for job in &mut self.jobs {
            job.cancelled = true;
            if let Some(engine) = self.session.jobs.iter().find(|engine| engine.id == job.engine) {
                engine.progress.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// The progress of the running exports since it was last reported, and the results of the
    /// exports that finished.
    pub(crate) fn poll_jobs(&mut self) -> Vec<Value> {
        let mut messages = Vec::new();
        let mut finished = Vec::new();
        for (k, job) in self.jobs.iter_mut().enumerate() {
            let Some(engine) = self.session.jobs.iter().find(|engine| engine.id == job.engine) else {
                finished.push((k, Err("the engine has no such job".to_string())));
                continue;
            };
            let outcome = engine.result.lock().unwrap_or_else(PoisonError::into_inner).clone();
            if let Some(outcome) = outcome {
                finished.push((k, outcome.map(|_| ())));
                continue;
            }
            let progress = (engine.progress.done.load(Ordering::Relaxed), engine.progress.total.load(Ordering::Relaxed));
            if job.reported != Some(progress) {
                job.reported = Some(progress);
                let phase = engine.progress.status.lock().unwrap_or_else(PoisonError::into_inner).clone();
                messages.push(json!({"type": "progress", "requestId": job.request, "done": progress.0, "total": progress.1, "phase": phase}));
            }
        }
        for (k, outcome) in finished.into_iter().rev() {
            let job = self.jobs.remove(k);
            messages.push(self.settle_job(job, outcome));
        }
        messages
    }

    /// The `jobResult` of a finished export, journaled first with the revision it rendered: its
    /// outputs when it succeeded, else `job_cancelled` (the host cancelled it) or `job_failed` (the
    /// export failed, or its files do not read), with its folder emptied. It names the revision and
    /// sequence it rendered, the font set's digest and the families its text substitutes.
    fn settle_job(&mut self, job: Job, outcome: Result<(), String>) -> Value {
        let revision = job.revision.to_string();
        let mut answer = match outcome {
            Ok(()) => match outputs(&job.folder, &job.request) {
                Ok(files) => json!({"type": "jobResult", "requestId": job.request, "ok": true, "outputs": files}),
                Err(e) => {
                    discard(&job.folder);
                    failed(&job.request, journal::JOB_FAILED, &format!("the export's files do not read: {e}"))
                }
            },
            Err(why) => {
                discard(&job.folder);
                let code = if job.cancelled || why == "cancelled" { "job_cancelled" } else { journal::JOB_FAILED };
                failed(&job.request, code, &why)
            }
        };
        answer["kind"] = json!(EXPORT_MEDIA);
        answer["revision"] = json!(revision);
        answer["sequence"] = json!(job.sequence.to_string());
        answer["fontsetDigest"] = json!(self.fonts.digest);
        answer["substituted"] = json!(job.substituted);
        let outcome = if answer["ok"] == true { "applied" } else { "not_applied" };
        let journaled = self.journal.settled(&job.request, outcome, answer.get("code").and_then(Value::as_str), &revision);
        if let Err(e) = journaled {
            answer["journalError"] = json!(e.to_string());
        }
        answer
    }

    /// Journal, as jobs of the worker (`<request>/index-<job>`), the background indexes that the
    /// import `request` started for `items`; answers their entries for the import's result.
    pub(crate) fn start_indexing(&mut self, request: &str, items: &[u64]) -> Vec<Value> {
        let imports = self.session.background_imports.iter().filter(|b| items.contains(&b.item.0));
        let started: Vec<(u64, u64, String)> = imports.map(|b| (b.job, b.item.0, b.path.clone())).collect();
        let mut entries = Vec::new();
        for (engine, item, path) in started {
            let id = format!("{request}/index-{engine}");
            let mut entry = json!({"requestId": id, "kind": INDEX, "item": item});
            if let Err(e) = self.journal.intent(&id, INDEX, journal::JOB) {
                entry["journalError"] = json!(e.to_string());
            }
            entries.push(entry);
            self.indexing.push(Indexing { request: id, engine, item, path });
        }
        entries
    }

    /// The background indexes that finished since the last poll, once the engine has put in what
    /// they indexed: when the project moved from `revision_before`, its new revision is written as
    /// a checkpoint and journaled, and a `revision` message says so (`cause`: the indexes; none when
    /// other engine work moved it); then each index's `jobResult`, journaled.
    pub(crate) fn poll_indexing(&mut self, revision_before: u64) -> Vec<Value> {
        let pending: Vec<u64> = self.session.background_imports.iter().map(|b| b.job).collect();
        let jobs = std::mem::take(&mut self.indexing);
        let (running, finished): (Vec<Indexing>, Vec<Indexing>) = jobs.into_iter().partition(|job| pending.contains(&job.engine));
        self.indexing = running;
        let mut messages = Vec::new();
        let mut checkpoint = Value::Null;
        let revision = self.session.revision;
        if revision != revision_before {
            let cause: Vec<&str> = finished.iter().map(|job| job.request.as_str()).collect();
            let mut message = json!({"type": "revision", "revisionBefore": revision_before.to_string(), "revisionAfter": revision.to_string(), "cause": cause});
            match self.checkpoint() {
                Ok(file) => {
                    message["checkpoint"] = file.clone();
                    checkpoint = file;
                }
                Err(e) => message["checkpointError"] = json!(e.to_string()),
            }
            if let Err(e) = self.journal.revision(revision_before, revision, &cause, &checkpoint) {
                message["journalError"] = json!(e.to_string());
            }
            messages.push(message);
        }
        for job in finished {
            messages.push(self.settle_index(job, &checkpoint));
        }
        messages
    }

    /// The `jobResult` of the finished index `job`, journaled: `ok` when the engine indexed the
    /// stream and its item, still naming the file, now holds the whole stream (with the checkpoint
    /// of that revision); else `job_failed` and the item keeps what the stream's head holds.
    fn settle_index(&mut self, job: Indexing, checkpoint: &Value) -> Value {
        let revision = self.session.revision.to_string();
        let outcome = match self.session.jobs.iter().find(|engine| engine.id == job.engine) {
            None => Err("the engine has no such job".to_string()),
            Some(engine) => match engine.result.lock().unwrap_or_else(PoisonError::into_inner).clone() {
                Some(result) => result.map(|_| ()),
                // the index marks itself finished just before it stores its result; an error it
                // met is in its progress by then
                None => match engine.progress.error.lock().unwrap_or_else(PoisonError::into_inner).clone() {
                    Some(why) => Err(why),
                    None => Ok(()),
                },
            },
        };
        let names = |m: &filmcraft_engine::project::MediaClip| matches!(&m.media, MediaRef::File { path } if *path == job.path);
        let holds = self.session.project.item(ItemId(job.item)).and_then(|item| item.as_media()).is_some_and(names);
        let mut answer = match outcome {
            Ok(()) if holds => json!({"type": "jobResult", "requestId": job.request, "ok": true, "outputs": []}),
            Ok(()) => failed(&job.request, journal::JOB_FAILED, "the item no longer names the file: nothing was put in"),
            Err(why) => failed(&job.request, journal::JOB_FAILED, &why),
        };
        answer["kind"] = json!(INDEX);
        answer["item"] = json!(job.item);
        answer["revision"] = json!(revision);
        if answer["ok"] == true && !checkpoint.is_null() {
            answer["checkpoint"] = checkpoint.clone();
        }
        let outcome = if answer["ok"] == true { "applied" } else { "not_applied" };
        let journaled = self.journal.settled(&job.request, outcome, answer.get("code").and_then(Value::as_str), &revision);
        if let Err(e) = journaled {
            answer["journalError"] = json!(e.to_string());
        }
        answer
    }

    /// Settle the indexes still running when the worker stops: `job_cancelled`, journaled (the
    /// items keep what their streams' heads hold).
    pub(crate) fn drop_indexing(&mut self) -> Vec<Value> {
        let revision = self.session.revision.to_string();
        let mut messages = Vec::new();
        for job in std::mem::take(&mut self.indexing) {
            let mut answer = failed(&job.request, "job_cancelled", "the worker stopped before the index ended");
            answer["kind"] = json!(INDEX);
            answer["item"] = json!(job.item);
            if let Err(e) = self.journal.settled(&job.request, "not_applied", Some("job_cancelled"), &revision) {
                answer["journalError"] = json!(e.to_string());
            }
            messages.push(answer);
        }
        messages
    }
}
