//! The LTS native worker of the Video editor engine (slice NW-1a; master plan v3 W7, RD-2, RD-3).
//!
//! One worker process holds one contained editing session of the engine for the LTS native host
//! (`lts-native-host`, NW-1b). The host starts it with its session workspace and drives it over the
//! Editor Worker Protocol, EWP v1 ([`PROTOCOL`]): newline-delimited JSON messages on stdin and
//! stdout, at most [`MAX_LINE_BYTES`] per line either way ([`line_of`]). Bytes never travel inline:
//! images, exports and project snapshots are files under the workspace, reported with their size and
//! sha256 (`reconcile/native-worker-architecture.md` §5.2 and Appendix A).
//!
//! The workspace holds `files/` (staged sources and fonts, each named `lts-<sha256>.<ext>`),
//! `project/` (snapshots), `previews/` (model-visible images), `outputs/<requestId>/` (exports)
//! and `journal.jsonl`.
//!
//! - [`agent_view_policy`]: the browser hook's own policy file, compiled here through `#[path]`, so
//!   both runtimes share one table, one precondition order and one set of envelopes; `ready`
//!   reports the sha256 of the file (T-NW-POL, T-NW-PRE).
//! - [`ContainedServices`]: the engine reads staged files by their content name only, writes only
//!   under the project and outputs folders, and lists nothing (T-NW-SVC, T-NW-ESC).
//! - [`startup`]: no system font scan (OC-33 E1) and software decoding only (T-NW-ENV); the fonts a
//!   session renders with are the bundled ones and those `hello` registers by their bytes.
//! - Edits run whole within their call: preconditions, the table, the pre-checks (a font family no
//!   loaded face holds is `font_not_loaded`) and the engine run, and a scene detection or a mask
//!   tracking runs to its end (`wait`). An import opens its file first, and a file the decoders
//!   refuse is refused `format_unsupported` before the import runs.
//! - Reads and jobs: frames from the engine's own compositor (T-NW-FR-01); exports under
//!   `outputs/<requestId>/`, bound to the revision they render, the same bytes every time, and
//!   nothing left by a cancelled one (T-NW-EXP).
//! - [`journal`]: `intent` before the engine runs and `settled` after; a project revision that
//!   moves between requests (a background index) is written and journaled too; the host settles
//!   what a lost worker left (T-NW-JRN).
//! - [`imaging`]: fresh PNG and JPEG without metadata, checked by their structure (T-NW-IMG).
//! - `reach` (slice RN, native reach, 2026-10-23): the checks of the table's native-only entries
//!   before they run, the import of a caption file staged by handle (`importCaptions`), the `media`
//!   and `watermark` reads and an export's caption sidecar (T-NW-RN).
//!
//! EWP v1 as this worker serves it (host → worker, then worker → host):
//!
//! - `hello {protocol, mode: "session", engine: "filmcraft", sessionId, instanceId, limits?, fonts?,
//!   decode?}` → `ready {protocol, engine, build {policySha256, policyVersion, systemFontScan,
//!   hardwareDecoding, workerSha256, fontsetDigest, …}, operations[], readKinds[], jobKinds[],
//!   limits, fonts[], fontsetDigest}`; nothing else is served before it. `limits` may lower
//!   `maxSide`, `maxSheetCells`, `maxPreviewBytes` and set `maxExportSeconds` (`threads` and
//!   `mediaCacheMiB` are the host's, through the process it starts); `fonts` lists
//!   `{name, path: "files/lts-<sha256>.<ext>", sha256}`; `decode.hardware` is false;
//! - `stage {stageId, handle, path, bytes}` → `staged {stageId, ok, path?, code?}`;
//! - `execute {requestId, operation, params, expect, deadlineMs}` → `result {requestId, envelope,
//!   guarantee: "atomic", checkpoint?}`, the operation one of the table's (`expect` holds the
//!   decimal `revision` and `activeSequence`, and optionally `historyHead`, `linkedSelection` and
//!   `targeting`, as the hook's `executeTyped` takes them);
//! - `import {requestId, handle, expect, deadlineMs}` → `result {…, media[{item, warnings,
//!   damage}], jobs[]?}` and `open {requestId, path, expectSha256}` → `result {…, guarantee:
//!   "unavailable", substituted[]}`, the worker's own typed operations; and `importCaptions
//!   {requestId, handle, offsetTicks?, dropBeforeOffset?, windowTicks?, name?, expect, deadlineMs}`
//!   → `result {…, envelope {result {format, sequence, track, captions, dropped, offsetTicks,
//!   warnings}}}` (slice RN);
//! - `read {readId, kind: frames|fonts|query|media|watermark, params, deadlineMs?}` → `readResult
//!   {readId, ok, value, files[]}` (`media {item, levels?}` and `watermark`, slice RN);
//! - `job {requestId, kind: export_media, params, expect, deadlineMs}` (`params` the table's media
//!   export keys, the path aside, and `captionSidecar`, `rtlLineMarks`, slice RN) → `jobAccepted {requestId,
//!   revision, sequence}`, then `progress` and one `jobResult {requestId, kind, ok, code?, revision,
//!   outputs[]}`; `cancel {requestId}` → `cancelAccepted`; an import's background index answers
//!   `jobResult {kind: "index"}`, after a `revision {revisionBefore, revisionAfter, cause,
//!   checkpoint}` when the project moved;
//! - `ping {seq}` → `pong {seq, revision, queue, rssBytes}`; `shutdown` → the settled jobs, then
//!   `bye`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

/// The typed policy of the browser hook (`apps/filmcraft-web/src/agent_view_policy.rs`), the same
/// file compiled natively: one table for both runtimes (risk R11 of the native worker design).
#[path = "../../filmcraft-web/src/agent_view_policy.rs"]
pub mod agent_view_policy;

mod fonts;
pub mod imaging;
mod jobs;
pub mod journal;
mod reach;
mod reads;
pub mod services;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use filmcraft_engine::project::{ItemId, Project, ProjectView};
use filmcraft_engine::{EditorState, EngineError, History, MediaPool, Session};
use filmcraft_media::MediaError;
use serde_json::{Value, json};

use crate::agent_view_policy as policy;

pub use services::ContainedServices;

/// The protocol a `hello` names and `ready` answers with.
pub const PROTOCOL: &str = "lts.editor-worker.v1";

/// The longest line either side sends, in bytes (INV-27 parity).
pub const MAX_LINE_BYTES: usize = 1 << 20;

/// The engine a `hello` names.
pub const ENGINE: &str = "filmcraft";

/// The policy file as this worker compiled it: `ready` reports its sha256, which is the hook's.
const POLICY_SOURCE: &[u8] = include_bytes!("../../filmcraft-web/src/agent_view_policy.rs");

/// How long the worker process waits for a line before it looks at its jobs again.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How long `shutdown` waits for the cancelled jobs to stop.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(30);

/// The commands of the table whose run the engine may leave to finish in the background
/// (`wait: false`, its default): a worker runs them to their end within the call, so the result
/// names what the edit changed and nothing lands between two requests.
const WAITED: [&str; 2] = ["clip.sceneEditDetection", "masks.track"];

/// The process-wide guards a worker sets before the engine runs: no font lookup walks the system
/// font folders (`filmcraft_engine::graphics::set_system_scan(false)`, OC-33 E1) and every decoder
/// is a software one.
pub fn startup() {
    filmcraft_engine::graphics::set_system_scan(false);
    filmcraft_codecs::hw::set_hardware_decoding(false);
}

/// Milliseconds since the Unix epoch, the clock of an EWP deadline.
fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(f64::NAN, |d| d.as_secs_f64() * 1000.0)
}

/// What a session may spend, from `hello`'s `limits`: each may only lower the worker's own cap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Limits {
    /// The longest side of a frame a read renders, in pixels.
    pub(crate) max_side: u32,
    /// The most frames one read renders.
    pub(crate) max_frames: usize,
    /// The most bytes one model-visible image holds.
    pub(crate) max_preview_bytes: Option<u64>,
    /// The longest export, in seconds of the sequence.
    pub(crate) max_export_seconds: Option<f64>,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_side: reads::MAX_SIDE, max_frames: reads::MAX_FRAMES, max_preview_bytes: None, max_export_seconds: None }
    }
}

impl Limits {
    /// The limits `hello` sets (`limits`, each key optional): `maxSide` and `maxSheetCells` lower
    /// the worker's own caps, `maxPreviewBytes` and `maxExportSeconds` set caps, and `threads` and
    /// `mediaCacheMiB` are the host's to enforce on the process it starts. A key of another kind or
    /// a value that is not a positive number is refused.
    fn of_hello(limits: Option<&Value>) -> Result<Limits, String> {
        let mut out = Limits::default();
        let keys = match limits {
            None | Some(Value::Null) => return Ok(out),
            Some(Value::Object(keys)) => keys,
            Some(_) => return Err("`limits` is an object".to_string()),
        };
        for (key, value) in keys {
            let whole = value.as_u64().filter(|&n| n > 0);
            let seconds = value.as_f64().filter(|n| n.is_finite() && *n > 0.0);
            match (key.as_str(), whole, seconds) {
                ("maxSide", Some(n), _) => out.max_side = out.max_side.min(u32::try_from(n).unwrap_or(u32::MAX)),
                ("maxSheetCells", Some(n), _) => out.max_frames = out.max_frames.min(usize::try_from(n).unwrap_or(usize::MAX)),
                ("maxPreviewBytes", Some(n), _) => out.max_preview_bytes = Some(n),
                ("maxExportSeconds", _, Some(n)) => out.max_export_seconds = Some(n),
                ("threads" | "mediaCacheMiB", Some(_), _) => {}
                _ => return Err(format!("`limits.{key}` is not a positive number of a limit this worker takes")),
            }
        }
        Ok(out)
    }

    /// `ready`'s report of the limits in force.
    fn report(self) -> Value {
        json!({
            "maxSide": self.max_side,
            "maxSheetCells": self.max_frames,
            "maxPreviewBytes": self.max_preview_bytes,
            "maxExportSeconds": self.max_export_seconds,
            "maxLineBytes": MAX_LINE_BYTES,
            "hostEnforced": ["threads", "mediaCacheMiB"],
        })
    }
}

/// One contained editing session of the engine.
pub struct Worker {
    root: PathBuf,
    services: Arc<ContainedServices>,
    session: Session,
    /// The editor state of a new session: a project the worker opens starts from it.
    fresh: EditorState,
    journal: journal::Journal,
    /// The exports that run.
    jobs: Vec<jobs::Job>,
    /// The streams the engine indexes in the background after an import, each a job of the worker.
    indexing: Vec<jobs::Indexing>,
    /// What the session may spend, from `hello`.
    limits: Limits,
    /// The fonts the session renders with: the bundled ones, and those `hello` registered.
    fonts: fonts::Fontset,
    /// The sha256 of the worker's executable, when it runs as one ([`serve`]).
    binary_sha256: Option<String>,
    /// The channel count of each sound track of an item's file, by item, once a `media` read looked (slice RN).
    layouts: BTreeMap<u64, Vec<u32>>,
    /// Whether `hello` was answered with `ready`.
    ready: bool,
    /// Whether `shutdown` was answered.
    stopped: bool,
}

impl Worker {
    /// A worker over the session workspace `root` (an existing folder), after [`startup`]. Its
    /// session decodes in software whatever its preferences are set to later, never looks for an
    /// image sequence beside a still (that probes neighbouring names) and transcribes nothing on
    /// import.
    pub fn new(root: &Path) -> std::io::Result<Worker> {
        startup();
        let services = Arc::new(ContainedServices::new(root)?);
        let mut session = Session::new(services.clone());
        session.prefs.playback.hardware_decoding = "off".to_string();
        session.prefs.media.hardware_decoding = false;
        session.prefs.media.import_image_sequences = false;
        session.prefs.media_analysis.auto_transcribe = false;
        let fresh = session.state.clone();
        let journal = journal::Journal::at(&root.join(journal::FILE_NAME));
        Ok(Worker {
            root: root.to_path_buf(),
            services,
            session,
            fresh,
            journal,
            jobs: Vec::new(),
            indexing: Vec::new(),
            limits: Limits::default(),
            fonts: fonts::Fontset::bundled(),
            binary_sha256: None,
            layouts: BTreeMap::new(),
            ready: false,
            stopped: false,
        })
    }

    /// The session workspace.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The engine session, for reading only.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The contained file services the session runs on.
    pub fn services(&self) -> &ContainedServices {
        &self.services
    }

    /// Whether the host asked the worker to stop (`shutdown`).
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Answer one host message with the messages it gets at once.
    pub fn handle(&mut self, message: &Value) -> Vec<Value> {
        let kind = message.get("type").and_then(Value::as_str).unwrap_or_default();
        let answers = match kind {
            "hello" => vec![self.hello(message)],
            _ if !self.ready => vec![error(message, "invalid_params", "the worker answers `hello` first")],
            "stage" => vec![self.stage(message)],
            "execute" => vec![self.execute(message)],
            "import" => vec![self.import(message)],
            "importCaptions" => vec![self.import_captions(message)],
            "open" => vec![self.open(message)],
            "read" => vec![reads::read(self, message)],
            "job" => vec![self.job(message)],
            "cancel" => vec![self.cancel(message)],
            "ping" => vec![self.pong(message)],
            "shutdown" => self.shutdown(),
            _ => vec![error(message, "invalid_params", &format!("`{kind}` is not a message of {PROTOCOL}"))],
        };
        // the worker has no interface: what the session announces to one is dropped
        self.session.drain_events();
        answers
    }

    /// The messages of background work since the last call: the exports' progress and results,
    /// and the indexes of imported streams the engine put in, after a `revision` message when the
    /// project moved (its new revision written and journaled first).
    pub fn poll(&mut self) -> Vec<Value> {
        let before = self.session.revision;
        self.session.poll_persistence();
        let mut messages = self.poll_jobs();
        messages.extend(self.poll_indexing(before));
        self.session.drain_events();
        messages
    }

    /// Answer one line of the host with the lines to write back ([`line_of`]).
    pub fn handle_line(&mut self, line: &str) -> Vec<String> {
        let answers = match serde_json::from_str::<Value>(line) {
            Ok(message) => self.handle(&message),
            Err(e) => vec![error(&Value::Null, "invalid_params", &format!("a line that is not JSON: {e}"))],
        };
        answers.iter().map(line_of).collect()
    }

    fn hello(&mut self, message: &Value) -> Value {
        let field = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or_default();
        if self.ready {
            return error(message, "invalid_params", "the worker answered `hello` already");
        }
        if field("protocol") != PROTOCOL || field("engine") != ENGINE {
            return error(message, "invalid_params", &format!("this worker speaks {PROTOCOL} for {ENGINE}"));
        }
        if !matches!(field("mode"), "" | "session") {
            return error(message, "invalid_params", "this worker runs mode `session` only");
        }
        if message.pointer("/decode/hardware").and_then(Value::as_bool) == Some(true) {
            return error(message, "invalid_params", "this worker decodes in software only: `decode.hardware` is false");
        }
        let limits = match Limits::of_hello(message.get("limits")) {
            Ok(limits) => limits,
            Err(why) => return error(message, "invalid_params", &why),
        };
        let fonts = match fonts::register(&self.services, message.get("fonts")) {
            Ok(fonts) => fonts,
            Err(refusal) => return error(message, refusal.code, &refusal.message),
        };
        self.limits = limits;
        self.fonts = fonts;
        self.ready = true;
        let operations: Vec<&str> = policy::POLICY.iter().filter(|e| e.runtimes != policy::Runtimes::BrowserOnly).map(|e| e.command).collect();
        let build = json!({
            "policySha256": services::sha256_hex(POLICY_SOURCE),
            "policyVersion": policy::RESULT_VERSION,
            "systemFontScan": filmcraft_engine::graphics::system_scan_enabled(),
            "hardwareDecoding": filmcraft_codecs::hw::hardware_decoding(),
            "decoder": "rust",
            "encoder": "builtin",
            "worker": env!("CARGO_PKG_VERSION"),
            "workerSha256": self.binary_sha256,
            "fork": ENGINE,
            "head": option_env!("FILMCRAFT_FORK_HEAD"),
            "diffSha256": option_env!("FILMCRAFT_FORK_DIFF_SHA256"),
            "fontsetDigest": self.fonts.digest,
        });
        json!({
            "type": "ready",
            "protocol": PROTOCOL,
            "engine": ENGINE,
            "mode": "session",
            "sessionId": message.get("sessionId"),
            "instanceId": message.get("instanceId"),
            "build": build,
            "operations": operations,
            "readKinds": reads::KINDS,
            "jobKinds": [jobs::EXPORT_MEDIA, jobs::INDEX],
            "limits": self.limits.report(),
            "fonts": self.fonts.listing(),
            "fontsetDigest": self.fonts.digest,
        })
    }

    /// `pong`: the `seq` of the `ping`, the project revision, how many jobs run and the process's
    /// resident memory where the worker can read it ([`rss_bytes`]).
    fn pong(&self, message: &Value) -> Value {
        let revision = self.session.revision.to_string();
        let queue = self.jobs.len() + self.indexing.len();
        json!({"type": "pong", "seq": message.get("seq"), "revision": revision, "queue": queue, "rssBytes": rss_bytes()})
    }

    fn stage(&self, message: &Value) -> Value {
        let text = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or_default();
        let stage_id = message.get("stageId").cloned().unwrap_or(Value::Null);
        match self.services.stage(text("handle"), text("path"), message.get("bytes").and_then(Value::as_u64)) {
            Ok(staged) => json!({"type": "staged", "stageId": stage_id, "ok": true, "path": staged.path, "bytes": staged.bytes}),
            Err(refusal) => json!({"type": "staged", "stageId": stage_id, "ok": false, "code": refusal.code, "message": refusal.message}),
        }
    }

    fn execute(&mut self, message: &Value) -> Value {
        let Some(request) = request_id(message) else { return error(message, "invalid_params", "`requestId` must be a non-empty string") };
        let operation = message.get("operation").and_then(Value::as_str).unwrap_or_default().to_string();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let expect = message.get("expect").cloned();
        let deadline = deadline_of(message);
        self.journaled(&request, &operation, true, |worker| worker.typed(&operation, params, expect.as_ref(), deadline))
    }

    /// `executeTyped` natively, in one call (T-NW-PRE, T-NW-POL-02): the preconditions and the table
    /// (`admit`), the keyframe, template, ripple and insert pre-checks, a scene detection or a mask
    /// tracking made to run to its end ([`WAITED`]), the fonts the call names ([`fonts::Fontset::check`]),
    /// the checks of the native-only entries (`reach::precheck`, slice RN) and the dispatch.
    fn typed(&mut self, command: &str, params: Value, expect: Option<&Value>, deadline_ms: f64) -> Value {
        let observed = observe(&self.session);
        let expect = expectation(expect, deadline_ms, &observed);
        if let Err(code) = policy::admit(command, &params, &expect, &observed, policy::Runtime::HERE) {
            return policy::refused(code, observed.revision);
        }
        let session = &self.session;
        if let Some(code) = policy::keyframe_refusal(command, &params, || sequence_view(session)) {
            return policy::refused(code, observed.revision);
        }
        if let Some(code) = policy::template_refusal(command, &params, || sequence_view(session)) {
            return policy::refused(code, observed.revision);
        }
        let in_the_way = policy::sync_locked_tracks(command, &params, observed.linked_selection, || sequence_view(session));
        if !in_the_way.is_empty() {
            return policy::sync_locked(observed.revision, in_the_way);
        }
        let params = match waited(command, params) {
            Ok(params) => params,
            Err(why) => return refused_why("invalid_params", observed.revision, why),
        };
        let params = match self.fonts.check(command, params) {
            Ok(params) => params,
            Err(why) => return refused_why("font_not_loaded", observed.revision, &why),
        };
        if let Err(why) = reach::precheck(&self.session, command, &params) {
            return refused_why("invalid_params", observed.revision, &why);
        }
        dispatch(&mut self.session, command, &params, observed.revision)
    }

    /// The worker's own import: a staged handle into the project, bound to the session as the
    /// caller read it. Its `result` also names, for each item, the warnings opening its file gave
    /// and what its source concealed so far (`media`), and the background indexes it started
    /// (`jobs`, each a job of the worker).
    fn import(&mut self, message: &Value) -> Value {
        let Some(request) = request_id(message) else { return error(message, "invalid_params", "`requestId` must be a non-empty string") };
        let handle = message.get("handle").and_then(Value::as_str).unwrap_or_default().to_string();
        let expect = message.get("expect").cloned();
        let deadline = deadline_of(message);
        let mut answer = self.journaled(&request, "import", true, |worker| worker.import_staged(&handle, expect.as_ref(), deadline));
        if answer["envelope"]["outcome"] == "applied" {
            let listed = answer["envelope"]["result"]["items"].as_array().cloned().unwrap_or_default();
            let items: Vec<u64> = listed.iter().filter_map(Value::as_u64).collect();
            answer["media"] = json!(items.iter().map(|&item| self.media_report(item)).collect::<Vec<_>>());
            let jobs = self.start_indexing(&request, &items);
            if !jobs.is_empty() {
                answer["jobs"] = json!(jobs);
            }
        }
        answer
    }

    /// What the source of `item` reports: the warnings opening its file gave (a file cut short, a
    /// stream the editor does not decode) and the samples it concealed so far.
    fn media_report(&self, item: u64) -> Value {
        let source = self.session.media.cached(ItemId(item));
        let warnings = source.as_ref().map(|source| source.warnings()).unwrap_or_default();
        let damage = source.as_ref().map(|source| source.damage()).unwrap_or_default();
        json!({"item": item, "warnings": warnings, "damage": damage})
    }

    fn import_staged(&mut self, handle: &str, expect: Option<&Value>, deadline_ms: f64) -> Value {
        let observed = observe(&self.session);
        let expect = expectation(expect, deadline_ms, &observed);
        if let Err(code) = preconditions(&expect, &observed) {
            return policy::refused(code, observed.revision);
        }
        let Some(path) = self.services.staged_path(handle) else {
            return refused_why("source_not_staged", observed.revision, "the handle names no staged file");
        };
        if !filmcraft_media::is_importable(Path::new(&path)) {
            return refused_why("format_unsupported", observed.revision, "the worker imports media files: video, sound and still pictures");
        }
        // a file the decoders refuse is refused before the import runs, so the refusal changes nothing
        if let Err(e) = filmcraft_engine::commands::import_probe(&self.session, &path) {
            let code = match e {
                EngineError::Media(MediaError::Io(_) | MediaError::Offline(_)) | EngineError::Other(_) => "staged_file_corrupt",
                _ => "format_unsupported",
            };
            return refused_why(code, observed.revision, &e.to_string());
        }
        dispatch(&mut self.session, "file.import", &json!({"paths": [path]}), observed.revision)
    }

    /// The worker's own open of a project file the host placed in `project/`. No precondition binds
    /// it (`guarantee: "unavailable"`); its result names the families the project's text names that
    /// no loaded face holds (`substituted`).
    fn open(&mut self, message: &Value) -> Value {
        let Some(request) = request_id(message) else { return error(message, "invalid_params", "`requestId` must be a non-empty string") };
        let path = message.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
        let sha = message.get("expectSha256").and_then(Value::as_str).unwrap_or_default().to_string();
        let mut answer = self.journaled(&request, "open", false, |worker| worker.open_project(&path, &sha));
        answer["guarantee"] = json!("unavailable");
        if answer["envelope"]["outcome"] == "applied" {
            answer["substituted"] = json!(fonts::substituted(&self.session.project));
        }
        answer
    }

    /// Open `project/<name>` when its bytes hash to `expect_sha256`. The media a project names
    /// resolve through the contained services only: a path outside the workspace stays offline,
    /// unread, and the result names it `media_unresolved` (T-NW-ESC). Folders a project names for
    /// generated files (previews, auto-saves, ingest copies) are dropped, so nothing is written
    /// outside the workspace either.
    fn open_project(&mut self, path: &str, expect_sha256: &str) -> Value {
        let revision = self.session.revision;
        let Some(name) = path.strip_prefix("project/").filter(|name| services::plain(name)) else {
            return refused_why("invalid_params", revision, "an open names a file of project/");
        };
        let bytes = match self.services.read_project(name) {
            Ok(bytes) => bytes,
            Err(e) => return refused_why("invalid_params", revision, &e.to_string()),
        };
        if services::sha256_hex(&bytes) != expect_sha256 {
            return refused_why("staged_file_corrupt", revision, "the project file does not hash to expectSha256");
        }
        let loaded = match filmcraft_format::decode(&bytes) {
            Ok(loaded) => loaded,
            Err(e) => return refused_why("invalid_params", revision, &e.to_string()),
        };
        let (schema, migrated) = (loaded.schema_version, loaded.migrated());
        let mut project = loaded.project;
        project.settings.scratch = Default::default();
        project.settings.ingest = Default::default();
        self.install(project, loaded.view);
        let missing = filmcraft_engine::relink::refresh(&mut self.session);
        let unresolved = missing.iter().map(|m| json!({"item": m.item, "path": m.path, "status": m.status, "code": "media_unresolved"}));
        let unresolved: Vec<Value> = unresolved.collect();
        let sequence = self.session.state.active_sequence.map(|id| id.0.to_string());
        let result = json!({"schemaVersion": schema, "migrated": migrated, "activeSequence": sequence, "unresolved": unresolved});
        policy::settle("file.open", &json!({}), revision, self.session.revision, None, policy::Answer::Ok(result))
    }

    /// Make `project` the session's, as the engine's own open does, with nothing outside the
    /// workspace: no preview folder beside a project path, no recent-projects entry, no path.
    fn install(&mut self, project: Project, view: Option<ProjectView>) {
        let session = &mut self.session;
        let proxies = session.media.use_proxies();
        session.media = Arc::new(MediaPool::default());
        session.media.set_use_proxies(proxies);
        session.offline = Default::default();
        let first = project.sequences().next().map(|item| item.id);
        session.project = Arc::new(project);
        session.history = History { limit: 200, ..Default::default() };
        session.state = self.fresh.clone();
        session.state.active_sequence = first;
        session.state.open_sequences = first.into_iter().collect();
        session.path = None;
        session.revision += 1;
        session.saved_revision = session.revision;
        session.loaded_schema = filmcraft_format::SCHEMA_VERSION;
        if let Some(view) = view.filter(|_| session.prefs.timeline.restore_open_sequences) {
            session.restore_project_view(view);
        }
    }

    /// Journal `operation` of `request` (a mutation) before it touches the engine, run it, and
    /// journal how it settled before the `result` goes back; an applied edit that moved the
    /// revision is first written as a checkpoint when `checkpoint` (the durable revision a lost
    /// worker's session forks from). A request the journal does not take runs nothing
    /// (`workspace_write_failed`).
    fn journaled(&mut self, request: &str, operation: &str, checkpoint: bool, run: impl FnOnce(&mut Worker) -> Value) -> Value {
        let before = self.session.revision;
        if let Err(e) = self.journal.intent(request, operation, journal::MUTATION) {
            return result(request, refused_why("workspace_write_failed", before, &format!("the journal did not take the request: {e}")));
        }
        let mut answer = result(request, run(self));
        if checkpoint && answer["envelope"]["outcome"] == "applied" && self.session.revision != before {
            match self.checkpoint() {
                Ok(file) => answer["checkpoint"] = file,
                Err(e) => answer["checkpointError"] = json!(e.to_string()),
            }
        }
        let envelope = &answer["envelope"];
        let text = |key: &str| envelope.get(key).and_then(Value::as_str);
        let settled = self.journal.settled(request, text("outcome").unwrap_or("unknown"), text("code"), text("revisionAfter").unwrap_or_default());
        if let Err(e) = settled {
            answer["journalError"] = json!(e.to_string());
        }
        answer
    }

    /// The project as it is now, written as `project/rev-<revision>-<sha256 prefix>.fcproj`; its
    /// file entry. The name holds the hash of the bytes, so a checkpoint of another worker on the
    /// same workspace (whose revisions count from 1 again) never takes its place.
    fn checkpoint(&self) -> std::io::Result<Value> {
        let bytes = filmcraft_format::encode_with_view(&self.session.project, Some(&self.session.project_view()), false);
        let sha = services::sha256_hex(&bytes);
        let name = format!("rev-{}-{}.fcproj", self.session.revision, sha.get(..12).unwrap_or_default());
        self.services.write_new(&[services::PROJECT], &name, &bytes)?;
        let path = format!("{}/{name}", services::PROJECT);
        Ok(json!({"path": path, "bytes": bytes.len(), "sha256": sha, "revision": self.session.revision.to_string()}))
    }

    /// Stop: cancel the running exports, wait for them to settle and answer their results, settle
    /// the indexes still running, then `bye`.
    fn shutdown(&mut self) -> Vec<Value> {
        self.stopped = true;
        self.cancel_jobs();
        let started = Instant::now();
        let mut messages = Vec::new();
        while !self.jobs.is_empty() && started.elapsed() < SHUTDOWN_WAIT {
            messages.extend(self.poll_jobs().into_iter().filter(|m| m["type"] == "jobResult"));
            if !self.jobs.is_empty() {
                std::thread::sleep(POLL_INTERVAL);
            }
        }
        messages.extend(self.drop_indexing());
        messages.push(json!({"type": "bye"}));
        messages
    }
}

impl Drop for Worker {
    /// The export threads of a dropped worker stop at their next batch.
    fn drop(&mut self) {
        self.cancel_jobs();
    }
}

/// An `error` message about `message`: what it was, its request or read when it names one, a
/// registered code and why.
pub(crate) fn error(message: &Value, code: &str, why: &str) -> Value {
    let about = message.get("type").cloned().unwrap_or(Value::Null);
    let request = message.get("requestId").or_else(|| message.get("readId")).cloned().unwrap_or(Value::Null);
    json!({"type": "error", "about": about, "requestId": request, "code": code, "message": why})
}

/// The line of `answer` to write: the answer itself when it fits in [`MAX_LINE_BYTES`] (INV-27
/// parity). Otherwise a `readResult` is refused `budget_exceeded` (the files it wrote stay); a
/// `result` keeps its envelope and every field but the engine's own result, which it says it left
/// out (`resultOmitted`; a query reads it again); and any other message is an `error`,
/// `budget_exceeded`.
pub fn line_of(answer: &Value) -> String {
    let line = answer.to_string();
    if line.len() <= MAX_LINE_BYTES {
        return line;
    }
    let bytes = line.len();
    let why = format!("the answer is {bytes} bytes, over the protocol's {MAX_LINE_BYTES} a line");
    let fitted = match answer.get("type").and_then(Value::as_str) {
        Some("readResult") => {
            json!({"type": "readResult", "readId": answer.get("readId"), "ok": false, "code": "budget_exceeded", "message": why, "rev": answer.get("rev")})
        }
        Some("result") => {
            let mut fitted = answer.clone();
            if let Some(envelope) = fitted.get_mut("envelope").and_then(Value::as_object_mut) {
                envelope.remove("result");
            }
            fitted["resultOmitted"] = json!({"bytes": bytes, "why": why});
            fitted
        }
        _ => error(answer, "budget_exceeded", &why),
    };
    let line = fitted.to_string();
    if line.len() <= MAX_LINE_BYTES { line } else { error(answer, "budget_exceeded", &why).to_string() }
}

/// The process's resident memory in bytes where the standard library can read it without a system
/// call of its own (`/proc/self/statm` on Linux); `None` elsewhere: on Windows the host reads the
/// worker's memory from the Job Object it runs in.
fn rss_bytes() -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    pages.checked_mul(4096)
}

/// The `result` of `request`: its envelope, every precondition checked in the same call.
fn result(request: &str, envelope: Value) -> Value {
    json!({"type": "result", "requestId": request, "envelope": envelope, "guarantee": "atomic"})
}

/// A refusal envelope decided before dispatch, with what the worker found in `detail`.
fn refused_why(code: &str, revision: u64, why: &str) -> Value {
    let mut envelope = policy::refused(code, revision);
    envelope["detail"] = json!(why);
    envelope
}

/// The request id a message names: a non-empty string of at most 256 bytes.
fn request_id(message: &Value) -> Option<String> {
    message.get("requestId").and_then(Value::as_str).filter(|id| !id.is_empty() && id.len() <= 256).map(str::to_string)
}

/// A message's deadline (milliseconds since the epoch); one that is missing or not a number never
/// holds.
fn deadline_of(message: &Value) -> f64 {
    message.get("deadlineMs").and_then(Value::as_f64).unwrap_or(f64::NAN)
}

/// `params` of `command` made to run to its end within the call: a scene detection or a mask
/// tracking ([`WAITED`]) without `wait` waits, and one with `wait: false` is refused.
fn waited(command: &str, mut params: Value) -> Result<Value, &'static str> {
    if !WAITED.contains(&command) {
        return Ok(params);
    }
    if params.get("wait") == Some(&Value::Bool(false)) {
        return Err("a native worker runs scene detection and mask tracking to their end within the call: `wait` is true or left out");
    }
    if let Some(keys) = params.as_object_mut() {
        keys.insert("wait".to_string(), Value::Bool(true));
    }
    Ok(params)
}

/// The digest of the active sequence's track targeting, as the hook computes it.
fn targeting_of(session: &Session) -> String {
    let targeting = session.targeting();
    let targeted: Vec<u64> = targeting.targeted.iter().map(|track| track.0).collect();
    policy::targeting_digest(&targeted, targeting.video_dest.map(|track| track.0), targeting.audio_dest.map(|track| track.0))
}

/// What the worker reads in the call that dispatches: no person shares a native session, so it is
/// busy only with its own trim playback.
fn observe(session: &Session) -> policy::Observed {
    let undo = &session.history.undo;
    let history_head = policy::HistoryHead { length: undo.len() as u64, label: undo.last().map(|(label, _)| label.clone()).unwrap_or_default() };
    policy::Observed {
        now_ms: now_ms(),
        revision: session.revision,
        active_sequence: session.state.active_sequence.map(|id| id.0),
        linked_selection: session.state.linked_selection,
        targeting: targeting_of(session),
        busy: session.trim_play.active(),
        history_head,
    }
}

/// `{length, label}`, the history head a caller read.
fn history_head_of(value: &Value) -> Option<policy::HistoryHead> {
    let head = value.as_object().filter(|head| head.len() == 2)?;
    Some(policy::HistoryHead { length: head.get("length")?.as_u64()?, label: head.get("label")?.as_str()?.to_string() })
}

/// The caller's side of the preconditions, from a message's `expect` and `deadlineMs`. As in the
/// hook, a precondition sent in a form the worker does not read can never hold: it stands as a
/// value the session cannot have, so the check refuses it with its own code, in its place in the
/// order.
fn expectation(expect: Option<&Value>, deadline_ms: f64, observed: &policy::Observed) -> policy::Expect {
    let field = |key: &str| expect.and_then(|expect| expect.get(key));
    let revision = field("revision").and_then(Value::as_str).and_then(|revision| revision.parse::<u64>().ok());
    let active_sequence = match field("activeSequence") {
        Some(Value::Null) => Some(None),
        Some(Value::String(id)) => id.parse::<u64>().ok().map(Some),
        _ => None,
    };
    let linked_selection = match field("linkedSelection") {
        None | Some(Value::Null) => Some(None),
        Some(Value::Bool(linked)) => Some(Some(*linked)),
        Some(_) => None,
    };
    let targeting = match field("targeting") {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(digest)) => Some(Some(digest.clone())),
        Some(_) => None,
    };
    let history_head = match field("historyHead") {
        None | Some(Value::Null) => Some(None),
        Some(head) => history_head_of(head).map(Some),
    };
    let other_head = policy::HistoryHead { length: observed.history_head.length.wrapping_add(1), label: String::new() };
    policy::Expect {
        revision: revision.unwrap_or(observed.revision.wrapping_add(1)),
        active_sequence: active_sequence.unwrap_or(if observed.active_sequence.is_some() { None } else { Some(0) }),
        linked_selection: linked_selection.unwrap_or(Some(!observed.linked_selection)),
        targeting: targeting.unwrap_or(Some(String::new())),
        deadline_ms,
        history_head: history_head.unwrap_or(Some(other_head)),
    }
}

/// The preconditions of the worker's own operations, in the order the hook's `admit` checks them
/// for the table's: the deadline, the revision, the active sequence, then linked selection, the
/// track targeting and the history head when the caller names them, and the engine's own business.
fn preconditions(expect: &policy::Expect, observed: &policy::Observed) -> Result<(), &'static str> {
    let in_time = observed.now_ms.is_finite() && expect.deadline_ms.is_finite() && observed.now_ms <= expect.deadline_ms;
    if !in_time {
        return Err("deadline_elapsed");
    }
    if expect.revision != observed.revision {
        return Err("revision_conflict");
    }
    if expect.active_sequence != observed.active_sequence {
        return Err("sequence_changed");
    }
    if expect.linked_selection.is_some_and(|linked| linked != observed.linked_selection) {
        return Err("linked_selection_changed");
    }
    if expect.targeting.as_ref().is_some_and(|targeting| *targeting != observed.targeting) {
        return Err("targeting_changed");
    }
    if observed.busy {
        return Err("editor_busy");
    }
    if expect.history_head.as_ref().is_some_and(|head| *head != observed.history_head) {
        return Err("history_head_conflict");
    }
    Ok(())
}

/// The active sequence as the engine serializes it: the view the keyframe, template, ripple and
/// insert pre-checks read (null when none is open).
fn sequence_view(session: &Session) -> Value {
    session.active_sequence().and_then(|sequence| serde_json::to_value(sequence).ok()).unwrap_or_default()
}

/// The marks a typed marks edit reports instead of the engine's null result: the active
/// sequence's In and Out marks after the edit, half-open.
fn marks_of(session: &Session, command: &str) -> Option<Value> {
    let sequence = session.active_sequence()?;
    policy::marks_result(command, sequence.mark_in.map(|tick| tick.0), sequence.mark_out.map(|tick| tick.0), session.sequence_rate().frame_duration().0)
}

/// Run `command` with `params` on the session, admitted already, and settle it with the hook's
/// own rules.
fn dispatch(session: &mut Session, command: &str, params: &Value, revision_before: u64) -> Value {
    let answer = match session.execute(command, params.clone()) {
        Ok(result) => policy::Answer::Ok(marks_of(session, command).unwrap_or(result)),
        Err(EngineError::Disabled(named, _)) => policy::Answer::Disabled(named),
        Err(error) => policy::Answer::Failed(error.to_string()),
    };
    let label = session.history.undo.last().map(|(label, _)| label.as_str());
    policy::settle(command, params, revision_before, session.revision, label, answer)
}

/// A line of the host, as the reader thread hands it over.
enum Line {
    Text(String),
    /// A line longer than [`MAX_LINE_BYTES`], skipped whole.
    TooLong,
    /// The end of input: the host closed stdin.
    End,
}

/// Skip the rest of an over-long line.
fn skip_line(input: &mut impl BufRead) {
    loop {
        let mut rest = Vec::new();
        match input.by_ref().take(MAX_LINE_BYTES as u64).read_until(b'\n', &mut rest) {
            Ok(n) if n > 0 && rest.last() != Some(&b'\n') => {}
            _ => return,
        }
    }
}

/// Read the host's lines from stdin and hand them over, each at most [`MAX_LINE_BYTES`] long (a
/// longer one is skipped whole and reported), until the input ends.
fn read_lines(sender: Sender<Line>) {
    let mut input = std::io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let line = match (&mut input).take(MAX_LINE_BYTES as u64 + 1).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => Line::End,
            Ok(_) if line.ends_with(b"\n") => {
                let text = line.strip_suffix(b"\r\n").or_else(|| line.strip_suffix(b"\n")).unwrap_or(line.as_slice());
                Line::Text(String::from_utf8_lossy(text).into_owned())
            }
            Ok(_) if line.len() > MAX_LINE_BYTES => {
                skip_line(&mut input);
                Line::TooLong
            }
            Ok(_) => Line::Text(String::from_utf8_lossy(&line).into_owned()),
        };
        let end = matches!(line, Line::End);
        if sender.send(line).is_err() || end {
            return;
        }
    }
}

/// The worker process: the workspace from `--workspace <folder>` (else the current folder), then one
/// answer per line of stdin, and the messages of its jobs as they come, until stdin closes or the
/// host sends `shutdown`. `ready` names the sha256 of the executable (`workerSha256`), which the
/// host checks against its manifest.
pub fn serve(args: &[OsString]) -> std::io::Result<()> {
    let root = workspace_arg(args)?;
    let mut worker = Worker::new(&root)?;
    worker.binary_sha256 = std::env::current_exe().and_then(|exe| services::file_digest(&exe)).ok().map(|(_, sha)| sha);
    let (sender, lines) = channel();
    let _reader = std::thread::Builder::new().name("ewp-stdin".to_string()).spawn(move || read_lines(sender))?;
    let mut stdout = std::io::stdout().lock();
    loop {
        let answers = match lines.recv_timeout(POLL_INTERVAL) {
            Ok(Line::Text(line)) => worker.handle_line(&line),
            Ok(Line::TooLong) => vec![line_of(&error(&Value::Null, "invalid_params", "a line is longer than the protocol allows"))],
            Ok(Line::End) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => Vec::new(),
        };
        for answer in answers {
            writeln!(stdout, "{answer}")?;
        }
        for message in worker.poll() {
            writeln!(stdout, "{}", line_of(&message))?;
        }
        stdout.flush()?;
        if worker.stopped() {
            return Ok(());
        }
    }
    for message in worker.shutdown() {
        writeln!(stdout, "{}", line_of(&message))?;
    }
    stdout.flush()
}

/// The folder after `--workspace`, else the current folder.
fn workspace_arg(args: &[OsString]) -> std::io::Result<PathBuf> {
    match args.iter().position(|arg| arg == "--workspace") {
        Some(i) => args.get(i + 1).map(PathBuf::from).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "--workspace needs a folder")),
        None => std::env::current_dir(),
    }
}
