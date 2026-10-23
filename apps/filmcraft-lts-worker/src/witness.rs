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
//! The witness reads the revision, the saved revision and the path; the project and the media pool by identity (a
//! commit replaces the project's `Arc`) and the snapshot playback shows (the session's own project); every undo and
//! redo step (its label and its snapshot) and the merge key; the command journal; the events the session announces;
//! the editor state, serialized, and the clipboard; trim playback; and the background work (jobs, stepped exports,
//! media, mask and scene jobs, background indexes and the export queue).

use std::sync::Arc;

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
    /// The undo and redo steps (labels and snapshots) and the merge key.
    history: History,
    /// The command journal's length and the command of its last entry.
    journal: (usize, Option<String>),
    events: Vec<Event>,
    state: EditorState,
    /// Whether trim playback ran.
    trim: bool,
    /// The background work: jobs, stepped exports, media, mask and scene jobs, background indexes, the export queue.
    background: [usize; 7],
}

impl Witness {
    /// The session as the engine is about to find it.
    pub(crate) fn of(session: &Session) -> Witness {
        Witness {
            revision: session.revision,
            saved_revision: session.saved_revision,
            path: session.path.clone(),
            project: Arc::clone(&session.project),
            media: Arc::clone(&session.media),
            history: session.history.clone(),
            journal: journal_of(session),
            events: session.events.clone(),
            state: session.state.clone(),
            trim: session.trim_play.active(),
            background: background_of(session),
        }
    }

    /// The envelope `settled` refined by the witness (see the module documentation): an `unknown` whose engine error
    /// (`message`) changed nothing in `session` is the engine's refusal, and one that changed something says so; every
    /// other envelope is as settled.
    pub(crate) fn refine(&self, session: &Session, mut settled: Value, message: Option<&str>) -> Value {
        let Some(message) = message.filter(|_| settled["outcome"] == "unknown") else { return settled };
        if !self.holds(session) {
            settled["witness"] = json!("changed");
            return settled;
        }
        let mut refusal = policy::refused(ENGINE_REFUSED, self.revision);
        refusal["detail"] = json!(message.chars().take(DETAIL_CHARS).collect::<String>());
        refusal["proof"] = json!("witness");
        refusal["witness"] = json!("unchanged");
        refusal
    }

    /// Whether `session` is as the witness saw it: every field equal, the project, the media pool and every undo and redo
    /// snapshot the same `Arc`, and playback showing the session's own project.
    fn holds(&self, session: &Session) -> bool {
        let shown = session.previews.live.project().is_none_or(|live| Arc::ptr_eq(&live, &session.project));
        session.revision == self.revision
            && session.saved_revision == self.saved_revision
            && session.path == self.path
            && Arc::ptr_eq(&session.project, &self.project)
            && Arc::ptr_eq(&session.media, &self.media)
            && shown
            && same_steps(&session.history.undo, &self.history.undo)
            && same_steps(&session.history.redo, &self.history.redo)
            && session.history.merge_key == self.history.merge_key
            && journal_of(session) == self.journal
            && session.events == self.events
            && same_state(&session.state, &self.state)
            && session.trim_play.active() == self.trim
            && background_of(session) == self.background
    }
}

/// Whether two undo (or redo) stacks hold the same steps: each label equal and each snapshot the same `Arc`.
fn same_steps(now: &[(String, Arc<Project>)], seen: &[(String, Arc<Project>)]) -> bool {
    now.len() == seen.len() && now.iter().zip(seen).all(|((label, snapshot), (was, then))| label == was && Arc::ptr_eq(snapshot, then))
}

/// Whether the editor state is as it was: its serialized form (the playheads, selections, targeting, edit points and
/// views) and the clipboard, which that form leaves out, by its size.
fn same_state(now: &EditorState, seen: &EditorState) -> bool {
    let clipboard = |state: &EditorState| (state.clipboard.len(), state.clipboard_markers.len());
    let serialized = |state: &EditorState| serde_json::to_value(state).ok();
    clipboard(now) == clipboard(seen) && serialized(now).is_some_and(|value| Some(value) == serialized(seen))
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
