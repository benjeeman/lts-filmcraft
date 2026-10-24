//! The witness of a dispatched edit (wave native-acts, owner decision OC-47 D11; R-NW-1, native-worker-architecture.md
//! §5.5 and Appendix B). The engine answers a command that failed with its error alone, whether the command committed
//! nothing (it refused from inside, before any step: `Session::edit` is copy-on-write, so a failing edit commits
//! nothing) or committed a step before it failed. The worker tells the two apart by what the call could change, read
//! just before the engine runs it and again after its error:
//!
//! - nothing changed: the engine refused the call. It settles `not_applied` / `engine_refused` at the revision it
//!   started from, with the engine's own message (`detail`: the line the engine's Events log took for it, the one
//!   change a refusal makes, by design, T-FI-01c) and its proof (`proof: "witness"`, `witness: "unchanged"`), and the
//!   session goes on;
//! - something changed: a real partial mutation. It settles `unknown` / `engine_error_after_dispatch` as before, saying
//!   what the witness saw (`witness: "changed"`), and the host fences the session and retires it.
//!
//! What the witness reads:
//!
//! - the revision, the saved revision and the path;
//! - the project and the media pool by identity (a commit replaces the project's `Arc`), the media pool's proxy switch,
//!   and the snapshot playback shows (the session's own project);
//! - every undo and redo step (its label and its snapshot) and the merge key;
//! - the command journal and the events the session announces;
//! - the editor state, serialized, and the clipboard and its markers, which that form leaves out, entry by entry;
//! - the preferences;
//! - trim playback;
//! - the background work: jobs, stepped exports, media, mask and scene jobs, background indexes and the export queue;
//! - the files the engine wrote through the worker's services, by their count. Every write lands whole or not at all
//!   (`filmcraft_format::atomic_write`: a temporary file, then a rename; the temporary file is removed when either
//!   fails), so a write that failed left nothing, and an error after a write that landed (none of the table's
//!   interchange exports has one: the write is their last step) is a change.
//!
//! What it does not read, and why no command the worker dispatches (an edit of the table, or its own import) changes it
//! and then fails:
//!
//! - the media pool's caches (decoded sources, proxies found): they hold what was read, never what the project is;
//! - a job's cancel flag, an atomic of its progress: `jobs.cancel` sets it as its last step, after which it cannot fail;
//! - the relink state: only the import refreshes it, as its last step, after which it cannot fail either;
//! - the effect and export presets, the keyboard shortcuts, the mixer, multi-camera and voice-over recorders and the
//!   Media Browser: no command the worker dispatches touches them;
//! - the Events log, which takes the failure's own line by design (the refusal's `detail`);
//! - a file written outside the services: the engine's media export streams to its path with `std::fs`, and the table
//!   leaves the media export to the worker's own job (`file.exportMedia` is the browser's only).
//!
//! A command the worker comes to dispatch later, one that changes any of these and can fail after it, needs the witness
//! to read it first.

use std::sync::Arc;

use filmcraft_engine::autosave::Preferences;
use filmcraft_engine::project::Project;
use filmcraft_engine::{EditorState, Event, History, MediaPool, Session};
use serde_json::{Value, json};

use crate::agent_view_policy as policy;

/// The code of an engine error after dispatch that changed nothing (registered: `not_applied`, no fence).
const ENGINE_REFUSED: &str = "engine_refused";

/// The longest engine text a refusal carries in `detail`, as an `unknown` envelope carries it.
const DETAIL_CHARS: usize = 300;

/// What a call could change in the session, as it was just before the engine ran it.
pub(crate) struct Witness {
    revision: u64,
    saved_revision: u64,
    path: Option<String>,
    project: Arc<Project>,
    media: Arc<MediaPool>,
    /// Whether the media pool reads proxies: the one switch of the pool a command could turn.
    proxies: bool,
    /// The undo and redo steps (labels and snapshots) and the merge key.
    history: History,
    /// The command journal's length and the command of its last entry.
    journal: (usize, Option<String>),
    events: Vec<Event>,
    state: EditorState,
    prefs: Preferences,
    /// Whether trim playback ran.
    trim: bool,
    /// The background work: jobs, stepped exports, media, mask and scene jobs, background indexes, the export queue.
    background: [usize; 7],
    /// How many files the worker's services had written.
    written: u64,
}

impl Witness {
    /// The session as the engine is about to find it, its services having written `written` files.
    pub(crate) fn of(session: &Session, written: u64) -> Witness {
        Witness {
            revision: session.revision,
            saved_revision: session.saved_revision,
            path: session.path.clone(),
            project: Arc::clone(&session.project),
            media: Arc::clone(&session.media),
            proxies: session.media.use_proxies(),
            history: session.history.clone(),
            journal: journal_of(session),
            events: session.events.clone(),
            state: session.state.clone(),
            prefs: session.prefs.clone(),
            trim: session.trim_play.active(),
            background: background_of(session),
            written,
        }
    }

    /// The envelope `settled` refined by the witness (see the module documentation): an `unknown` whose engine error
    /// (`message`) changed nothing in `session`, its services having written `written` files, is the engine's refusal,
    /// and one that changed something says so; every other envelope is as settled.
    pub(crate) fn refine(&self, session: &Session, written: u64, mut settled: Value, message: Option<&str>) -> Value {
        let Some(message) = message.filter(|_| settled["outcome"] == "unknown") else { return settled };
        if !self.holds(session, written) {
            settled["witness"] = json!("changed");
            return settled;
        }
        let mut refusal = policy::refused(ENGINE_REFUSED, self.revision);
        refusal["detail"] = json!(message.chars().take(DETAIL_CHARS).collect::<String>());
        refusal["proof"] = json!("witness");
        refusal["witness"] = json!("unchanged");
        refusal
    }

    /// Whether `session` is as the witness saw it, its services having written `written` files: every field equal, the
    /// project, the media pool and every undo and redo snapshot the same `Arc`, and playback showing the session's own
    /// project.
    fn holds(&self, session: &Session, written: u64) -> bool {
        let shown = session.previews.live.project().is_none_or(|live| Arc::ptr_eq(&live, &session.project));
        session.revision == self.revision
            && session.saved_revision == self.saved_revision
            && session.path == self.path
            && Arc::ptr_eq(&session.project, &self.project)
            && Arc::ptr_eq(&session.media, &self.media)
            && session.media.use_proxies() == self.proxies
            && shown
            && same_steps(&session.history.undo, &self.history.undo)
            && same_steps(&session.history.redo, &self.history.redo)
            && session.history.merge_key == self.history.merge_key
            && journal_of(session) == self.journal
            && session.events == self.events
            && same_state(&session.state, &self.state)
            && session.prefs == self.prefs
            && session.trim_play.active() == self.trim
            && background_of(session) == self.background
            && written == self.written
    }
}

/// Whether two undo (or redo) stacks hold the same steps: each label equal and each snapshot the same `Arc`.
fn same_steps(now: &[(String, Arc<Project>)], seen: &[(String, Arc<Project>)]) -> bool {
    now.len() == seen.len() && now.iter().zip(seen).all(|((label, snapshot), (was, then))| label == was && Arc::ptr_eq(snapshot, then))
}

/// Whether the editor state is as it was: its serialized form (the playheads, selections, targeting, edit points and
/// views), and the clipboard and its markers, which that form leaves out, entry by entry.
fn same_state(now: &EditorState, seen: &EditorState) -> bool {
    let serialized = |state: &EditorState| serde_json::to_value(state).ok();
    let clipboard = now.clipboard == seen.clipboard && now.clipboard_markers == seen.clipboard_markers;
    clipboard && serialized(now).is_some_and(|value| Some(value) == serialized(seen))
}

/// The command journal's length and the command of its last entry.
fn journal_of(session: &Session) -> (usize, Option<String>) {
    (session.journal.len(), session.journal.last().map(|(command, _)| command.clone()))
}

/// The background work in the session `s`: jobs, stepped exports, media, mask and scene jobs, background indexes and the
/// export queue.
fn background_of(s: &Session) -> [usize; 7] {
    [s.jobs.len(), s.stepped.len(), s.media_jobs.len(), s.mask_jobs.len(), s.scene_jobs.len(), s.background_imports.len(), s.export_queue.items.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_engine::Job;
    use filmcraft_engine::project::{ClipId, Label, Marker, MarkerId, MarkerKind};
    use filmcraft_engine::time::Tick;

    /// The message of the engine's refusal in these tests.
    const MESSAGE: &str = "nothing to do";

    /// What the hook's rules settle an engine error after dispatch as, at a session's first revision: `unknown`.
    fn failed(message: &str) -> Value {
        policy::settle("sequence.closeGap", &json!({}), 1, 1, None, policy::Answer::Failed(message.to_string()))
    }

    /// A sequence marker named `name`, as the clipboard holds one.
    fn marker(name: &str) -> Marker {
        let (start, duration, kind) = (Tick(0), Tick(0), MarkerKind::default());
        Marker { id: MarkerId(1), start, duration, name: name.to_string(), comment: String::new(), kind, color: Label::Green }
    }

    /// An engine error that changed nothing is the engine's refusal: `not_applied` / `engine_refused` at the revision the
    /// call started from, with the engine's message, at most 300 characters of it, and its proof.
    #[test]
    fn an_error_that_changed_nothing_is_the_engines_refusal() {
        let session = Session::default();
        let witness = Witness::of(&session, 3);
        let settled = witness.refine(&session, 3, failed(MESSAGE), Some(MESSAGE));
        assert_eq!((&settled["outcome"], &settled["code"]), (&json!("not_applied"), &json!(ENGINE_REFUSED)), "{settled}");
        assert_eq!(settled["revisionAfter"], json!(session.revision.to_string()), "{settled}");
        let proof = (&settled["detail"], &settled["proof"], &settled["witness"]);
        assert_eq!(proof, (&json!(MESSAGE), &json!("witness"), &json!("unchanged")), "{settled}");
        let long = "x".repeat(DETAIL_CHARS + 100);
        let settled = witness.refine(&session, 3, failed(&long), Some(&long));
        assert_eq!(settled["detail"].as_str().map(|detail| detail.chars().count()), Some(DETAIL_CHARS), "{settled}");
    }

    /// An envelope that is not `unknown`, or an `unknown` without an engine error, is as settled.
    #[test]
    fn only_an_engine_error_is_refined() {
        let session = Session::default();
        let witness = Witness::of(&session, 0);
        let applied = policy::settle("markers.add", &json!({}), 1, 2, Some("Add Marker"), policy::Answer::Ok(json!({"marker": 1})));
        assert_eq!(witness.refine(&session, 0, applied.clone(), None), applied);
        let refused = policy::refused("revision_conflict", 1);
        assert_eq!(witness.refine(&session, 0, refused.clone(), Some(MESSAGE)), refused);
        assert_eq!(witness.refine(&session, 0, failed(MESSAGE), None), failed(MESSAGE));
    }

    /// An engine error after `change` (`what`), made to a session after the witness read it (to the session, or to the
    /// count of the files its services wrote), is a partial mutation: `unknown` / `engine_error_after_dispatch` as
    /// settled, saying `witness: "changed"`.
    fn changed_by(what: &str, change: impl FnOnce(&mut Session, &mut u64)) {
        let mut session = Session::default();
        session.state.clipboard_markers = vec![marker("one")];
        let mut written = 0;
        let witness = Witness::of(&session, written);
        change(&mut session, &mut written);
        let settled = witness.refine(&session, written, failed(MESSAGE), Some(MESSAGE));
        assert_eq!((&settled["outcome"], &settled["code"]), (&json!("unknown"), &json!("engine_error_after_dispatch")), "{what}: {settled}");
        assert_eq!(settled["witness"], json!("changed"), "{what}: {settled}");
    }

    /// An engine error after any change the witness reads besides the revision is a partial mutation. One change at a
    /// time: the editor state alone (a selection), the clipboard's markers alone (as many as before, another name), the
    /// preferences, the media pool's proxy switch, a background job, and a file the services wrote.
    #[test]
    fn an_error_after_any_change_is_a_partial_mutation() {
        changed_by("the selection", |s, _| s.state.selection.push(ClipId(7)));
        changed_by("a clipboard marker, as many as before", |s, _| s.state.clipboard_markers.iter_mut().for_each(|kept| kept.name.push('!')));
        changed_by("the preferences", |s, _| {
            s.prefs.timeline.restore_open_sequences = !s.prefs.timeline.restore_open_sequences;
        });
        changed_by("the proxy switch", |s, _| s.media.set_use_proxies(!s.media.use_proxies()));
        changed_by("a job", |s, _| s.jobs.push(Job { id: 1, label: "Export".to_string(), progress: Default::default(), result: Default::default() }));
        changed_by("a file written", |_, written| {
            *written += 1;
        });
    }
}
