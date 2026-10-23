//! LTS agent-hook certification for the Video editor engine: slices F0, FF1 and R0 of the LTS editor
//! harness (design §6.3 and Appendix A.2; master plan v2 §3.1; invariants INV-4, INV-12, INV-13,
//! INV-15, INV-18, INV-20, INV-28, INV-32, INV-34, INV-38 and INV-41).
//!
//! The LTS adapter may report a refusal as `not_applied` only when every engine call before it was
//! a certified pure read. These characterization tests pin that purity down on the current engine
//! through its public API only (no engine source changes): the query ids the adapter reads through
//! `window.filmcraft.executeChecked`, the encoder behind `projectSnapshot()`, and the reads behind
//! the `agentView` hook (`Session::try_render_program_at` for frames, `scopes::scope_signal` for
//! scopes). They pass on the unmodified engine; a failure is a finding that switches the design to
//! the fallback stated on that test.
//!
//! Slice FF1 adds the `filmcraft.agentView` hook (`apps/filmcraft-web/src/agent_view.rs`). Its pure
//! half, `agent_view_policy.rs` beside it (the typed policy table, the precondition order of
//! `executeTyped`, the envelope builders and the verdict of the pending-media loop), depends only on
//! `serde_json` and is compiled here through `#[path]`, exactly as the web crate compiles it. The FF1
//! helpers replay what the hook runs in its UI-thread closures, as `execute_checked` does for
//! `executeChecked`, and the FF1 tests pin the engine behaviour the hook's typed outcomes rely on.
//!
//! Slice R0 pre-declares the newsroom families in that table before the one WASM build of the hook
//! (master plan v2 §3.1): tier A and tier B entries, each with exact keys its command documents, the
//! keys a call must give (a key the engine fails without, or a target or a time it would otherwise
//! take from the person's selection or playhead) and what some calls need beyond them (one of
//! several keys, keys that come together, a key that one value of another needs), the names of every
//! closed set of values limited, a rule for the keys a call may leave out that name a time, a target,
//! a font or a switch the engine would read as left out when given a value of another type, every
//! time within the engine's own `Tick::MAX` of zero and every number of frames, speed, count and
//! duration within a bound, so that none overflows the engine's arithmetic, checks inside a
//! transcript, a graphic layer's properties and a template's values, the effect parameters a command
//! reaches as pairs, and each entry marked for both runtimes, for a native worker only (the browser
//! refuses those with `native_only`) or, for the media export into the browser's file table, for the
//! browser only (a native worker refuses it with `use_worker_export`); the legacy entries require the
//! keys the engine fails without, limit their names and type the keys they may leave out too; the
//! caller's linked-selection setting and track targeting as preconditions (`linked_selection_changed`
//! and `targeting_changed`, right after `sequence_changed`; a lift, an extract and a transcript
//! selection, extract or lift need the targeting) and a wider busy (dialogs and windows, drafts on the
//! monitor, the person's recording passes); and pure pre-checks before dispatch over the engine's
//! serde view of the active sequence: a keyframe added where one sits (`keyframe_exists`: the
//! engine's add is a toggle that would take it away) or edited where none sits
//! (`keyframe_not_found`), a template checkbox given anything but `true` or `false`
//! (`control_kind_mismatch`: where it shows a layer, the engine would read it as `true`), and a
//! ripple trim, ripple delete or closed gap with a sync-locked track in its way (`sync_locked`), where
//! FF1 left the engine's own refusal `unknown`. A native worker compiles the same file, so both
//! runtimes share the table.
//!
//! Slice R1 (master plan v3 W3) closes the fork's entry points and widens the table. `executeChecked` refuses every
//! call outside its typed tables with `command_not_allowed` before anything runs: its tables are the certified reads
//! (each with the keys it was certified with) and the legacy edits the lab's mirror adapter sends there (a place only as
//! an overwrite), so no edit whose preconditions or pre-checks only `executeTyped` runs is reached through it (D13,
//! backlog item 16); its own preconditions are refused with their registered codes too. In the LTS presentation
//! `filmcraft.request` and `filmcraft.execute` send reads only. Widen-1 types a transition's settings over the engine's
//! catalogue of video transitions (placed where the engine places them: centred on a cut, or at a clip's edge), a
//! layer's place by index, an insert (refused `sync_locked` when it would cut a clip on a sync-locked track it does not
//! name), the sequence marks (reported half-open) and a track's lock, output and mute; a scene detection that only
//! looks (OC-33 E3) is the table's one read. Every code the entry points answer or reject with is registered in the
//! closed code registry (RD-6).
//!
//! | Id | Test | Hook |
//! |---|---|---|
//! | T-FI-01b | `certified_queries_leave_session_state_unchanged` | `executeChecked` with the six certified query ids |
//! | T-FI-01c | `failing_query_appends_only_a_log_line` | the declared residual of a failing certified query |
//! | T-FI-01d | `newsroom_queries_leave_session_state_unchanged` | `executeChecked` with the four newsroom query ids (R0) |
//! | T-FI-01e | `edit_state_and_graphics_reads_leave_session_state_unchanged` | `executeChecked` with the two reads of R1 |
//! | T-FI-02 | `project_snapshot_encode_is_pure` | `projectSnapshot()` |
//! | T-FI-05 | `render_program_at_leaves_state_unchanged` | `agentView.renderFrame` |
//! | T-FI-07 | `render_program_at_reports_sequence_resolution_and_snaps_time` | `agentView.renderFrame` |
//! | T-FI-08 | `render_scale_bounds_refused_without_allocation` | `agentView.renderFrame` |
//! | T-FI-09 | `scope_signal_is_pure` | `agentView.scopes` |
//! | T-FI-10a | `disabled_command_errors_before_running` | `agentView.executeTyped` refusals |
//! | T-FI-10b | `trim_reports_the_applied_delta` | `agentView.executeTyped` trims, ripple deletes, closed gaps and the ripple pre-check |
//! | T-FI-11 | `render_and_scopes_report_incomplete_when_media_is_pending` | `agentView.renderFrame`, `agentView.scopes` |
//! | T-FI-12 | `typed_policy_rejects_raw_ids_unknown_keys_and_tier3` | the `executeTyped` policy table (R1: widen-1, the read tier) |
//! | T-FI-13 | `linked_selection_changed_refused_before_dispatch` | the `executeTyped` linked-selection precondition |
//! | T-FI-14 | `existing_keyframe_refused_before_dispatch` | the `executeTyped` keyframe pre-check (CC-25) |
//! | T-FI-15 | `targeting_changed_refused_before_dispatch` | the `executeTyped` track-targeting precondition |
//! | T-FI-16 | `checkbox_value_refused_before_dispatch` | the `executeTyped` template pre-check |
//! | T-FI-17 | `out_of_table_id_refused_at_fork` | `executeChecked`: calls outside its typed tables, its own preconditions (R1, D13) |
//! | T-FI-18 | `widened_transitions_take_settings_where_the_engine_aligns_them` | the `executeTyped` transitions (widen-1) |
//! | T-FI-19 | `insert_that_would_split_a_sync_locked_track_refused_before_dispatch` | the `executeTyped` insert pre-check (widen-1) |
//! | T-FI-20 | `sequence_marks_reported_half_open` | the `executeTyped` sequence marks (widen-1, m22) |
//! | T-FI-21 | `track_mute_enable_and_lock_typed` | the `executeTyped` track state (widen-1) |
//! | T-FI-22 | `arrange_layer_takes_an_index` | the `executeTyped` layer order (widen-1) |
//! | T-FI-23 | `look_only_scene_detection_reads_and_changes_nothing` | the `executeTyped` read tier (OC-33 E3) |
//! | T-FI-24 | `hook_codes_are_registered` | the codes the entry points answer or reject with (RD-6) |
//! | T-FI-25 | `raw_members_send_only_reads_in_the_lts_presentation` | `filmcraft.request`, `filmcraft.execute` in the LTS presentation (R1, D13) |
//!
//! Public-API limit: `DynamicTrim` and `PlayAround` keep their base project and clocks private, so
//! trim playback is compared through its public fields plus the identity of the session's project.
//!
//! No browser, media file, network, model call or saved project is involved: the demo project
//! renders synthetic generator footage, and T-FI-11's still is a PNG made in memory and served by an
//! in-memory host.
//!
//! The contract of `agent_view_policy.rs` these tests compile against (plan FF1 and master plan v2
//! §3.1, design Appendix A.2); `agent_view.rs` uses every item of it:
//!
//! ```text
//! RESULT_VERSION                  "filmcraft.typed-execute.v1"
//! EXPORT_ROOT                     "/lts-exports/": the one folder a typed export writes in
//! MAX_FRAME_SIDE, MAX_COORDINATE  the longest frame side the engine takes (32768 px), and twice it: the furthest a
//!                                 point of mask or shape geometry may lie from zero
//! MAX_SECONDS                     the longest duration or pause a call may give (86400 s)
//! MAX_TICKS, TICKS_PER_SECOND     the furthest a time may lie from zero (the engine's Tick::MAX), and ticks in a second
//! MAX_WORD_TICKS, MAX_SPEAKER     the longest word of a transcript (600 s) and its highest speaker index (999)
//! MAX_FRAMES, MIN_SPEED, MAX_SPEED   the most frames of a transition or a tracking pass (100000), and a clip's
//!                                 slowest and fastest speed in percent (0.01 and 100000)
//! Runtimes::{Both, NativeOnly, BrowserOnly}   where an entry may run
//! Runtime::{Browser, Native}      where executeTyped runs; Runtime::HERE is Browser on wasm32, else Native
//! Rule::{OneOf { both, native }, ExportPath, Ticks, Within(min, max), Between(min, max), Seconds, Id, Ids, IdOrName,
//!        Bool, True, Text, Name, Object(fields), Props, Values, Transcript, Given, Point, Points, Moves}
//!                                 what a limited value must be
//! Need::{AnyOf(groups), Together(keys), When { key, holds, then }}   what a call must give beyond its required keys
//! Kind::{Number, Point}, Target = (effect, [(param, Kind)])   an effect and the parameters of it an entry reaches
//! POLICY: [Entry]                 Entry { command, keys, required, needs, history_head, targeting, runtimes, rules,
//!                                 targets }: the reviewed commands, the parameter keys each accepts, the keys a call
//!                                 must give and what it needs beyond them, whether it needs the caller's history head
//!                                 and track targeting, where it may run, the values some of its keys are limited to
//!                                 and the effect parameters it reaches
//! allowed(command, &params, Runtime) -> Result<&Entry, "command_not_allowed" | "native_only" | "use_worker_export">
//! targeting_digest(&targeted, video, audio) -> "fnv1a64:" and 16 hex digits: a track targeting, in any order
//! HistoryHead { length, label }   the undo stack as `history.list` reports it (label "" when it is empty)
//! Expect { revision, active_sequence, linked_selection, targeting, deadline_ms, history_head }  the caller's side
//! Observed { now_ms, revision, active_sequence, linked_selection, targeting, busy, history_head }  read in the closure
//! admit(command, &params, &Expect, &Observed, Runtime) -> Result<&Entry, code>, the first that fails of
//!     deadline_elapsed, revision_conflict, sequence_changed, linked_selection_changed, targeting_changed,
//!     editor_busy, history_head_conflict, then the table's command_not_allowed, native_only or use_worker_export (a
//!     missing history head where the entry needs one is history_head_conflict, and a missing targeting where it
//!     needs one targeting_changed; otherwise a linked selection or a targeting the caller leaves out is not checked)
//! keyframe_refusal(command, &params, || view) -> Option<"keyframe_exists" | "keyframe_not_found">, from
//!     the engine's serde view of the active sequence (None: no refusal)
//! template_refusal(command, &params, || view) -> Option<"control_kind_mismatch">: a template checkbox given
//!     anything but true or false, from the same view (None: no refusal)
//! sync_locked_tracks(command, &params, linked_selection, || view) -> the sync-locked tracks in the way of
//!     a ripple trim, a ripple delete or a closed gap, from the same view (empty: none)
//! sync_locked(revision, tracks) -> envelope              a ripple refused before dispatch
//! refused(code, revision) -> envelope                    a refusal decided before dispatch
//! Answer::{Ok(result), Disabled(command the engine named), Failed(message)}   what `Session::execute` answered
//! settle(command, &params, revision_before, revision_after, history_label, Answer) -> envelope
//! attempt(pending, now_ms, deadline_ms) -> Option<complete>   one verdict of the renderFrame/scopes loop
//! FrameMeta { width, height, tick, frame, revision, active_sequence }
//! frame_report(&FrameMeta, complete, attempts) -> the renderFrame fields besides the pixels
//!
//! Slice R1 adds:
//! READS: [Entry]                  the certified reads executeChecked serves: queries, each with the keys it was certified with
//! CHECKED_EDITS: [id]             the legacy edits executeChecked runs besides them, the ones the lab's mirror adapter sends
//! checked(command, &params, Runtime) -> Result<(), "command_not_allowed">, what executeChecked admits: a read of READS,
//!     or an edit of CHECKED_EDITS that allowed() lets run there, a place only as an overwrite
//! Refusal = (code, message)       a refusal of executeChecked decided before dispatch
//! CheckedExpect { revision, active_sequence, deadline_ms }, CheckedExpect::from_call(revision, &context) -> Result<
//!     CheckedExpect, Refusal>: executeChecked's preconditions, or deadline_elapsed, revision_conflict or sequence_changed
//!     for one in a form it does not read
//! checked_guard(&CheckedExpect, now_ms, revision, active_sequence) -> Result<(), Refusal>: the same three, just before
//!     executeChecked dispatches
//! checked_failure(command, disabled, revision_before, revision_after) -> Option<"command_disabled">, once the engine failed
//! CONTROL_READS: [method], raw_request(method, &params) -> Result<(), "command_not_allowed">: what filmcraft.request and
//!     filmcraft.execute send in the LTS presentation (reads only)
//! Rule::IndexOr(names)            a position: one of these names, or an index from 0 within u32
//! Need::Without { key, holds, others }   none of `others` when `key` holds a value of the rule `holds`
//! Kind::{Color, Switch, Choice(options)}   the kinds of a transition's settings
//! Entry.settings: [Target]        the transitions a call may name with `effect`, each with the settings `params` may give
//! Entry.reads_with: Option<key>   the switch with which a call only reads (tier 0)
//! reads_only(command, &params) -> bool   whether a call only reads
//! settle(..) -> envelope          as before, except that a read's envelope names no history label (the newest undo
//!                                 label is then an earlier edit's)
//! marks_result(command, mark_in, mark_out, frame) -> Option<{"marks": {in, outInclusive, endExclusive}}>, what a typed
//!     marks edit reports, half-open (None: the engine's result stands)
//! NOT_APPLIED, APPLIED, UNKNOWN, READ_FAILURES   the codes the hook answers with, each registered
//! ```

use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use filmcraft_engine::panels::{Level, LogEntry};
use filmcraft_engine::scopes::scope_signal;
use filmcraft_engine::{EngineError, Event, Services, Session, command_specs, find_command};
use filmcraft_export::Format;
use filmcraft_media::SharedReader;
use filmcraft_media::reader::ByteReader;
use filmcraft_project::vtransition::VIDEO_TRANSITION_FOLDERS;
use filmcraft_project::{AudioType, ClipId, EffectKind, ItemId, ParamKind, ParamValue, Project, Transition, TransitionAlign, effect_defs, find_effect};
use filmcraft_scopes::Signal;
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use serde_json::{Map, Value, json};

/// The pure half of `filmcraft.agentView` (slices FF1 and R0), compiled natively as the web crate
/// compiles it.
#[path = "../../../apps/filmcraft-web/src/agent_view_policy.rs"]
mod agent_view_policy;

use crate::agent_view_policy as policy;

/// An item id the demo project does not have.
const UNKNOWN_ITEM: u64 = 999_999_999;

// ------------------------------------------------------------------------------------ fixtures

/// The demo project as `file.openDemoProject` opens it on a fresh `Session::default()`.
fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).expect("the demo project opens");
    s
}

/// The first clip on V1 of the active sequence and its media item.
fn first_v1_clip(s: &Session) -> (ClipId, ItemId) {
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let clip = seq.video_tracks.first().and_then(|track| track.items.first()).expect("the demo has a clip on V1");
    (clip.id, clip.item)
}

/// The demo while a dynamic (J/K/L) roll of the first cut runs: trim playback that the next
/// journaled command commits as one undo step. Set up as the engine's own trim tests do.
fn demo_with_dynamic_trim() -> Session {
    let mut s = demo();
    for track in ["V2", "V3", "A2", "A3"] {
        let _ = s.execute("timeline.setTrack", json!({"track": track, "syncLock": false}));
    }
    let (clip, _) = first_v1_clip(&s);
    s.execute("trim.selectEditPoint", json!({"clip": clip.0, "edge": "out", "kind": "roll"})).expect("the first cut is selected");
    s.execute("trim.shuttle", json!({"direction": "forward", "clock": 10.0})).expect("dynamic trimming starts");
    s.execute("trim.tick", json!({"clock": 10.5})).expect("dynamic trimming moves the cut");
    assert!(s.trim_play.dynamic.as_ref().is_some_and(|d| d.offset > Tick::ZERO), "the fixture holds a live dynamic trim");
    s
}

/// The demo while Trim Monitor loop playback runs around the first cut.
fn demo_with_play_around() -> Session {
    let mut s = demo();
    let (clip, _) = first_v1_clip(&s);
    s.execute("trim.selectEditPoint", json!({"clip": clip.0, "edge": "out", "kind": "roll"})).expect("the first cut is selected");
    s.execute("trim.playAround", json!({"clock": 100.0})).expect("loop playback starts");
    assert!(s.trim_play.around.as_ref().is_some_and(|a| a.looping), "the fixture loops around the cut");
    s
}

/// The demo while the person records, the pass started as the engine starts it (`command`): a
/// voice-over take (from the synthetic input a headless session has), an automation pass of the
/// Audio Track Mixer that writes (A1 is put in Write mode first, so the pass records its volume, pan
/// and mute from the start), or a multi-camera switching pass.
fn demo_recording(command: &str) -> Session {
    let mut s = demo();
    if command == "mixer.recordStart" {
        s.execute("mixer.setStrip", json!({"strip": "A1", "mode": "Write"})).expect("A1 writes automation");
    }
    s.execute(command, json!({})).unwrap_or_else(|e| panic!("`{command}` starts a recording pass: {e}"));
    assert!(engine_busy(&s), "`{command}` records");
    s
}

/// The sessions every certified read runs on: idle, and with trim playback that a journaled
/// command would settle first (`Session::execute` settles it for journaled specs only).
fn fixtures() -> [(&'static str, Session); 3] {
    [("idle demo", demo()), ("live dynamic trim", demo_with_dynamic_trim()), ("play-around loop", demo_with_play_around())]
}

// ------------------------------------------------------------------------------------ session state

/// The session's project compared by identity: a pure read never replaces it.
#[derive(Clone)]
struct SameProject(Arc<Project>);

impl PartialEq for SameProject {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for SameProject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Project@{:p}", Arc::as_ptr(&self.0))
    }
}

/// Everything a certified read must leave as it was.
#[derive(Clone, Debug, PartialEq)]
struct Fingerprint {
    revision: u64,
    saved_revision: u64,
    path: Option<String>,
    project: SameProject,
    /// The snapshot `Session::execute` republishes to playback after every command.
    playback_project: Option<SameProject>,
    undo: Vec<String>,
    redo: Vec<String>,
    merge_key: Option<String>,
    journal_len: usize,
    journal_last: Option<(String, Value)>,
    events: Vec<Event>,
    log: Vec<LogEntry>,
    active_sequence: Option<ItemId>,
    playhead: Tick,
    selection: Vec<ClipId>,
    /// The whole serialized editor state: playheads, selections, edit points, targeting, views.
    state: Value,
    /// A dynamic trim's offset, speed and limit flag.
    trim_dynamic: Option<(Tick, f64, bool)>,
    /// A play-around's range and loop flag.
    trim_around: Option<(Tick, Tick, bool)>,
    /// Jobs, stepped exports, media jobs, mask tracking and scene detection in progress.
    background: [usize; 5],
}

fn fingerprint(s: &Session) -> Fingerprint {
    Fingerprint {
        revision: s.revision,
        saved_revision: s.saved_revision,
        path: s.path.clone(),
        project: SameProject(s.project.clone()),
        playback_project: s.previews.live.project().map(SameProject),
        undo: s.history.undo.iter().map(|(label, _)| label.clone()).collect(),
        redo: s.history.redo.iter().map(|(label, _)| label.clone()).collect(),
        merge_key: s.history.merge_key.clone(),
        journal_len: s.journal.len(),
        journal_last: s.journal.last().cloned(),
        events: s.events.clone(),
        log: s.log.entries.iter().cloned().collect(),
        active_sequence: s.state.active_sequence,
        playhead: s.playhead(),
        selection: s.state.selection.clone(),
        state: serde_json::to_value(&s.state).expect("the editor state serializes"),
        trim_dynamic: s.trim_play.dynamic.as_ref().map(|d| (d.offset, d.speed, d.at_limit)),
        trim_around: s.trim_play.around.as_ref().map(|a| (a.start, a.end, a.looping)),
        background: [s.jobs.len(), s.stepped.len(), s.media_jobs.len(), s.mask_jobs.len(), s.scene_jobs.len()],
    }
}

/// The fingerprint without the Events panel log, the one thing a failing query may change.
fn without_log(f: Fingerprint) -> Fingerprint {
    Fingerprint { log: Vec::new(), ..f }
}

// ------------------------------------------------------------------------------------ hook paths

/// A rejection of `window.filmcraft.executeChecked`: the registered `not_applied` code its `Error` carries when it was
/// decided before anything ran, none for a failure after dispatch, and its message.
#[derive(Debug, PartialEq)]
struct Rejection {
    code: Option<&'static str>,
    message: String,
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (code {:?})", self.message, self.code)
    }
}

/// What `window.filmcraft.executeChecked(revision, command, params, {activeSequence, deadlineMs})` runs when the mirror
/// adapter calls it ([`execute_checked_at`]): the revision and the active sequence as decimal strings, the deadline
/// ahead of the clock.
fn execute_checked(s: &mut Session, revision: u64, active_sequence: Option<u64>, command: &str, params: Value) -> Result<Value, Rejection> {
    let context = json!({"activeSequence": active_sequence.map(|id| id.to_string()), "deadlineMs": DEADLINE_MS});
    execute_checked_at(s, &revision.to_string(), command, params, &context, NOW_MS)
}

/// What `window.filmcraft.executeChecked(revision, command, params, context)` runs (`apps/filmcraft-web/src/api.rs`),
/// its arguments as JSON and the clock passed in. Since slice R1: its typed tables first ([`policy::checked`], in the
/// browser), then the form of its preconditions ([`policy::CheckedExpect::from_call`]), both before its UI-thread
/// closure; in the closure their guard ([`policy::checked_guard`]), then `Session::execute`, replying
/// `{result, revision}`; a failure is `command_disabled` when the engine refused the command itself
/// ([`policy::checked_failure`]). A refusal carries its code, and nothing has run. Every decision is the policy
/// file's, and `api.rs` asks for them in this order (T-FI-25).
fn execute_checked_at(s: &mut Session, revision: &str, command: &str, params: Value, context: &Value, now_ms: f64) -> Result<Value, Rejection> {
    let refused = |(code, message): policy::Refusal| Rejection { code: Some(code), message };
    let outside = |code: &'static str| {
        let message = format!("{code}: executeChecked does not run `{command}` with these parameters");
        Rejection { code: Some(code), message }
    };
    policy::checked(command, &params, policy::Runtime::Browser).map_err(outside)?;
    let expect = policy::CheckedExpect::from_call(revision, context).map_err(refused)?;
    policy::checked_guard(&expect, now_ms, s.revision, s.state.active_sequence.map(|id| id.0)).map_err(refused)?;
    let before = s.revision;
    match s.execute(command, params) {
        Ok(result) => Ok(json!({"result": result, "revision": s.revision.to_string()})),
        Err(error) => {
            let disabled = match &error {
                EngineError::Disabled(named, _) => Some(named.as_str()),
                _ => None,
            };
            Err(Rejection { code: policy::checked_failure(command, disabled, before, s.revision), message: error.to_string() })
        }
    }
}

/// The query ids the LTS adapter reads through `executeChecked` (design §6.3), with the parameters
/// it sends: the active sequence and the media item of the first V1 clip.
fn certified_queries(s: &Session) -> [(&'static str, Value); 6] {
    let sequence = s.state.active_sequence.expect("the demo opens its main sequence");
    let (_, media) = first_v1_clip(s);
    [
        ("project.inspect", json!({})),
        ("sequence.inspect", json!({"item": sequence.0})),
        ("fonts.list", json!({"system": false})),
        ("history.list", json!({})),
        ("metadata.get", json!({"item": media.0})),
        ("media.colorInfo", json!({"item": media.0})),
    ]
}

/// Bitwise equality of two frames (NaN-safe).
fn same_frame(a: &filmcraft_render::Image, b: &filmcraft_render::Image) -> bool {
    a.w == b.w && a.h == b.h && a.px.iter().map(|v| v.to_bits()).eq(b.px.iter().map(|v| v.to_bits()))
}

// ------------------------------------------------------------------------------------ certification

/// T-FI-01b. Each certified query, run the way `executeChecked` runs it, leaves the session as it
/// was (revision, saved revision, journal, selection, playhead, active sequence, log, trim playback,
/// history, events, the project and the whole editor state), on an idle session and while trim
/// playback runs; each spec is a non-journaled, always-enabled query.
///
/// Plan fallback if this fails: Film pre-checks after queries stay `unknown`; L1-A-film keeps only
/// pure-JS refusals.
#[test]
fn certified_queries_leave_session_state_unchanged() {
    for (fixture, mut s) in fixtures() {
        for (id, params) in certified_queries(&s) {
            let spec = find_command(id).unwrap_or_else(|| panic!("`{id}` is a registered command"));
            assert!(!spec.journal, "`{id}` is a query: never journaled, so `Session::execute` never settles trim playback for it");
            assert!((spec.enabled)(&s).is_ok(), "`{id}` is enabled on the {fixture} fixture");
            let before = fingerprint(&s);
            let (revision, sequence) = (s.revision, s.state.active_sequence.map(|seq| seq.0));
            let reply = execute_checked(&mut s, revision, sequence, id, params.clone()).unwrap_or_else(|e| panic!("`{id}` failed ({fixture}): {e}"));
            assert!(!reply["result"].is_null(), "`{id}` answers: {reply}");
            assert_eq!(reply["revision"], json!(revision.to_string()), "`{id}` reports the unchanged revision");
            assert_eq!(fingerprint(&s), before, "`{id}` {params} changed session state on the {fixture} fixture");
        }
    }
}

/// T-FI-01c, the declared residual. A certified query that fails (an unknown item) appends exactly
/// one Events panel line (level error, source the query id, text the error) and changes nothing
/// else; the same failure again folds into that line (`count` 2) instead of adding one. The
/// rejection carries no `code`: the query failed after dispatch (slice R1).
///
/// Fallback if this fails (derived from design §2.6): a certified read that fails is reported as
/// `unknown` rather than `precheck_read_failed`, and the adapter checks existence through
/// `project.inspect` before item queries.
#[test]
fn failing_query_appends_only_a_log_line() {
    for (fixture, mut s) in fixtures() {
        assert!(s.project.item(ItemId(UNKNOWN_ITEM)).is_none(), "{UNKNOWN_ITEM} is not an item of the demo");
        for id in ["sequence.inspect", "metadata.get", "media.colorInfo"] {
            let params = json!({"item": UNKNOWN_ITEM});
            let before = fingerprint(&s);
            let (revision, sequence) = (s.revision, s.state.active_sequence.map(|seq| seq.0));
            let error = execute_checked(&mut s, revision, sequence, id, params.clone()).expect_err("an unknown item fails the query");
            assert_eq!(error.code, None, "a failure after dispatch carries no not_applied code: {error}");
            let after = fingerprint(&s);
            assert_eq!(after.log.len(), before.log.len() + 1, "`{id}` {params} appends one log line on the {fixture} fixture");
            assert_eq!(&after.log[..before.log.len()], &before.log[..], "earlier log lines stay as they were");
            let line = after.log.last().expect("the appended line");
            assert_eq!((line.level, line.source.as_str(), line.message.as_str(), line.count), (Level::Error, id, error.message.as_str(), 1));
            assert_eq!(without_log(after.clone()), without_log(before.clone()), "`{id}` {params} changed more than the log on the {fixture} fixture");
            let again = execute_checked(&mut s, revision, sequence, id, params).expect_err("the same failure");
            assert_eq!(again, error);
            let folded = fingerprint(&s);
            assert_eq!(folded.log.len(), after.log.len(), "a repeated failure adds no line");
            assert_eq!(folded.log.last().map(|l| l.count), Some(2), "a repeated failure bumps the newest line's count");
            assert_eq!(without_log(folded), without_log(before), "a repeated `{id}` failure changed more than the log on the {fixture} fixture");
        }
    }
}

/// The demo with a two-word transcript on the media of the first V1 clip and a caption track that
/// holds one caption: the newsroom reads then have something to answer.
fn demo_with_transcript_and_captions() -> Session {
    let mut s = demo();
    let (_, media) = first_v1_clip(&s);
    let second = TICKS_PER_SECOND;
    let words = json!([
        {"text": "Hello", "start": 2 * second, "end": 2 * second + second / 2, "speaker": 0},
        {"text": "world", "start": 3 * second, "end": 3 * second + second / 2, "speaker": 0},
    ]);
    s.execute("transcript.set", json!({"item": media.0, "transcript": {"language": "en", "words": words}})).expect("the transcript imports");
    s.execute("captions.newTrack", json!({"format": "Subtitle"})).expect("a caption track opens");
    s.execute("captions.add", json!({"text": "Hello world", "time": 0})).expect("a caption goes on it");
    s
}

/// [`demo_with_transcript_and_captions`] with a title at its start (a graphic clip with one text
/// layer, answered as well) and an elliptical mask on the Opacity of the first V1 clip: the graphics
/// and mask commands then have targets.
fn demo_with_graphics() -> (Session, ClipId) {
    let mut s = demo_with_transcript_and_captions();
    let title = s.execute("graphics.newText", json!({"text": "Lower third", "time": 0})).expect("a title goes on the timeline");
    let title = title["clip"].as_u64().map(ClipId).unwrap_or_else(|| panic!("the title's clip: {title}"));
    let (first, _) = first_v1_clip(&s);
    s.execute("masks.add", json!({"clip": first.0, "effect": "opacity"})).expect("a mask goes on the first shot's opacity");
    (s, title)
}

/// The query ids the adapter reads for the newsroom families of slice R0 (master plan v2 §3.1), with
/// the parameters it sends: the transcript, the caption tracks, the background jobs (exports) and the
/// effect catalogue with its parameter details (the generated schemas of design §4).
fn newsroom_queries() -> [(&'static str, Value); 4] {
    [("transcript.inspect", json!({})), ("captions.list", json!({})), ("jobs.list", json!({})), ("effects.list", json!({"detail": true}))]
}

/// T-FI-01d. The newsroom reads of slice R0, run the way `executeChecked` runs them, leave the
/// session as it was (as T-FI-01b checks it), on an idle session, while trim playback runs and on a
/// project with a transcript and captions; each spec is a non-journaled, always-enabled query, and
/// on that project they answer what it holds.
///
/// Fallback if this fails (as for T-FI-01b): a pre-check that needs the failing read stays
/// `unknown`, and the adapter does not offer the read as certified.
#[test]
fn newsroom_queries_leave_session_state_unchanged() {
    let mut sessions: Vec<(&str, Session)> = fixtures().into_iter().collect();
    sessions.push(("transcript and captions", demo_with_transcript_and_captions()));
    for (fixture, mut s) in sessions {
        for (id, params) in newsroom_queries() {
            let spec = find_command(id).unwrap_or_else(|| panic!("`{id}` is a registered command"));
            assert!(!spec.journal, "`{id}` is a query: never journaled, so `Session::execute` never settles trim playback for it");
            assert!((spec.enabled)(&s).is_ok(), "`{id}` is enabled on the {fixture} fixture");
            let before = fingerprint(&s);
            let (revision, sequence) = (s.revision, s.state.active_sequence.map(|seq| seq.0));
            let reply = execute_checked(&mut s, revision, sequence, id, params.clone()).unwrap_or_else(|e| panic!("`{id}` failed ({fixture}): {e}"));
            assert!(!reply["result"].is_null(), "`{id}` answers: {reply}");
            assert_eq!(reply["revision"], json!(revision.to_string()), "`{id}` reports the unchanged revision");
            assert_eq!(fingerprint(&s), before, "`{id}` {params} changed session state on the {fixture} fixture");
        }
    }
    // on the project that has them, the reads answer its transcript and its caption
    let mut s = demo_with_transcript_and_captions();
    let (revision, sequence) = (s.revision, s.state.active_sequence.map(|seq| seq.0));
    let transcript = execute_checked(&mut s, revision, sequence, "transcript.inspect", json!({})).expect("the transcript reads");
    assert!(transcript["result"]["words"].as_array().is_some_and(|words| !words.is_empty()), "the transcript's words: {transcript}");
    assert_eq!(transcript["result"]["items"][0]["words"], json!(2), "{transcript}");
    let captions = execute_checked(&mut s, revision, sequence, "captions.list", json!({})).expect("the captions read");
    assert_eq!(captions["result"]["tracks"].as_array().map(Vec::len), Some(1), "{captions}");
    assert_eq!(captions["result"]["tracks"][0]["captions"].as_array().map(Vec::len), Some(1), "{captions}");
}

/// T-FI-01e (slice R1). The two reads `executeChecked` serves since R1 besides those of F0 and R0
/// (`policy::READS`), run the way it runs them, leave the session as it was (as T-FI-01b checks it):
/// the editor state (`state.inspect`, which the edit-state read of W5 needs: the targeting, the
/// patching, linked selection and the In and Out marks among it) on an idle session, while trim
/// playback runs and on a project with a graphic, and the layers of a graphic clip named by its id
/// (`graphics.list`). Each spec is a non-journaled, always-enabled query, and the layers read answers
/// what the graphic holds.
///
/// Fallback if this fails (as for T-FI-01b): the failing read leaves `policy::READS`, and the adapter
/// reads that state from the project snapshot instead.
#[test]
fn edit_state_and_graphics_reads_leave_session_state_unchanged() {
    let (graphics, title) = demo_with_graphics();
    let mut sessions = fixtures().into_iter().map(|(fixture, s)| (fixture, s, vec![("state.inspect", json!({}))])).collect::<Vec<_>>();
    sessions.push(("graphics", graphics, vec![("state.inspect", json!({})), ("graphics.list", json!({"clip": title.0}))]));
    for (fixture, mut s, reads) in sessions {
        for (id, params) in reads {
            let spec = find_command(id).unwrap_or_else(|| panic!("`{id}` is a registered command"));
            assert!(!spec.journal, "`{id}` is a query: never journaled, so `Session::execute` never settles trim playback for it");
            assert!((spec.enabled)(&s).is_ok(), "`{id}` is enabled on the {fixture} fixture");
            let before = fingerprint(&s);
            let (revision, sequence) = (s.revision, s.state.active_sequence.map(|seq| seq.0));
            let reply = execute_checked(&mut s, revision, sequence, id, params.clone()).unwrap_or_else(|e| panic!("`{id}` failed ({fixture}): {e}"));
            assert!(reply["result"].is_object(), "`{id}` answers: {reply}");
            assert_eq!(reply["revision"], json!(revision.to_string()), "`{id}` reports the unchanged revision");
            assert_eq!(fingerprint(&s), before, "`{id}` {params} changed session state on the {fixture} fixture");
        }
    }
    // the layers read answers the title's one text layer
    let (mut s, title) = demo_with_graphics();
    let (revision, sequence) = (s.revision, s.state.active_sequence.map(|seq| seq.0));
    let listed = execute_checked(&mut s, revision, sequence, "graphics.list", json!({"clip": title.0})).expect("the title's layers read");
    let layer = &listed["result"]["layers"][0];
    assert_eq!((layer["kind"].as_str(), layer["text"].as_str()), (Some("text"), Some("Lower third")), "{listed}");
}

/// T-FI-02. `projectSnapshot()` returns `encode_with_view(&project, Some(&project_view()), false)`:
/// encoding twice gives identical bytes and leaves the session unchanged, and the bytes are a
/// loadable project file (current schema, same items) that carries the view, within the hook's
/// 8 MiB limit.
///
/// Consequence if this fails (design §6.3): `projectSnapshot()` leaves the certified read set, so
/// checkpoints and outputs are not served from it.
#[test]
fn project_snapshot_encode_is_pure() {
    for (fixture, s) in fixtures() {
        let before = fingerprint(&s);
        let first = filmcraft_format::encode_with_view(&s.project, Some(&s.project_view()), false);
        let second = filmcraft_format::encode_with_view(&s.project, Some(&s.project_view()), false);
        assert!(first == second, "the snapshot is deterministic on the {fixture} fixture");
        assert_eq!(fingerprint(&s), before, "encoding the snapshot changed session state on the {fixture} fixture");
        assert!(!first.is_empty() && first.len() <= 8 * 1024 * 1024, "the snapshot fits the hook's 8 MiB limit ({} bytes)", first.len());
        let loaded = filmcraft_format::decode(&first).expect("the snapshot is a loadable project file");
        assert_eq!(loaded.schema_version, filmcraft_format::SCHEMA_VERSION);
        assert_eq!(loaded.view, Some(s.project_view()), "the snapshot carries what is open");
        assert!(loaded.project.items.keys().eq(s.project.items.keys()), "the snapshot has the project's items");
    }
}

/// T-FI-05. `try_render_program_at` (and `render_program_at`), the read behind
/// `agentView.renderFrame`, renders the active sequence at any time and scale without moving the
/// playhead or changing anything else, on an idle session and while trim playback runs.
///
/// Consequence if this fails (design §6.3, INV-20): `agentView.renderFrame` leaves the certified read
/// set, and frames stay UI screenshots labelled `exact:false`.
#[test]
fn render_program_at_leaves_state_unchanged() {
    for (fixture, s) in fixtures() {
        let seq = s.active_sequence().expect("the demo opens its main sequence");
        let rate = s.sequence_rate();
        let before = fingerprint(&s);
        // the playhead, the first shot, a time inside a frame, the overlay with its drop shadow and
        // opacity keyframes, and times before the start and after the end
        let times = [s.playhead(), rate.tick_of(30), rate.tick_of(60) + Tick(1), rate.tick_of(150), Tick(-1), seq.duration() + rate.tick_of(10)];
        for t in times {
            for scale in [0.125, 0.25] {
                let frame = s.try_render_program_at(scale, t).unwrap_or_else(|e| panic!("frame at {t:?} x{scale} ({fixture}): {e}"));
                assert_eq!((frame.w, frame.h), filmcraft_render::output_size(seq, scale), "frame size at x{scale}");
                let again = s.render_program_at(scale, t).expect("the same frame through `render_program_at`");
                assert!(same_frame(&again, &frame), "both entry points render the same frame at {t:?} x{scale} ({fixture})");
                assert_eq!(fingerprint(&s), before, "rendering {t:?} x{scale} changed session state on the {fixture} fixture");
            }
        }
    }
}

/// T-FI-07. The frame `agentView.renderFrame` returns is the active sequence's own frame times
/// `scale`, never a panel size: 1920x1080 at scale 1 for the demo, and with the hook's
/// `scale = min(1, maxSide / long edge)` its long edge is exactly `min(maxSide, long edge)`. Time
/// snaps down to the frame that contains it, times before the start render frame 0, and with no
/// active sequence there is no frame (`NoSequence`).
///
/// Consequence if this fails (INV-20): `renderFrame` reports the size and the frame tick it
/// computes itself instead of relying on the engine.
#[test]
fn render_program_at_reports_sequence_resolution_and_snaps_time() {
    let s = demo();
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let (width, height) = (seq.settings.width, seq.settings.height);
    assert_eq!((width, height), (1920, 1080), "the demo's main sequence is 1080p");
    let rate = s.sequence_rate();
    let before = fingerprint(&s);
    let render = |scale: f32, t: Tick| s.try_render_program_at(scale, t).unwrap_or_else(|e| panic!("frame at {t:?} x{scale}: {e}"));

    // the sequence's own frame size times `scale`
    let full = render(1.0, rate.tick_of(60));
    assert_eq!((full.w, full.h), (1920, 1080), "scale 1 renders at the sequence resolution");
    for scale in [0.5, 0.25, 1.0 / 3.0] {
        let frame = render(scale, rate.tick_of(60));
        assert_eq!((frame.w, frame.h), filmcraft_render::output_size(seq, scale), "x{scale}");
    }
    // the hook's scale for a `maxSide` budget never exceeds it and never upscales
    let long_edge = width.max(height);
    for max_side in [2048u32, 1568, 512, 160, 16] {
        let frame = render((max_side as f32 / long_edge as f32).min(1.0), rate.tick_of(60));
        assert_eq!(frame.w.max(frame.h), max_side.min(long_edge) as usize, "maxSide {max_side}");
    }

    // time snaps down to the frame that contains it
    let frame_60 = render(0.125, rate.tick_of(60));
    let last_tick_of_60 = rate.tick_of(61) - Tick(1);
    assert_eq!(rate.snap(last_tick_of_60), rate.tick_of(60));
    for t in [rate.tick_of(60) + Tick(1), rate.tick_of(60) + Tick(rate.frame_duration().0 / 2), last_tick_of_60] {
        assert!(same_frame(&render(0.125, t), &frame_60), "{t:?} lies in frame 60 and renders frame 60");
    }
    assert!(!same_frame(&render(0.125, rate.tick_of(61)), &frame_60), "neighbouring demo frames differ, so the equalities above are meaningful");
    // times before the start render frame 0
    let frame_0 = render(0.125, Tick::ZERO);
    for t in [Tick(-1), -rate.frame_duration(), Tick::from_seconds_f64(-5.0)] {
        assert!(same_frame(&render(0.125, t), &frame_0), "{t:?} renders frame 0");
    }
    assert_eq!(fingerprint(&s), before, "rendering changed session state");

    // only the active sequence renders: with none open there is no frame
    assert!(matches!(Session::default().try_render_program_at(0.125, Tick::ZERO), Err(EngineError::NoSequence)), "no active sequence, no frame");
}

/// T-FI-08. Scales that are not finite and positive, and scales whose frame is over the image
/// limits (`MAX_FRAME_SIDE`, `MAX_FRAME_PIXELS`, or a side that is not even a `u32`), are refused
/// with a typed error before anything is rendered. From scale 1e6 up no frame buffer could be
/// allocated at all, so a typed refusal that returns proves the check runs before rendering; the
/// sizes below that are refused by the same check. Tiny positive scales are bounded below by one
/// pixel. The session is unchanged throughout.
///
/// Consequence if this fails (INV-28): `agentView.renderFrame` bounds `scale` itself before it calls
/// the engine (it computes `scale = min(1, maxSide / long edge)` with `maxSide <= 2048`).
#[test]
fn render_scale_bounds_refused_without_allocation() {
    const NOT_POSITIVE: &str = "must be finite and positive";
    const NOT_A_U32: &str = "exceeds the image size limits";
    for (fixture, s) in fixtures() {
        let seq = s.active_sequence().expect("the demo opens its main sequence");
        let t = s.playhead();
        let before = fingerprint(&s);
        let too_large = |scale: f32| {
            let (w, h) = filmcraft_render::output_size(seq, scale);
            format!("cannot render a {w}x{h} frame")
        };
        let refused: [(f32, String); 11] = [
            (0.0, NOT_POSITIVE.into()),
            (-0.0, NOT_POSITIVE.into()),
            (-1.0, NOT_POSITIVE.into()),
            (f32::NAN, NOT_POSITIVE.into()),
            (f32::INFINITY, NOT_POSITIVE.into()),
            (f32::NEG_INFINITY, NOT_POSITIVE.into()),
            // a side over `MAX_FRAME_SIDE`
            (17.1, too_large(17.1)),
            // sides within it, the area over `MAX_FRAME_PIXELS`
            (16.0, too_large(16.0)),
            // sides of a billion pixels and more: no allocation could succeed
            (1.0e6, too_large(1.0e6)),
            // sides that are not even a `u32`
            (1.0e30, NOT_A_U32.into()),
            (f32::MAX, NOT_A_U32.into()),
        ];
        for (scale, why) in refused {
            match s.try_render_program_at(scale, t) {
                Err(EngineError::Other(message)) => assert!(message.contains(&why), "scale {scale}: {message:?} does not say {why:?}"),
                other => panic!("scale {scale} must be refused before rendering ({fixture}): {:?}", other.map(|frame| (frame.w, frame.h))),
            }
            assert!(s.render_program_at(scale, t).is_none(), "scale {scale}: no frame");
            assert_eq!(fingerprint(&s), before, "refusing scale {scale} changed session state on the {fixture} fixture");
        }
        // tiny positive scales are bounded below by one pixel, not refused
        for scale in [1.0e-4, 1.0e-6] {
            let frame = s.try_render_program_at(scale, t).unwrap_or_else(|e| panic!("scale {scale}: {e}"));
            assert_eq!((frame.w, frame.h), (1, 1), "scale {scale}");
        }
        assert_eq!(fingerprint(&s), before);
    }
}

/// T-FI-09. `scopes::scope_signal`, the read behind `agentView.scopes`, is pure: the same signal
/// twice (SDR and HDR) and the session unchanged; the SDR signal is the decimated picture that
/// `renderFrame` returns for the same frame (same compositor, same options); with no active
/// sequence there is no signal. It neither bounds `scale` nor snaps `t`: the hook does both first.
///
/// Consequence if this fails (design §6.3): `agentView.scopes` leaves the certified read set, and
/// scopes are computed by the trusted parent from certified frames instead.
#[test]
fn scope_signal_is_pure() {
    for (fixture, s) in fixtures() {
        let rate = s.sequence_rate();
        let before = fingerprint(&s);
        for t in [rate.snap(s.playhead()), rate.tick_of(60), rate.tick_of(150)] {
            for hdr in [false, true] {
                let first = scope_signal(&s, t, 0.25, hdr).expect("an active sequence has a scope signal");
                let second = scope_signal(&s, t, 0.25, hdr).expect("an active sequence has a scope signal");
                assert!(!first.is_empty() && first.len() == first.w * first.h, "a {}x{} signal", first.w, first.h);
                assert!(first == second, "the scope signal (hdr {hdr}) at {t:?} is deterministic on the {fixture} fixture");
                assert_eq!(fingerprint(&s), before, "reading scopes (hdr {hdr}) at {t:?} changed session state on the {fixture} fixture");
            }
            let frame = s.try_render_program_at(0.25, t).unwrap_or_else(|e| panic!("frame at {t:?}: {e}"));
            let from_frame = Signal::from_rgba8(frame.w, frame.h, &frame.over_black_rgba8());
            assert!(scope_signal(&s, t, 0.25, false) == Some(from_frame), "the SDR scopes read the `renderFrame` picture at {t:?} ({fixture})");
        }
        assert_eq!(fingerprint(&s), before);
    }
    assert!(scope_signal(&Session::default(), Tick::ZERO, 0.25, false).is_none(), "no active sequence, no signal");
}

// ------------------------------------------------------------------------------------ FF1 fixtures

/// The clock reading the replayed hook closures see.
const NOW_MS: f64 = 1_700_000_000_000.0;

/// A dispatch deadline ahead of [`NOW_MS`].
const DEADLINE_MS: f64 = NOW_MS + 15_000.0;

/// The closed `not_applied` codes of `executeTyped` (design Appendix A.2, and the eight of slice R0:
/// `linked_selection_changed`, `targeting_changed`, `native_only`, `use_worker_export`,
/// `sync_locked`, `keyframe_exists`, `keyframe_not_found` and `control_kind_mismatch`).
const NOT_APPLIED_CODES: [&str; 15] = [
    "deadline_elapsed",
    "revision_conflict",
    "sequence_changed",
    "linked_selection_changed",
    "targeting_changed",
    "editor_busy",
    "command_disabled",
    "command_not_allowed",
    "native_only",
    "use_worker_export",
    "history_head_conflict",
    "sync_locked",
    "keyframe_exists",
    "keyframe_not_found",
    "control_kind_mismatch",
];

/// The fields an `executeTyped` envelope may carry: design Appendix A.2, the bounded engine text
/// `detail` of design §7.1, which never classifies anything, and the tracks a `sync_locked`
/// refusal names (MC §9.4).
const ENVELOPE_FIELDS: [&str; 11] = [
    "resultVersion",
    "outcome",
    "code",
    "result",
    "revisionBefore",
    "revisionAfter",
    "intentSatisfied",
    "actual",
    "historyLabel",
    "detail",
    "syncLockedTracks",
];

/// The `executeTyped` policy table of slice FF1 (plan FF1, design §4.4): the edits the legacy video
/// operations dispatch (seek, new sequence, place, title, trim, dissolve) and the newsroom set (C1).
/// Not in it: `graphics.setText` and `file.saveCopy`, which only the untyped legacy path uses, and a
/// marker deletion, for which the engine has no command.
const FF1_COMMANDS: [&str; 22] = [
    // legacy operations
    "playhead.set",
    "file.newSequence",
    "timeline.place",
    "graphics.newText",
    "graphics.set",
    "timeline.trim",
    "sequence.applyVideoTransition",
    // newsroom set (C1)
    "graphics.newShape",
    "lumetri.setSection",
    "timeline.razor",
    "timeline.move",
    "sequence.lift",
    "sequence.extract",
    "sequence.closeGap",
    "edit.rippleDelete",
    "clip.enable",
    "edit.duplicate",
    "markers.add",
    "markers.edit",
    "clip.audioGain",
    "edit.undo",
    "edit.redo",
];

/// The entries slice R0 adds (master plan v2 §3.1), each with the exact keys it allows and the keys
/// every call must give: tier A (mandatory), tier B (best effort), the native-only commands and the
/// browser-only media export. Every key is one its command documents (`commands()`). A required key
/// is one the engine fails without inside the command (a sound type, a transcript, a new speaker
/// name, a mask's move, a text type, an alignment), a target or a time the engine would otherwise
/// take from the person's own state (the playhead, the clip, caption, mask or layer selection, the
/// Source monitor, the default audio transition), or the effect parameter a call names; a key the
/// engine fills in with a fixed default of its own stays optional. What some calls need beyond
/// these is in [`NEEDS`]. Left out on purpose: time aliases in frames, seconds or timecode
/// (INV-32), undo-merge keys (`merge`, `begin`), `wait` (except in the
/// two native-only entries that document it, scene detection and mask tracking, which keep it, and
/// which the browser refuses whatever their keys), a preset or raw settings that could carry another
/// format or path (`preset`, `settings`), the system encoders (`hardwareEncoding`), a loudness pass
/// that measures the whole export in one call (`loudnessLufs`), ripple switches the ripple pre-check
/// does not cover (`ripple`), mask geometry given as `path`, a subclip's `clip` (its `item` and
/// bounds say the same without the Source monitor) and the custom export range in float seconds.
const R0_ENTRIES: &[(&str, &[&str], &[&str])] = &[
    // tier A: Motion and fit/fill, sync lock, audio transitions, keyframes, export and its jobs
    ("effects.setParam", &["clip", "effect", "param", "value", "time"], &["clip", "effect", "param", "value", "time"]),
    ("clip.fillFrame", &["clips"], &["clips"]),
    ("clip.fitToFrame", &["clips"], &["clips"]),
    ("timeline.setTrack", &["track", "syncLock"], &["track", "syncLock"]),
    ("sequence.applyAudioTransition", &["clip", "effect", "frames"], &["clip", "effect", "frames"]),
    ("effects.addKeyframe", &["clip", "effect", "param", "time"], &["clip", "effect", "param", "time"]),
    ("effects.setKeyframe", &["clip", "effect", "param", "mediaTime", "value", "inInfluence", "outInfluence"], &["clip", "effect", "param", "mediaTime"]),
    ("effects.setInterpolation", &["clip", "effect", "param", "mediaTime", "interpolation"], &["clip", "effect", "param", "mediaTime", "interpolation"]),
    ("effects.deleteKeyframe", &["clip", "effect", "param", "mediaTime"], &["clip", "effect", "param", "mediaTime"]),
    (
        "file.exportMedia",
        &[
            "path",
            "format",
            "width",
            "height",
            "fps",
            "bitrateKbps",
            "bitrateMode",
            "scale",
            "audio",
            "quality",
            "burnCaptions",
            "captionSidecar",
            "proresProfile",
            "dnxProfile",
            "apvProfile",
            "mxfVideoCodec",
            "sequence",
            "range",
        ],
        &["path", "format"],
    ),
    ("jobs.cancel", &["job"], &["job"]),
    // tier A: the transcript
    ("transcript.set", &["item", "transcript"], &["item", "transcript"]),
    ("transcript.select", &["from", "to"], &["from"]),
    ("transcript.extract", &["from", "to"], &["from"]),
    ("transcript.lift", &["from", "to"], &["from"]),
    ("transcript.removePauses", &["minSeconds", "keepSeconds"], &[]),
    ("transcript.removeFillers", &["fillers"], &[]),
    ("transcript.createCaptions", &["maxChars", "lines", "minSeconds", "maxSeconds", "gapFrames", "format", "name"], &[]),
    ("transcript.renameSpeaker", &["speaker", "name", "item"], &["speaker", "name"]),
    // tier A: captions
    ("captions.newTrack", &["format", "name", "language"], &[]),
    ("captions.add", &["track", "text", "time", "durationSeconds"], &["time"]),
    ("captions.setText", &["caption", "text", "speaker"], &["caption"]),
    ("captions.setTimes", &["caption", "startTime", "endTime"], &["caption"]),
    (
        "captions.setStyle",
        &["track", "font", "size", "color", "background", "backgroundColor", "align", "anchor", "margin", "lineSpacing", "outline", "outlineColor", "reset"],
        &[],
    ),
    ("captions.split", &["caption", "time"], &["caption", "time"]),
    ("captions.merge", &["captions"], &["captions"]),
    ("captions.delete", &["captions"], &["captions"]),
    // tier B: redaction effects and masks without tracking
    ("effects.apply", &["clips", "effect"], &["clips", "effect"]),
    ("masks.add", &["clip", "effect", "shape", "center", "size"], &["clip", "effect"]),
    ("masks.set", &["clip", "effect", "mask", "name", "inverted", "mode", "feather", "opacity", "expansion", "time"], &["clip", "effect", "mask", "time"]),
    ("masks.translate", &["clip", "effect", "mask", "delta"], &["clip", "effect", "mask", "delta"]),
    ("masks.remove", &["clip", "effect", "mask"], &["clip", "effect", "mask"]),
    // tier B: transitions, speed, holds and subclips
    ("sequence.setTransition", &["transition", "params", "reverse", "reset"], &["transition"]),
    ("clip.speedDuration", &["clips", "speed", "reverse", "interpolation"], &["clips"]),
    ("clip.frameHold", &["clips", "time"], &["clips", "time"]),
    ("clip.makeSubclip", &["item", "name", "start", "end", "restrictTrims"], &["item"]),
    // tier B: Essential Sound types and ducking
    ("essentialSound.setType", &["clips", "type"], &["clips", "type"]),
    ("essentialSound.clearType", &["clips"], &["clips"]),
    ("essentialSound.generateDucking", &["clips"], &["clips"]),
    // tier B: graphics style and template values
    ("graphics.setTextType", &["clip", "layer", "type"], &["clip", "layer", "type"]),
    ("graphics.setCharStyle", &["clip", "layer", "start", "end", "style", "clear"], &["clip", "layer"]),
    ("graphics.align", &["clip", "layers", "align", "to"], &["clip", "layers", "align"]),
    ("graphics.distribute", &["clip", "layers", "axis", "space"], &["clip", "layers"]),
    ("graphics.arrangeLayer", &["clip", "layer", "to"], &["clip", "layer"]),
    ("graphics.template.set", &["clip", "control", "value", "values"], &["clip"]),
    // tier B: interchange for human finishing tools (J35), never for conform
    ("file.exportEdl", &["path"], &["path"]),
    ("file.exportOtio", &["path"], &["path"]),
    ("file.exportFcpxml", &["path"], &["path"]),
    // native only
    ("clip.sceneEditDetection", &["clips", "sensitivity", "minShotFrames", "applyCuts", "createSubclips", "generateMarkers", "wait"], &["clips"]),
    ("masks.track", &["clip", "effect", "mask", "direction", "frames", "method", "wait"], &["clip", "effect", "mask"]),
];

/// The FF1 entries that require keys since slice R0: each key the engine fails without inside the
/// command (a seek's time, the item to place, a graphic's properties, the clip to trim, the moves, a
/// Lumetri section, the marker to edit); the four keys of a video transition, which the engine would
/// otherwise take from the person's default transition and duration and the playhead; and a closed
/// gap's track and time and a ripple delete's clips, so the ripple pre-check sees what the engine
/// will close. No other FF1 entry requires one.
const FF1_REQUIRED: &[(&str, &[&str])] = &[
    ("playhead.set", &["time"]),
    ("timeline.place", &["item"]),
    ("graphics.set", &["props"]),
    ("timeline.trim", &["clip"]),
    ("sequence.applyVideoTransition", &["clip", "effect", "frames", "edge"]),
    ("lumetri.setSection", &["section"]),
    ("timeline.move", &["moves"]),
    ("sequence.closeGap", &["track", "time"]),
    ("edit.rippleDelete", &["clips"]),
    ("markers.edit", &["marker"]),
];

/// What the entries that need more than their always-required keys ask for, as [`need_text`] writes
/// it: one of several groups of keys (`|`, a group's keys joined by `+`), keys that come together or
/// not at all, and a key that one value of another key needs. No other entry needs anything more.
const NEEDS: &[(&str, &[&str])] = &[
    ("graphics.newShape", &["points if shape is path"]),
    ("file.exportMedia", &["width+height or neither"]),
    ("transcript.renameSpeaker", &["item if speaker is an index"]),
    ("captions.setText", &["text|speaker"]),
    ("graphics.setCharStyle", &["style|clear"]),
    ("graphics.template.set", &["control+value|values", "control+value or neither"]),
];

/// The keys a call may leave out that the table types anyway, because the engine reads a value of
/// another type there as the key left out and fills it in itself: from the playhead, the person's
/// clip or layer selection, the source patch, the Source monitor's In and Out points, the active
/// sequence, the first caption track, the person's linked-selection setting or text preferences;
/// or as an overwrite, a toggle, the whole text, the free track at the playhead, a new clip there,
/// or the font a caption track had (a track of another type fails inside the command instead).
/// Each has a rule, so such a value is refused before dispatch; left out, each still takes that
/// state (FF3 names them). Every switch, time, bounded number and speed is pinned besides: the
/// switches from the params docs, the rest in [`TICK_KEYS`] and [`WITHIN`].
const TYPED_OPTIONAL: &[(&str, &[&str])] = &[
    ("timeline.place", &["track", "audioTrack", "time", "insert", "sourceIn", "duration"]),
    ("graphics.newText", &["time", "track", "literal", "font", "fontStyle"]),
    ("graphics.set", &["clip", "layer", "time"]),
    ("graphics.newShape", &["clip"]),
    ("lumetri.setSection", &["clip", "on"]),
    ("timeline.razor", &["time", "track", "clip"]),
    ("timeline.move", &["insert", "linked"]),
    ("clip.enable", &["clips"]),
    ("markers.add", &["time"]),
    ("clip.audioGain", &["clips"]),
    ("captions.add", &["track"]),
    ("captions.setStyle", &["track", "font"]),
    ("file.exportMedia", &["sequence"]),
    ("graphics.setCharStyle", &["start", "end"]),
];

/// The keys that take a time, or a difference of times, in ticks: each within the engine's own
/// `Tick::MAX` of zero ([`policy::MAX_TICKS`]), past which the engine's unchecked arithmetic on times
/// overflows. Every key a params doc types as `ticks` is one, and so are a caption's new bounds,
/// whose doc lists them as alternatives; the time of a move and the times of a transcript's words
/// are checked inside their rules. No other key is in ticks.
const TICK_KEYS: &[(&str, &[&str])] = &[
    ("playhead.set", &["time"]),
    ("timeline.place", &["time", "sourceIn", "duration"]),
    ("graphics.newText", &["time"]),
    ("graphics.set", &["time"]),
    ("timeline.trim", &["delta"]),
    ("timeline.razor", &["time"]),
    ("sequence.closeGap", &["time"]),
    ("markers.add", &["time"]),
    ("effects.setParam", &["time"]),
    ("effects.addKeyframe", &["time"]),
    ("effects.setKeyframe", &["mediaTime"]),
    ("effects.setInterpolation", &["mediaTime"]),
    ("effects.deleteKeyframe", &["mediaTime"]),
    ("captions.add", &["time"]),
    ("captions.setTimes", &["startTime", "endTime"]),
    ("captions.split", &["time"]),
    ("masks.set", &["time"]),
    ("clip.frameHold", &["time"]),
    ("clip.makeSubclip", &["start", "end"]),
];

/// The whole numbers the table bounds, with their bounds: a frame side within the engine's longest,
/// a number of frames up to the most the engine's own transition settings take, the shortest shot of
/// scene detection within the engine's own range, and the characters, lines and gap of captions made
/// from the transcript, which the engine multiplies without a check; and since slice RN a media item's
/// start timecode in frames (a day at 1000 frames a second) and a sequence's frame size, natively.
/// No other rule bounds a whole number.
const WITHIN: &[(&str, &str, i64, i64)] = &[
    ("sequence.applyVideoTransition", "frames", 1, policy::MAX_FRAMES),
    ("sequence.applyAudioTransition", "frames", 1, policy::MAX_FRAMES),
    ("masks.track", "frames", 1, policy::MAX_FRAMES),
    ("clip.sceneEditDetection", "minShotFrames", 1, 10_000),
    ("transcript.createCaptions", "maxChars", 1, 200),
    ("transcript.createCaptions", "lines", 1, 2),
    ("transcript.createCaptions", "gapFrames", 0, 1000),
    ("file.exportMedia", "width", 1, policy::MAX_FRAME_SIDE),
    ("file.exportMedia", "height", 1, policy::MAX_FRAME_SIDE),
    ("clip.modifyTimecode", "frame", 0, policy::MAX_TIMECODE_FRAMES),
    ("sequence.settings", "width", 1, policy::MAX_FRAME_SIDE),
    ("sequence.settings", "height", 1, policy::MAX_FRAME_SIDE),
];

/// The durations and pause lengths a call may give in seconds, each from 0 to a day
/// ([`policy::MAX_SECONDS`]): a negative time kept around a pause would cut into the words around
/// it, and a larger number overflow the engine's sums of times. No other key is in seconds.
const SECONDS_KEYS: [(&str, &str); 6] = [
    ("graphics.newText", "seconds"),
    ("transcript.removePauses", "minSeconds"),
    ("transcript.removePauses", "keepSeconds"),
    ("transcript.createCaptions", "minSeconds"),
    ("transcript.createCaptions", "maxSeconds"),
    ("captions.add", "durationSeconds"),
];

/// The commands that act on the tracks the person targets, or select clips on them (selecting
/// transcript words moves the playhead, which with Selection Follows Playhead selects the clips under
/// it on the targeted tracks), and so need the caller's track targeting.
const TARGETED: [&str; 5] = ["sequence.lift", "sequence.extract", "transcript.select", "transcript.extract", "transcript.lift"];

/// The entries slice R1 adds (master plan v3 W3, m22), each with the exact keys it allows and the keys every call must
/// give: the sequence marks, set at a time in ticks the call names (never the playhead) on the program only, since the
/// source marks a `target` would reach stay engine-only, and cleared together.
const R1_ENTRIES: &[(&str, &[&str], &[&str])] =
    &[("markers.markIn", &["time"], &["time"]), ("markers.markOut", &["time"], &["time"]), ("markers.clearInOut", &[], &[])];

/// The entries widen-1 widens (master plan v3 W3, topology §7.3), with the keys each allows and requires since R1: a
/// transition's settings (`params`, typed by transition), a track's lock, output (`enabled`) and mute beside its sync
/// lock (a call names the track and at least one of them, [`R1_NEEDS`]), and the switch with which scene detection only
/// looks. `graphics.arrangeLayer` keeps its keys (its `to` takes an index as well) and `timeline.place` its own (an
/// insert names its time).
const WIDEN_1: &[(&str, &[&str], &[&str])] = &[
    ("sequence.applyVideoTransition", &["clip", "effect", "frames", "edge", "reverse", "params"], &["clip", "effect", "frames", "edge"]),
    ("timeline.setTrack", &["track", "syncLock", "locked", "enabled", "muted"], &["track"]),
    ("clip.sceneEditDetection", &["clips", "sensitivity", "minShotFrames", "applyCuts", "createSubclips", "generateMarkers", "wait", "lookOnly"], &["clips"]),
];

/// What the entries R1 widens need beyond their required keys, as [`need_text`] writes it: an insert names its time (the
/// insert pre-check looks there), a track call names at least one switch (without one the engine makes an empty undo
/// step), and a scene detection that only looks names no apply choice.
const R1_NEEDS: &[(&str, &[&str])] = &[
    ("timeline.place", &["time if insert is true"]),
    ("timeline.setTrack", &["syncLock|locked|enabled|muted"]),
    ("clip.sceneEditDetection", &["no applyCuts+createSubclips+generateMarkers if lookOnly is true"]),
];

/// The keys in ticks of the entries R1 adds.
const R1_TICK_KEYS: &[(&str, &[&str])] = &[("markers.markIn", &["time"]), ("markers.markOut", &["time"])];

/// The entries slice RN adds (native reach, 2026-10-23: vertical shorts from a broadcast episode, which only a native
/// worker opens), each with the exact keys it allows and the keys every call must give, every one native only: Modify
/// Audio Channels and Breakout to Mono on media items by id (the clip mode's `channels` is not a key: it would take
/// the person's clip selection when no item has sound), a media item's start timecode in frames (never a timecode
/// text), the active sequence's frame size, name and mix (not its rate), and another sequence opened as the active one by
/// its item.
const RN_ENTRIES: &[(&str, &[&str], &[&str])] = &[
    ("clip.audioChannels", &["items", "format", "clips"], &["items"]),
    ("clip.breakoutToMono", &["items"], &["items"]),
    ("clip.modifyTimecode", &["item", "frame", "tapeName", "reset"], &["item"]),
    ("sequence.settings", &["width", "height", "name", "mix"], &[]),
    ("sequence.open", &["item"], &["item"]),
];

/// What the entries RN adds need beyond their required keys, as [`need_text`] writes it: a timecode edit sets a frame,
/// a tape name or the reset, and a settings call gives the frame size whole and changes something.
const RN_NEEDS: &[(&str, &[&str])] =
    &[("clip.modifyTimecode", &["frame|tapeName|reset"]), ("sequence.settings", &["width+height or neither", "width+height|name|mix"])];

/// The effects only a native worker adds with `effects.apply` since slice RN, and the entries through which it reaches
/// their parameters: Crop's edges and feather and Auto Reframe's offset and zoom, set and keyframed as Motion is.
const RN_NATIVE_EFFECTS: [&str; 2] = ["crop", "auto_reframe"];
const RN_REACHING: [&str; 5] = ["effects.setParam", "effects.addKeyframe", "effects.setKeyframe", "effects.setInterpolation", "effects.deleteKeyframe"];

/// The certified reads `executeChecked` serves since slice R1 (`policy::READS`), each with the exact keys it takes and
/// the keys it requires: the six of F0 (T-FI-01b), the four newsroom reads of R0 (T-FI-01d) and the editor state and the
/// layers of a graphic of R1 (T-FI-01e), with the parameters those tests certify. A read that would otherwise take the
/// person's selection names its target (`metadata.get` and `media.colorInfo` an item, `graphics.list` a clip).
const CERTIFIED_READS: &[(&str, &[&str], &[&str])] = &[
    ("project.inspect", &[], &[]),
    ("sequence.inspect", &["item"], &[]),
    ("fonts.list", &["system"], &[]),
    ("history.list", &[], &[]),
    ("metadata.get", &["item"], &["item"]),
    ("media.colorInfo", &["item"], &["item"]),
    ("transcript.inspect", &[], &[]),
    ("captions.list", &[], &[]),
    ("jobs.list", &[], &[]),
    ("effects.list", &["detail"], &[]),
    ("state.inspect", &[], &[]),
    ("graphics.list", &["clip"], &["clip"]),
];

/// The codes of the closed code registry whose sources name this fork, each with the kinds the registry gives it:
/// `docs/contracts/registry/codes.registry.v1.json` of the LTS mirror, registry 1.0.0 (the same twenty, with the same
/// kinds, at sha256 `0950d952…` and `ac5b10a3…`, generated on 2026-10-18, and at `0b2de7e4…`, generated on
/// 2026-10-19). A code the hook answers with that the registry lacks reaches the broker as `unknown` and fences (master
/// plan v3 §1.1, RD-6); this copy is checked again whenever the registry is regenerated, until the registry's own tests
/// compare its Film codes with the hook's lists.
const REGISTERED_FILM_CODES: [(&str, &[&str]); 20] = [
    ("budget_exceeded", &["not_applied", "read_failure"]),
    ("clamped", &["applied"]),
    ("command_disabled", &["not_applied"]),
    ("command_not_allowed", &["not_applied"]),
    ("control_kind_mismatch", &["not_applied"]),
    ("deadline_elapsed", &["not_applied"]),
    ("editor_busy", &["not_applied", "read_failure"]),
    ("engine_error_after_dispatch", &["unknown"]),
    ("history_head_conflict", &["not_applied"]),
    ("keyframe_exists", &["not_applied"]),
    ("keyframe_not_found", &["not_applied"]),
    ("linked_selection_changed", &["not_applied"]),
    ("native_only", &["not_applied"]),
    ("read_failed", &["read_failure"]),
    ("revision_conflict", &["not_applied"]),
    ("sequence_changed", &["not_applied"]),
    ("sequence_not_active", &["not_applied", "read_failure"]),
    ("sync_locked", &["not_applied"]),
    ("targeting_changed", &["not_applied"]),
    ("use_worker_export", &["not_applied"]),
];

/// Calls the table refuses for a key they leave out, and the engine, given the same call, fails on
/// from inside the command (not as disabled): what the required keys and the needs stand for. Each
/// runs on the demo with a transcript, a caption, a title and a mask ([`demo_with_graphics`]), whose
/// first caption, first V1 clip, the media of that clip, first A1 clip and title are filled in where
/// a call names `{caption}`, `{video}`, `{item}`, `{audio}` or `{graphic}`.
const ENGINE_FAILS_WITHOUT: &[(&str, &str)] = &[
    ("playhead.set", r#"{}"#),
    ("timeline.place", r#"{"time": 0}"#),
    ("timeline.trim", r#"{"edge": "out", "mode": "regular", "delta": 0}"#),
    ("timeline.move", r#"{"insert": false}"#),
    ("lumetri.setSection", r#"{"on": true}"#),
    ("markers.edit", r#"{"name": "a marker"}"#),
    ("graphics.newShape", r#"{"shape": "path"}"#),
    ("file.exportMedia", r#"{"path": "/lts-exports/t-fi-12/out.mp4", "format": "h264", "width": 640}"#),
    ("transcript.set", r#"{"item": "{item}"}"#),
    ("transcript.renameSpeaker", r#"{"speaker": "Speaker 1"}"#),
    ("transcript.renameSpeaker", r#"{"speaker": 0, "name": "Anchor"}"#),
    ("captions.setText", r#"{"caption": "{caption}"}"#),
    ("essentialSound.setType", r#"{"clips": ["{audio}"]}"#),
    ("masks.translate", r#"{"clip": "{video}", "effect": "opacity", "mask": 0}"#),
    ("graphics.setTextType", r#"{"clip": "{graphic}", "layer": 0}"#),
    ("graphics.align", r#"{"clip": "{graphic}", "layers": [0]}"#),
    ("graphics.setCharStyle", r#"{"clip": "{graphic}", "layer": 0}"#),
    ("graphics.template.set", r#"{"clip": "{graphic}"}"#),
];

/// The commands only a native worker runs (master plan v2 §3.1): scene detection, mask tracking and
/// the ducking analysis, which work for as long as they take and would hold the browser's one
/// thread, and the audio transition, which lands at the clip edge nearer to the person's playhead
/// (its params doc lists no `edge`, so the typed path cannot name one); and since slice RN (native
/// reach, 2026-10-23) the entries the browser's table never had ([`RN_ENTRIES`]), so its view stays
/// the one it was.
const NATIVE_ONLY_COMMANDS: [&str; 9] = [
    "sequence.applyAudioTransition",
    "essentialSound.generateDucking",
    "clip.sceneEditDetection",
    "masks.track",
    "clip.audioChannels",
    "clip.breakoutToMono",
    "clip.modifyTimecode",
    "sequence.settings",
    "sequence.open",
];

/// The command only the browser runs: the media export into its file table. A native worker renders
/// through its own export, with host-built paths (MC §10), so this table refuses it there.
const BROWSER_ONLY_COMMANDS: [&str; 1] = ["file.exportMedia"];

/// The formats `file.exportMedia` writes as a stepped job with a built-in encoder, which the browser
/// advances between UI frames (`filmcraft_export::stepped`, `Format::has_builtin_encoder`).
const STEPPED_FORMATS: [&str; 7] = ["h264", "prores", "dnxhr", "apv", "mjpeg", "mxf-op1a", "mxf-opatom"];

/// The formats it writes in one synchronous run, which would hold the browser for the whole export:
/// GIF, WAV and image sequences, native only.
const BLOCKING_FORMATS: [&str; 5] = ["gif", "wav", "png", "tiff", "bmp"];

/// The formats the table refuses in both runtimes: HEVC, which has no built-in encoder (only one the
/// system registers), and AIFF.
const REFUSED_FORMATS: [&str; 2] = ["hevc", "aiff"];

/// Registered engine commands the typed path never reaches, in either runtime: tier 3 (INV-13:
/// ambient file paths, project replacement, preferences, permanent deletes and model downloads),
/// `file.exportFrame`, which moves the playhead (frames are read with `renderFrame`),
/// `effects.toggleAnimation`, which acts at the playhead, and `sequence.makeSubsequence`, which
/// takes the person's clip selection, track targeting and In/Out (no key binds them).
const NEVER_COMMANDS: [&str; 26] = [
    "file.open",
    "file.save",
    "file.saveAs",
    "file.saveCopy",
    "file.revert",
    "file.import",
    "file.exportFrame",
    "file.newProject",
    "file.openDemoProject",
    "file.exportInterchange",
    "file.exportAaf",
    "file.exportOmf",
    "export.quick",
    "captions.import",
    "captions.export",
    "graphics.newFromFile",
    "lumetri.setInputLut",
    "lumetri.setLook",
    "lut.import",
    "prefs.set",
    "prefs.reset",
    "mediaCache.clean",
    "sequence.deleteRenderFiles",
    "transcript.downloadModel",
    "effects.toggleAnimation",
    "sequence.makeSubsequence",
];

/// Parameter keys that would name an ambient path or carry a raw command or a script.
const FORBIDDEN_KEYS: [&str; 12] = ["path", "paths", "file", "files", "dir", "directory", "url", "lut", "command", "commands", "steps", "script"];

/// Where T-FI-11's host serves its still.
const STILL: &str = "/media/still.png";

/// The deadline of the replayed pending-media loops: their clock passes it at its fifth reading.
const LOOP_DEADLINE_MS: f64 = 50.0;

/// The attempts a pending-media loop makes before [`LOOP_DEADLINE_MS`] ends it.
const PENDING_ATTEMPTS: u32 = 5;

/// A bound that stops a pending-media loop that would never end.
const MAX_ATTEMPTS: u32 = 1_000;

/// The history head as `history.list` reports it: the undo stack's length and newest label.
fn history_head(s: &Session) -> policy::HistoryHead {
    policy::HistoryHead { length: s.history.undo.len() as u64, label: s.history.undo.last().map(|(label, _)| label.clone()).unwrap_or_default() }
}

/// The track targeting of the active sequence as `watermark()` reports it and `executeTyped` reads it
/// (`agent_view.rs`): the digest of its targeted tracks and its source patching.
fn targeting_of(s: &Session) -> String {
    let targeting = s.targeting();
    let targeted: Vec<u64> = targeting.targeted.iter().map(|track| track.0).collect();
    policy::targeting_digest(&targeted, targeting.video_dest.map(|track| track.0), targeting.audio_dest.map(|track| track.0))
}

/// What keeps the editor busy that the session itself holds (`agent_view.rs` reads the same): the
/// person's recording passes (a voice-over take, an automation pass of the Audio Track Mixer that
/// writes, a multi-camera switching pass) and trim playback. An automation pass that writes nothing,
/// which the editor runs through every forward playback at normal speed, is not busy.
fn engine_busy(s: &Session) -> bool {
    s.voiceover.recording() || !s.mixrec.gestures.is_empty() || s.mcrec.active() || s.trim_play.active()
}

/// What a caller expects right after a fresh `context()`, `watermark()` and `history.list` read: the
/// session as it is, its linked-selection setting and track targeting included, with the deadline
/// ahead.
fn expect_now(s: &Session) -> policy::Expect {
    policy::Expect {
        revision: s.revision,
        active_sequence: s.state.active_sequence.map(|id| id.0),
        linked_selection: Some(s.state.linked_selection),
        targeting: Some(targeting_of(s)),
        deadline_ms: DEADLINE_MS,
        history_head: Some(history_head(s)),
    }
}

/// The shots on V1 of the active sequence, in timeline order.
fn v1_clips(s: &Session) -> Vec<ClipId> {
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    seq.video_tracks.first().expect("the demo has a V1").items.iter().map(|clip| clip.id).collect()
}

/// The timeline durations of a shot and of the sound clip linked to it.
fn durations(s: &Session, clip: ClipId) -> (Tick, Tick) {
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let (_, picture) = seq.find_item(clip).expect("the shot is on the timeline");
    let link = picture.link.expect("the demo's shots are linked to their sound");
    let sound = seq.audio_tracks.iter().flat_map(|track| track.items.iter()).find(|item| item.link == Some(link)).expect("the linked sound clip");
    (picture.duration, sound.duration)
}

/// The demo with the sync lock of A2 off, as the engine's own ripple tests set it up: A2 holds the
/// music bed under the whole edit, which on a sync-locked track is in the way of every ripple that
/// closes time.
fn demo_without_music_sync_lock() -> Session {
    let mut s = demo();
    s.execute("timeline.setTrack", json!({"track": "A2", "syncLock": false})).expect("the sync lock of A2 turns off");
    let a2 = s.active_sequence().and_then(|seq| seq.audio_tracks.get(1)).expect("the demo has an A2");
    assert!(!a2.sync_lock, "A2 is no longer sync-locked");
    s
}

/// The parameter keys the policy table allows for `command`.
fn policy_keys(command: &str) -> BTreeSet<&'static str> {
    policy::POLICY.iter().filter(|entry| entry.command == command).flat_map(|entry| entry.keys.iter().copied()).collect()
}

/// The keys the policy table requires for `command`.
fn required_keys(command: &str) -> BTreeSet<&'static str> {
    policy::POLICY.iter().filter(|entry| entry.command == command).flat_map(|entry| entry.required.iter().copied()).collect()
}

/// The entry of `command` in the policy table.
fn entry_of(command: &str) -> &'static policy::Entry {
    policy::POLICY.iter().find(|entry| entry.command == command).unwrap_or_else(|| panic!("the table holds `{command}`"))
}

/// The keys a need names: every key of every group, the keys that come together, the key whose
/// value decides and the key that value needs, or the key whose value decides and the keys it excludes.
fn need_keys(need: &policy::Need) -> Vec<&'static str> {
    match need {
        policy::Need::AnyOf(groups) => groups.iter().flat_map(|group| group.iter().copied()).collect(),
        policy::Need::Together(keys) => keys.to_vec(),
        policy::Need::When { key, then, .. } => vec![*key, *then],
        policy::Need::Without { key, others, .. } => std::iter::once(*key).chain(others.iter().copied()).collect(),
    }
}

/// A need as [`NEEDS`] writes it.
fn need_text(need: &policy::Need) -> String {
    let condition = |holds: &policy::Rule| match holds {
        policy::Rule::Id => "an index".to_string(),
        policy::Rule::True => "true".to_string(),
        policy::Rule::OneOf { both, native: [] } => both.join("|"),
        other => format!("{other:?}"),
    };
    match need {
        policy::Need::AnyOf(groups) => groups.iter().map(|group| group.join("+")).collect::<Vec<String>>().join("|"),
        policy::Need::Together(keys) => format!("{} or neither", keys.join("+")),
        policy::Need::When { key, holds, then } => format!("{then} if {key} is {}", condition(holds)),
        policy::Need::Without { key, holds, others } => format!("no {} if {key} is {}", others.join("+"), condition(holds)),
    }
}

/// Whether `value` meets the condition `rule` of a need: an index, a switch that is on, or one of
/// some names.
fn meets(rule: &policy::Rule, value: &Value) -> bool {
    match rule {
        policy::Rule::Id => value.as_u64().is_some(),
        policy::Rule::True => value.as_bool() == Some(true),
        policy::Rule::OneOf { both, native } => value.as_str().is_some_and(|name| both.contains(&name) || native.contains(&name)),
        other => panic!("no need of the table depends on a {other:?} value"),
    }
}

/// A value that meets the condition `rule` of a need ([`meets`]).
fn condition_value(rule: &policy::Rule) -> Value {
    match rule {
        policy::Rule::Id => json!(0),
        policy::Rule::True => json!(true),
        policy::Rule::OneOf { both, .. } => json!(both.first().expect("a condition names a value")),
        other => panic!("no need of the table depends on a {other:?} value"),
    }
}

/// The rule that limits `key` in `entry`, if one does.
fn rule_of(entry: &policy::Entry, key: &str) -> Option<&'static policy::Rule> {
    entry.rules.iter().find(|(name, _)| *name == key).map(|(_, rule)| rule)
}

/// The values `entry` limits `key` to, as (anywhere, native only), when a `OneOf` rule limits it.
fn one_of(entry: &policy::Entry, key: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    match rule_of(entry, key)? {
        policy::Rule::OneOf { both, native } => Some((*both, *native)),
        _ => None,
    }
}

/// A value `rule` takes in both runtimes: the first such string, a file under the export root, or a
/// value of the rule's type.
fn value_anywhere(rule: &policy::Rule) -> Value {
    match rule {
        policy::Rule::OneOf { both, .. } => json!(both.first().expect("a limited key takes some value in both runtimes")),
        policy::Rule::ExportPath => json!(format!("{}t-fi-12/out.mp4", policy::EXPORT_ROOT)),
        policy::Rule::Ticks => json!(0),
        policy::Rule::Within(min, _) => json!(min),
        policy::Rule::Between(min, _) => json!(min),
        policy::Rule::Seconds => json!(1.0),
        policy::Rule::Id => json!(1),
        policy::Rule::Ids => json!([1]),
        policy::Rule::IdOrName => json!("V1"),
        policy::Rule::Bool => json!(false),
        policy::Rule::True => json!(true),
        policy::Rule::Text => json!(""),
        policy::Rule::Name => json!("Anchor"),
        policy::Rule::Object(fields) => Value::Object(Map::from_iter([(fields.first().unwrap_or(&"value").to_string(), json!(1))])),
        policy::Rule::Props => json!({"opacity": 50.0, "enabled": true}),
        policy::Rule::Values => json!({"name": "Anchor"}),
        policy::Rule::Transcript => json!({"language": "en", "words": [{"text": "Hello", "start": 0, "end": 1, "speaker": 0}]}),
        policy::Rule::Given => json!(1),
        policy::Rule::Point => json!([960.0, 540.0]),
        policy::Rule::Points => json!([[0.0, 0.0], [100.0, 0.0], [0.0, 100.0]]),
        policy::Rule::Moves => json!([{"clip": 1, "track": "V1", "time": 0}]),
        policy::Rule::IndexOr(names) => json!(names.first().expect("a position takes a name")),
        policy::Rule::ChannelMap => json!([[0, 1]]),
    }
}

/// Values `rule` takes in neither runtime: other types (any value but null is a template value),
/// and the near misses of its own.
fn values_refused(rule: &policy::Rule) -> Vec<Value> {
    let mut values = if *rule == policy::Rule::Given { vec![Value::Null] } else { vec![Value::Null, json!(["a"]), json!({})] };
    let far = 2.0 * policy::MAX_COORDINATE;
    let past = policy::MAX_TICKS + 1;
    values.extend(match rule {
        policy::Rule::OneOf { .. } | policy::Rule::ExportPath => vec![json!(""), json!("not a value it takes"), json!(1), json!(true)],
        policy::Rule::Ticks => vec![json!("0"), json!(1.5), json!(u64::MAX), json!(true), json!(i64::MIN), json!(i64::MAX), json!(past), json!(-past)],
        policy::Rule::Within(min, max) => vec![json!(min - 1), json!(max + 1), json!("1"), json!(1.5), json!(true)],
        policy::Rule::Between(min, max) => vec![json!(min - 1.0), json!(max + 1.0), json!("1"), json!(true)],
        policy::Rule::Seconds => vec![json!(-1.0), json!(-0.1), json!(policy::MAX_SECONDS + 1.0), json!(u64::MAX), json!("1"), json!(true)],
        policy::Rule::Id => vec![json!(""), json!("1"), json!(-1), json!(1.5), json!(true)],
        policy::Rule::Ids => vec![json!([]), json!([1.5]), json!([-1]), json!(["1"]), json!(1), json!("1"), json!(true)],
        policy::Rule::IdOrName => vec![json!(""), json!(-1), json!(1.5), json!(true)],
        policy::Rule::Bool => vec![json!(""), json!("true"), json!(0), json!(1)],
        policy::Rule::True => vec![json!(false), json!("true"), json!(1)],
        policy::Rule::Text => vec![json!(1), json!(true)],
        policy::Rule::Name => vec![json!(""), json!("   "), json!(1), json!(true)],
        policy::Rule::Object([]) => vec![json!("value"), json!(1), json!(true)],
        policy::Rule::Object(_) => vec![json!({"notAField": 1}), json!("value"), json!(1), json!(true)],
        policy::Rule::Props => vec![json!({"enabled": "false"}), json!({"enabled": null}), json!({"opacity": null}), json!("value"), json!(1)],
        policy::Rule::Values => vec![json!({"Show Role": null}), json!("value"), json!(1), json!(true)],
        policy::Rule::Transcript => transcripts_refused(),
        policy::Rule::Given => Vec::new(),
        policy::Rule::Point => vec![json!([1.0]), json!([1.0, 2.0, 3.0]), json!(["1", 2.0]), json!([far, 0.0]), json!([0.0, -far]), json!(1)],
        policy::Rule::Points => vec![json!([]), json!([[0, 0], [1, 1]]), json!([[0, 0], [1, 1], [far, 0]]), json!([[0, 0], [1, 1], "x"]), json!(1)],
        policy::Rule::Moves => moves_refused(),
        policy::Rule::IndexOr(_) => vec![json!(""), json!("not a value it takes"), json!(-1), json!(1.5), json!(u64::from(u32::MAX) + 1), json!(true)],
        policy::Rule::ChannelMap => {
            let (past, many) = (policy::MAX_CHANNELS, Value::Array(vec![json!([0]); policy::MAX_CHANNELS as usize + 1]));
            let long = Value::Array(vec![json!([0, 1]), json!(vec![0; policy::MAX_CHANNELS as usize + 1])]);
            vec![json!([]), json!([[]]), json!([0, 1]), json!([[past]]), json!([[-1]]), json!([[1.5]]), json!([["0"]]), json!(0), many, long]
        }
    });
    values
}

/// Moves `timeline.move` takes in neither runtime: none, one missing its time, one with another key,
/// and one with a clip, a track or a time of another type, or a time past the engine's `Tick::MAX`.
fn moves_refused() -> Vec<Value> {
    vec![
        json!([]),
        json!([{"clip": 1, "track": "V1"}]),
        json!([{"clip": 1, "track": "V1", "time": 0, "insert": true}]),
        json!([{"clip": "1", "track": "V1", "time": 0}]),
        json!([{"clip": 1, "track": "", "time": 0}]),
        json!([{"clip": 1, "track": "V1", "time": 1.5}]),
        json!([{"clip": 1, "track": "V1", "time": i64::MAX}]),
        json!({"clip": 1, "track": "V1", "time": 0}),
    ]
}

/// Transcripts `transcript.set` takes in neither runtime: one without a word (the engine would clear
/// the item's transcript), a field the engine's record does not have, and words with a blank or
/// missing text, a time of another type, missing or past the engine's `Tick::MAX`, an end before the
/// start or more than ten minutes after it, or a speaker index past [`policy::MAX_SPEAKER`].
fn transcripts_refused() -> Vec<Value> {
    let word = |fields: Value| json!({"words": [fields]});
    vec![
        json!({"language": "en"}),
        json!({"words": []}),
        json!({"words": "Hello"}),
        json!({"words": [{"text": "Hello", "start": 0, "end": 1}], "notAField": 1}),
        word(json!({"text": " ", "start": 0, "end": 1})),
        word(json!({"start": 0, "end": 1})),
        word(json!({"text": "Hello", "start": "0", "end": 1})),
        word(json!({"text": "Hello", "start": 0})),
        word(json!({"text": "Hello", "start": i64::MAX, "end": i64::MAX})),
        word(json!({"text": "Hello", "start": 2, "end": 1})),
        word(json!({"text": "Hello", "start": 0, "end": policy::MAX_WORD_TICKS + 1})),
        word(json!({"text": "Hello", "start": 0, "end": 1, "speaker": policy::MAX_SPEAKER + 1})),
        word(json!({"text": "Hello", "start": 0, "end": 1, "speaker": -1})),
        json!("Hello"),
    ]
}

/// A value of `kind`.
fn kind_value(kind: policy::Kind) -> Value {
    match kind {
        policy::Kind::Number => json!(120.0),
        policy::Kind::Point => json!([960.0, 540.0]),
        policy::Kind::Color => json!("#ff8000"),
        policy::Kind::Switch => json!(true),
        policy::Kind::Choice(options) => json!(options.first().expect("a choice has options")),
    }
}

/// The kind of setting a parameter of a transition takes, as the table writes it from the engine's own kind (`None`:
/// a kind no setting takes, such as a text).
fn kind_of(kind: &ParamKind) -> Option<policy::Kind> {
    match kind {
        ParamKind::Float { .. } | ParamKind::Angle => Some(policy::Kind::Number),
        ParamKind::Point => Some(policy::Kind::Point),
        ParamKind::Color => Some(policy::Kind::Color),
        ParamKind::Bool => Some(policy::Kind::Switch),
        ParamKind::Choice(options) => Some(policy::Kind::Choice(options)),
        _ => None,
    }
}

/// The value [`valid_params`] gives `key` of `entry`: the first effect parameter it reaches (`effect`,
/// `param`, `value`), a value its rule takes in both runtimes, or null for a key nothing limits.
fn param_value(entry: &policy::Entry, key: &str) -> Value {
    let target = entry.targets.first().map(|(effect, params)| (*effect, params.first().copied()));
    match (key, target) {
        ("effect", Some((effect, _))) => json!(effect),
        ("param", Some((_, Some((param, _))))) => json!(param),
        ("value", Some((_, Some((_, kind))))) => kind_value(kind),
        ("params", _) if !entry.settings.is_empty() => settings_value(entry),
        _ => rule_of(entry, key).map_or(Value::Null, value_anywhere),
    }
}

/// The settings [`param_value`] gives a transition: the first setting of the transition the entry's
/// `effect` value names, with a value of its kind (none when it has none).
fn settings_value(entry: &policy::Entry) -> Value {
    let effect = param_value(entry, "effect");
    let settings = entry.settings.iter().find(|(id, _)| effect.as_str() == Some(*id)).map(|(_, settings)| *settings).unwrap_or_default();
    Value::Object(settings.iter().take(1).map(|(name, kind)| (name.to_string(), kind_value(*kind))).collect())
}

/// Parameters `entry` takes in both runtimes, apart from where it runs: every key it allows (`all`),
/// or only the keys it requires and the first group of each of its either-or needs, each with the
/// value [`param_value`] gives it; a key that one value of another key needs comes along when that
/// value is there.
fn valid_params(entry: &policy::Entry, all: bool) -> Map<String, Value> {
    let mut keys: Vec<&str> = if all { entry.keys.to_vec() } else { entry.required.to_vec() };
    if !all {
        for need in entry.needs {
            if let policy::Need::AnyOf(groups) = need {
                keys.extend(groups.first().into_iter().flat_map(|group| group.iter().copied()));
            }
        }
    }
    let mut params: Map<String, Value> = keys.iter().map(|key| (key.to_string(), param_value(entry, key))).collect();
    for need in entry.needs {
        if let policy::Need::When { key, holds, then } = need
            && params.get(*key).is_some_and(|value| meets(holds, value))
        {
            params.insert(then.to_string(), param_value(entry, then));
        }
    }
    params
}

/// The names a command's params doc writes: every run of letters, digits, `.`, `_` and `-`.
fn doc_names(doc: &str) -> BTreeSet<&str> {
    doc.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))).filter(|name| !name.is_empty()).collect()
}

/// Whether the engine's own parser reads `name` as a value of `key` of `command`, where a name is not
/// in its command's params doc: transitions and effects by id, label colours, caption formats, mask
/// modes and tracking methods, time interpolations, audio types, export formats, bitrate modes and
/// MXF codecs.
fn parsed_by_engine(command: &str, key: &str, name: &str) -> bool {
    match (command, key) {
        ("sequence.applyVideoTransition", "effect") => find_effect(name).is_some_and(|def| def.kind == EffectKind::VideoTransition),
        ("sequence.applyAudioTransition", "effect") => find_effect(name).is_some_and(|def| def.kind == EffectKind::AudioTransition),
        ("effects.apply", "effect") => find_effect(name).is_some_and(|def| def.kind == EffectKind::Video),
        ("markers.add" | "markers.edit", "color") => filmcraft_project::Label::from_name(name).is_some_and(|label| label.name() == name),
        ("transcript.createCaptions" | "captions.newTrack", "format") => {
            filmcraft_project::CaptionFormat::from_name(name).is_some_and(|format| format.label() == name)
        }
        ("masks.set", "mode") => filmcraft_project::MaskMode::from_name(name).is_some(),
        ("masks.track", "method") => filmcraft_project::TrackMethod::from_name(name).is_some(),
        ("clip.speedDuration", "interpolation") => filmcraft_project::TimeInterpolation::from_name(name).is_some_and(|mode| mode.name() == name),
        ("essentialSound.setType", "type") => AudioType::parse(name).is_some_and(|parsed| parsed.id() == name),
        ("file.exportMedia", "format") => Format::from_name(name).is_some(),
        ("file.exportMedia", "bitrateMode") => serde_json::from_value::<filmcraft_export::BitrateMode>(json!(name)).is_ok(),
        ("file.exportMedia", "mxfVideoCodec") => serde_json::from_value::<filmcraft_export::MxfVideoCodec>(json!(name)).is_ok(),
        _ => false,
    }
}

/// The parameter keys a command's params doc names: every quoted name right before a `:`, with the
/// alternatives of one key (`"startTime|startFrame|…":`) split at `|`.
fn documented_keys(doc: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut rest = doc;
    while let Some(open) = rest.find('"') {
        let quoted = &rest[open + 1..];
        let Some(close) = quoted.find('"') else { break };
        let after = &quoted[close + 1..];
        if after.trim_start().starts_with(':') {
            keys.extend(quoted[..close].split('|').map(str::to_string));
        }
        rest = after;
    }
    keys
}

/// The documented keys ([`documented_keys`]) with the first word of the type their params doc gives
/// them: `"reverse":bool?` gives `reverse` and `bool`, `"time":ticks|…` gives `time` and `ticks`, and a
/// type that starts with no letter (a quoted value, a list) gives an empty word.
fn documented_types(doc: &str) -> Vec<(String, String)> {
    let mut types = Vec::new();
    let mut rest = doc;
    while let Some(open) = rest.find('"') {
        let quoted = &rest[open + 1..];
        let Some(close) = quoted.find('"') else { break };
        let after = &quoted[close + 1..];
        if let Some(typed) = after.trim_start().strip_prefix(':') {
            let word: String = typed.trim_start().chars().take_while(char::is_ascii_alphabetic).collect();
            types.extend(quoted[..close].split('|').map(|key| (key.to_string(), word.clone())));
        }
        rest = after;
    }
    types
}

/// The engine's serde view of the active sequence, built as the hook builds it for the keyframe,
/// template and ripple pre-checks (`agent_view.rs`): null when none is open.
fn sequence_view(s: &Session) -> Value {
    s.active_sequence().and_then(|sequence| serde_json::to_value(sequence).ok()).unwrap_or_default()
}

/// T-FI-11's host: one file served from memory, whose bytes can be held back the way the web host's
/// Blob reader holds back a range it is still fetching (`apps/filmcraft-web/src/fs.rs`): the read
/// fails with `WouldBlock` and marks `filmcraft_media::pending`.
struct LoadingHost {
    bytes: Arc<[u8]>,
    loading: Arc<AtomicBool>,
}

/// The host's random-access reader (on the web, `fs::BlobReader`).
struct LoadingReader {
    bytes: Arc<[u8]>,
    loading: Arc<AtomicBool>,
}

impl LoadingHost {
    fn open(&self, path: &str) -> io::Result<LoadingReader> {
        if path != STILL {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("{path}: not found")));
        }
        Ok(LoadingReader { bytes: self.bytes.clone(), loading: self.loading.clone() })
    }
}

impl ByteReader for LoadingReader {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if self.loading.load(Ordering::SeqCst) {
            return Err(filmcraft_media::pending::would_block());
        }
        let range = usize::try_from(offset).ok().and_then(|start| self.bytes.get(start..start.checked_add(buf.len())?));
        buf.copy_from_slice(range.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "read past the end of the file"))?);
        Ok(())
    }
}

impl Services for LoadingHost {
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        filmcraft_media::reader::read_range(&self.open(path)?, 0, self.bytes.len())
    }

    fn write_file(&self, path: &str, _data: &[u8]) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, format!("{path}: the test host is read-only")))
    }

    fn file_size(&self, path: &str) -> io::Result<u64> {
        self.open(path).map(|reader| reader.len())
    }

    fn read_range(&self, path: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        filmcraft_media::reader::read_range(&self.open(path)?, offset, len)
    }

    fn reader(&self, path: &str) -> Option<io::Result<SharedReader>> {
        Some(self.open(path).map(|reader| Arc::new(reader) as SharedReader))
    }
}

/// T-FI-11's session: a 64x36 25 fps sequence showing one still imported from the host, whose
/// opened media source is then forgotten, as when a project reopens before the browser has fetched
/// its media: the next render has to read the file.
fn still_on_timeline() -> (Session, Arc<AtomicBool>) {
    let mut png = Vec::new();
    let still = image::RgbaImage::from_pixel(64, 36, image::Rgba([200, 40, 160, 255]));
    still.write_to(&mut io::Cursor::new(&mut png), image::ImageFormat::Png).expect("the still encodes as PNG");
    let loading = Arc::new(AtomicBool::new(false));
    let mut s = Session::new(Arc::new(LoadingHost { bytes: png.into(), loading: loading.clone() }));
    s.execute("file.newSequence", json!({"name": "Pending media", "width": 64, "height": 36, "fps": 25})).expect("a 64x36 sequence opens");
    let imported = s.execute("file.import", json!({"paths": [STILL]})).expect("the host's still imports");
    let item = imported["items"][0].as_u64().unwrap_or_else(|| panic!("one imported item: {imported}"));
    let place = json!({"item": item, "track": "V1", "time": 0, "duration": 2 * TICKS_PER_SECOND, "insert": false});
    s.execute("timeline.place", place).expect("the still goes on V1");
    s.media.clear();
    (s, loading)
}

/// The clock of the replayed loops: one reading per attempt, 16 ms apart from 0 (the hook sleeps
/// 16 ms between attempts), so [`LOOP_DEADLINE_MS`] passes at the fifth reading.
fn clock_from_zero() -> impl FnMut() -> f64 {
    let mut now = -16.0;
    move || {
        now += 16.0;
        now
    }
}

/// The `renderFrame` fields besides the pixels (design Appendix A.2): the tick and the revision as
/// decimal strings (INV-32), the frame index, the active sequence, `complete` and `attempts`.
fn frame_report_of(s: &Session, frame: &filmcraft_render::Image, tick: Tick, complete: bool, attempts: u32) -> Value {
    json!({
        "width": frame.w,
        "height": frame.h,
        "tick": tick.0.to_string(),
        "frame": s.sequence_rate().frame_at(tick),
        "revision": s.revision.to_string(),
        "activeSequence": s.state.active_sequence.map(|id| id.0.to_string()),
        "complete": complete,
        "attempts": attempts,
    })
}

// ------------------------------------------------------------------------------------ FF1 hook paths

/// What `filmcraft.agentView.executeTyped(revision, activeSequence, command, params, {deadlineMs,
/// historyHead?, linkedSelection?, targeting?})` runs in its single UI-thread closure
/// (`apps/filmcraft-web/src/agent_view.rs`), with the clock and the person's input passed in
/// (`ui_busy`: a pointer held down, keyboard focus in a text field, an open dialog, modal or window,
/// or a pen or text draft on the monitor): the facts read just before dispatch (the track targeting
/// and the engine's own busy states among them), the policy's pre-dispatch decision in the browser,
/// the keyframe, template and ripple pre-checks over the serde view of the active sequence, then
/// `Session::execute` and the versioned envelope.
fn execute_typed(s: &mut Session, command: &str, params: Value, expect: &policy::Expect, now_ms: f64, ui_busy: bool) -> Value {
    execute_typed_in(policy::Runtime::Browser, s, command, params, expect, now_ms, ui_busy)
}

/// The sequence marks a typed marks edit reports after it runs (`agent_view.rs` reads the same):
/// the active sequence's In and Out marks and its frame duration, through `policy::marks_result`.
fn marks_of(s: &Session, command: &str) -> Option<Value> {
    let seq = s.active_sequence()?;
    policy::marks_result(command, seq.mark_in.map(|tick| tick.0), seq.mark_out.map(|tick| tick.0), s.sequence_rate().frame_duration().0)
}

/// [`execute_typed`] in `runtime`: a native worker compiles the same policy file and runs the same
/// closure; a typed marks edit answers the marks it leaves ([`marks_of`]) instead of the engine's
/// null result (slice R1).
fn execute_typed_in(runtime: policy::Runtime, s: &mut Session, command: &str, params: Value, expect: &policy::Expect, now_ms: f64, ui_busy: bool) -> Value {
    let observed = policy::Observed {
        now_ms,
        revision: s.revision,
        active_sequence: s.state.active_sequence.map(|id| id.0),
        linked_selection: s.state.linked_selection,
        targeting: targeting_of(s),
        busy: ui_busy || engine_busy(s),
        history_head: history_head(s),
    };
    if let Err(code) = policy::admit(command, &params, expect, &observed, runtime) {
        return policy::refused(code, observed.revision);
    }
    if let Some(code) = policy::keyframe_refusal(command, &params, || sequence_view(s)) {
        return policy::refused(code, observed.revision);
    }
    if let Some(code) = policy::template_refusal(command, &params, || sequence_view(s)) {
        return policy::refused(code, observed.revision);
    }
    let in_the_way = policy::sync_locked_tracks(command, &params, observed.linked_selection, || sequence_view(s));
    if !in_the_way.is_empty() {
        return policy::sync_locked(observed.revision, in_the_way);
    }
    let answer = match s.execute(command, params.clone()) {
        Ok(result) => policy::Answer::Ok(marks_of(s, command).unwrap_or(result)),
        Err(EngineError::Disabled(named, _)) => policy::Answer::Disabled(named),
        Err(error) => policy::Answer::Failed(error.to_string()),
    };
    let label = s.history.undo.last().map(|(label, _)| label.as_str());
    policy::settle(command, &params, observed.revision, s.revision, label, answer)
}

/// Checks an `executeTyped` envelope: the exact result version, the outcome, both revisions as
/// decimal strings, a refusal code exactly on `not_applied`, and no field outside the contract.
fn assert_envelope(envelope: &Value, outcome: &str, code: Option<&str>, before: u64, after: u64) {
    let fields = envelope.as_object().unwrap_or_else(|| panic!("an envelope is an object: {envelope}"));
    for field in fields.keys() {
        assert!(ENVELOPE_FIELDS.contains(&field.as_str()), "`{field}` is not an executeTyped field: {envelope}");
    }
    assert_eq!(policy::RESULT_VERSION, "filmcraft.typed-execute.v1");
    assert_eq!(envelope["resultVersion"], json!(policy::RESULT_VERSION), "{envelope}");
    assert_eq!(envelope["outcome"], json!(outcome), "{envelope}");
    assert_eq!(envelope["revisionBefore"], json!(before.to_string()), "{envelope}");
    assert_eq!(envelope["revisionAfter"], json!(after.to_string()), "{envelope}");
    match code {
        Some(code) => {
            assert_eq!(outcome, "not_applied", "only a refusal names a not_applied code");
            assert!(NOT_APPLIED_CODES.contains(&code), "`{code}` is not an executeTyped code");
            assert_eq!(envelope["code"], json!(code), "{envelope}");
        }
        None => {
            let code = envelope.get("code").and_then(Value::as_str);
            assert!(code.is_none_or(|code| !NOT_APPLIED_CODES.contains(&code)), "an {outcome} envelope names no refusal code: {envelope}");
        }
    }
}

/// One typed trim through [`execute_typed`], checked against the shot and its linked sound before
/// and after; answers the delta the edge moved, in ticks.
fn typed_trim(s: &mut Session, clip: ClipId, edge: &str, mode: &str, requested: i64) -> i64 {
    let (picture, sound) = durations(s, clip);
    let (expect, revision) = (expect_now(s), s.revision);
    let params = json!({"clip": clip.0, "edge": edge, "mode": mode, "delta": requested});
    let envelope = execute_typed(s, "timeline.trim", params, &expect, NOW_MS, false);
    let (picture_after, sound_after) = durations(s, clip);
    let moved = if edge == "in" { (picture - picture_after).0 } else { (picture_after - picture).0 };
    assert_eq!(sound_after - sound, picture_after - picture, "the linked sound moved with the picture ({edge} {mode} {requested})");
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    assert_eq!(envelope["result"], json!({"delta": moved}), "the engine answers the delta it applied: {envelope}");
    assert_eq!(envelope["actual"]["deltaTicks"], json!(moved.to_string()), "{envelope}");
    assert_eq!(envelope["intentSatisfied"], json!(moved == requested), "{envelope}");
    let label = if mode == "ripple" { "Ripple Trim" } else { "Trim" };
    assert_eq!(envelope["historyLabel"], json!(label), "{envelope}");
    moved
}

/// Checks that the ripple pre-check refuses the typed `command` with `params` on `s` before dispatch,
/// naming exactly `tracks` and running nothing; then that the engine itself, given the same edit,
/// refuses it from inside the command, naming one of them, and changes nothing but the Events log.
fn assert_sync_refused(s: &mut Session, command: &str, params: Value, tracks: &[&str]) {
    let (expect, before) = (expect_now(s), fingerprint(s));
    let envelope = execute_typed(s, command, params.clone(), &expect, NOW_MS, false);
    assert_envelope(&envelope, "not_applied", Some("sync_locked"), before.revision, before.revision);
    assert_eq!(envelope["syncLockedTracks"], json!(tracks), "`{command}` {params}: {envelope}");
    assert_eq!(fingerprint(s), before, "the refused `{command}` {params} ran nothing, the Events log included");
    let error = s.execute(command, params.clone()).expect_err("the engine refuses the same edit").to_string();
    assert!(tracks.iter().any(|track| error.contains(&format!("{track} is sync-locked"))), "`{command}` {params}: the engine says {error}");
    let after = fingerprint(s);
    assert_eq!(without_log(after.clone()), without_log(before.clone()), "the engine's refusal of `{command}` {params} changed nothing but the Events log");
    assert_eq!(after.log.len(), before.log.len() + 1, "the engine's refusal appends one Events line");
}

/// What `filmcraft.agentView.renderFrame({tick, maxSide, deadlineMs})` runs (`agent_view.rs`):
/// `scale = min(1, maxSide / long edge)` and the time snapped to its frame, then one UI-thread
/// closure per attempt (clear the pending flag, render the active sequence at that time without
/// moving the playhead, read the flag) until the verdict ends the loop. Between attempts the hook
/// awaits the host's fetches (`fs::idle`, then `fs::sleep(16)`), where `between` runs here; the
/// clock is read once per attempt, after the render.
fn render_frame(
    s: &Session,
    max_side: u32,
    at: Tick,
    deadline_ms: f64,
    mut clock: impl FnMut() -> f64,
    mut between: impl FnMut(u32),
) -> (filmcraft_render::Image, Value) {
    let seq = s.active_sequence().expect("an active sequence");
    let scale = (max_side as f32 / seq.settings.width.max(seq.settings.height) as f32).min(1.0);
    let rate = s.sequence_rate();
    let tick = rate.snap(at.max(Tick::ZERO));
    let mut attempts = 0;
    loop {
        attempts += 1;
        assert!(attempts <= MAX_ATTEMPTS, "the pending-media loop ends by its deadline");
        let _ = filmcraft_media::pending::take();
        let frame = s.try_render_program_at(scale, tick).expect("the active sequence renders");
        let pending = filmcraft_media::pending::take();
        if let Some(complete) = policy::attempt(pending, clock(), deadline_ms) {
            let meta = policy::FrameMeta {
                width: frame.w,
                height: frame.h,
                tick: tick.0,
                frame: rate.frame_at(tick),
                revision: s.revision,
                active_sequence: s.state.active_sequence.map(|id| id.0),
            };
            return (frame, policy::frame_report(&meta, complete, attempts));
        }
        between(attempts);
    }
}

/// What `filmcraft.agentView.scopes({tick, scopes, maxSide})` runs before it measures: the same loop
/// around `scopes::scope_signal` at the snapped time (the signal neither snaps nor bounds; the hook
/// does both). Answers the last signal, whether it is complete, and the attempts.
fn scope_attempts(s: &Session, at: Tick, deadline_ms: f64, mut clock: impl FnMut() -> f64, mut between: impl FnMut(u32)) -> (Signal, bool, u32) {
    let tick = s.sequence_rate().snap(at.max(Tick::ZERO));
    let mut attempts = 0;
    loop {
        attempts += 1;
        assert!(attempts <= MAX_ATTEMPTS, "the pending-media loop ends by its deadline");
        let _ = filmcraft_media::pending::take();
        let signal = scope_signal(s, tick, 1.0, false).expect("an active sequence has a scope signal");
        let pending = filmcraft_media::pending::take();
        if let Some(complete) = policy::attempt(pending, clock(), deadline_ms) {
            return (signal, complete, attempts);
        }
        between(attempts);
    }
}

// ------------------------------------------------------------------------------------ FF1 certification

/// T-FI-10a. Every `not_applied` that `executeTyped` reports is decided before the command runs.
/// The refusals the hook decides itself come in a fixed order (deadline, revision, active sequence,
/// linked selection, track targeting, busy, history head, policy table) and leave the session
/// exactly as it was, the Events log included. The engine refuses a disabled command
/// (`EngineError::Disabled`, raised by `Session::execute` before `run`) at the cost of one Warning
/// line and nothing else: that is `command_disabled`. A disabled error naming another command (a
/// nested `execute`), a revision that moved, or any other engine error is `unknown`. Live trim
/// playback is busy, because `Session::execute` settles it before its enabled check, even for a
/// command it then refuses as disabled; so is a recording pass the person runs (a voice-over take,
/// an automation pass of the Audio Track Mixer that writes, a multi-camera switching pass), which the
/// engine would let an edit land in. The automation pass the editor runs through every forward
/// playback at normal speed, which writes nothing while every strip reads, is not busy: a typed edit
/// during plain playback runs. Undo and redo need the caller's own history head (INV-34).
///
/// Fallback if this fails (INV-4, INV-41): `executeTyped` reports every engine error as `unknown`,
/// and only the adapter's pre-checks after certified reads (L1) mint a Film `not_applied`.
#[test]
fn disabled_command_errors_before_running() {
    // the refusals the hook decides itself, in order: each round makes one more precondition hold
    let order = [
        "deadline_elapsed",
        "revision_conflict",
        "sequence_changed",
        "linked_selection_changed",
        "targeting_changed",
        "editor_busy",
        "history_head_conflict",
        "command_not_allowed",
    ];
    for holding in 0..=order.len() {
        let mut s = demo();
        let right = expect_now(&s);
        let expect = policy::Expect {
            deadline_ms: if holding > 0 { right.deadline_ms } else { NOW_MS - 1.0 },
            revision: if holding > 1 { right.revision } else { right.revision + 1 },
            active_sequence: if holding > 2 { right.active_sequence } else { Some(UNKNOWN_ITEM) },
            linked_selection: if holding > 3 { right.linked_selection } else { Some(!s.state.linked_selection) },
            targeting: if holding > 4 { right.targeting } else { Some("a targeting nobody read".into()) },
            history_head: if holding > 6 { right.history_head } else { Some(policy::HistoryHead { length: 7, label: "a step nobody took".into() }) },
        };
        let (pointer_down, command) = (holding < 6, if holding < 8 { "file.save" } else { "sequence.lift" });
        let before = fingerprint(&s);
        let envelope = execute_typed(&mut s, command, json!({}), &expect, NOW_MS, pointer_down);
        match order.get(holding).copied() {
            Some(code) => {
                assert_envelope(&envelope, "not_applied", Some(code), before.revision, before.revision);
                assert_eq!(fingerprint(&s), before, "`{code}` is decided before dispatch: nothing ran, the Events log included");
            }
            // every precondition holds: the command reaches the engine, which finds no In and Out points
            None => assert_envelope(&envelope, "not_applied", Some("command_disabled"), before.revision, before.revision),
        }
    }

    // the deadline: a dispatch at the deadline itself is in time; a clock or a deadline that is not
    // a finite number never is
    let deadlines = [
        (NOW_MS, NOW_MS, "command_disabled"),
        (NOW_MS, f64::NAN, "deadline_elapsed"),
        (NOW_MS, f64::INFINITY, "deadline_elapsed"),
        (f64::NAN, DEADLINE_MS, "deadline_elapsed"),
    ];
    for (now_ms, deadline_ms, code) in deadlines {
        let mut s = demo();
        let (expect, revision) = (policy::Expect { deadline_ms, ..expect_now(&s) }, s.revision);
        let envelope = execute_typed(&mut s, "sequence.lift", json!({}), &expect, now_ms, false);
        assert_envelope(&envelope, "not_applied", Some(code), revision, revision);
    }

    // disabled commands: the engine refuses them before `run`, at the cost of one Warning line
    let disabled_commands = [
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("sequence.lift", json!({})),
        ("sequence.extract", json!({})),
        ("edit.rippleDelete", json!({"clips": [UNKNOWN_ITEM]})),
        ("clip.enable", json!({})),
        ("clip.enable", json!({"clips": [UNKNOWN_ITEM]})),
        ("edit.duplicate", json!({})),
    ];
    for (command, params) in disabled_commands {
        let mut s = demo();
        let spec = find_command(command).unwrap_or_else(|| panic!("`{command}` is a registered command"));
        let why = (spec.enabled)(&s).expect_err("disabled on the idle demo: nothing selected, no In or Out point, no history");
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let envelope = execute_typed(&mut s, command, params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("command_disabled"), before.revision, before.revision);
        let after = fingerprint(&s);
        assert_eq!(without_log(after.clone()), without_log(before.clone()), "`{command}` {params} changed more than the Events log");
        assert_eq!(after.log.len(), before.log.len() + 1, "`{command}` {params} appends one Events line");
        let line = after.log.last().expect("the appended line");
        let message = format!("command `{command}` is not available right now: {why}");
        assert_eq!((line.level, line.source.as_str(), line.message.as_str(), line.count), (Level::Warning, command, message.as_str(), 1));
    }

    // busy: a pointer held down in the editor, a live dynamic trim, a trim-mode loop or a recording
    // pass of the person's refuses every typed command before dispatch
    let busy = [
        ("pointer down", demo(), true),
        ("live dynamic trim", demo_with_dynamic_trim(), false),
        ("play-around loop", demo_with_play_around(), false),
        ("voice-over take", demo_recording("audio.voiceover.start"), false),
        ("automation write pass", demo_recording("mixer.recordStart"), false),
        ("multi-camera pass", demo_recording("multicam.recordStart"), false),
    ];
    for (fixture, mut s, pointer_down) in busy {
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        for entry in policy::POLICY.iter() {
            let envelope = execute_typed(&mut s, entry.command, json!({}), &expect, NOW_MS, pointer_down);
            assert_envelope(&envelope, "not_applied", Some("editor_busy"), before.revision, before.revision);
            assert_eq!(fingerprint(&s), before, "`{}` was refused as busy on the {fixture} fixture without touching it", entry.command);
        }
        if s.trim_play.active() {
            // why busy comes first: `Session::execute` settles trim playback before its enabled
            // check, so dispatching a command that it then refuses as disabled has changed the session
            let dispatched = s.execute("sequence.lift", json!({}));
            assert!(matches!(&dispatched, Err(EngineError::Disabled(id, _)) if id == "sequence.lift"), "{dispatched:?}");
            assert!(!s.trim_play.active(), "dispatching settled the trim playback of the {fixture} fixture");
            assert_ne!(without_log(fingerprint(&s)), without_log(before), "a disabled command left the {fixture} fixture as it was");
        } else if !pointer_down {
            // a recording pass: the engine itself lets an edit land in the person's pass and keeps
            // recording, so only the hook keeps an agent's edit out of it
            let landed = s.execute("markers.add", json!({"time": 0, "name": "an edit during the pass"}));
            assert!(landed.is_ok() && engine_busy(&s), "the engine ran the edit during the {fixture} and kept recording: {landed:?}");
        }
    }
    // the automation pass the editor starts whenever playback runs forward at normal speed writes
    // nothing while every strip reads: plain playback is not busy, a typed edit runs, the pass goes on
    let mut s = demo();
    s.execute("mixer.recordStart", json!({})).expect("the editor's automation pass starts");
    assert!(s.mixrec.active() && !engine_busy(&s), "a pass that writes nothing is not busy");
    let (expect, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "markers.add", json!({"time": 0, "name": "an edit during playback"}), &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    assert!(s.mixrec.active() && s.mixrec.gestures.is_empty(), "the pass goes on and still writes nothing");

    // only the dispatched command's own disabled error, with its revision unchanged, is a refusal
    let (r, no_params) = (41, json!({}));
    let disabled = |command: &str| policy::Answer::Disabled(command.to_string());
    assert_envelope(&policy::settle("sequence.lift", &no_params, r, r, None, disabled("sequence.lift")), "not_applied", Some("command_disabled"), r, r);
    // `lumetri.setSection` may dispatch `effects.apply` first: a disabled error from that proves nothing
    assert_envelope(&policy::settle("lumetri.setSection", &no_params, r, r, None, disabled("effects.apply")), "unknown", None, r, r);
    assert_envelope(&policy::settle("sequence.lift", &no_params, r, r + 1, Some("Lift"), disabled("sequence.lift")), "unknown", None, r, r + 1);
    let failed = policy::Answer::Failed("edit failed: no such clip".into());
    assert_envelope(&policy::settle("timeline.trim", &no_params, r, r, None, failed), "unknown", None, r, r);
    // on the engine: trimming a shot that is not there fails inside `run`, so it is `unknown`
    let mut s = demo();
    let (expect, before) = (expect_now(&s), fingerprint(&s));
    let missing = json!({"clip": UNKNOWN_ITEM, "edge": "out", "mode": "regular", "delta": 1});
    let envelope = execute_typed(&mut s, "timeline.trim", missing, &expect, NOW_MS, false);
    assert_envelope(&envelope, "unknown", None, before.revision, before.revision);
    assert_eq!(without_log(fingerprint(&s)), without_log(before), "the failed trim changed nothing but the Events log");

    // undo and redo need the caller's own history head: the one its last edit produced (INV-34)
    let mut s = demo();
    let last = *v1_clips(&s).last().expect("the demo has shots on V1");
    let shorter = json!({"clip": last.0, "edge": "out", "mode": "regular", "delta": -s.sequence_rate().frame_duration().0});
    let expect = expect_now(&s);
    assert_eq!(execute_typed(&mut s, "timeline.trim", shorter, &expect, NOW_MS, false)["outcome"], json!("applied"));
    let mine = history_head(&s);
    assert_eq!(mine, policy::HistoryHead { length: 1, label: "Trim".into() });
    let stale = [
        ("edit.undo", None),
        ("edit.undo", Some(policy::HistoryHead { length: mine.length, label: "Dynamic Trim".into() })),
        ("edit.undo", Some(policy::HistoryHead { length: mine.length + 1, label: mine.label.clone() })),
        ("edit.redo", None),
    ];
    for (command, head) in stale {
        let (expect, before) = (policy::Expect { history_head: head, ..expect_now(&s) }, fingerprint(&s));
        let envelope = execute_typed(&mut s, command, json!({}), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("history_head_conflict"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "`{command}` was refused before dispatch");
    }
    let (expect, revision) = (expect_now(&s), s.revision);
    let undone = execute_typed(&mut s, "edit.undo", json!({}), &expect, NOW_MS, false);
    assert_envelope(&undone, "applied", None, revision, revision + 1);
    assert_eq!(undone["result"], json!({"undone": "Trim"}), "{undone}");
    assert!(undone.get("historyLabel").is_none_or(Value::is_null), "nothing is left to undo: {undone}");
    let expect = expect_now(&s);
    let redone = execute_typed(&mut s, "edit.redo", json!({}), &expect, NOW_MS, false);
    assert_envelope(&redone, "applied", None, revision + 1, revision + 2);
    assert_eq!((&redone["result"], &redone["historyLabel"]), (&json!({"redone": "Trim"}), &json!("Trim")), "{redone}");
}

/// T-FI-10b. A typed trim reports what it applied. `timeline.trim` answers the delta by which the
/// shot's edge, and its linked sound's, actually moved; the envelope turns it into
/// `actual.deltaTicks` (a decimal string, INV-32), with `intentSatisfied` true exactly when that is
/// the requested delta, and names the undo step. A trim clamped by the next shot or by the end of
/// the media is applied, not refused, and one with nothing left to give is still one undo step and
/// one revision.
///
/// A ripple that would close time under a clip on a sync-locked track (or leave a clip there no
/// room to move back) is refused before dispatch by the ripple pre-check of slice R0: `not_applied`,
/// `sync_locked`, naming the tracks in its way in `syncLockedTracks`, with nothing run (no engine
/// call: the Events log is unchanged too) and the revision unchanged. That replaces the `unknown`
/// (`engine_error_after_dispatch`) FF1 pinned for the sync-locked demo ripple: the engine refuses
/// such a ripple from inside the command (`run`, the mutation entry point), so after dispatch it
/// could never be `not_applied` (INV-4). The engine still refuses each ripple the pre-check refuses
/// here, which pins that the pre-check stands for a real refusal; a ripple that opens time is never
/// in anything's way. The same holds for the table's other ripples that can meet that refusal: a
/// ripple delete (of the listed clips and, with linked selection on, their linked partners) and a
/// closed gap (named by its track and a time in it); one with nothing in its way runs. A closed gap
/// at a time a clip covers is left to the engine, which refuses it from inside the command: a known
/// `unknown`.
///
/// Fallback if this fails (INV-41): typed trims take `actual` from a certified `sequence.inspect`
/// read-back after the trim, as the L1 clamp rule does, instead of from the engine's answer; if only
/// the pre-check fails, a ripple blocked by a sync-locked track stays `unknown`, as in FF1.
#[test]
fn trim_reports_the_applied_delta() {
    let frame = demo().sequence_rate().frame_duration().0;
    // one frame on either edge, in both modes: exactly as requested (the ripple runs with the sync
    // lock of A2 off: with it on, the music bed there is in its way, below)
    for (shot, edge, mode) in [(2, "in", "regular"), (5, "out", "regular"), (5, "out", "ripple")] {
        let mut s = if mode == "ripple" { demo_without_music_sync_lock() } else { demo() };
        let clip = v1_clips(&s)[shot];
        let requested = if edge == "in" { frame } else { -frame };
        assert_eq!(typed_trim(&mut s, clip, edge, mode, requested), requested, "shot {shot}: {edge} edge, {mode}");
    }
    // the same ripple on the demo as it opens: A2 is sync-locked and its music bed runs under the
    // frame the ripple would close
    let mut s = demo();
    let last = v1_clips(&s)[5];
    assert_sync_refused(&mut s, "timeline.trim", json!({"clip": last.0, "edge": "out", "mode": "ripple", "delta": -frame}), &["A2"]);
    // a longer last shot opens time: nothing is in its way, so the demo as it opens ripples it
    let mut s = demo();
    let last = v1_clips(&s)[5];
    assert_eq!(typed_trim(&mut s, last, "out", "ripple", frame), frame, "a longer last shot ripples with A2 sync-locked");
    // a shorter head of the third shot closes the frame after its cut, which both the overlay on V2
    // and the music bed on A2 cover
    let mut s = demo();
    let third = v1_clips(&s)[2];
    assert_sync_refused(&mut s, "timeline.trim", json!({"clip": third.0, "edge": "in", "mode": "ripple", "delta": frame}), &["V2", "A2"]);
    // with linked selection off the third shot's sound keeps its length, so the next sound on
    // sync-locked A1 has no room to move back (A2's sync lock is off here)
    let mut s = demo_without_music_sync_lock();
    s.execute("sequence.linkedSelection", json!({"on": false})).expect("linked selection turns off");
    let third = v1_clips(&s)[2];
    assert_sync_refused(&mut s, "timeline.trim", json!({"clip": third.0, "edge": "out", "mode": "ripple", "delta": -frame}), &["A1"]);

    // a ripple delete closes the time its clips covered: the first shot's and, with linked selection
    // on, its sound's, which the music bed on sync-locked A2 covers (the engine enables a ripple
    // delete only while something is selected, so the shot is)
    let mut s = demo();
    let first = v1_clips(&s)[0];
    s.state.selection = vec![first];
    assert_sync_refused(&mut s, "edit.rippleDelete", json!({"clips": [first.0]}), &["A2"]);
    // the third shot's time is under the overlay on V2 as well (A2's sync lock is off here)
    let mut s = demo_without_music_sync_lock();
    let third = v1_clips(&s)[2];
    s.state.selection = vec![third];
    assert_sync_refused(&mut s, "edit.rippleDelete", json!({"clips": [third.0]}), &["V2"]);
    // with linked selection off the last shot's own sound stays on sync-locked A1, in the way
    let mut s = demo_without_music_sync_lock();
    s.execute("sequence.linkedSelection", json!({"on": false})).expect("linked selection turns off");
    let last = v1_clips(&s)[5];
    s.state.selection = vec![last];
    assert_sync_refused(&mut s, "edit.rippleDelete", json!({"clips": [last.0]}), &["A1"]);
    // with linked selection on, nothing is in the way of the last shot and its sound: it runs
    let mut s = demo_without_music_sync_lock();
    let last = v1_clips(&s)[5];
    s.state.selection = vec![last];
    let (expect, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "edit.rippleDelete", json!({"clips": [last.0]}), &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    assert_eq!(envelope["historyLabel"], json!("Ripple Delete"), "{envelope}");
    assert!(!v1_clips(&s).contains(&last), "the last shot is gone");

    // a closed gap: the empty start of V2, before the overlay, which every sync-locked track with a
    // clip there is in the way of; the track is named as the engine names it, or by its id (each on
    // a fresh demo: the same engine failure twice folds into one Events line)
    let second = TICKS_PER_SECOND;
    for by_id in [false, true] {
        let mut s = demo();
        let v2 = s.active_sequence().and_then(|seq| seq.video_tracks.get(1)).map(|track| track.id.0).expect("the demo has a V2");
        let track = if by_id { json!(v2) } else { json!("V2") };
        assert_sync_refused(&mut s, "sequence.closeGap", json!({"track": track, "time": 2 * second}), &["V1", "A1", "A2"]);
    }
    // with those sync locks off, the gap closes: the overlay moves to the start
    let mut s = demo();
    for track in ["V1", "A1", "A2"] {
        s.execute("timeline.setTrack", json!({"track": track, "syncLock": false})).expect("the sync lock turns off");
    }
    let (expect, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "sequence.closeGap", json!({"track": "v2", "time": 2 * second}), &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    let overlay_start = s.active_sequence().and_then(|seq| seq.video_tracks.get(1)).and_then(|track| track.items.first()).map(|clip| clip.start);
    assert_eq!(overlay_start, Some(Tick::ZERO), "the overlay moved to the start of V2");
    // a time the overlay covers has no gap to close: the pre-check leaves that to the engine, which
    // refuses it from inside the command (a known `unknown`)
    let mut s = demo();
    let (expect, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "sequence.closeGap", json!({"track": "V2", "time": 7 * second}), &expect, NOW_MS, false);
    assert_envelope(&envelope, "unknown", None, revision, revision);

    // the next shot leaves no room: nothing moves, and the trim is still applied
    let mut s = demo();
    let third = v1_clips(&s)[2];
    assert_eq!(typed_trim(&mut s, third, "out", "regular", frame), 0, "the third shot cannot grow into the fourth");
    // past the end of the media: clamped there; with no media left, nothing moves
    let mut s = demo();
    let last = v1_clips(&s)[5];
    let hour = 3600 * TICKS_PER_SECOND;
    let moved = typed_trim(&mut s, last, "out", "regular", hour);
    assert!(moved > 0 && moved < hour, "the last shot extends to the end of its media, not by an hour: {moved}");
    assert_eq!(typed_trim(&mut s, last, "out", "regular", hour), 0, "no media is left after the clamp");
}

/// T-FI-11. While the bytes of a clip's media are still being fetched (the host's reader fails
/// with `WouldBlock` and marks `filmcraft_media::pending`, as the web host's Blob reader does),
/// `try_render_program_at` and `scope_signal` still answer a picture, with the media pool's slate
/// where the clip belongs, and leave the pending flag set; the session does not change. The hook's
/// loop (per attempt: clear the flag, render, read the flag; retry after the host's fetches until
/// the deadline) therefore reports `complete:false` with its attempts when the bytes never arrive,
/// and the true picture with `complete:true` once they do (INV-20: a `complete:false` frame is never
/// presented as the truth).
///
/// Consequence if this fails: the Film adapter treats every `renderFrame` and `scopes` answer as
/// incomplete until FF3's fakes and QR-4F prove the loop live.
#[test]
fn render_and_scopes_report_incomplete_when_media_is_pending() {
    let (s, loading) = still_on_timeline();
    let rate = s.sequence_rate();
    let at = rate.tick_of(25) + Tick(1);
    let tick = rate.snap(at);
    let before = fingerprint(&s);
    let arrive_after_first = |attempt: u32| {
        if attempt == 1 {
            loading.store(false, Ordering::SeqCst);
        }
    };

    // the bytes never arrive: a slate, reported incomplete after every attempt the deadline allows
    loading.store(true, Ordering::SeqCst);
    let (slate, report) = render_frame(&s, 2048, at, LOOP_DEADLINE_MS, clock_from_zero(), |_| {});
    assert_eq!(report, frame_report_of(&s, &slate, tick, false, PENDING_ATTEMPTS));
    assert!(!filmcraft_media::pending::is_set(), "every attempt reads and clears the pending flag");
    // the bytes arrive while the hook waits after its first attempt: the second one is complete
    let (frame, report) = render_frame(&s, 2048, at, LOOP_DEADLINE_MS, clock_from_zero(), arrive_after_first);
    assert_eq!(report, frame_report_of(&s, &frame, tick, true, 2));
    let picture = s.try_render_program_at(1.0, at).expect("the still renders once its bytes are there");
    assert!(same_frame(&frame, &picture), "the complete frame is the picture");
    assert!(!same_frame(&slate, &picture), "the frame rendered while the bytes were loading is not the picture");
    // the bytes are there from the start: complete at the first attempt
    s.media.clear();
    let (frame, report) = render_frame(&s, 2048, at, LOOP_DEADLINE_MS, clock_from_zero(), |_| {});
    assert_eq!(report, frame_report_of(&s, &frame, tick, true, 1));
    assert!(same_frame(&frame, &picture), "a frame complete at once is the picture");

    // scopes read the same picture through the same loop
    let truth = Signal::from_rgba8(picture.w, picture.h, &picture.over_black_rgba8());
    s.media.clear();
    loading.store(true, Ordering::SeqCst);
    let (signal, complete, attempts) = scope_attempts(&s, at, LOOP_DEADLINE_MS, clock_from_zero(), |_| {});
    assert_eq!((complete, attempts), (false, PENDING_ATTEMPTS), "scopes over a loading clip are incomplete");
    assert!(signal != truth, "the slate's scopes are not the picture's");
    let (signal, complete, attempts) = scope_attempts(&s, at, LOOP_DEADLINE_MS, clock_from_zero(), arrive_after_first);
    assert_eq!((complete, attempts), (true, 2), "scopes are complete once the bytes are there");
    assert!(signal == truth, "complete scopes read the picture");
    assert!(!filmcraft_media::pending::is_set(), "every attempt reads and clears the pending flag");
    assert_eq!(fingerprint(&s), before, "rendering, waiting and retrying changed session state");

    // the verdict: a complete attempt always ends the loop; a pending one is retried up to and at
    // the deadline; a clock or a deadline that is not a finite number ends it
    assert_eq!(policy::attempt(false, NOW_MS, NOW_MS - 1.0), Some(true));
    assert_eq!(policy::attempt(true, NOW_MS, NOW_MS), None);
    assert_eq!(policy::attempt(true, NOW_MS + 0.5, NOW_MS), Some(false));
    for (now_ms, deadline_ms) in [(NOW_MS, f64::NAN), (NOW_MS, f64::INFINITY), (f64::NAN, NOW_MS)] {
        assert_eq!(policy::attempt(true, now_ms, deadline_ms), Some(false), "clock {now_ms}, deadline {deadline_ms}");
    }
}

/// T-FI-12. The `executeTyped` policy table is closed and exact (INV-12, INV-13, INV-15). It holds
/// the reviewed edits of FF1 and R0, each registered once and journaled, each allowed only parameter
/// keys the command documents, R0's with exactly the keys R0 reviewed and requiring exactly the keys
/// R0 reviewed (a key the engine fails without, or a target or a time it would otherwise take from
/// the person's selection or playhead), FF1's requiring exactly the keys pinned for them, and the
/// few calls that need more (one of several keys, keys that come together, a key that one value of
/// another needs) needing exactly that; the engine itself fails without what is required or needed.
/// Every key a requirement or a need names has a rule, so a value of another type cannot fall back
/// to the engine's default either; so has every key a call may leave out that the engine would fill
/// in itself from a value of another type ([`TYPED_OPTIONAL`]: a time, a target, a font), every
/// switch a command documents (`bool`), and every number the engine would overflow on: every time,
/// in ticks, within the engine's own `Tick::MAX` of zero ([`TICK_KEYS`]; past it the engine's own
/// arithmetic overflows, at it the engine computes, which this test runs with overflow checks on);
/// the frames of a transition or a tracking pass, the lines, characters and gap of captions made from
/// the transcript ([`WITHIN`]); a clip's speed, within the editor's own range; durations and pauses,
/// in seconds from 0 to a day; and a transcript's words, each with its text, its times and a length
/// of at most ten minutes (without a word the engine clears the item's transcript, which it does
/// here). A graphic layer's visibility is `true` or `false`, and neither a property nor a template
/// value is null. Every closed set of names is limited to names the engine documents or parses. A
/// key that would name an ambient path appears only where a rule limits it to a file under the
/// export root. Trims and seeks take ticks only (INV-32), undo and redo no parameter but the caller's
/// history head, and only a lift, an extract and a transcript selection, extract or lift need the
/// caller's track targeting. Limited values are checked:
/// in the browser the stepped export formats with a built-in encoder run, GIF, WAV and image
/// sequences are native only, and HEVC and AIFF never run; a frame size is whole pixels the engine
/// takes; the effect parameters a call may set or keyframe are pairs the engine defines, animatable,
/// of the kind the entry checks, on effects without a text parameter, so no LUT file is reachable
/// (Lumetri's are tier 3). Scene detection, mask tracking, the ducking analysis and the audio
/// transition run in a native worker only, and the browser refuses them, and the native-only values,
/// with `native_only`; a native worker refuses the media export, which writes outside its file
/// services, with `use_worker_export`; both only once nothing else in the table refuses the call.
/// Every other registered command (queries, tier 3, `file.exportFrame`, `effects.toggleAnimation`,
/// `sequence.makeSubsequence` and the rest), every id that names no registered command, unknown keys,
/// keys in another case, required keys left out, needs not met, values outside their rule, effect
/// parameters an entry does not reach or values of another kind, and parameters that are not an
/// object are `command_not_allowed`, in both runtimes.
///
/// Slice R1 (pinned last, so that every check before it holds for the table as R0 left it too) adds
/// the sequence marks, set at a time in ticks on the program only; widens a video transition to the
/// engine's catalogue of the ten Video Transitions folders, each with its own settings typed by kind
/// (Legacy and Obsolete left out, no text, an option by its exact name or index, a colour as hex or
/// as components from 0 to 1); a track's lock, output and mute beside its sync lock (one or more,
/// never none, never its solo, name, volume or pan); a layer's place by index within `u32`; an insert
/// that names its time; and the read tier: a scene detection that only looks, which names no apply
/// choice, stays native only and is the table's one read.
///
/// If this fails, the hook is not packaged (FF2): no raw passthrough is a release condition.
#[test]
fn typed_policy_rejects_raw_ids_unknown_keys_and_tier3() {
    let runtimes = [policy::Runtime::Browser, policy::Runtime::Native];
    let table: BTreeSet<&str> = policy::POLICY.iter().map(|entry| entry.command).collect();
    assert_eq!(table.len(), policy::POLICY.len(), "one entry per command");
    let reviewed: BTreeSet<&str> = FF1_COMMANDS.iter().copied().chain(R0_ENTRIES.iter().map(|(id, _, _)| *id)).collect();
    assert_eq!(reviewed.len(), FF1_COMMANDS.len() + R0_ENTRIES.len(), "R0 adds commands FF1 did not have, each once");
    assert!(reviewed.is_subset(&table), "the reviewed commands of FF1 and R0 are in the table (all of it is pinned with R1's below)");
    assert_eq!(policy::Runtime::HERE, policy::Runtime::Native, "compiled natively, the file serves a native worker");
    for entry in policy::POLICY.iter() {
        let id = entry.command;
        assert_eq!(command_specs().iter().filter(|spec| spec.id == id).count(), 1, "`{id}` names exactly one registered engine command");
        let spec = find_command(id).unwrap_or_else(|| panic!("`{id}` is a registered command"));
        assert!(spec.journal, "`{id}` is an edit: queries are read through executeChecked, never executeTyped");
        assert_eq!(entry.history_head, matches!(id, "edit.undo" | "edit.redo"), "`{id}`: only undo and redo need the caller's history head");
        assert_eq!(entry.targeting, TARGETED.contains(&id), "`{id}`: only what acts on the targeted tracks needs the caller's targeting");
        let keys: BTreeSet<&str> = entry.keys.iter().copied().collect();
        assert_eq!(keys.len(), entry.keys.len(), "`{id}` lists each key once");
        let documented = documented_keys(spec.params);
        for key in &keys {
            assert!(documented.contains(*key), "`{id}` allows `{key}`, which it does not document: {}", spec.params);
            if FORBIDDEN_KEYS.contains(key) {
                assert!(matches!(rule_of(entry, key), Some(policy::Rule::ExportPath)), "`{id}` allows `{key}`, an ambient path or a raw command");
            }
        }
        for (key, _) in entry.rules {
            assert!(keys.contains(key), "`{id}` limits `{key}`, which it does not allow");
        }
        // a required key is allowed, and limited by a rule or naming the effect parameter reached
        for key in entry.required {
            assert!(keys.contains(key), "`{id}` requires `{key}`, which it does not allow");
            let names_target = !entry.targets.is_empty() && matches!(*key, "effect" | "param" | "value");
            assert!(rule_of(entry, key).is_some() || names_target, "`{id}` requires `{key}`, which no rule limits");
        }
        // so is every key a need names
        for need in entry.needs {
            for key in need_keys(need) {
                assert!(keys.contains(key), "`{id}` needs `{key}`, which it does not allow");
                assert!(rule_of(entry, key).is_some(), "`{id}` needs `{key}`, which no rule limits");
            }
        }
        // every switch the command documents is typed, and every key it documents in ticks
        for (key, kind) in documented_types(spec.params) {
            if keys.contains(key.as_str()) && kind == "bool" {
                assert!(matches!(rule_of(entry, &key), Some(policy::Rule::Bool | policy::Rule::True)), "`{id}`: the switch `{key}` is typed");
            }
            if keys.contains(key.as_str()) && kind == "ticks" {
                assert_eq!(rule_of(entry, &key), Some(&policy::Rule::Ticks), "`{id}`: `{key}` is a time in ticks");
            }
        }
        if !entry.targets.is_empty() {
            assert!(["effect", "param"].iter().all(|key| entry.required.contains(key)), "`{id}` reaches effect parameters: it requires `effect` and `param`");
        }
        // the keys decide which parameters may come, the required keys which must, and the rules and
        // the effect parameters reached which values they take; then where the entry runs
        let all = valid_params(entry, true);
        let minimal = valid_params(entry, false);
        let natively = if entry.runtimes == policy::Runtimes::BrowserOnly { Err("use_worker_export") } else { Ok(id) };
        let in_browser = if entry.runtimes == policy::Runtimes::NativeOnly { Err("native_only") } else { Ok(id) };
        for params in [Value::Object(all.clone()), Value::Object(minimal)] {
            assert_eq!(policy::allowed(id, &params, policy::Runtime::Native).map(|found| found.command), natively, "`{id}` {params} natively");
            assert_eq!(policy::allowed(id, &params, policy::Runtime::Browser).map(|found| found.command), in_browser, "`{id}` {params} in the browser");
        }
        let mut refused = vec![Value::Null, json!([]), json!(id), json!(1), json!(true)];
        for extra in FORBIDDEN_KEYS.into_iter().chain(["unknownKey"]) {
            let mut params = all.clone();
            params.insert(extra.to_string(), json!("C:\\Windows\\win.ini"));
            refused.push(Value::Object(params));
        }
        for key in &keys {
            let upper = key.to_ascii_uppercase();
            if upper != *key {
                refused.push(Value::Object(Map::from_iter([(upper, json!(1))])));
            }
        }
        for key in entry.required {
            let mut params = all.clone();
            params.remove(*key);
            refused.push(Value::Object(params));
        }
        for (key, rule) in entry.rules {
            for value in values_refused(rule) {
                let mut params = all.clone();
                params.insert(key.to_string(), value);
                refused.push(Value::Object(params));
            }
        }
        // each need: without what it asks for the call is refused, with it the call runs
        let mut accepted = Vec::new();
        for need in entry.needs {
            match need {
                policy::Need::AnyOf(groups) => {
                    let mut none = all.clone();
                    for key in need_keys(need) {
                        none.remove(key);
                    }
                    refused.push(Value::Object(none.clone()));
                    for group in groups.iter() {
                        let mut alone = none.clone();
                        for key in group.iter() {
                            alone.insert(key.to_string(), param_value(entry, key));
                        }
                        accepted.push(Value::Object(alone.clone()));
                        for key in group.iter().filter(|_| group.len() > 1) {
                            let mut short = alone.clone();
                            short.remove(*key);
                            refused.push(Value::Object(short));
                        }
                    }
                }
                policy::Need::Together(together) => {
                    let mut neither = all.clone();
                    for key in together.iter() {
                        neither.remove(*key);
                        let mut half = all.clone();
                        half.remove(*key);
                        refused.push(Value::Object(half));
                    }
                    accepted.push(Value::Object(neither));
                }
                policy::Need::When { key, holds, then } => {
                    let mut with = all.clone();
                    with.insert(key.to_string(), condition_value(holds));
                    accepted.push(Value::Object(with.clone()));
                    with.remove(*then);
                    refused.push(Value::Object(with));
                }
                policy::Need::Without { key, holds, others } => {
                    let mut without = all.clone();
                    without.insert(key.to_string(), condition_value(holds));
                    for other in others.iter() {
                        without.remove(*other);
                    }
                    accepted.push(Value::Object(without.clone()));
                    for other in others.iter() {
                        let mut with_one = without.clone();
                        with_one.insert(other.to_string(), param_value(entry, other));
                        refused.push(Value::Object(with_one));
                    }
                }
            }
        }
        for params in accepted {
            assert_eq!(policy::allowed(id, &params, policy::Runtime::Native).map(|found| found.command), natively, "`{id}` {params} natively");
            assert_eq!(policy::allowed(id, &params, policy::Runtime::Browser).map(|found| found.command), in_browser, "`{id}` {params} in the browser");
        }
        for params in refused {
            for runtime in runtimes {
                assert_eq!(policy::allowed(id, &params, runtime).err(), Some("command_not_allowed"), "`{id}` {params} ({runtime:?})");
            }
        }
    }
    // R0's entries allow exactly the keys R0 reviewed and require exactly the keys it reviewed; FF1's
    // require exactly the keys pinned for them; and the needs are exactly those pinned
    for (id, keys, required) in R0_ENTRIES {
        if WIDEN_1.iter().any(|(widened, _, _)| widened == id) {
            // widen-1 changed it: pinned with R1's entries below
            continue;
        }
        assert_eq!(policy_keys(id), keys.iter().copied().collect::<BTreeSet<&str>>(), "`{id}`: the keys R0 reviewed, exactly");
        assert_eq!(required_keys(id), required.iter().copied().collect::<BTreeSet<&str>>(), "`{id}`: the keys R0 requires, exactly");
    }
    for id in FF1_COMMANDS {
        let pinned: BTreeSet<&str> =
            FF1_REQUIRED.iter().find(|(command, _)| *command == id).map(|(_, keys)| keys.iter().copied().collect()).unwrap_or_default();
        assert_eq!(required_keys(id), pinned, "`{id}`: the keys it requires");
    }
    let needing: BTreeSet<&str> = policy::POLICY.iter().filter(|entry| !entry.needs.is_empty()).map(|entry| entry.command).collect();
    let r1_needing: BTreeSet<&str> = R1_NEEDS.iter().chain(RN_NEEDS).map(|(id, _)| *id).collect();
    let needing_before_r1: BTreeSet<&str> = needing.difference(&r1_needing).copied().collect();
    assert_eq!(needing_before_r1, NEEDS.iter().map(|(id, _)| *id).collect::<BTreeSet<&str>>(), "the entries that need more than their required keys");
    for (id, needs) in NEEDS {
        let written: Vec<String> = entry_of(id).needs.iter().map(need_text).collect();
        assert_eq!(written, needs.iter().map(|need| need.to_string()).collect::<Vec<String>>(), "`{id}`: what it needs");
    }
    // the keys a call may leave out that the engine would fill in itself from a value of another type
    // are typed all the same, and the durations and pauses are seconds from 0 to a day
    for (id, keys) in TYPED_OPTIONAL {
        let entry = entry_of(id);
        for key in keys.iter() {
            assert!(rule_of(entry, key).is_some() && !entry.required.contains(key), "`{id}` types `{key}`, which a call may leave out");
        }
    }
    for entry in policy::POLICY.iter() {
        for (key, rule) in entry.rules {
            let in_seconds = SECONDS_KEYS.contains(&(entry.command, *key));
            assert_eq!(*rule == policy::Rule::Seconds, in_seconds, "`{}`: `{key}` is in seconds exactly when it is listed", entry.command);
        }
    }
    // every time is in ticks within the engine's own `Tick::MAX` of zero, where four times still sum
    // within `i64`; the bounded whole numbers and the speed are exactly those pinned
    assert_eq!((policy::MAX_TICKS, -policy::MAX_TICKS > Tick::MIN.0), (Tick::MAX.0, true), "a time lies within the engine's own bound");
    assert!(policy::MAX_TICKS.checked_mul(4).is_some(), "four times within the bound sum within i64");
    assert_eq!((policy::TICKS_PER_SECOND, policy::MAX_WORD_TICKS), (TICKS_PER_SECOND, 600 * TICKS_PER_SECOND), "a word lasts ten minutes at most");
    let (mut ticked, mut bounded, mut fractional) = (BTreeSet::new(), BTreeSet::new(), Vec::new());
    for entry in policy::POLICY.iter() {
        for (key, rule) in entry.rules {
            match rule {
                policy::Rule::Ticks => {
                    ticked.insert((entry.command, *key));
                }
                policy::Rule::Within(min, max) => {
                    bounded.insert((entry.command, *key, *min, *max));
                }
                policy::Rule::Between(min, max) => fractional.push((entry.command, *key, *min, *max)),
                _ => {}
            }
        }
    }
    let r1_new = |command: &str| R1_ENTRIES.iter().any(|(id, _, _)| *id == command);
    let pinned: BTreeSet<(&str, &str)> = TICK_KEYS.iter().flat_map(|(id, keys)| keys.iter().map(move |key| (*id, *key))).collect();
    let ticked_before_r1: BTreeSet<(&str, &str)> = ticked.iter().copied().filter(|(command, _)| !r1_new(command)).collect();
    assert_eq!(ticked_before_r1, pinned, "the keys in ticks, exactly");
    assert_eq!(bounded, WITHIN.iter().copied().collect::<BTreeSet<(&str, &str, i64, i64)>>(), "the bounded whole numbers, exactly");
    assert_eq!(fractional, vec![("clip.speedDuration", "speed", policy::MIN_SPEED, policy::MAX_SPEED)], "the one bounded number that may be fractional");
    assert_eq!((policy::MIN_SPEED, policy::MAX_SPEED, policy::MAX_FRAMES), (0.01, 100_000.0, 100_000), "the editor's own speed range and frame limit");
    // the calls the verification found to overflow the engine's arithmetic on times, and their like
    // for each kind of bounded value, are refused in both runtimes
    let past = policy::MAX_TICKS + 1;
    let overflowing = [
        ("timeline.trim", json!({"clip": 1, "edge": "out", "mode": "regular", "delta": i64::MIN})),
        ("captions.setTimes", json!({"caption": 1, "startTime": i64::MAX})),
        ("timeline.place", json!({"item": 1, "duration": i64::MAX})),
        ("playhead.set", json!({"time": past})),
        ("timeline.move", json!({"moves": [{"clip": 1, "track": "V1", "time": -past}]})),
        ("effects.setKeyframe", json!({"clip": 1, "effect": "opacity", "param": "opacity", "mediaTime": past})),
        ("transcript.set", json!({"item": 1, "transcript": {"words": [{"text": "Hello", "start": past, "end": past}]}})),
        ("sequence.applyVideoTransition", json!({"clip": 1, "effect": "cross_dissolve", "frames": policy::MAX_FRAMES + 1, "edge": "in"})),
        ("clip.speedDuration", json!({"clips": [1], "speed": 0.001})),
    ];
    for (id, params) in &overflowing {
        for runtime in runtimes {
            assert_eq!(policy::allowed(id, params, runtime).err(), Some("command_not_allowed"), "`{id}` {params} ({runtime:?})");
        }
    }
    // at the bound itself the engine's arithmetic holds (this test runs with overflow checks on, as a
    // native worker on the dev profile does): a typed trim of the last shot by the bound either way
    // is clamped to what the shot can give and reported as such, and a caption bound there either
    // way is refused by the engine from inside the command, which changes nothing but the Events log
    for delta in [-policy::MAX_TICKS, policy::MAX_TICKS] {
        let mut s = demo();
        let last = *v1_clips(&s).last().expect("the demo has shots on V1");
        let moved = typed_trim(&mut s, last, "out", "regular", delta);
        assert!(moved != delta && moved.signum() != -delta.signum(), "a trim by {delta} is clamped: {moved}");
    }
    let mut s = demo_with_transcript_and_captions();
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let caption = seq.caption_tracks.first().and_then(|track| track.captions.first()).map(|caption| caption.id.0).expect("the fixture has a caption");
    for start in [-policy::MAX_TICKS, policy::MAX_TICKS] {
        let before = without_log(fingerprint(&s));
        assert!(s.execute("captions.setTimes", json!({"caption": caption, "startTime": start})).is_err(), "a caption cannot start at {start}");
        assert_eq!(without_log(fingerprint(&s)), before, "the refused caption bounds changed nothing but the Events log");
    }
    // a transcript without words is refused: the engine would take it and clear the item's transcript
    let (_, media) = first_v1_clip(&s);
    let empty = json!({"item": media.0, "transcript": {"language": "en"}});
    for runtime in runtimes {
        assert_eq!(policy::allowed("transcript.set", &empty, runtime).err(), Some("command_not_allowed"), "a transcript without words ({runtime:?})");
    }
    let reply = s.execute("transcript.set", empty).expect("the engine takes a transcript without words");
    assert_eq!(reply["words"], json!(0), "{reply}");
    assert!(s.project.transcripts.get(&media).is_some_and(|transcript| transcript.words.is_empty()), "the engine cleared the item's transcript");
    // what the required keys and the needs stand for: the table refuses each of these calls, and the
    // engine, given the same call, fails on it from inside the command, not as disabled
    let (fixture, title) = demo_with_graphics();
    let (video, item) = first_v1_clip(&fixture);
    let seq = fixture.active_sequence().expect("the demo opens its main sequence");
    let caption = seq.caption_tracks.first().and_then(|track| track.captions.first()).map(|caption| caption.id.0).expect("the fixture has a caption");
    let audio = seq.audio_tracks.first().and_then(|track| track.items.first()).map(|clip| clip.id.0).expect("the demo has a clip on A1");
    for (id, text) in ENGINE_FAILS_WITHOUT {
        let text = text.replace("\"{item}\"", &item.0.to_string()).replace("\"{video}\"", &video.0.to_string());
        let text = text.replace("\"{caption}\"", &caption.to_string()).replace("\"{audio}\"", &audio.to_string());
        let text = text.replace("\"{graphic}\"", &title.0.to_string());
        let params: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("`{id}` {text}: {e}"));
        for runtime in runtimes {
            assert_eq!(policy::allowed(id, &params, runtime).err(), Some("command_not_allowed"), "`{id}` {params} ({runtime:?})");
        }
        let (mut s, _) = demo_with_graphics();
        match s.execute(id, params.clone()) {
            Err(EngineError::Disabled(..)) | Ok(_) => panic!("the engine should fail `{id}` {params} from inside the command"),
            Err(_) => {}
        }
    }
    // trims and seeks take ticks only (INV-32); undo and redo take no parameter
    assert_eq!(policy_keys("timeline.trim"), BTreeSet::from(["clip", "delta", "edge", "mode"]), "a trim's delta is in ticks, never in frames");
    assert_eq!(policy_keys("playhead.set"), BTreeSet::from(["time"]), "a seek is in ticks, never in seconds, frames or timecode");
    assert!(policy_keys("edit.undo").is_empty() && policy_keys("edit.redo").is_empty(), "undo and redo take no parameter");

    // where entries run: the browser refuses the native-only ones and a native worker the media
    // export, each only once nothing else in the table refuses the call
    let running =
        |place: policy::Runtimes| policy::POLICY.iter().filter(|entry| entry.runtimes == place).map(|entry| entry.command).collect::<BTreeSet<&str>>();
    assert_eq!(running(policy::Runtimes::NativeOnly), BTreeSet::from(NATIVE_ONLY_COMMANDS), "the native-only commands");
    assert_eq!(running(policy::Runtimes::BrowserOnly), BTreeSet::from(BROWSER_ONLY_COMMANDS), "the browser-only commands");
    assert_eq!(policy::allowed("masks.track", &json!({"unknownKey": 1}), policy::Runtime::Browser).err(), Some("command_not_allowed"));
    assert_eq!(policy::allowed("file.exportMedia", &json!({"unknownKey": 1}), policy::Runtime::Native).err(), Some("command_not_allowed"));

    // the export formats, as the engine's export crate has them: a stepped job with a built-in
    // encoder runs in the browser, one synchronous run with a built-in encoder only natively, and
    // HEVC (stepped, but only a system encoder writes it) and AIFF nowhere
    for format in Format::ALL {
        let name = format.id();
        assert_eq!(Format::from_name(name), Some(format), "`{name}` names its format");
        let lists = [STEPPED_FORMATS.contains(&name), BLOCKING_FORMATS.contains(&name), REFUSED_FORMATS.contains(&name)];
        assert_eq!(lists.iter().filter(|&&listed| listed).count(), 1, "`{name}` is in exactly one of the format lists");
        let builtin_stepped = filmcraft_export::stepped(format) && format.has_builtin_encoder();
        assert_eq!(STEPPED_FORMATS.contains(&name), builtin_stepped, "`{name}`: a stepped job with a built-in encoder, or not");
        if BLOCKING_FORMATS.contains(&name) {
            assert!(!filmcraft_export::stepped(format) && format.has_builtin_encoder(), "`{name}` is written in one run, by a built-in encoder");
        }
    }
    assert!(filmcraft_export::stepped(Format::Hevc) && !Format::Hevc.has_builtin_encoder(), "HEVC is a stepped format only a system encoder writes");
    // exports write only under the export root, in the browser; a native worker exports through its
    // own host-built paths
    let out = format!("{}t-fi-12/out.mp4", policy::EXPORT_ROOT);
    let export = |format: &str, path: &str| json!({"format": format, "path": path});
    for format in STEPPED_FORMATS {
        let in_browser = policy::allowed("file.exportMedia", &export(format, &out), policy::Runtime::Browser).map(|found| found.command);
        assert_eq!(in_browser, Ok("file.exportMedia"), "{format} in the browser");
    }
    for format in BLOCKING_FORMATS {
        assert_eq!(policy::allowed("file.exportMedia", &export(format, &out), policy::Runtime::Browser).err(), Some("native_only"), "{format} in the browser");
    }
    for format in STEPPED_FORMATS.into_iter().chain(BLOCKING_FORMATS) {
        assert_eq!(policy::allowed("file.exportMedia", &export(format, &out), policy::Runtime::Native).err(), Some("use_worker_export"), "{format} natively");
    }
    for format in REFUSED_FORMATS.into_iter().chain(["GIF", "H264", "h264 ", "mp4", "webm", "h265", "x265"]) {
        for runtime in runtimes {
            assert_eq!(policy::allowed("file.exportMedia", &export(format, &out), runtime).err(), Some("command_not_allowed"), "{format} ({runtime:?})");
        }
    }
    // neither the system encoders, nor a loudness pass that measures the whole export in one call,
    // nor a preset or raw settings, nor a synchronous run
    for (key, value) in
        [("hardwareEncoding", json!("auto")), ("loudnessLufs", json!(-16.0)), ("preset", json!("Match Source")), ("settings", json!({})), ("wait", json!(true))]
    {
        let mut params = export("h264", &out);
        params[key] = value;
        for runtime in runtimes {
            assert_eq!(policy::allowed("file.exportMedia", &params, runtime).err(), Some("command_not_allowed"), "`{key}` ({runtime:?})");
        }
    }
    let too_long = format!("{}{}.mp4", policy::EXPORT_ROOT, "a".repeat(600));
    let ambient = [
        "C:\\Windows\\win.ini",
        "/files/lts-0000.mp4",
        "/opfs/autosave.fcproj",
        "/exports/out.mp4",
        "out.mp4",
        "//lts-exports/out.mp4",
        "/LTS-EXPORTS/out.mp4",
        "/lts-exports",
        "/lts-exports/",
        "/lts-exports/t-fi-12/",
        "/lts-exports/../win.ini",
        "/lts-exports/t-fi-12/../../win.ini",
        "/lts-exports/./out.mp4",
        "/lts-exports/...",
        "/lts-exports/t-fi-12//out.mp4",
        "/lts-exports/out.mp4.",
        "/lts-exports/out file.mp4",
        "/lts-exports/C:\\out.mp4",
        "/lts-exports/t-fi-12\\out.mp4",
        // Windows devices, with or without an extension, in any case
        "/lts-exports/CON",
        "/lts-exports/nul.mp4",
        "/lts-exports/aux.tar.gz",
        "/lts-exports/Prn.mov",
        "/lts-exports/t-fi-12/com1.mp4",
        "/lts-exports/COM0.wav",
        "/lts-exports/lpt9.gif",
        too_long.as_str(),
    ];
    for path in ambient {
        for format in ["h264", "gif"] {
            for runtime in runtimes {
                assert_eq!(
                    policy::allowed("file.exportMedia", &export(format, path), runtime).err(),
                    Some("command_not_allowed"),
                    "{format} to {path:?} ({runtime:?})"
                );
            }
        }
        for id in ["file.exportEdl", "file.exportOtio", "file.exportFcpxml"] {
            for runtime in runtimes {
                assert_eq!(policy::allowed(id, &json!({"path": path}), runtime).err(), Some("command_not_allowed"), "`{id}` to {path:?} ({runtime:?})");
            }
        }
    }
    // names that only look like a device are files
    for name in ["console.mp4", "com10.mp4", "auxiliary.mov", "nul0.mp4", "lpt.mp4", "t-fi-12/con-1.mp4"] {
        let path = format!("{}{name}", policy::EXPORT_ROOT);
        assert!(policy::allowed("file.exportMedia", &export("h264", &path), policy::Runtime::Browser).is_ok(), "{path}");
    }
    let cut = format!("{}t-fi-12/cut.edl", policy::EXPORT_ROOT);
    for id in ["file.exportEdl", "file.exportOtio", "file.exportFcpxml"] {
        for runtime in runtimes {
            assert_eq!(policy::allowed(id, &json!({"path": cut}), runtime).map(|found| found.command), Ok(id), "`{id}` ({runtime:?})");
        }
    }
    // the custom range would need float seconds (INV-32): only the sequence's own ranges
    for range in ["entire", "inOut", "workArea"] {
        assert!(policy::allowed("file.exportMedia", &json!({"path": out, "format": "h264", "range": range}), policy::Runtime::Browser).is_ok(), "{range}");
    }
    for params in [json!({"path": out, "format": "h264", "range": "custom"}), json!({"path": out, "format": "h264", "startSeconds": 1.0})] {
        assert_eq!(policy::allowed("file.exportMedia", &params, policy::Runtime::Browser).err(), Some("command_not_allowed"), "{params}");
    }

    // the effect parameters a call may set or keyframe are ones the engine defines, animatable, of
    // the kind the entry checks, on effects without a text parameter; the effects a call may add or
    // fade with are the engine's, of their kind, without a text parameter either. Lumetri names its
    // LUT files in text parameters (`input_lut`, `look_lut`), and its commands for them are tier 3
    let lumetri = find_effect("lumetri").expect("the engine defines Lumetri Color");
    assert!(lumetri.params.iter().any(|param| param.kind == ParamKind::Text), "Lumetri names its LUT files in text parameters");
    for entry in policy::POLICY.iter() {
        for (effect, params) in entry.targets.iter().chain(entry.native_targets) {
            let def = find_effect(effect).unwrap_or_else(|| panic!("`{}` reaches effect `{effect}`, which the engine does not define", entry.command));
            assert!(def.params.iter().all(|param| param.kind != ParamKind::Text), "`{}` reaches `{effect}`, which has a text parameter", entry.command);
            for (param, kind) in params.iter() {
                let known = def.param(param).unwrap_or_else(|| panic!("`{}` reaches `{effect}.{param}`, which the engine does not define", entry.command));
                assert!(known.animatable, "`{}` reaches `{effect}.{param}`, which cannot be animated", entry.command);
                let of_kind = match kind {
                    policy::Kind::Number => matches!(known.default, ParamValue::Float(_)),
                    policy::Kind::Point => matches!(known.default, ParamValue::Vec2(_)),
                    policy::Kind::Color => matches!(known.default, ParamValue::Color(_)),
                    policy::Kind::Switch => matches!(known.default, ParamValue::Bool(_)),
                    policy::Kind::Choice(_) => matches!(known.default, ParamValue::Choice(_)),
                };
                assert!(of_kind, "`{}`: `{effect}.{param}` takes a {kind:?}", entry.command);
            }
        }
        if let Some((effects, native_effects)) = one_of(entry, "effect") {
            let native_only: &[&str] = if entry.command == "effects.apply" { &RN_NATIVE_EFFECTS } else { &[] };
            assert_eq!(native_effects, native_only, "`{}`: the effects only a native worker adds (slice RN)", entry.command);
            let effects: Vec<&str> = effects.iter().chain(native_effects.iter()).copied().collect();
            let kind = match entry.command {
                "sequence.applyAudioTransition" => EffectKind::AudioTransition,
                "sequence.applyVideoTransition" => EffectKind::VideoTransition,
                _ => EffectKind::Video,
            };
            for effect in effects {
                let def = find_effect(effect).unwrap_or_else(|| panic!("`{}` names effect `{effect}`, which the engine does not define", entry.command));
                assert!(def.params.iter().all(|param| param.kind != ParamKind::Text), "`{}` reaches `{effect}`, which has a text parameter", entry.command);
                assert_eq!(def.kind, kind, "`{}` names `{effect}`, an effect of another kind", entry.command);
            }
        }
    }
    let (types, _) = one_of(policy::POLICY.iter().find(|entry| entry.command == "essentialSound.setType").expect("the table sets audio types"), "type")
        .expect("the audio type is limited");
    assert!(types.iter().all(|name| AudioType::parse(name).is_some_and(|parsed| parsed.id() == *name)), "the audio types are the engine's: {types:?}");
    // every name a rule limits a key to is one the command's params doc writes or the engine's own
    // parser reads as a value of that key
    for entry in policy::POLICY.iter() {
        let spec = find_command(entry.command).unwrap_or_else(|| panic!("`{}` is a registered command", entry.command));
        let documented = doc_names(spec.params);
        for (key, rule) in entry.rules {
            let names: Vec<&str> = match rule {
                policy::Rule::OneOf { both, native } => both.iter().chain(native.iter()).copied().collect(),
                policy::Rule::IndexOr(names) => names.to_vec(),
                _ => continue,
            };
            for name in names {
                let known = documented.contains(name) || parsed_by_engine(entry.command, key, name);
                assert!(known, "`{}` limits `{key}` to `{name}`, which neither its params doc nor the engine names", entry.command);
            }
        }
    }
    // a new shape is one of the engine's shapes, by its own name
    let (shapes, _) = one_of(entry_of("graphics.newShape"), "shape").expect("the shapes are limited");
    let engine_shapes: Vec<String> = filmcraft_project::graphic::SHAPE_OPTS.iter().map(|shape| shape.to_ascii_lowercase()).collect();
    assert_eq!(shapes.iter().map(|shape| shape.to_string()).collect::<Vec<String>>(), engine_shapes, "the shapes of a new shape layer");
    // the export profiles name distinct profiles (the engine reads an unknown name as its default)
    let export = entry_of("file.exportMedia");
    let profiles = |key: &str| one_of(export, key).map(|(names, _)| names).unwrap_or_else(|| panic!("`{key}` is limited"));
    let prores: BTreeSet<String> = profiles("proresProfile").iter().map(|name| format!("{:?}", filmcraft_export::prores_profile(name))).collect();
    let dnx: BTreeSet<String> = profiles("dnxProfile").iter().map(|name| format!("{:?}", filmcraft_export::dnx_profile(name))).collect();
    let apv: BTreeSet<String> = profiles("apvProfile").iter().map(|name| format!("{:?}", filmcraft_export::apv_profile(name))).collect();
    assert_eq!((prores.len(), dnx.len(), apv.len()), (4, 4, 4), "four distinct ProRes, DNxHR and APV profiles: {prores:?} {dnx:?} {apv:?}");
    // a frame size is whole pixels within the engine's longest side; mask and shape geometry stays
    // within twice that of zero
    assert_eq!(policy::MAX_FRAME_SIDE, i64::from(filmcraft_project::MAX_FRAME_SIDE), "the longest frame side is the engine's");
    assert_eq!(policy::MAX_COORDINATE, 2.0 * policy::MAX_FRAME_SIDE as f64);
    for key in ["width", "height"] {
        assert_eq!(rule_of(export, key), Some(&policy::Rule::Within(1, policy::MAX_FRAME_SIDE)), "`{key}` of an export");
    }
    // captions from the transcript: one or two lines, as the engine documents them, at most 200
    // characters a line and a gap of at most 1000 frames, so that the products the engine takes of
    // them without a check stay far from overflowing; a day in seconds is a whole number of ticks
    let doc = find_command("transcript.createCaptions").expect("the engine makes captions from the transcript").params;
    assert!(doc.contains(r#""lines":1|2?"#), "the engine documents one or two lines: {doc}");
    let made = entry_of("transcript.createCaptions");
    for (key, min, max) in [("lines", 1, 2), ("maxChars", 1, 200), ("gapFrames", 0, 1000)] {
        assert_eq!(rule_of(made, key), Some(&policy::Rule::Within(min, max)), "`{key}` of captions from the transcript");
    }
    assert_eq!(Tick::from_seconds_f64(policy::MAX_SECONDS), Tick(86_400 * TICKS_PER_SECOND), "a day in ticks");
    // `effects.setParam` sets the Motion intrinsic only (CC-26), and the keyframe commands animate
    // Motion, Opacity and Volume: each effect and parameter checked as a pair, each value of its kind
    let motion = json!({"clip": 1, "effect": "motion", "param": "position", "value": [960.0, 540.0], "time": 0});
    let fade = json!({"clip": 1, "effect": "opacity", "param": "opacity", "mediaTime": 0, "value": 50.0});
    for (id, params) in [("effects.setParam", &motion), ("effects.setKeyframe", &fade)] {
        for runtime in runtimes {
            assert_eq!(policy::allowed(id, params, runtime).map(|found| found.command), Ok(id), "`{id}` {params} ({runtime:?})");
        }
    }
    let set_param = |effect: &str, param: &str, value: Value| json!({"clip": 1, "effect": effect, "param": param, "value": value, "time": 0});
    let not_reached = [
        set_param("lumetri", "input_lut", json!("C:\\look.cube")),
        set_param("lumetri", "look_lut", json!("look.cube")),
        set_param("Motion", "position", json!([0.0, 0.0])),
        set_param("motion", "Position", json!([0.0, 0.0])),
        set_param("motion", "opacity", json!(50.0)),
        set_param("motion", "blurriness", json!(40.0)),
        set_param("motion", "uniform_scale", json!(false)),
        set_param("opacity", "opacity", json!(50.0)),
        set_param("volume", "level", json!(-6.0)),
        set_param("gaussian_blur", "blurriness", json!(40.0)),
        set_param("mosaic", "horizontal", json!(20.0)),
        set_param("motion", "position", json!(960.0)),
        set_param("motion", "position", json!([960.0])),
        set_param("motion", "position", json!([960.0, "540"])),
        set_param("motion", "scale", json!([1.0, 2.0])),
        set_param("motion", "rotation", json!("90")),
    ];
    for params in &not_reached {
        for runtime in runtimes {
            assert_eq!(policy::allowed("effects.setParam", params, runtime).err(), Some("command_not_allowed"), "{params} ({runtime:?})");
        }
    }
    let keyframe_on = |effect: &str, param: &str| json!({"clip": 1, "effect": effect, "param": param, "time": 0});
    for (effect, param) in [("motion", "position"), ("motion", "anchor"), ("opacity", "opacity"), ("volume", "level")] {
        assert!(policy::allowed("effects.addKeyframe", &keyframe_on(effect, param), policy::Runtime::Browser).is_ok(), "{effect}.{param}");
    }
    for (effect, param) in
        [("motion", "opacity"), ("opacity", "level"), ("volume", "position"), ("volume", "opacity"), ("opacity", "blend"), ("time_remap", "speed")]
    {
        for runtime in runtimes {
            assert_eq!(
                policy::allowed("effects.addKeyframe", &keyframe_on(effect, param), runtime).err(),
                Some("command_not_allowed"),
                "{effect}.{param} ({runtime:?})"
            );
        }
    }

    // no raw passthrough: every other registered command is refused, whatever it is, in both runtimes
    for spec in command_specs() {
        if !table.contains(spec.id) {
            for runtime in runtimes {
                assert_eq!(policy::allowed(spec.id, &json!({}), runtime).err(), Some("command_not_allowed"), "`{}` is not a typed command", spec.id);
            }
        }
    }
    for id in NEVER_COMMANDS {
        assert!(find_command(id).is_some(), "`{id}` is a registered engine command, so refusing it means something");
        assert!(!table.contains(id), "`{id}` is never typed");
    }
    // ids that name no registered command: near misses, other channels, and the marker deletion
    // the newsroom set planned (no engine command: an open C1 item)
    assert!(find_command("markers.delete").is_none(), "marker deletion has no engine command");
    let forged = [
        "",
        "markers.delete",
        "engine.execute",
        "ui.click",
        "TIMELINE.TRIM",
        "Timeline.trim",
        "timeline.trim ",
        " timeline.trim",
        "timeline.trim\0",
        "timeline/trim",
        "timeline.trim.x",
        "timeline",
    ];
    for id in forged {
        for runtime in runtimes {
            assert_eq!(policy::allowed(id, &json!({}), runtime).err(), Some("command_not_allowed"), "{id:?} ({runtime:?})");
        }
    }

    // ---- slice R1 (master plan v3 W3): the entries R1 adds and the ones widen-1 widens, pinned last so that every
    // check above holds for the table as R0 left it as well
    let r1: BTreeSet<&str> = R1_ENTRIES.iter().map(|(id, _, _)| *id).collect();
    let rn: BTreeSet<&str> = RN_ENTRIES.iter().map(|(id, _, _)| *id).collect();
    let r1_table: BTreeSet<&str> = reviewed.union(&r1).copied().collect();
    assert_eq!(table, r1_table.union(&rn).copied().collect::<BTreeSet<&str>>(), "the reviewed table of slice RN, exactly");
    for (id, keys, required) in R1_ENTRIES.iter().chain(WIDEN_1) {
        assert_eq!(policy_keys(id), keys.iter().copied().collect::<BTreeSet<&str>>(), "`{id}`: the keys R1 reviewed, exactly");
        assert_eq!(required_keys(id), required.iter().copied().collect::<BTreeSet<&str>>(), "`{id}`: the keys R1 requires, exactly");
    }
    let all_needs: BTreeSet<&str> = NEEDS.iter().chain(R1_NEEDS).chain(RN_NEEDS).map(|(id, _)| *id).collect();
    assert_eq!(needing, all_needs, "the entries that need more, R1's and RN's included");
    for (id, keys, required) in RN_ENTRIES {
        assert_eq!(policy_keys(id), keys.iter().copied().collect::<BTreeSet<&str>>(), "`{id}`: the keys RN reviewed, exactly");
        assert_eq!(required_keys(id), required.iter().copied().collect::<BTreeSet<&str>>(), "`{id}`: the keys RN requires, exactly");
        assert_eq!(entry_of(id).runtimes, policy::Runtimes::NativeOnly, "`{id}` runs in a native worker only");
    }
    for (id, needs) in RN_NEEDS {
        let written: Vec<String> = entry_of(id).needs.iter().map(need_text).collect();
        assert_eq!(written, needs.iter().map(|need| need.to_string()).collect::<Vec<String>>(), "`{id}`: what it needs since RN");
    }
    let reaching: BTreeSet<&str> = policy::POLICY.iter().filter(|entry| !entry.native_targets.is_empty()).map(|entry| entry.command).collect();
    assert_eq!(reaching, BTreeSet::from(RN_REACHING), "the entries that reach effect parameters natively only");
    for id in RN_REACHING {
        let reached: BTreeSet<&str> = entry_of(id).native_targets.iter().map(|(effect, _)| *effect).collect();
        assert_eq!(reached, BTreeSet::from(RN_NATIVE_EFFECTS), "`{id}` reaches Crop and Auto Reframe natively");
    }
    for (id, needs) in R1_NEEDS {
        let written: Vec<String> = entry_of(id).needs.iter().map(need_text).collect();
        assert_eq!(written, needs.iter().map(|need| need.to_string()).collect::<Vec<String>>(), "`{id}`: what it needs since R1");
    }
    let r1_ticked: BTreeSet<(&str, &str)> = ticked.iter().copied().filter(|(command, _)| r1_new(command)).collect();
    let r1_tick_keys: BTreeSet<(&str, &str)> = R1_TICK_KEYS.iter().flat_map(|(id, keys)| keys.iter().map(move |key| (*id, *key))).collect();
    assert_eq!(r1_ticked, r1_tick_keys, "a mark is set at a time in ticks");

    // a mark is set at a time in ticks on the program: no source mark, no time in frames or seconds, never the playhead
    let unmarked = [json!({}), json!({"time": 0, "target": "source"}), json!({"time": 0, "target": "program"}), json!({"frame": 30}), json!({"time": "0"})];
    for runtime in runtimes {
        for command in ["markers.markIn", "markers.markOut"] {
            assert_eq!(policy::allowed(command, &json!({"time": 0}), runtime).map(|found| found.command), Ok(command), "`{command}` ({runtime:?})");
            for params in &unmarked {
                assert_eq!(policy::allowed(command, params, runtime).err(), Some("command_not_allowed"), "`{command}` {params} ({runtime:?})");
            }
        }
        assert!(policy::allowed("markers.clearInOut", &json!({}), runtime).is_ok(), "clearing takes no key ({runtime:?})");
        assert_eq!(policy::allowed("markers.clearInOut", &json!({"time": 0}), runtime).err(), Some("command_not_allowed"), "({runtime:?})");
    }

    // an insert names its time, where the insert pre-check looks; an overwrite may still take the playhead
    let inserts = [
        (json!({"item": 1, "insert": true, "time": 0}), Ok("timeline.place")),
        (json!({"item": 1, "insert": true}), Err("command_not_allowed")),
        (json!({"item": 1, "insert": false}), Ok("timeline.place")),
        (json!({"item": 1}), Ok("timeline.place")),
    ];
    for (params, decision) in inserts {
        for runtime in runtimes {
            assert_eq!(policy::allowed("timeline.place", &params, runtime).map(|found| found.command), decision, "{params} ({runtime:?})");
        }
    }

    // a track's lock, output and mute beside its sync lock: one or more of them, never none, and never its solo, name,
    // volume or pan (widen-2)
    let switched = [json!({"track": "A1", "muted": true}), json!({"track": 2, "enabled": false, "locked": true}), json!({"track": "V1", "syncLock": false})];
    for params in &switched {
        for runtime in runtimes {
            assert_eq!(policy::allowed("timeline.setTrack", params, runtime).map(|found| found.command), Ok("timeline.setTrack"), "{params} ({runtime:?})");
        }
    }
    let unswitched = [
        json!({"track": "A1"}),
        json!({"track": "A1", "solo": true}),
        json!({"track": "A1", "name": "Music"}),
        json!({"track": "A1", "volumeDb": -6.0}),
        json!({"track": "A1", "pan": 10.0}),
        json!({"muted": true}),
    ];
    for params in &unswitched {
        for runtime in runtimes {
            assert_eq!(policy::allowed("timeline.setTrack", params, runtime).err(), Some("command_not_allowed"), "{params} ({runtime:?})");
        }
    }

    // an arranged layer goes to a place by name or to an index from the back, within `u32` so that the browser's
    // 32-bit `usize` reads it as a native worker does (the engine reads another number as 0, the back)
    let arrange = entry_of("graphics.arrangeLayer");
    assert_eq!(rule_of(arrange, "to"), Some(&policy::Rule::IndexOr(&["front", "back", "forward", "backward"])), "where a layer may go");
    for to in [json!("front"), json!("backward"), json!(0), json!(2), json!(u32::MAX)] {
        let params = json!({"clip": 1, "layer": 0, "to": to});
        for runtime in runtimes {
            let found = policy::allowed("graphics.arrangeLayer", &params, runtime).map(|found| found.command);
            assert_eq!(found, Ok("graphics.arrangeLayer"), "{params} ({runtime:?})");
        }
    }

    // a video transition is any of the engine's catalogue in the ten Video Transitions folders (Legacy and Obsolete
    // left out), by id, with the settings of its own parameters, each by kind and none a text: neither a setting the
    // transition does not have nor a value of another kind reaches the engine, which fails inside the command on the
    // first and reads the second its own way
    let transition = entry_of("sequence.applyVideoTransition");
    let catalogue: BTreeSet<&str> = effect_defs()
        .iter()
        .filter(|def| def.kind == EffectKind::VideoTransition && def.category.first() == Some(&"Video Transitions"))
        .filter(|def| def.category.get(1).is_some_and(|folder| VIDEO_TRANSITION_FOLDERS.contains(folder)))
        .map(|def| def.id)
        .collect();
    assert_eq!(catalogue.len(), 84, "the ten folders hold 84 transitions");
    let (names, native_names) = one_of(transition, "effect").expect("the video transitions are limited");
    assert!(native_names.is_empty(), "no transition is native only");
    assert_eq!(names.iter().copied().collect::<BTreeSet<&str>>(), catalogue, "the typed transitions are the engine's catalogue, exactly");
    let listed: BTreeSet<&str> = transition.settings.iter().map(|(id, _)| *id).collect();
    assert_eq!((listed.len(), &listed), (transition.settings.len(), &catalogue), "each typed transition lists its settings, once");
    for (id, settings) in transition.settings {
        let def = find_effect(id).unwrap_or_else(|| panic!("the engine defines `{id}`"));
        let engine: Vec<(&str, Option<policy::Kind>)> = def.params.iter().map(|param| (param.id, kind_of(&param.kind))).collect();
        let typed: Vec<(&str, Option<policy::Kind>)> = settings.iter().map(|(name, kind)| (*name, Some(*kind))).collect();
        assert_eq!(typed, engine, "`{id}`: its settings are its parameters, in order, by kind");
    }
    let call = |effect: &str, settings: Value| json!({"clip": 1, "effect": effect, "frames": 24, "edge": "out", "params": settings});
    let taken = [
        call("linear_wipe", json!({})),
        call("linear_wipe", json!({"angle": 45.0, "feather": 10.0, "border_width": 4.0, "border_color": "#ff8000", "antialias": "High"})),
        call("linear_wipe", json!({"antialias": 3, "border_color": [1.0, 0.5, 0.0]})),
        call("linear_wipe", json!({"border_color": "#ff800080"})),
        call("linear_wipe", json!({"border_color": [1.0, 0.5, 0.0, 0.5]})),
        call("luma_fade", json!({"source": "Incoming", "softness": 50.0, "invert": true})),
        call("cross_zoom", json!({"center": [960.0, 540.0], "strength": 80.0})),
        call("push", json!({"direction": 1})),
    ];
    for params in &taken {
        for runtime in runtimes {
            let found = policy::allowed("sequence.applyVideoTransition", params, runtime).map(|found| found.command);
            assert_eq!(found, Ok("sequence.applyVideoTransition"), "{params} ({runtime:?})");
        }
    }
    let not_taken = [
        call("linear_wipe", json!({"speed": 1.0})),
        call("linear_wipe", json!({"direction": "From East"})),
        call("linear_wipe", json!({"angle": "45"})),
        call("linear_wipe", json!({"angle": null})),
        call("linear_wipe", json!({"antialias": "high"})),
        call("linear_wipe", json!({"antialias": 4})),
        call("linear_wipe", json!({"antialias": -1})),
        call("linear_wipe", json!({"antialias": 1.5})),
        call("linear_wipe", json!({"border_color": "orange"})),
        call("linear_wipe", json!({"border_color": "ff8000"})),
        call("linear_wipe", json!({"border_color": "#ff80"})),
        call("linear_wipe", json!({"border_color": "#gg8000"})),
        call("linear_wipe", json!({"border_color": [1.0, 0.5]})),
        call("linear_wipe", json!({"border_color": [2.0, 0.0, 0.0]})),
        call("linear_wipe", json!({"border_color": [1.0, 0.5, 0.0, 0.5, 1.0]})),
        call("luma_fade", json!({"invert": "yes"})),
        call("luma_fade", json!({"source": "Both"})),
        call("cross_zoom", json!({"center": [960.0]})),
        call("push", json!({"blocks": 6.0})),
        call("linear_wipe", json!([])),
        call("linear_wipe", json!("angle")),
        call("linear_wipe", Value::Null),
        call("wipe", json!({})),
        call("gradient_wipe", json!({})),
        call("cross_dissolve_legacy", json!({})),
        call("Cross Dissolve", json!({})),
        call("PUSH", json!({})),
        call("constant_power", json!({})),
        call("gaussian_blur", json!({})),
    ];
    for params in &not_taken {
        for runtime in runtimes {
            assert_eq!(policy::allowed("sequence.applyVideoTransition", params, runtime).err(), Some("command_not_allowed"), "{params} ({runtime:?})");
        }
    }

    // the read tier: a scene detection that only looks (OC-33 E3) is the table's one read, native only as every scene
    // detection is, and it names no apply choice
    let readers: Vec<(&str, &str)> = policy::POLICY.iter().filter_map(|entry| entry.reads_with.map(|key| (entry.command, key))).collect();
    assert_eq!(readers, [("clip.sceneEditDetection", "lookOnly")], "the one read of the table");
    assert_eq!(rule_of(entry_of("clip.sceneEditDetection"), "lookOnly"), Some(&policy::Rule::Bool), "the look-only switch is a switch");
    let look = json!({"clips": [1], "lookOnly": true, "minShotFrames": 6, "sensitivity": 50});
    assert!(policy::reads_only("clip.sceneEditDetection", &look), "a look-only call reads");
    let natively = policy::allowed("clip.sceneEditDetection", &look, policy::Runtime::Native).map(|found| found.command);
    assert_eq!(natively, Ok("clip.sceneEditDetection"), "a native worker runs it");
    let in_browser = policy::allowed("clip.sceneEditDetection", &look, policy::Runtime::Browser).err();
    assert_eq!(in_browser, Some("native_only"), "native only, as every scene detection");
    for edit in [json!({"clips": [1]}), json!({"clips": [1], "lookOnly": false, "applyCuts": true})] {
        assert!(!policy::reads_only("clip.sceneEditDetection", &edit), "{edit} is an edit");
    }
    assert!(!policy::reads_only("timeline.trim", &json!({"lookOnly": true})), "no other command reads");
    for apply in ["applyCuts", "createSubclips", "generateMarkers"] {
        for value in [true, false] {
            let mut mixed = look.clone();
            mixed[apply] = json!(value);
            for runtime in runtimes {
                assert_eq!(policy::allowed("clip.sceneEditDetection", &mixed, runtime).err(), Some("command_not_allowed"), "{mixed} ({runtime:?})");
            }
        }
    }
}

/// T-FI-13. Linked selection decides what a trim or a move takes along (`with_links`), and turning
/// it off or on bumps no revision, so the caller's revision cannot catch the change (fact check
/// C15). `executeTyped` therefore checks the setting the caller read, when it names one, right after
/// the active sequence and before busy: a stale one is `linked_selection_changed`, decided before
/// anything runs, and with the setting read afresh the same edit runs. A setting the caller leaves
/// out is not checked.
///
/// Fallback if this fails: the adapter reads the setting (`state.inspect`) in the same round trip
/// before each trim and move, and refuses a change itself, without the hook's atomicity.
#[test]
fn linked_selection_changed_refused_before_dispatch() {
    let frame = demo().sequence_rate().frame_duration().0;
    // the last shot one frame shorter, by a regular trim
    let shorter = |s: &Session| {
        let last = *v1_clips(s).last().expect("the demo has shots on V1");
        (last, json!({"clip": last.0, "edge": "out", "mode": "regular", "delta": -frame}))
    };
    // the setting bumps no revision, yet it decides whether a trim takes the linked sound along
    let mut s = demo();
    assert!(s.state.linked_selection, "the demo opens with linked selection on");
    let (read, revision) = (expect_now(&s), s.revision);
    s.execute("sequence.linkedSelection", json!({"on": false})).expect("linked selection turns off");
    assert!(!s.state.linked_selection, "linked selection is off");
    assert_eq!(s.revision, revision, "turning linked selection off bumps no revision");
    // a caller that read it on is refused before dispatch: nothing runs, the Events log included
    let (last, trim) = shorter(&s);
    let before = fingerprint(&s);
    let envelope = execute_typed(&mut s, "timeline.trim", trim.clone(), &read, NOW_MS, false);
    assert_envelope(&envelope, "not_applied", Some("linked_selection_changed"), revision, revision);
    assert_eq!(fingerprint(&s), before, "`linked_selection_changed` is decided before dispatch");
    // read afresh, the same trim runs, and leaves the linked sound as it was
    let (picture, sound) = durations(&s, last);
    let fresh = expect_now(&s);
    let envelope = execute_typed(&mut s, "timeline.trim", trim, &fresh, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    assert_eq!(durations(&s, last), (picture - Tick(frame), sound), "with linked selection off, the trim leaves the linked sound alone");
    // with it on, as the demo opens, the same trim takes the sound along
    let mut s = demo();
    let (revision, (last, trim)) = (s.revision, shorter(&s));
    let (picture, sound) = durations(&s, last);
    let fresh = expect_now(&s);
    let envelope = execute_typed(&mut s, "timeline.trim", trim, &fresh, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    assert_eq!(durations(&s, last), (picture - Tick(frame), sound - Tick(frame)), "with linked selection on, the linked sound moves too");

    // a caller that names no linked selection is not checked against it
    let mut s = demo();
    let (revision, unbound) = (s.revision, policy::Expect { linked_selection: None, ..expect_now(&s) });
    s.execute("sequence.linkedSelection", json!({"on": false})).expect("linked selection turns off");
    let (_, trim) = shorter(&s);
    let envelope = execute_typed(&mut s, "timeline.trim", trim, &unbound, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);

    // its place in the order: after the active sequence, before busy (a person's input and live
    // trim playback alike, which it leaves running)
    let mut s = demo();
    let stale = policy::Expect { linked_selection: Some(!s.state.linked_selection), ..expect_now(&s) };
    let elsewhere = policy::Expect { active_sequence: Some(UNKNOWN_ITEM), ..stale.clone() };
    let (_, trim) = shorter(&s);
    let before = fingerprint(&s);
    for (expect, code) in [(&elsewhere, "sequence_changed"), (&stale, "linked_selection_changed")] {
        let envelope = execute_typed(&mut s, "timeline.trim", trim.clone(), expect, NOW_MS, true);
        assert_envelope(&envelope, "not_applied", Some(code), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "`{code}` is decided before dispatch");
    }
    let mut s = demo_with_dynamic_trim();
    let stale = policy::Expect { linked_selection: Some(!s.state.linked_selection), ..expect_now(&s) };
    let before = fingerprint(&s);
    let envelope = execute_typed(&mut s, "sequence.lift", json!({}), &stale, NOW_MS, false);
    assert_envelope(&envelope, "not_applied", Some("linked_selection_changed"), before.revision, before.revision);
    assert_eq!(fingerprint(&s), before, "the dynamic trim is still live and nothing else changed");

    // the decision itself, on the policy alone (a lift needs the caller's track targeting as well)
    let (head, targeting) = (policy::HistoryHead::default(), policy::targeting_digest(&[1, 2], Some(1), Some(2)));
    let observed = policy::Observed {
        now_ms: NOW_MS,
        revision: 3,
        active_sequence: Some(7),
        linked_selection: true,
        targeting: targeting.clone(),
        busy: false,
        history_head: head.clone(),
    };
    for (linked_selection, decision) in [(None, Ok("sequence.lift")), (Some(true), Ok("sequence.lift")), (Some(false), Err("linked_selection_changed"))] {
        let expect = policy::Expect {
            revision: 3,
            active_sequence: Some(7),
            linked_selection,
            targeting: Some(targeting.clone()),
            deadline_ms: DEADLINE_MS,
            history_head: Some(head.clone()),
        };
        let admitted = policy::admit("sequence.lift", &json!({}), &expect, &observed, policy::Runtime::Browser).map(|entry| entry.command);
        assert_eq!(admitted, decision, "a caller's linked selection {linked_selection:?} against a current one of true");
    }
}

/// The demo's overlay on V2 (the plasma clip in the corner, which fades in and out with opacity
/// keyframes) and where it starts on the timeline.
fn overlay(s: &Session) -> (ClipId, Tick) {
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let clip = seq.video_tracks.get(1).and_then(|track| track.items.first()).expect("the demo has an overlay on V2");
    (clip.id, clip.start)
}

/// The media times of the opacity keyframes of `clip`.
fn opacity_keys(s: &Session, clip: ClipId) -> Vec<Tick> {
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let (_, item) = seq.find_item(clip).expect("the clip is on the timeline");
    let opacity = item.effect("opacity").and_then(|effect| effect.param("opacity"));
    opacity.map(|param| param.keyframes.iter().map(|key| key.time).collect()).unwrap_or_default()
}

/// T-FI-14 (CC-25). `effects.addKeyframe` is a toggle in the engine: where a keyframe already sits at
/// the media time it resolves (the sequence time `time`, held to the clip, through
/// `TrackItem::source_time_at`), it takes that keyframe away and still succeeds. Typed, such an add
/// is refused before dispatch as `keyframe_exists`, with nothing run and nothing changed (no engine
/// call: the Events log is unchanged too), and an add where no keyframe sits runs. The pre-check
/// takes the sequence time to media time exactly as the engine does: on a clip played in reverse at
/// double speed, the pre-check refuses an add at every time where the engine itself has just added
/// one, and lets one through where it has not; a frame hold maps every time to the held frame.
/// `effects.setKeyframe` and `effects.setInterpolation` fail inside the command where no keyframe
/// sits at `mediaTime`, and `effects.deleteKeyframe` there changes nothing yet makes an undo step:
/// typed, those are `keyframe_not_found`, decided before dispatch. The keyframe commands never fall
/// back to the playhead: `time` (an add) and `mediaTime` (the others) are required whole numbers.
/// `clip.enable` is a toggle as well, and no key says which way: typed, it disables a shot that is
/// enabled, with its linked sound, and the same call enables them again; so the caller reads
/// `enabled` at the revision it binds before it sends one.
///
/// Fallback if this fails: the keyframe commands leave the browser's table until the engine's add
/// stops toggling (an OD-7 fork change), and keyframes are set with `effects.setKeyframe` only.
#[test]
fn existing_keyframe_refused_before_dispatch() {
    let s = demo();
    let rate = s.sequence_rate();
    let (overlay_clip, start) = overlay(&s);
    let frames = |indices: &[i64]| indices.iter().map(|&index| rate.tick_of(index)).collect::<Vec<Tick>>();
    assert_eq!(opacity_keys(&s, overlay_clip), frames(&[0, 12, 84, 95]), "the overlay fades with opacity keyframes at media frames 0, 12, 84 and 95");
    let add = |time: Tick| json!({"clip": overlay_clip.0, "effect": "opacity", "param": "opacity", "time": time.0});
    // the engine, given a fresh add at media frame 12, takes that keyframe away and still succeeds
    let mut engine = demo();
    engine.execute("effects.addKeyframe", add(start + rate.tick_of(12))).expect("the engine runs the add");
    assert_eq!(opacity_keys(&engine, overlay_clip), frames(&[0, 84, 95]), "the engine's add took the keyframe at frame 12 away");
    // typed, the same add is refused before dispatch, and so are adds at the overlay's start (media
    // frame 0) and before it (held to the start): nothing runs
    for time in [start + rate.tick_of(12), start, Tick::ZERO] {
        let mut s = demo();
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let envelope = execute_typed(&mut s, "effects.addKeyframe", add(time), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("keyframe_exists"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "the refused add at {time:?} ran nothing, the Events log included");
    }
    // where no keyframe sits, the add runs and adds one
    let mut s = demo();
    let (expect, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "effects.addKeyframe", add(start + rate.tick_of(30)), &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    assert_eq!(opacity_keys(&s, overlay_clip), frames(&[0, 12, 30, 84, 95]), "the add put a keyframe at media frame 30");

    // editing or deleting a keyframe needs one at `mediaTime`
    let keyframe = |command: &str, frame: i64| {
        let mut params = json!({"clip": overlay_clip.0, "effect": "opacity", "param": "opacity", "mediaTime": rate.tick_of(frame).0});
        match command {
            "effects.setKeyframe" => params["value"] = json!(50.0),
            "effects.setInterpolation" => params["interpolation"] = json!("hold"),
            _ => {}
        }
        params
    };
    for command in ["effects.setKeyframe", "effects.setInterpolation", "effects.deleteKeyframe"] {
        let mut s = demo();
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let envelope = execute_typed(&mut s, command, keyframe(command, 13), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("keyframe_not_found"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "the refused `{command}` ran nothing, the Events log included");
        let (expect, revision) = (expect_now(&s), s.revision);
        let envelope = execute_typed(&mut s, command, keyframe(command, 84), &expect, NOW_MS, false);
        assert_envelope(&envelope, "applied", None, revision, revision + 1);
    }
    // what those refusals stand for: the engine fails an edit where no keyframe sits inside the
    // command, and a delete there changes nothing but still makes a revision and an undo step
    let mut s = demo();
    assert!(s.execute("effects.setKeyframe", keyframe("effects.setKeyframe", 13)).is_err(), "no keyframe at media frame 13 to edit");
    let revision = s.revision;
    s.execute("effects.deleteKeyframe", keyframe("effects.deleteKeyframe", 13)).expect("the engine runs the delete");
    assert_eq!((s.revision, opacity_keys(&s, overlay_clip)), (revision + 1, frames(&[0, 12, 84, 95])), "a delete of nothing is a revision");

    // the time domains, on the overlay played in reverse at double speed: the pre-check's media
    // time is the engine's, so once the engine itself has added a keyframe at a time, the pre-check
    // refuses an add there; before and after the clip, a time is held to its first and last tick
    let mut s = demo();
    s.state.selection = vec![overlay_clip];
    s.execute("clip.speedDuration", json!({"clips": [overlay_clip.0], "speed": 200.0, "reverse": true})).expect("the overlay plays backwards at 200 %");
    let item = s.active_sequence().and_then(|seq| seq.find_item(overlay_clip)).map(|(_, item)| item.clone()).expect("the overlay");
    assert!(item.reverse, "the overlay plays in reverse");
    let end = item.start + item.duration;
    let times = [
        Tick::ZERO,
        item.start,
        item.start + Tick(1),
        item.start + rate.tick_of(7) + Tick(3),
        item.start + item.duration.mul_ratio(1, 3),
        end - Tick(1),
        end,
        end + rate.tick_of(50),
    ];
    let refusal = |s: &Session, time: Tick| policy::keyframe_refusal("effects.addKeyframe", &add(time), || sequence_view(s));
    for time in times {
        let media = item.source_time_at(time.clamp(item.start, (end - Tick(1)).max(item.start)));
        let taken = opacity_keys(&s, overlay_clip).contains(&media);
        assert_eq!(refusal(&s, time), taken.then_some("keyframe_exists"), "an add at {time:?} (media {media:?})");
        if !taken {
            s.execute("effects.addKeyframe", add(time)).expect("the engine adds a keyframe");
            assert!(opacity_keys(&s, overlay_clip).contains(&media), "the engine added its keyframe at media {media:?}");
            assert_eq!(refusal(&s, time), Some("keyframe_exists"), "an add at {time:?} again");
        }
    }
    // a frame hold shows one media frame at every time: an add anywhere is at that frame
    let mut held = sequence_view(&demo());
    for (hold, decision) in [(84, Some("keyframe_exists")), (30, None)] {
        held["video_tracks"][1]["items"][0]["frame_hold"] = json!(rate.tick_of(hold).0);
        for time in [Tick::ZERO, start, start + rate.tick_of(30), start + rate.tick_of(90)] {
            let refused = policy::keyframe_refusal("effects.addKeyframe", &add(time), || held.clone());
            assert_eq!(refused, decision, "an add at {time:?} on a hold of media frame {hold}");
        }
    }

    // never the playhead: an add names its time and the others their keyframe's media time, as
    // whole numbers of ticks, in both runtimes
    let without = |params: &Value, key: &str| {
        let mut params = params.clone();
        if let Some(fields) = params.as_object_mut() {
            fields.remove(key);
        }
        params
    };
    for runtime in [policy::Runtime::Browser, policy::Runtime::Native] {
        assert!(policy::allowed("effects.addKeyframe", &add(start), runtime).is_ok(), "an add with its time ({runtime:?})");
        let mut wrong = add(start);
        for value in [json!("0"), json!(0.5), Value::Null] {
            wrong["time"] = value;
            assert_eq!(policy::allowed("effects.addKeyframe", &wrong, runtime).err(), Some("command_not_allowed"), "{wrong} ({runtime:?})");
        }
        assert_eq!(
            policy::allowed("effects.addKeyframe", &without(&add(start), "time"), runtime).err(),
            Some("command_not_allowed"),
            "an add without its time"
        );
        for command in ["effects.setKeyframe", "effects.setInterpolation", "effects.deleteKeyframe"] {
            assert!(policy::allowed(command, &keyframe(command, 12), runtime).is_ok(), "`{command}` with its media time ({runtime:?})");
            let refused = policy::allowed(command, &without(&keyframe(command, 12), "mediaTime"), runtime).err();
            assert_eq!(refused, Some("command_not_allowed"), "`{command}` without its media time ({runtime:?})");
        }
    }

    // `clip.enable` is a toggle too: on a shot that is enabled, as every clip of the demo is, a typed
    // call disables the shot and its linked sound, and the same call enables them again
    let mut s = demo();
    let shot = v1_clips(&s)[0];
    let link = s.active_sequence().and_then(|seq| seq.find_item(shot)).and_then(|(_, item)| item.link).expect("the demo's shots are linked to their sound");
    let states = |s: &Session| -> Vec<bool> {
        let seq = s.active_sequence().expect("the demo opens its main sequence");
        seq.all_tracks().flat_map(|track| track.items.iter()).filter(|item| item.link == Some(link)).map(|item| item.enabled).collect()
    };
    assert!(states(&s).len() >= 2 && states(&s).iter().all(|on| *on), "the shot and its sound are enabled");
    for on in [false, true] {
        let (expect, revision) = (expect_now(&s), s.revision);
        let envelope = execute_typed(&mut s, "clip.enable", json!({"clips": [shot.0]}), &expect, NOW_MS, false);
        assert_envelope(&envelope, "applied", None, revision, revision + 1);
        assert!(states(&s).iter().all(|enabled| *enabled == on), "a typed clip.enable leaves the shot and its sound enabled {on}: {envelope}");
    }
}

/// T-FI-15. A lift, an extract and a transcript extract or lift act on the tracks the person
/// targets (`Session::targeting`: the targeted tracks, and where a source's picture and sound are
/// patched), selecting transcript words may select clips on them (below), and changing the targeting
/// bumps no revision, so the caller's revision cannot catch the change. `executeTyped` therefore
/// checks the targeting the caller read (`watermark().targeting`, a digest that does not depend on
/// the order in which the tracks were targeted), right after the linked-selection setting and before
/// busy: a stale one is `targeting_changed`, decided before anything runs, and so is a missing one
/// for those five commands, which need it. Read afresh, the same transcript lift runs and cuts
/// exactly what the engine itself cuts under the person's targeting, which is not what it would have
/// cut under the targeting the stale caller read. Any other command checks a targeting the caller
/// names, and leaves out one it does not name. Selecting, extracting or lifting words moves the
/// playhead to their start; selecting them also sets the In point there. With the person's Selection
/// Follows Playhead setting on, the moved playhead also replaces the person's clip selection with the
/// clips under it on the targeted tracks, exactly as the engine's own seek to that time does; with
/// the setting off, the selection stays.
///
/// Fallback if this fails: `transcript.extract` and `transcript.lift` leave the table (residuals),
/// and the adapter reads the targeting itself just before a lift or an extract, without the hook's
/// atomicity.
#[test]
fn targeting_changed_refused_before_dispatch() {
    // the digest: the same tracks in any order, or listed twice, are the same targeting; other
    // tracks, or another patch, are another
    let digest = policy::targeting_digest;
    let base = digest(&[1, 2, 3], Some(1), Some(4));
    assert_eq!(digest(&[3, 1, 2, 3], Some(1), Some(4)), base, "the same tracks in another order");
    assert_ne!(digest(&[1, 2], Some(1), Some(4)), base, "other tracks");
    assert_ne!(digest(&[1, 2, 3], None, Some(4)), base, "no picture patch");
    assert_ne!(digest(&[1, 2, 3], Some(4), Some(1)), base, "the patches swapped");
    assert!(base.starts_with("fnv1a64:") && base.len() == 24, "{base}");

    // on the engine: the person untargets A1, which moves no revision and changes the digest;
    // targeting it again, which puts it at the end of the list, gives the first digest back
    let mut s = demo_with_transcript_and_captions();
    let (read, revision) = (expect_now(&s), s.revision);
    s.execute("timeline.setTargeting", json!({"track": "A1", "targeted": false})).expect("A1 is no longer targeted");
    assert_eq!(s.revision, revision, "targeting bumps no revision");
    assert_ne!(Some(targeting_of(&s)), read.targeting, "the digest follows the targeting");
    let mut again = demo_with_transcript_and_captions();
    for targeted in [false, true] {
        again.execute("timeline.setTargeting", json!({"track": "A1", "targeted": targeted})).expect("A1's targeting changes");
    }
    assert_eq!(Some(targeting_of(&again)), read.targeting, "the same tracks, targeted in another order, are the same targeting");

    // a caller that read the targeting before, or names none, is refused before dispatch by each of
    // the five commands that act on the targeted tracks or select clips on them
    let words = json!({"from": 0});
    let targeted = [
        ("transcript.select", words.clone()),
        ("transcript.extract", words.clone()),
        ("transcript.lift", words.clone()),
        ("sequence.lift", json!({})),
        ("sequence.extract", json!({})),
    ];
    for (command, params) in targeted {
        let unbound = policy::Expect { targeting: None, ..expect_now(&s) };
        for expect in [&read, &unbound] {
            let before = fingerprint(&s);
            let envelope = execute_typed(&mut s, command, params.clone(), expect, NOW_MS, false);
            assert_envelope(&envelope, "not_applied", Some("targeting_changed"), revision, revision);
            assert_eq!(fingerprint(&s), before, "`{command}` was refused before dispatch, the Events log included");
        }
    }

    // read afresh, the lift runs and cuts what the engine cuts under the person's targeting (A1 keeps
    // its sound), not what it would have cut under the targeting the stale caller read; the
    // playhead moves to the start of the words
    let mut engine = demo_with_transcript_and_captions();
    engine.execute("timeline.setTargeting", json!({"track": "A1", "targeted": false})).expect("A1 is no longer targeted");
    let cut = engine.execute("transcript.lift", words.clone()).expect("the engine lifts the words");
    let mut stale = demo_with_transcript_and_captions();
    stale.execute("transcript.lift", words.clone()).expect("the engine lifts the words on every targeted track");
    let fresh = expect_now(&s);
    let envelope = execute_typed(&mut s, "transcript.lift", words.clone(), &fresh, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    assert_eq!(envelope["result"], cut, "the typed lift answers what the engine's own does: {envelope}");
    assert_eq!(sequence_view(&s), sequence_view(&engine), "the typed lift cut what the engine cuts under the person's targeting");
    assert_ne!(sequence_view(&s), sequence_view(&stale), "under the targeting the stale caller read, A1 would have lost its sound as well");
    let start = cut["start"].as_i64().map(Tick).expect("the start of the words");
    assert_eq!(s.playhead(), start, "a lift moves the playhead to the start of the words");

    // with the targeting read afresh, selecting words sets the In point at their start and moves the
    // playhead there
    let mut s = demo_with_transcript_and_captions();
    let (fresh, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "transcript.select", words.clone(), &fresh, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    let start = envelope["result"]["start"].as_i64().map(Tick).expect("the start of the words");
    assert_eq!((s.playhead(), s.active_sequence().and_then(|seq| seq.mark_in)), (start, Some(start)), "{envelope}");

    // with the person's Selection Follows Playhead setting on, the moved playhead also replaces their
    // clip selection with the clips under it on the targeted tracks, as the engine's own seek there
    // does; with the setting off, their selection stays as they left it
    let mut seek = demo_with_transcript_and_captions();
    seek.execute("sequence.selectionFollowsPlayhead", json!({"on": true})).expect("the selection follows the playhead");
    seek.execute("playhead.set", json!({"time": start.0})).expect("the engine seeks to the words");
    let under = seek.state.selection.clone();
    for follows in [true, false] {
        let mut s = demo_with_transcript_and_captions();
        s.execute("sequence.selectionFollowsPlayhead", json!({"on": follows})).expect("the setting changes");
        let last = *v1_clips(&s).last().expect("the demo has shots on V1");
        assert!(!under.is_empty() && !under.contains(&last), "the last shot is not under the words: {under:?}");
        s.state.selection = vec![last];
        let (fresh, revision) = (expect_now(&s), s.revision);
        let envelope = execute_typed(&mut s, "transcript.select", words.clone(), &fresh, NOW_MS, false);
        assert_envelope(&envelope, "applied", None, revision, revision + 1);
        let kept = if follows { under.clone() } else { vec![last] };
        assert_eq!(s.state.selection, kept, "the clip selection with Selection Follows Playhead {follows}: {envelope}");
    }

    // any other command checks a targeting the caller names, and leaves out one it does not name
    let stale = policy::Expect { targeting: Some("a targeting nobody read".into()), ..expect_now(&s) };
    let before = fingerprint(&s);
    let envelope = execute_typed(&mut s, "playhead.set", json!({"time": 0}), &stale, NOW_MS, false);
    assert_envelope(&envelope, "not_applied", Some("targeting_changed"), before.revision, before.revision);
    assert_eq!(fingerprint(&s), before, "a stale targeting refuses any command before dispatch");
    let unbound = policy::Expect { targeting: None, ..expect_now(&s) };
    let envelope = execute_typed(&mut s, "playhead.set", json!({"time": 0}), &unbound, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, before.revision, before.revision);

    // its place in the order: after the linked-selection setting, before busy
    let both = policy::Expect { linked_selection: Some(!s.state.linked_selection), ..stale.clone() };
    let before = fingerprint(&s);
    for (expect, code) in [(&both, "linked_selection_changed"), (&stale, "targeting_changed")] {
        let envelope = execute_typed(&mut s, "transcript.lift", words.clone(), expect, NOW_MS, true);
        assert_envelope(&envelope, "not_applied", Some(code), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "`{code}` is decided before dispatch");
    }
}

/// The demo with the built-in lower third (`builtin:lower-third-slab`) at its start, and the clip it
/// made: a graphic made from a template, whose "Show Role" checkbox shows or hides the role line.
fn demo_with_lower_third() -> (Session, u64) {
    let mut s = demo();
    let applied = s.execute("graphics.template.apply", json!({"template": "builtin:lower-third-slab", "time": 0})).expect("the lower third applies");
    let clip = applied["clip"].as_u64().unwrap_or_else(|| panic!("the lower third's clip: {applied}"));
    (s, clip)
}

/// Whether the lower third's role line shows, as its "Show Role" checkbox reads.
fn role_shown(s: &mut Session, clip: u64) -> bool {
    let read = s.execute("graphics.template.controls", json!({"clip": clip})).expect("the lower third's controls read");
    let role = read["controls"].as_array().and_then(|controls| controls.iter().find(|control| control["id"] == json!("show_role"))).cloned();
    let role = role.unwrap_or_else(|| panic!("the lower third has a Show Role control: {read}"));
    assert_eq!(role["kind"], json!("checkbox"), "{role}");
    role["value"].as_bool().unwrap_or_else(|| panic!("the role's visibility reads: {role}"))
}

/// T-FI-16. `graphics.template.set` passes a template control's value on as it is given, and the
/// engine reads a value other than `true` or `false` of a checkbox that shows or hides a layer as
/// `true`: given the string `"false"`, the lower third's hidden role line shows again, and the call
/// succeeds. Typed, a checkbox given anything but `true` or `false`, named by its id or by its name
/// in any case, alone or among other values, is refused before dispatch as `control_kind_mismatch`,
/// with nothing run (no engine call: the Events log is unchanged too). A null value never reaches
/// the engine either, and neither does a layer's visibility of another type in `graphics.set`: the
/// table refuses those (`command_not_allowed`). With `true` or `false` the typed set runs and does
/// what it says; a value of another kind of control passes the pre-check; and a clip made from no
/// template, or another command, is never refused by it.
///
/// Fallback if this fails: the adapter reads the template's controls (`graphics.template.controls`)
/// and sends booleans for its checkboxes, without the hook's atomicity.
#[test]
fn checkbox_value_refused_before_dispatch() {
    // on the engine: false hides the role, and the string "false" shows it again
    let (mut s, clip) = demo_with_lower_third();
    assert!(role_shown(&mut s, clip), "the role shows as the template comes");
    s.execute("graphics.template.set", json!({"clip": clip, "control": "show_role", "value": false})).expect("the engine hides the role");
    assert!(!role_shown(&mut s, clip), "false hides the role");
    s.execute("graphics.template.set", json!({"clip": clip, "values": {"Show Role": "false"}})).expect("the engine takes a string for the checkbox");
    assert!(role_shown(&mut s, clip), "the engine read the string \"false\" as true and showed the role again");

    // typed, with the role hidden: a checkbox given anything but true or false is refused before
    // dispatch, by its id or its name in any case, alone or among other values
    let (mut s, clip) = demo_with_lower_third();
    s.execute("graphics.template.set", json!({"clip": clip, "control": "show_role", "value": false})).expect("the engine hides the role");
    let mismatched = [
        json!({"clip": clip, "values": {"show_role": "false"}}),
        json!({"clip": clip, "values": {"SHOW ROLE": 0}}),
        json!({"clip": clip, "control": "Show Role", "value": "false"}),
        json!({"clip": clip, "control": "show_role", "value": {"on": false}}),
        json!({"clip": clip, "values": {"name": "Dana Levi", "show_role": "no"}}),
    ];
    for params in mismatched {
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let envelope = execute_typed(&mut s, "graphics.template.set", params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("control_kind_mismatch"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "{params} was refused before dispatch, the Events log included");
    }
    // a null value, and a layer's visibility of another type, never reach the engine: the table
    // refuses them
    let untyped = [
        ("graphics.template.set", json!({"clip": clip, "values": {"Show Role": null}})),
        ("graphics.template.set", json!({"clip": clip, "control": "show_role", "value": null})),
        ("graphics.set", json!({"clip": clip, "layer": 0, "props": {"enabled": "false"}})),
    ];
    for (command, params) in untyped {
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let envelope = execute_typed(&mut s, command, params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("command_not_allowed"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "`{command}` {params} was refused before dispatch, the Events log included");
    }
    assert!(!role_shown(&mut s, clip), "the role is still hidden");

    // with true or false the typed set runs and does what it says, and a value of another kind of
    // control passes the pre-check
    let typed = [(json!({"clip": clip, "control": "Show Role", "value": true}), true), (json!({"clip": clip, "values": {"show_role": false}}), false)];
    for (params, shown) in typed {
        let (expect, revision) = (expect_now(&s), s.revision);
        let envelope = execute_typed(&mut s, "graphics.template.set", params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "applied", None, revision, revision + 1);
        assert_eq!(role_shown(&mut s, clip), shown, "{params}: {envelope}");
    }
    let (expect, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "graphics.template.set", json!({"clip": clip, "values": {"name": "Dana Levi"}}), &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);

    // the pre-check reads the clip's template from the engine's serde view: a clip made from no
    // template, a control the template does not have, and another command are never refused by it
    let view = sequence_view(&s);
    let refusal = |command: &str, params: Value| policy::template_refusal(command, &params, || view.clone());
    let shot = v1_clips(&s)[0];
    assert_eq!(refusal("graphics.template.set", json!({"clip": clip, "values": {"show_role": 1}})), Some("control_kind_mismatch"));
    assert_eq!(refusal("graphics.template.set", json!({"clip": shot.0, "values": {"show_role": 1}})), None, "a clip made from no template");
    assert_eq!(refusal("graphics.template.set", json!({"clip": clip, "values": {"no_such_control": 1}})), None, "no such control");
    assert_eq!(refusal("graphics.set", json!({"clip": clip, "values": {"show_role": 1}})), None, "another command");
}

// ------------------------------------------------------------------------------------ slice R1

/// T-FI-17 (slice R1; D13, backlog item 16). `window.filmcraft.executeChecked` refuses at the fork every call outside
/// its typed tables with `command_not_allowed`, before anything runs (no engine call: the Events log is unchanged too).
/// Since FF1 it ran any engine id, so the mirror adapter was the only allowlist (topology INV-12). Its tables are the
/// certified reads (`policy::READS`: the queries T-FI-01b, T-FI-01d and T-FI-01e certify pure, each with the keys it
/// was certified with) and the legacy edits the lab's mirror adapter sends there (`policy::CHECKED_EDITS`: a seek, a
/// new sequence, a place, a title and its colour, a dissolve and a trim), under the table's own keys and values, so the
/// lab keeps working on the R1 build until FF3 routes its writes through `executeTyped` (W5). Every other edit of the
/// table runs through `executeTyped` only, whose preconditions and pre-checks `executeChecked` does not run: an insert
/// across the overlay and the music bed (which the engine cuts in two, in silence: T-FI-19), an add where an opacity
/// keyframe sits (which the engine's toggle takes away: T-FI-14) and a template checkbox given a string (which the
/// engine reads as `true`: T-FI-16) are refused here with nothing run, while the same place as an overwrite runs.
/// Every rejection `executeChecked` decides before dispatch carries a registered `not_applied` code: its own
/// preconditions are refused in `executeTyped`'s order (`deadline_elapsed`, `revision_conflict`, `sequence_changed`),
/// one sent in a form it does not read included (it can never hold), with nothing run, and a command the engine
/// refuses itself before running it is `command_disabled`; a failure after dispatch carries none (T-FI-01c).
///
/// If this fails, the hook is not packaged (FF2): a fork entry point refuses what its tables do not hold (D13).
#[test]
fn out_of_table_id_refused_at_fork() {
    let runtimes = [policy::Runtime::Browser, policy::Runtime::Native];
    // every code a refusal of executeChecked carries is a registered not_applied code of the hook
    let refused_with = |code: &'static str| {
        assert!(policy::NOT_APPLIED.contains(&code), "`{code}` is a not_applied code of the hook");
        Some(code)
    };
    let not_allowed = refused_with("command_not_allowed");
    // on the engine, where executeChecked ran each of these before R1: a new project would have replaced the person's, a
    // preference or a save written outside the project, and a clear, a selection or a lift changed the edit
    let refused_on_the_engine = [
        ("file.newProject", json!({})),
        ("file.openDemoProject", json!({})),
        ("prefs.set", json!({"key": "timeline.stillDuration", "value": 1.0})),
        ("file.save", json!({})),
        ("markers.clearAll", json!({})),
        ("timeline.select", json!({"clips": [1]})),
        ("sequence.lift", json!({})),
        ("engine.execute", json!({"command": "file.newProject"})),
    ];
    for (id, params) in refused_on_the_engine {
        let mut s = demo();
        let (revision, sequence, before) = (s.revision, s.state.active_sequence.map(|seq| seq.0), fingerprint(&s));
        let refusal = execute_checked(&mut s, revision, sequence, id, params.clone()).expect_err("refused before dispatch");
        assert_eq!(refusal.code, not_allowed, "`{id}` {params}: {refusal}");
        assert_eq!(fingerprint(&s), before, "the refused `{id}` ran nothing, the Events log included");
    }
    // undo and redo need the caller's history head, which executeChecked cannot carry: refused even with an edit to undo
    let mut s = demo();
    let (revision, sequence) = (s.revision, s.state.active_sequence.map(|seq| seq.0));
    let last = *v1_clips(&s).last().expect("the demo has shots on V1");
    let shorter = json!({"clip": last.0, "edge": "out", "mode": "regular", "delta": -s.sequence_rate().frame_duration().0});
    let trimmed = execute_checked(&mut s, revision, sequence, "timeline.trim", shorter).expect("a legacy edit runs");
    assert_eq!(trimmed["revision"], json!(s.revision.to_string()), "{trimmed}");
    let (revision, before) = (s.revision, fingerprint(&s));
    let undo = execute_checked(&mut s, revision, sequence, "edit.undo", json!({})).expect_err("undo needs a history head");
    assert_eq!(undo.code, not_allowed, "{undo}");
    assert_eq!(fingerprint(&s), before, "the trim is still there: nothing ran");

    // the reads: exactly the certified queries, each a query the engine never journals, with the keys it was certified
    // with and nothing an edit takes
    let reads: BTreeSet<&str> = policy::READS.iter().map(|read| read.command).collect();
    assert_eq!(reads.len(), policy::READS.len(), "one entry per read");
    assert_eq!(reads, CERTIFIED_READS.iter().map(|(id, _, _)| *id).collect::<BTreeSet<&str>>(), "the certified reads, exactly");
    for (id, keys, required) in CERTIFIED_READS {
        let read = policy::READS.iter().find(|read| read.command == *id).unwrap_or_else(|| panic!("`{id}` is a read"));
        let spec = find_command(id).unwrap_or_else(|| panic!("`{id}` is a registered command"));
        assert!(!spec.journal, "`{id}` is a query: executeChecked reads it");
        let (read_keys, read_required): (BTreeSet<&str>, BTreeSet<&str>) = (read.keys.iter().copied().collect(), read.required.iter().copied().collect());
        assert_eq!(read_keys, keys.iter().copied().collect::<BTreeSet<&str>>(), "`{id}`: its keys");
        assert_eq!(read_required, required.iter().copied().collect::<BTreeSet<&str>>(), "`{id}`: its required keys");
        let documented = documented_keys(spec.params);
        for key in read.keys {
            assert!(documented.contains(*key) && rule_of(read, key).is_some(), "`{id}` reads `{key}`, documented and typed: {}", spec.params);
        }
        let plain = !read.history_head && !read.targeting && read.runtimes == policy::Runtimes::Both && read.targets.is_empty();
        assert!(plain && read.settings.is_empty() && read.needs.is_empty() && read.reads_with.is_none(), "`{id}`: a read takes nothing an edit does");
    }
    // the certified reads pass with the parameters the adapter sends; another key, a required key left out, a value of
    // another type or parameters that are not an object do not
    let s = demo();
    let (_, title) = demo_with_graphics();
    let extra = [("state.inspect", json!({})), ("graphics.list", json!({"clip": title.0}))];
    for (id, params) in certified_queries(&s).into_iter().chain(newsroom_queries()).chain(extra) {
        for runtime in runtimes {
            assert_eq!(policy::checked(id, &params, runtime), Ok(()), "`{id}` {params} ({runtime:?})");
        }
    }
    let off_table = [
        ("project.inspect", json!({"item": 1})),
        ("sequence.inspect", json!({"item": "1"})),
        ("fonts.list", json!({"system": "false"})),
        ("metadata.get", json!({})),
        ("metadata.get", json!({"clip": 1})),
        ("media.colorInfo", json!({"item": -1})),
        ("transcript.inspect", json!({"paragraphGapSeconds": 2.0})),
        ("captions.list", json!({"track": "C1"})),
        ("effects.list", json!({"detail": true, "kind": "Video"})),
        ("state.inspect", json!([])),
        ("graphics.list", json!({})),
        ("graphics.list", json!({"clip": 1.5})),
        ("history.list", Value::Null),
    ];
    for (id, params) in off_table {
        for runtime in runtimes {
            assert_eq!(policy::checked(id, &params, runtime), Err("command_not_allowed"), "`{id}` {params} ({runtime:?})");
        }
    }

    // the edits the lab's mirror adapter sends through executeChecked today (`creative-filmcraft-adapter.mjs`: seek, new
    // sequence, place, title and its colour, dissolve, trim) run as before, in the browser, and are the only edits it runs
    let (clip, item) = first_v1_clip(&s);
    let frame = s.sequence_rate().frame_duration().0;
    let mut sequence_params = json!({"name": "Cut", "width": 1920, "height": 1080, "fps": 25, "sampleRate": 48000, "video": 3, "audio": 3});
    for (key, value) in [("mix", "Stereo"), ("trackType", "Standard")] {
        sequence_params[key] = json!(value);
    }
    let mut title_params = json!({"text": "Title", "position": [960.0, 540.0], "newClip": true, "time": 0, "seconds": 2.0, "size": 64.0});
    for (key, value) in [("font", json!("Inter")), ("fontStyle", json!("Regular")), ("track", json!(1)), ("literal", json!(true))] {
        title_params[key] = value;
    }
    let place = json!({"item": item.0, "track": "V1", "audioTrack": "A1", "time": 0, "sourceIn": 0, "duration": TICKS_PER_SECOND, "insert": false});
    let legacy = [
        ("playhead.set", json!({"time": 0})),
        ("file.newSequence", sequence_params),
        ("timeline.place", place),
        ("graphics.newText", title_params),
        ("graphics.set", json!({"clip": clip.0, "layer": 0, "time": 0, "props": {"fill_color": "#ffffff"}})),
        ("sequence.applyVideoTransition", json!({"clip": clip.0, "edge": "out", "effect": "cross_dissolve", "frames": 12, "reverse": false})),
        ("timeline.trim", json!({"clip": clip.0, "edge": "out", "mode": "regular", "delta": -frame})),
        ("timeline.trim", json!({"clip": clip.0, "edge": "out", "mode": "ripple", "delta": -frame})),
    ];
    for (id, params) in &legacy {
        assert_eq!(policy::checked(id, params, policy::Runtime::Browser), Ok(()), "`{id}` {params}");
    }
    let legacy: BTreeSet<&str> = legacy.iter().map(|(id, _)| *id).collect();
    assert_eq!(legacy, policy::CHECKED_EDITS.iter().copied().collect::<BTreeSet<&str>>(), "the legacy edits, exactly");
    for id in policy::CHECKED_EDITS {
        let entry = entry_of(id);
        let plain = !entry.history_head && !entry.targeting && entry.runtimes == policy::Runtimes::Both && entry.targets.is_empty();
        assert!(plain && entry.reads_with.is_none(), "`{id}` needs no precondition or pre-check executeChecked does not run");
    }

    // every other edit of the table is refused, in both runtimes, with parameters the table takes; so is a key outside
    // the table
    for entry in policy::POLICY.iter().filter(|entry| !policy::CHECKED_EDITS.contains(&entry.command)) {
        let (id, params) = (entry.command, Value::Object(valid_params(entry, false)));
        assert!(runtimes.into_iter().any(|runtime| policy::allowed(id, &params, runtime).is_ok()), "`{id}` {params} is in the table");
        for runtime in runtimes {
            assert_eq!(policy::checked(id, &params, runtime), Err("command_not_allowed"), "`{id}` {params} ({runtime:?})");
        }
    }
    let in_frames = json!({"clip": clip.0, "edge": "out", "mode": "regular", "deltaFrames": 1});
    assert_eq!(policy::checked("timeline.trim", &in_frames, policy::Runtime::Browser), Err("command_not_allowed"), "a key outside the table");

    // what that spares: an insert at the start of the third shot across the overlay on V2 and the music bed on A2 (the
    // engine cuts both in two: T-FI-19), an add where the overlay's opacity keyframe at media frame 12 sits (the engine's
    // toggle takes it away: T-FI-14) and the lower third's checkbox given the string "false" (the engine reads it as
    // true: T-FI-16) are refused before anything runs, the Events log included
    let (at, _) = clip_span(&s, v1_clips(&s)[2]);
    let (overlay_clip, start) = overlay(&s);
    let keyframe = json!({"clip": overlay_clip.0, "effect": "opacity", "param": "opacity", "time": (start + s.sequence_rate().tick_of(12)).0});
    let (lower_third, role) = demo_with_lower_third();
    let spared = [
        (demo(), "timeline.place", insert_at(&s, at)),
        (demo(), "effects.addKeyframe", keyframe),
        (lower_third, "graphics.template.set", json!({"clip": role, "values": {"Show Role": "false"}})),
    ];
    for (mut s, id, params) in spared {
        let (revision, sequence, before) = (s.revision, s.state.active_sequence.map(|seq| seq.0), fingerprint(&s));
        let refusal = execute_checked(&mut s, revision, sequence, id, params.clone()).expect_err("refused before dispatch");
        assert_eq!(refusal.code, not_allowed, "`{id}` {params}: {refusal}");
        assert_eq!(fingerprint(&s), before, "the refused `{id}` {params} ran nothing, the Events log included");
    }
    // the same place as an overwrite runs, as the adapter's place does: its 24 frames replace the start of the third shot
    // on V1 and A1, and the overlay and the music bed stay whole
    let mut placed = demo();
    let mut overwrite = insert_at(&s, at);
    overwrite["insert"] = json!(false);
    let (revision, sequence, counts) = (placed.revision, placed.state.active_sequence.map(|seq| seq.0), split_counts(&placed));
    let reply = execute_checked(&mut placed, revision, sequence, "timeline.place", overwrite).expect("an overwrite runs");
    assert_eq!(reply["revision"], json!((revision + 1).to_string()), "{reply}");
    assert_eq!(split_counts(&placed), counts, "the overwrite cut nothing on V2 and A2");

    // executeChecked's own preconditions are decided before dispatch in executeTyped's order, with nothing run: a call
    // that fails all three is deadline_elapsed, then revision_conflict, then sequence_changed, until it runs
    let order = ["deadline_elapsed", "revision_conflict", "sequence_changed"];
    for holding in 0..=order.len() {
        let mut s = demo();
        let revision = if holding > 1 { s.revision } else { s.revision + 1 };
        let sequence = if holding > 2 { s.state.active_sequence.map(|seq| seq.0.to_string()) } else { Some(UNKNOWN_ITEM.to_string()) };
        let context = json!({"activeSequence": sequence, "deadlineMs": if holding > 0 { DEADLINE_MS } else { NOW_MS - 1.0 }});
        let before = fingerprint(&s);
        let answer = execute_checked_at(&mut s, &revision.to_string(), "project.inspect", json!({}), &context, NOW_MS);
        match order.get(holding).copied() {
            Some(code) => {
                let refusal = answer.expect_err("refused before dispatch");
                assert_eq!(refusal.code, refused_with(code), "{context}: {refusal}");
                assert_eq!(fingerprint(&s), before, "`{code}` is decided before dispatch: nothing ran, the Events log included");
            }
            None => assert!(answer.is_ok_and(|reply| reply["revision"] == json!(revision.to_string())), "every precondition holds: the read runs"),
        }
    }
    // a precondition in a form executeChecked does not read can never hold: it is refused with the code of the one it
    // stands for, in the same order, with nothing run
    let (revision, sequence) = (s.revision.to_string(), s.state.active_sequence.map(|seq| seq.0.to_string()));
    let malformed = [
        (revision.as_str(), json!({"activeSequence": sequence}), "deadline_elapsed"),
        (revision.as_str(), json!({"activeSequence": sequence, "deadlineMs": "soon"}), "deadline_elapsed"),
        (revision.as_str(), Value::Null, "deadline_elapsed"),
        ("f:7", json!({"activeSequence": sequence, "deadlineMs": DEADLINE_MS}), "revision_conflict"),
        ("", json!({"activeSequence": sequence, "deadlineMs": DEADLINE_MS}), "revision_conflict"),
        ("-1", json!({"activeSequence": sequence, "deadlineMs": DEADLINE_MS}), "revision_conflict"),
        (revision.as_str(), json!({"deadlineMs": DEADLINE_MS}), "sequence_changed"),
        (revision.as_str(), json!({"activeSequence": s.state.active_sequence.map(|seq| seq.0), "deadlineMs": DEADLINE_MS}), "sequence_changed"),
        (revision.as_str(), json!({"activeSequence": "seven", "deadlineMs": DEADLINE_MS}), "sequence_changed"),
    ];
    for (revision, context, code) in malformed {
        let mut s = demo();
        let before = fingerprint(&s);
        let refusal = execute_checked_at(&mut s, revision, "project.inspect", json!({}), &context, NOW_MS).expect_err("refused before dispatch");
        assert_eq!(refusal.code, refused_with(code), "{revision:?} {context}: {refusal}");
        assert_eq!(fingerprint(&s), before, "{revision:?} {context} was refused before dispatch, the Events log included");
    }
    // a command the engine refuses itself before running it (no sequence is open to seek in) is command_disabled, at the
    // cost of the one Warning line T-FI-10a pins
    let mut s = Session::default();
    let (revision, before) = (s.revision, fingerprint(&s));
    let refusal = execute_checked(&mut s, revision, None, "playhead.set", json!({"time": 0})).expect_err("no sequence is open");
    assert_eq!(refusal.code, refused_with("command_disabled"), "{refusal}");
    assert_eq!(without_log(fingerprint(&s)), without_log(before), "the disabled seek changed nothing but the Events log");

    // every other registered command, and every id that names none, is refused, in both runtimes
    let typed: BTreeSet<&str> = policy::POLICY.iter().map(|entry| entry.command).collect();
    for spec in command_specs() {
        if !reads.contains(spec.id) && !typed.contains(spec.id) {
            for runtime in runtimes {
                assert_eq!(policy::checked(spec.id, &json!({}), runtime), Err("command_not_allowed"), "`{}` is outside the typed tables", spec.id);
            }
        }
    }
    for id in ["", "engine.execute", "ui.click", "PROJECT.INSPECT", "project.inspect ", "projectInspect", "markers.delete", "state.inspect\0"] {
        for runtime in runtimes {
            assert_eq!(policy::checked(id, &json!({}), runtime), Err("command_not_allowed"), "{id:?} ({runtime:?})");
        }
    }
}

/// The transition a typed `sequence.applyVideoTransition` applied, by the id its envelope's result
/// names.
fn transition_of(s: &Session, envelope: &Value) -> Transition {
    let id = envelope["result"]["transition"].as_u64().unwrap_or_else(|| panic!("the applied transition's id: {envelope}"));
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let found = seq.video_tracks.iter().flat_map(|track| track.transitions.iter()).find(|transition| transition.id.0 == id);
    found.cloned().unwrap_or_else(|| panic!("transition {id} is on a video track"))
}

/// Where clip `clip` starts and ends on the timeline (ticks).
fn clip_span(s: &Session, clip: ClipId) -> (Tick, Tick) {
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let (_, item) = seq.find_item(clip).expect("the clip is on the timeline");
    (item.start, item.end())
}

/// T-FI-18 (slice R1, widen-1; topology §7.3, G2). A typed video transition is any of the engine's
/// catalogue in the ten Video Transitions folders, with its own settings: Push from the east with
/// motion blur at a cut, a linear wipe at an angle with a coloured border at the end of the edit.
/// The engine places it where its own command places every transition: a two-sided transition
/// centred on the cut, a one-sided one ending at the clip's Out point (or starting at an In point
/// with nothing before it). It documents no key that picks Start at Cut or End at Cut for a
/// two-sided one, so the table offers none (an engine change, widen-2). A setting the transition
/// does not have, a value of another kind, a legacy or obsolete transition, a display name and an
/// audio transition are refused before dispatch (`command_not_allowed`, nothing run): the engine
/// would fail inside the command on the first and read the rest its own way. The parameterless
/// dissolves run as before.
///
/// Fallback if this fails: typed transitions stay the five dissolves of R0, and FF3 sets the
/// settings through `sequence.setTransition` after a dissolve, checked over the certified sequence
/// read.
#[test]
fn widened_transitions_take_settings_where_the_engine_aligns_them() {
    let rate = demo().sequence_rate();
    let frames = 24;
    let duration = rate.tick_of(frames);
    // two-sided: Push from the east with motion blur at the free cut between the second and the third shot, which the
    // engine centres on the cut
    let mut s = demo();
    let shots = v1_clips(&s);
    let (_, cut) = clip_span(&s, shots[1]);
    let (expect, revision) = (expect_now(&s), s.revision);
    let push = json!({"clip": shots[1].0, "edge": "out", "effect": "push", "frames": frames, "params": {"direction": "From East", "motion_blur": 20.0}});
    let envelope = execute_typed(&mut s, "sequence.applyVideoTransition", push, &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    let push = transition_of(&s, &envelope);
    assert_eq!((push.from, push.to), (Some(shots[1]), Some(shots[2])), "Push runs from the second shot into the third");
    assert_eq!((push.start, push.duration), (cut - duration.mul_ratio(1, 2), duration), "a two-sided transition is centred on the cut");
    assert_eq!(push.effect.effect, "push");
    assert_eq!(push.effect.param("direction").map(|param| &param.value), Some(&ParamValue::Choice(1)), "From East");
    assert_eq!(push.effect.param("motion_blur").map(|param| &param.value), Some(&ParamValue::Float(20.0)));
    // one-sided: a linear wipe with a coloured border at the end of the edit, which ends at the last shot's Out point
    let last = *shots.last().expect("the demo has shots on V1");
    let (_, end) = clip_span(&s, last);
    let (expect, revision) = (expect_now(&s), s.revision);
    let wipe = json!({"clip": last.0, "edge": "out", "effect": "linear_wipe", "frames": frames, "params": {"angle": 45.0, "border_color": "#ff8000"}});
    let envelope = execute_typed(&mut s, "sequence.applyVideoTransition", wipe, &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    let wipe = transition_of(&s, &envelope);
    assert_eq!((wipe.from, wipe.to), (Some(last), None), "nothing follows the last shot");
    assert_eq!((wipe.start, wipe.duration), (end - duration, duration), "a one-sided transition at an Out point ends there");
    assert_eq!(wipe.effect.param("angle").map(|param| &param.value), Some(&ParamValue::Float(45.0)));
    assert_eq!(wipe.effect.param("border_color").map(|param| &param.value), Some(&ParamValue::Color([1.0, 128.0 / 255.0, 0.0, 1.0])));
    // neither stores an alignment the call chose: the command documents no key for one
    assert_eq!((push.align, wipe.align), (TransitionAlign::CenterAtCut, TransitionAlign::CenterAtCut), "the engine's default alignment");
    let doc = find_command("sequence.applyVideoTransition").expect("the engine applies video transitions").params;
    assert!(documented_keys(doc).iter().all(|key| !key.to_ascii_lowercase().contains("align")), "no alignment key: {doc}");

    // refused before dispatch: nothing runs, the Events log included
    let refused = [
        ("push", json!({"speed": 1.0})),
        ("push", json!({"direction": "east"})),
        ("push", json!({"direction": 4})),
        ("push", json!({"motion_blur": "20"})),
        ("linear_wipe", json!({"border_color": "orange"})),
        ("linear_wipe", json!({"border_color": [1.0, 0.5]})),
        ("wipe", json!({})),
        ("gradient_wipe", json!({})),
        ("Cross Dissolve", json!({})),
        ("constant_power", json!({})),
    ];
    for (effect, params) in refused {
        let mut s = demo();
        let shots = v1_clips(&s);
        let call = json!({"clip": shots[1].0, "edge": "out", "effect": effect, "frames": frames, "params": params});
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let envelope = execute_typed(&mut s, "sequence.applyVideoTransition", call.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("command_not_allowed"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "{call} was refused before dispatch, the Events log included");
    }
    // what the first of them spares: the engine fails inside the command on a setting Push does not have
    let mut s = demo();
    let shots = v1_clips(&s);
    let unknown = json!({"clip": shots[1].0, "edge": "out", "effect": "push", "frames": frames, "params": {"speed": 1.0}});
    let error = s.execute("sequence.applyVideoTransition", unknown).expect_err("the engine fails on a setting Push does not have");
    assert!(!matches!(error, EngineError::Disabled(..)), "inside the command, not as disabled: {error}");
    // the parameterless dissolves run as before, at the free cut between the fourth and the fifth shot
    let mut s = demo();
    let shots = v1_clips(&s);
    let (expect, revision) = (expect_now(&s), s.revision);
    let dissolve = json!({"clip": shots[3].0, "edge": "out", "effect": "film_dissolve", "frames": frames});
    let envelope = execute_typed(&mut s, "sequence.applyVideoTransition", dissolve, &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
}

/// The media item of the demo's first V1 clip, placed for 24 frames at `time` on V1 and A1,
/// inserting (a whole number of frames: the engine snaps a placed duration to the frame, so one
/// second at the demo's 23.976 fps would place 24 frames as well).
fn insert_at(s: &Session, time: Tick) -> Value {
    let (_, media) = first_v1_clip(s);
    let duration = s.sequence_rate().tick_of(24);
    json!({"item": media.0, "track": "V1", "audioTrack": "A1", "time": time.0, "sourceIn": 0, "duration": duration.0, "insert": true})
}

/// The clips on the demo's second video track (the overlay) and second audio track (the music bed),
/// and the captions on its first caption track (0 without one).
fn split_counts(s: &Session) -> (usize, usize, usize) {
    let seq = s.active_sequence().expect("the demo opens its main sequence");
    let captions = seq.caption_tracks.first().map_or(0, |track| track.captions.len());
    (seq.video_tracks[1].items.len(), seq.audio_tracks[1].items.len(), captions)
}

/// T-FI-19 (slice R1, widen-1, m19). An insert (`timeline.place` with `insert`) shifts later
/// material on the tracks it places on and on every unlocked, sync-locked track, and cuts in two any
/// clip or caption that runs across its time on a track the call does not name: the engine never
/// refuses that (an insert raises no sync-lock conflict), it splits the music bed or the overlay in
/// silence. Typed, such an insert is refused before dispatch by the ripple pre-check (`sync_locked`,
/// naming the tracks in `syncLockedTracks`, video, then audio, then caption tracks; nothing run):
/// the agent turns those tracks' sync lock off, cuts there first, or overwrites. The tracks the call
/// names are its own (their clips are cut by design); every other track counts, a patched track the
/// call leaves out included. An insert names its time (a need: at the playhead the pre-check could
/// not look). One with nothing in its way runs, and an overwrite is never in anything's way.
///
/// Fallback if this fails: `insert` stays narrowed (the broker refuses it `invalid_params`) until FF3
/// checks the tracks over the certified sequence read.
#[test]
fn insert_that_would_split_a_sync_locked_track_refused_before_dispatch() {
    // at the start of the third shot (9.5 s) the overlay on V2 (6-10 s) and the music bed on A2 run across the insert
    let mut s = demo();
    let shots = v1_clips(&s);
    let (at, _) = clip_span(&s, shots[2]);
    let params = insert_at(&s, at);
    let (expect, before) = (expect_now(&s), fingerprint(&s));
    let envelope = execute_typed(&mut s, "timeline.place", params.clone(), &expect, NOW_MS, false);
    assert_envelope(&envelope, "not_applied", Some("sync_locked"), before.revision, before.revision);
    assert_eq!(envelope["syncLockedTracks"], json!(["V2", "A2"]), "{envelope}");
    assert_eq!(fingerprint(&s), before, "the refused insert ran nothing, the Events log included");
    // what the refusal stands for: the engine, given the same insert, cuts the overlay and the music bed in two
    let (overlay, music, _) = split_counts(&s);
    s.execute("timeline.place", params).expect("the engine inserts");
    assert_eq!(split_counts(&s), (overlay + 1, music + 1, 0), "the engine's insert cut the overlay and the music bed");

    // a caption track counts too: 24 frames into the first shot the caption there (0-3 s) and the music run across
    let mut s = demo_with_transcript_and_captions();
    let params = insert_at(&s, s.sequence_rate().tick_of(24));
    let (expect, before) = (expect_now(&s), fingerprint(&s));
    let envelope = execute_typed(&mut s, "timeline.place", params.clone(), &expect, NOW_MS, false);
    assert_envelope(&envelope, "not_applied", Some("sync_locked"), before.revision, before.revision);
    assert_eq!(envelope["syncLockedTracks"], json!(["A2", "C1"]), "{envelope}");
    assert_eq!(fingerprint(&s), before, "the refused insert ran nothing");
    let (overlay, music, captions) = split_counts(&s);
    s.execute("timeline.place", params).expect("the engine inserts");
    assert_eq!(split_counts(&s), (overlay, music + 1, captions + 1), "the engine's insert cut the music bed and the caption");

    // the tracks the call names are its own: placing the picture on V2 cuts the overlay by design, the music is in the way
    let mut s = demo();
    let mut params = insert_at(&s, at);
    params["track"] = json!("V2");
    let (expect, before) = (expect_now(&s), fingerprint(&s));
    let envelope = execute_typed(&mut s, "timeline.place", params, &expect, NOW_MS, false);
    assert_envelope(&envelope, "not_applied", Some("sync_locked"), before.revision, before.revision);
    assert_eq!(envelope["syncLockedTracks"], json!(["A2"]), "{envelope}");

    // with the sync lock of V2 and A2 off the insert runs: the third shot moves 24 frames later, the overlay and the
    // music bed stay whole and where they were
    let mut s = demo();
    for track in ["V2", "A2"] {
        s.execute("timeline.setTrack", json!({"track": track, "syncLock": false})).expect("the sync lock turns off");
    }
    let (shots, params) = (v1_clips(&s), insert_at(&s, at));
    let (expect, revision, counts) = (expect_now(&s), s.revision, split_counts(&s));
    let envelope = execute_typed(&mut s, "timeline.place", params, &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    assert_eq!(clip_span(&s, shots[2]).0, at + s.sequence_rate().tick_of(24), "the third shot moved by the 24 frames inserted");
    assert_eq!(split_counts(&s), counts, "the overlay and the music bed stay whole");
    // nothing runs across the start of the edit: an insert there runs with every sync lock on
    let mut s = demo();
    let params = insert_at(&s, Tick::ZERO);
    let (expect, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "timeline.place", params, &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);

    // an overwrite is never in anything's way, whatever it covers
    let s = demo();
    let view = sequence_view(&s);
    let mut overwrite = insert_at(&s, at);
    overwrite["insert"] = json!(false);
    assert!(policy::sync_locked_tracks("timeline.place", &overwrite, true, || view.clone()).is_empty(), "an overwrite shifts nothing");
}

/// T-FI-20 (slice R1, widen-1, m22, BL item 3). The sequence marks are typed: Mark In and Mark Out at
/// a time in ticks the call names (never the playhead), on the program only (the source marks a
/// `target` would reach stay engine-only), and Clear In and Out. The engine stores Out as the start
/// of the last frame the range holds, and its own inspect read returns that raw, so a range read
/// from it ends one frame early; the typed envelope reports the marks after the edit half-open
/// instead (`result.marks`: `in`, `outInclusive` and `endExclusive` one frame after it, as decimal
/// strings of ticks, null when unset), so the agent sees the conversion. An Out before the In clears
/// the In, as the engine's own Mark Out does. Every other command's result stays the engine's.
///
/// Fallback if this fails: FF3 reads the marks back over the certified sequence read and converts
/// them itself.
#[test]
fn sequence_marks_reported_half_open() {
    let mut s = demo();
    let rate = s.sequence_rate();
    let at = |frame: i64| rate.tick_of(frame).0;
    let ticks = |frame: i64| json!(at(frame).to_string());
    let marks = |set_in: Value, out: Value, end: Value| json!({"marks": {"in": set_in, "outInclusive": out, "endExclusive": end}});
    let steps = [
        ("markers.markIn", json!({"time": at(30)}), "Mark In", marks(ticks(30), Value::Null, Value::Null)),
        ("markers.markOut", json!({"time": at(59)}), "Mark Out", marks(ticks(30), ticks(59), ticks(60))),
        ("markers.markOut", json!({"time": at(10)}), "Mark Out", marks(Value::Null, ticks(10), ticks(11))),
        ("markers.clearInOut", json!({}), "Clear In and Out", marks(Value::Null, Value::Null, Value::Null)),
    ];
    for (command, params, label, reported) in steps {
        let (expect, revision) = (expect_now(&s), s.revision);
        let envelope = execute_typed(&mut s, command, params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "applied", None, revision, revision + 1);
        assert_eq!(envelope["result"], reported, "`{command}` {params}: {envelope}");
        assert_eq!(envelope["historyLabel"], json!(label), "{envelope}");
    }
    // the report itself: ticks as decimal strings, half-open, null where a mark is not set; any other command answers
    // the engine's result
    let reported = json!({"marks": {"in": "10", "outInclusive": "20", "endExclusive": "25"}});
    assert_eq!(policy::marks_result("markers.markOut", Some(10), Some(20), 5), Some(reported));
    let cleared = json!({"marks": {"in": null, "outInclusive": null, "endExclusive": null}});
    assert_eq!(policy::marks_result("markers.clearInOut", None, None, 5), Some(cleared));
    assert_eq!(policy::marks_result("timeline.trim", Some(10), Some(20), 5), None, "another command's result is the engine's");
    // the source marks stay engine-only, and a mark never takes the playhead
    for runtime in [policy::Runtime::Browser, policy::Runtime::Native] {
        for params in [json!({"time": 0, "target": "source"}), json!({}), json!({"seconds": 1.0})] {
            assert_eq!(policy::allowed("markers.markIn", &params, runtime).err(), Some("command_not_allowed"), "{params} ({runtime:?})");
        }
    }
}

/// T-FI-21 (slice R1, widen-1). A track's lock, output (`enabled`, the eye of a video track) and mute
/// are typed beside its sync lock: a call names the track and at least one of the four switches,
/// each `true` or `false`; its solo, name, volume and pan stay out (widen-2). Each typed call is one
/// undo step ("Track Settings") that sets what it names and nothing else, and the ripple pre-check
/// reads the lock it sets: with the music bed's track locked, the ripple it blocked on the demo runs.
///
/// Fallback if this fails: the track state stays engine-only, and the sync lock is the one typed
/// switch (R0).
#[test]
fn track_mute_enable_and_lock_typed() {
    let state = |s: &Session, video: bool, index: usize| {
        let seq = s.active_sequence().expect("the demo opens its main sequence");
        let track = if video { &seq.video_tracks[index] } else { &seq.audio_tracks[index] };
        (track.muted, track.enabled, track.locked, track.sync_lock)
    };
    let mut s = demo();
    // (muted, enabled, locked, syncLock) after each call
    let calls = [
        (json!({"track": "A2", "muted": true}), false, 1, (true, true, false, true)),
        (json!({"track": "V2", "enabled": false}), true, 1, (false, false, false, true)),
        (json!({"track": "A2", "locked": true}), false, 1, (true, true, true, true)),
        (json!({"track": "V3", "syncLock": false, "locked": false}), true, 2, (false, true, false, false)),
    ];
    for (params, video, index, expected) in calls {
        let (expect, revision) = (expect_now(&s), s.revision);
        let envelope = execute_typed(&mut s, "timeline.setTrack", params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "applied", None, revision, revision + 1);
        assert_eq!(envelope["historyLabel"], json!("Track Settings"), "{envelope}");
        assert_eq!(state(&s, video, index), expected, "{params}: (muted, enabled, locked, syncLock)");
    }
    // the ripple pre-check reads the lock: the last shot one frame shorter by a ripple, which the music bed on A2 was in
    // the way of while A2 was unlocked (T-FI-10b), runs now that A2 is locked
    let last = *v1_clips(&s).last().expect("the demo has shots on V1");
    let frame = s.sequence_rate().frame_duration().0;
    let ripple = json!({"clip": last.0, "edge": "out", "mode": "ripple", "delta": -frame});
    assert!(policy::sync_locked_tracks("timeline.trim", &ripple, true, || sequence_view(&s)).is_empty(), "a locked track is in no ripple's way");
    let (expect, revision) = (expect_now(&s), s.revision);
    let envelope = execute_typed(&mut s, "timeline.trim", ripple, &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, revision, revision + 1);
    // a call names the track and at least one switch, each a boolean; solo, name, volume and pan stay out
    let refused = [
        json!({"track": "A1"}),
        json!({"muted": true}),
        json!({"track": "A1", "muted": "true"}),
        json!({"track": "A1", "enabled": 0}),
        json!({"track": "A1", "solo": true}),
        json!({"track": "A1", "name": "Music"}),
        json!({"track": "A1", "volumeDb": -6.0}),
        json!({"track": "A1", "pan": 10.0}),
    ];
    for params in refused {
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let envelope = execute_typed(&mut s, "timeline.setTrack", params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("command_not_allowed"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "{params} was refused before dispatch");
    }
}

/// The kinds of the layers of graphic clip `clip`, back to front, as `graphics.list` lists them.
fn layer_kinds(s: &mut Session, clip: ClipId) -> Vec<String> {
    let listed = s.execute("graphics.list", json!({"clip": clip.0})).expect("the graphic's layers read");
    let layers = listed["layers"].as_array().cloned().unwrap_or_default();
    layers.iter().map(|layer| layer["kind"].as_str().unwrap_or_default().to_string()).collect()
}

/// T-FI-22 (slice R1, widen-1). `graphics.arrangeLayer` moves a layer to a place in the paint order
/// by name (front, back, forward, backward) or, since R1, to an index from the back, as its params
/// doc offers. The engine clamps an index past the last layer to the front and reports where the
/// layer landed (`result.layer`). It reads any other number (a negative one, a fraction) as 0, the
/// back, and an index past `u32` would wrap in the browser's 32-bit `usize`: the table takes an
/// index from 0 within `u32` only (`command_not_allowed` otherwise, nothing run).
///
/// Fallback if this fails: arranging stays by name (R0), and FF3 reaches an index with repeated
/// forward or backward steps.
#[test]
fn arrange_layer_takes_an_index() {
    let (mut s, title) = demo_with_graphics();
    for shape in ["rectangle", "ellipse"] {
        s.execute("graphics.newShape", json!({"shape": shape, "clip": title.0})).expect("a shape layer joins the title");
    }
    assert_eq!(layer_kinds(&mut s, title), ["text", "Rectangle", "Ellipse"], "new layers go in front");
    // (to, layer moved, where it lands, the order after)
    let moves = [
        (json!(0), 2, 0, ["Ellipse", "text", "Rectangle"]),
        (json!(1), 0, 1, ["text", "Ellipse", "Rectangle"]),
        (json!(7), 0, 2, ["Ellipse", "Rectangle", "text"]),
        (json!("back"), 2, 0, ["text", "Ellipse", "Rectangle"]),
    ];
    for (to, layer, landed, order) in moves {
        let (expect, revision) = (expect_now(&s), s.revision);
        let params = json!({"clip": title.0, "layer": layer, "to": to});
        let envelope = execute_typed(&mut s, "graphics.arrangeLayer", params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "applied", None, revision, revision + 1);
        assert_eq!(envelope["result"], json!({"clip": title.0, "layer": landed}), "{params}: {envelope}");
        assert_eq!(layer_kinds(&mut s, title), order, "{params}");
    }
    // an index the browser and a native worker would not read alike, or a value the engine reads as 0, is refused
    for to in [json!(-1), json!(1.5), json!("1"), json!("top"), json!(true), json!(u64::from(u32::MAX) + 1)] {
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let params = json!({"clip": title.0, "layer": 0, "to": to});
        let envelope = execute_typed(&mut s, "graphics.arrangeLayer", params.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("command_not_allowed"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "{params} was refused before dispatch");
    }
    // what the table spares: the engine reads -1 as 0 and sends the front layer to the back
    let before = layer_kinds(&mut s, title);
    s.execute("graphics.arrangeLayer", json!({"clip": title.0, "layer": 2, "to": -1})).expect("the engine takes -1");
    assert_eq!(layer_kinds(&mut s, title)[0], before[2], "-1 sent the front layer to the back");
}

/// The fingerprint without the jobs list and the journal, the two things a look-only scene detection
/// adds to (a finished job, the call's journal line).
fn without_job(f: Fingerprint) -> Fingerprint {
    Fingerprint { journal_len: 0, journal_last: None, background: [0, f.background[1], f.background[2], f.background[3], f.background[4]], ..f }
}

/// T-FI-23 (slice R1, OC-33 E3). A scene detection that only looks (`lookOnly: true`) is the table's
/// one read (tier 0, `policy::reads_only`): in a native worker it runs to the end and reports the
/// clip's cuts and its shots as half-open timeline ranges in ticks, and changes nothing (no revision,
/// no history step, the same project and editor state; it leaves a finished job in `jobs.list` and a
/// journal line). Its envelope names no history label: the newest one on the undo stack is an
/// earlier edit's. It stays native only, as every scene detection does (the analysis holds the
/// editor's thread for as long as it runs), and it names no apply choice: with one it would read as
/// an edit that applies nothing.
///
/// Fallback if this fails: Film's scene detection runs only on a scratch copy in the native worker
/// (RD-10).
#[test]
fn look_only_scene_detection_reads_and_changes_nothing() {
    let mut s = demo();
    // an edit first: the undo stack's newest label is then one the read must not report as its own
    s.execute("markers.add", json!({"time": 0, "name": "before the read"})).expect("a marker goes on the sequence");
    let shot = v1_clips(&s)[0];
    let (start, end) = clip_span(&s, shot);
    let look = json!({"clips": [shot.0], "lookOnly": true});
    let (expect, before) = (expect_now(&s), fingerprint(&s));
    // in the browser it is native only, as every scene detection: refused before anything runs
    let envelope = execute_typed(&mut s, "clip.sceneEditDetection", look.clone(), &expect, NOW_MS, false);
    assert_envelope(&envelope, "not_applied", Some("native_only"), before.revision, before.revision);
    assert_eq!(fingerprint(&s), before, "nothing ran in the browser");
    // in a native worker it reads
    assert!(policy::reads_only("clip.sceneEditDetection", &look), "a look-only call only reads");
    let envelope = execute_typed_in(policy::Runtime::Native, &mut s, "clip.sceneEditDetection", look.clone(), &expect, NOW_MS, false);
    assert_envelope(&envelope, "applied", None, before.revision, before.revision);
    assert!(envelope.get("historyLabel").is_none(), "no history step, and not the marker's either: {envelope}");
    let clips = envelope["result"]["clips"].as_array().cloned().unwrap_or_default();
    assert_eq!(clips.len(), 1, "one clip analysed: {envelope}");
    assert_eq!(clips[0]["clip"], json!(shot.0), "{envelope}");
    let ranges = clips[0]["shots"].as_array().cloned().unwrap_or_default();
    let shots: Vec<(i64, i64)> = ranges.iter().filter_map(|range| Some((range[0].as_i64()?, range[1].as_i64()?))).collect();
    assert_eq!(shots.first().map(|shot| shot.0), Some(start.0), "the shots start at the clip's start: {envelope}");
    assert_eq!(shots.last().map(|shot| shot.1), Some(end.0), "and end at its end, half-open: {envelope}");
    assert!(shots.windows(2).all(|pair| pair[0].1 == pair[1].0), "each shot starts where the one before it ends: {envelope}");
    let after = fingerprint(&s);
    assert_eq!((after.background[0], after.journal_len), (before.background[0] + 1, before.journal_len + 1), "a finished job, a journal line");
    assert_eq!(without_job(after), without_job(before), "nothing else changed: the revision, the history, the project, the state");
    // a look-only call names no apply choice
    for apply in ["applyCuts", "createSubclips", "generateMarkers"] {
        let mut mixed = look.clone();
        mixed[apply] = json!(false);
        let (expect, before) = (expect_now(&s), fingerprint(&s));
        let envelope = execute_typed_in(policy::Runtime::Native, &mut s, "clip.sceneEditDetection", mixed.clone(), &expect, NOW_MS, false);
        assert_envelope(&envelope, "not_applied", Some("command_not_allowed"), before.revision, before.revision);
        assert_eq!(fingerprint(&s), before, "{mixed} was refused before dispatch");
    }
    // the history label, decided in `policy::settle` in both runtimes: an edit names the newest undo label, a read none
    let read = policy::settle("clip.sceneEditDetection", &look, 7, 7, Some("Add Marker"), policy::Answer::Ok(json!({"clips": []})));
    assert!(read.get("historyLabel").is_none(), "a read names no history label: {read}");
    let edit = policy::settle("markers.add", &json!({"time": 0}), 7, 8, Some("Add Marker"), policy::Answer::Ok(Value::Null));
    assert_eq!(edit["historyLabel"], json!("Add Marker"), "an edit names its own: {edit}");
}

/// T-FI-24 (slice R1, RD-6). Every code `executeTyped` answers with and `executeChecked` rejects
/// with (`policy::NOT_APPLIED`, `APPLIED`, `UNKNOWN`) and every code `renderFrame` and `scopes`
/// reject a read with (`policy::READ_FAILURES`) is registered for its kind in the closed code
/// registry, and every code the registry names this fork for is one of them: none reaches the broker
/// as `unknown`, where it would fence (master plan v3 §1.1). The `not_applied` codes are the ones this
/// file checks every envelope against. `agentView.codes` lists them frozen whole: `agent_view.rs`
/// freezes each list, then the object that holds them.
///
/// If this fails, the hook is not packaged (FF2) until the registry or the hook is corrected.
#[test]
fn hook_codes_are_registered() {
    let registered = |code: &str| REGISTERED_FILM_CODES.iter().find(|(name, _)| *name == code).map(|(_, kinds)| *kinds);
    let lists = [(policy::NOT_APPLIED, "not_applied"), (policy::APPLIED, "applied"), (policy::UNKNOWN, "unknown"), (policy::READ_FAILURES, "read_failure")];
    let mut ours = BTreeSet::new();
    for (codes, kind) in lists {
        for code in codes {
            let kinds = registered(code).unwrap_or_else(|| panic!("`{code}` is not registered for this fork"));
            assert!(kinds.contains(&kind), "`{code}` is registered as {kinds:?}, not as {kind}");
            assert!(ours.insert(*code), "`{code}` is listed once");
        }
    }
    let all: BTreeSet<&str> = REGISTERED_FILM_CODES.iter().map(|(code, _)| *code).collect();
    assert_eq!(ours, all, "every code registered for this fork is one the hook answers with");
    let not_applied: BTreeSet<&str> = policy::NOT_APPLIED.iter().copied().collect();
    assert_eq!(not_applied, BTreeSet::from(NOT_APPLIED_CODES), "the not_applied codes every envelope is checked against");
    assert_eq!((policy::APPLIED, policy::UNKNOWN), (&["clamped"][..], &["engine_error_after_dispatch"][..]), "a clamp, an engine failure");
    let reads: BTreeSet<&str> = policy::READ_FAILURES.iter().copied().collect();
    assert_eq!(reads, BTreeSet::from(["read_failed", "sequence_not_active", "budget_exceeded"]), "the read failures of renderFrame and scopes");
    let view = include_str!("../../../apps/filmcraft-web/src/agent_view.rs");
    let frozen = ["js_sys::Object::values(listed)", "js_sys::Object::freeze(list)", "js_sys::Object::freeze(listed)"];
    let at: Vec<usize> = frozen.into_iter().filter_map(|call| view.find(call)).collect();
    assert!(at.len() == frozen.len() && at.windows(2).all(|pair| pair[0] < pair[1]), "agent_view.rs freezes each list of agentView.codes, then the object");
}

/// The methods the control channel answers (`crates/ui-egui/src/control.rs`, `handle`), on which
/// `window.filmcraft.request` sends: the string patterns of the arms of its match on the method, as rustfmt lays them
/// out (eight spaces in), up to the end of that match.
fn control_methods() -> BTreeSet<&'static str> {
    let source = include_str!("../../ui-egui/src/control.rs");
    let (_, body) = source.split_once("match req.method.as_str() {").expect("the control channel matches on the method");
    let mut methods = BTreeSet::new();
    for arm in body.lines().take_while(|line| *line != "    }").filter(|line| line.starts_with("        \"")) {
        let (patterns, _) = arm.split_once("=>").unwrap_or_else(|| panic!("an arm of the method match: {arm}"));
        methods.extend(patterns.split('|').map(|pattern| pattern.trim().trim_matches('"')));
    }
    methods
}

/// T-FI-25 (slice R1; D13, backlog item 16, INV-12). In the LTS presentation (`?embedded`) the members of
/// `window.filmcraft` that dispatch by name send reads only (`policy::raw_request`): `filmcraft.request` the control
/// channel's reads (`policy::CONTROL_READS`: the command list and the menu tree, the interface state and its elements,
/// the performance counters, a screenshot and the timeline's hit tests) and, through `engine.execute` or
/// `ui.menu.invoke`, a certified read with the keys it was certified with, which is all `filmcraft.execute` sends (the
/// mirror adapter's inspection reads go there). Every other method and command is refused with `command_not_allowed`
/// before anything is sent. Before R1 a page script reached every engine id through these members, tier 3 included,
/// and every action of the interface through the control channel's synthetic input (`ui.click`, `ui.key`, `ui.type` and
/// the rest). The decision is pure, so it is pinned here over every method the control channel answers; `api.rs`, which
/// only the browser compiles, is checked to ask for it in those two members, and for the decisions of `executeChecked`
/// (T-FI-17) in its own, in order. The standalone presentation keeps every method, as upstream does.
///
/// If this fails, the hook is not packaged (FF2): a fork entry point refuses what its tables do not hold (D13).
#[test]
fn raw_members_send_only_reads_in_the_lts_presentation() {
    let not_allowed: Result<(), &str> = Err("command_not_allowed");
    let commands = ["engine.execute", "ui.menu.invoke"];
    // every method the control channel answers is one of its reads, which is sent, or refused before anything is sent: a
    // command must name a read, and the synthetic input, the interface's state, playback and the window never go
    let methods = control_methods();
    let input = ["ui.click", "ui.move", "ui.drag", "ui.scroll", "ui.key", "ui.type", "ui.set", "ui.panel.show", "ui.playback", "app.quit"];
    for method in policy::CONTROL_READS.iter().chain(&input).chain(&commands) {
        assert!(methods.contains(method), "`{method}` is a method of the control channel: {methods:?}");
    }
    for method in &methods {
        let sent = if policy::CONTROL_READS.contains(method) { Ok(()) } else { not_allowed };
        assert_eq!(policy::raw_request(method, &json!({})), sent, "`{method}`");
    }
    for method in ["", "UI.INSPECT", "ui.inspect ", "ui.unknown", "executeChecked"] {
        assert_eq!(policy::raw_request(method, &json!({})), not_allowed, "{method:?}");
    }
    assert_eq!(policy::raw_request("ui.screenshot", &json!({"panel": "Program"})), Ok(()), "the adapter's preview capture");

    // a command is a certified read with the keys it was certified with, named as the control channel reads it (`command`,
    // else `id`) with its `params` (`{}` when left out): the mirror adapter's inspection reads and every other certified
    // read are sent, as executeChecked reads them too
    let s = demo();
    let (_, title) = demo_with_graphics();
    let extra = [("state.inspect", json!({})), ("graphics.list", json!({"clip": title.0}))];
    for (id, params) in certified_queries(&s).into_iter().chain(newsroom_queries()).chain(extra) {
        let by_command = json!({"command": id, "params": params});
        let by_id = json!({"id": id, "params": params});
        let by_id_after_a_number = json!({"command": 7, "id": id, "params": params});
        for method in commands {
            for named in [&by_command, &by_id, &by_id_after_a_number] {
                assert_eq!(policy::raw_request(method, named), Ok(()), "`{method}` {named}");
            }
        }
        assert_eq!(policy::checked(id, &params, policy::Runtime::Browser), Ok(()), "executeChecked reads `{id}` {params} too");
    }
    for method in commands {
        assert_eq!(policy::raw_request(method, &json!({"command": "project.inspect"})), Ok(()), "a read that takes no key, its params left out");
    }

    // refused: every edit of the table, the legacy ones executeChecked runs included (an edit names the revision it was
    // decided on there, or goes through executeTyped), every other registered command, a read with another key, without
    // its required key or with parameters that are not an object, a name in another case, a command named by both keys
    // (the channel reads `command`), and a request that names none
    let mut refused = vec![
        json!({}),
        json!({"params": {}}),
        json!({"Command": "project.inspect"}),
        json!({"command": "PROJECT.INSPECT"}),
        json!({"command": "file.save", "id": "project.inspect"}),
        json!({"command": "metadata.get", "params": {"clip": 1}}),
        json!({"command": "metadata.get", "params": {}}),
        json!({"command": "project.inspect", "params": []}),
        json!({"command": "project.inspect", "params": null}),
        json!({"command": "file.open", "params": {"path": "/projects/other.fcproj"}}),
        json!(["project.inspect"]),
        json!("project.inspect"),
    ];
    for entry in policy::POLICY {
        refused.push(json!({"command": entry.command, "params": Value::Object(valid_params(entry, false))}));
    }
    for spec in command_specs() {
        if !policy::READS.iter().any(|read| read.command == spec.id) {
            refused.push(json!({"command": spec.id}));
        }
    }
    for method in commands {
        for named in &refused {
            assert_eq!(policy::raw_request(method, named), not_allowed, "`{method}` {named}");
        }
    }

    // api.rs, which only the browser compiles, asks for these decisions in the members that dispatch by name: `request`
    // and `execute` through `by_name`, which in the LTS presentation asks `raw_request` before it sends anything; and for
    // executeChecked's (T-FI-17) in that member, in order: its tables, the form of its preconditions, then in its
    // UI-thread closure their guard, `Session::execute` and the code of a failure
    let api = include_str!("../../../apps/filmcraft-web/src/api.rs");
    let member = |name: &str| {
        let (_, installed) = api.split_once(&format!("\"{name}\",")).unwrap_or_else(|| panic!("api.rs installs `{name}`"));
        installed.split("set_fn(").next().unwrap_or_default()
    };
    let in_order = |text: &str, calls: &[&str]| {
        let at: Vec<usize> = calls.iter().copied().filter_map(|call| text.find(call)).collect();
        at.len() == calls.len() && at.windows(2).all(|pair| pair[0] < pair[1])
    };
    assert!(member("request").contains("by_name(&method, "), "`filmcraft.request` sends through by_name");
    assert!(member("execute").contains("by_name(\"engine.execute\", "), "`filmcraft.execute` sends through by_name");
    let (_, by_name) = api.split_once("fn by_name(").expect("api.rs has by_name");
    let by_name = by_name.split("\n}").next().unwrap_or_default();
    assert!(in_order(by_name, &["crate::embedded()", "policy::raw_request(method, &params)", "request(method, params)"]), "by_name: {by_name}");
    let checked = member("executeChecked");
    let decisions = [
        "policy::checked(&command, &params, policy::Runtime::HERE)",
        "policy::CheckedExpect::from_call(&revision, &context)",
        "crate::import::on_ui(",
        "policy::checked_guard(&expect, ",
        "session.execute(&command, params)",
        "policy::checked_failure(&command, ",
    ];
    assert!(in_order(checked, &decisions), "executeChecked: {checked}");
}

// ------------------------------------------------------------------------------------ slice RN (native reach)

/// T-FI-RN-01 (slice RN, native reach, 2026-10-23: vertical shorts cut from a broadcast episode, which only a native
/// worker opens). A native worker runs Modify Audio Channels and Breakout to Mono on media items by id, a media item's
/// start timecode in frames, the active sequence's frame size, name and mix, another sequence opened by its item (one
/// session cuts several shorts), and Crop and Auto Reframe (added with
/// `effects.apply`, set and keyframed through the entries that set Motion), each with checked parameters; the browser
/// refuses every one of them `native_only`, and its own entries are as they were. The caption file commands, Make
/// Subsequence, the nest switch and Field Options stay outside the table: a native worker imports and exports caption
/// files through its own operations, the subsequence and the switch take the person's selection and preferences, and
/// the renderer reads no field options. The certified reads a native host reads source facts and captions with stay
/// certified.
#[test]
fn native_reach_entries_run_in_a_native_worker_only() {
    let native = |command: &str, params: &Value| policy::allowed(command, params, policy::Runtime::Native).map(|entry| entry.command);
    let browser = |command: &str, params: &Value| policy::allowed(command, params, policy::Runtime::Browser).map(|entry| entry.command);
    let admitted = [
        ("clip.audioChannels", json!({"items": [1], "format": "stereo", "clips": [[2, 3]]})),
        ("clip.audioChannels", json!({"items": [1, 2]})),
        ("clip.audioChannels", json!({"items": [1], "format": "mono", "clips": [[0], [1], [2], [3]]})),
        ("clip.breakoutToMono", json!({"items": [1]})),
        ("clip.modifyTimecode", json!({"item": 1, "frame": 90_000})),
        ("clip.modifyTimecode", json!({"item": 1, "reset": true})),
        ("clip.modifyTimecode", json!({"item": 1, "tapeName": "EP 4"})),
        ("sequence.settings", json!({"width": 1080, "height": 1920})),
        ("sequence.settings", json!({"name": "Short 1", "mix": "Stereo"})),
        ("sequence.open", json!({"item": 7})),
        ("effects.apply", json!({"clips": [1], "effect": "crop"})),
        ("effects.apply", json!({"clips": [1], "effect": "auto_reframe"})),
        ("effects.setParam", json!({"clip": 1, "effect": "crop", "param": "left", "value": 12.5, "time": 0})),
        ("effects.setParam", json!({"clip": 1, "effect": "auto_reframe", "param": "offset", "value": [10.0, -4.0], "time": 0})),
        ("effects.addKeyframe", json!({"clip": 1, "effect": "crop", "param": "bottom", "time": 0})),
        ("effects.setKeyframe", json!({"clip": 1, "effect": "auto_reframe", "param": "zoom", "mediaTime": 0, "value": 120.0})),
        ("effects.setInterpolation", json!({"clip": 1, "effect": "crop", "param": "top", "mediaTime": 0, "interpolation": "linear"})),
        ("effects.deleteKeyframe", json!({"clip": 1, "effect": "crop", "param": "feather", "mediaTime": 0})),
    ];
    for (command, params) in &admitted {
        assert_eq!(native(command, params), Ok(*command), "`{command}` {params} natively");
        assert_eq!(browser(command, params), Err("native_only"), "`{command}` {params} in the browser");
    }
    let refused = [
        ("clip.audioChannels", json!({"format": "stereo"})),
        ("clip.audioChannels", json!({"items": [1], "format": "Stereo"})),
        ("clip.audioChannels", json!({"items": [1], "clips": [[256]]})),
        ("clip.audioChannels", json!({"items": [1], "clips": [[]]})),
        ("clip.audioChannels", json!({"items": [1], "clips": []})),
        ("clip.audioChannels", json!({"items": [1], "clips": [2, 3]})),
        ("clip.audioChannels", json!({"items": [1], "channels": [0, 1]})),
        ("clip.breakoutToMono", json!({})),
        ("clip.modifyTimecode", json!({"item": 1})),
        ("clip.modifyTimecode", json!({"item": 1, "timecode": "01:00:00:00"})),
        ("clip.modifyTimecode", json!({"item": 1, "frame": -1})),
        ("clip.modifyTimecode", json!({"item": 1, "frame": 86_400_001})),
        ("sequence.settings", json!({"width": 1080})),
        ("sequence.settings", json!({})),
        ("sequence.settings", json!({"width": 1080, "height": 1920, "fps": 25.0})),
        ("sequence.settings", json!({"mix": "stereo"})),
        ("sequence.open", json!({})),
        ("sequence.open", json!({"item": "7"})),
        ("effects.setParam", json!({"clip": 1, "effect": "crop", "param": "zoom", "value": true, "time": 0})),
        ("effects.setParam", json!({"clip": 1, "effect": "auto_reframe", "param": "aspect", "value": 1, "time": 0})),
        ("effects.setParam", json!({"clip": 1, "effect": "crop", "param": "left", "value": "10", "time": 0})),
        ("effects.apply", json!({"clips": [1], "effect": "lumetri"})),
        ("captions.import", json!({"path": "/files/x.srt"})),
        ("captions.export", json!({"path": "/lts-exports/x.srt"})),
        ("sequence.makeSubsequence", json!({})),
        ("sequence.nestSequences", json!({"on": true})),
        ("clip.fieldOptions", json!({"clips": [1], "processing": "alwaysDeinterlace"})),
    ];
    for (command, params) in &refused {
        for runtime in [policy::Runtime::Native, policy::Runtime::Browser] {
            assert_eq!(policy::allowed(command, params, runtime).err(), Some("command_not_allowed"), "`{command}` {params} ({runtime:?})");
        }
    }
    let unchanged = [
        ("effects.apply", json!({"clips": [1], "effect": "gaussian_blur"})),
        ("effects.setParam", json!({"clip": 1, "effect": "motion", "param": "position", "value": [960.0, 540.0], "time": 0})),
        ("effects.addKeyframe", json!({"clip": 1, "effect": "opacity", "param": "opacity", "time": 0})),
    ];
    for (command, params) in &unchanged {
        assert_eq!(browser(command, params), Ok(*command), "`{command}` {params} in the browser");
    }
    for (command, params) in [("metadata.get", json!({"item": 1})), ("media.colorInfo", json!({"item": 1})), ("captions.list", json!({}))] {
        assert!(policy::checked(command, &params, policy::Runtime::Native).is_ok(), "`{command}` is a certified read");
    }
}
