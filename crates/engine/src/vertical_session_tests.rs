//! Vertical Edit session plumbing (plan 63 §3.3, lane L2a): what undo steps record, the plan-only commit path (folding,
//! no-ops, refusals), the picture revision and the project epoch (T-N19, mechanics half), the schema written by content
//! through saves, auto-saves and recovery (T-N20, engine half), the id high-water marks, structured refusals, the clock,
//! the media pool generation, the preferences, the selection and the shortcut scopes. Synthetic media only: the demo
//! project's generators.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, SharedSource};
use filmcraft_project::vertical::{
    Arrival, Curve, DEFAULT_CURVE, Origin, OutputSize, Position, ProtectedRegion, Refusal, RegionMode, StoredWall, VerticalPlan, VerticalSet, WallOrigin,
    Window, Writer,
};
use filmcraft_project::{ItemId, Project};
use filmcraft_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::autosave::{AutosaveConfig, Preferences};
use crate::commands::CommandSpec;
use crate::media_test_util::tmp_dir;
use crate::settings::VerticalPrefs;
use crate::{Clock, EngineError, Event, IdCounter, MediaPool, PlanIdMarks, Session, StepMeta, VerticalSelection, empty_vertical_set};

/// The schema a plain project is written in: the one before Vertical Edit's step, which stays the last one.
const PLAIN: u32 = filmcraft_format::VERTICAL_SCHEMA - 1;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn active(s: &Session) -> ItemId {
    s.state.active_sequence.unwrap()
}

/// A vertical set with one empty 9:16 plan (id 1), as a project file holds it.
fn one_plan_set() -> VerticalSet {
    let stored = json!({"plans": [], "walls": {"useEditPoints": true}, "nextPlanId": 2, "nextPartId": 1, "nextRegionId": 1, "nextAnnotationId": 1});
    let mut set: VerticalSet = serde_json::from_value(stored).unwrap();
    set.plans.push(VerticalPlan::new(1, "9:16", OutputSize { width: 1080, height: 1920 }));
    set
}

/// Give the active sequence [`one_plan_set`] through the plan-only commit path; its id.
fn create_plan(s: &mut Session) -> ItemId {
    let seq = active(s);
    s.edit_plan_only(seq, "Vertical: new plan 9:16", None, |_, set, _| {
        *set = Some(Box::new(one_plan_set()));
        Ok(())
    })
    .unwrap();
    seq
}

/// Add a cut at `frame` to plan 1 of the active sequence as the planner allocates one: its id from the plan's counter.
fn add_cut(s: &mut Session, frame: i64) -> u32 {
    let seq = active(s);
    s.edit_plan_only(seq, "Vertical: add position", None, move |q, set, _| {
        let at = q.settings.frame_rate.tick_of(frame);
        let window = Window { cx: f64::from(q.settings.width) / 2.0, cy: f64::from(q.settings.height) / 2.0, h: f64::from(q.settings.height) };
        let plan = set.as_deref_mut().and_then(|set| set.plans.first_mut()).ok_or_else(|| EngineError::Other("no plan".into()))?;
        let id = plan.next_position_id;
        plan.next_position_id += 1;
        plan.positions.push(Position::new(id, at, window, Arrival::Cut, Origin::Manual, Writer::Human));
        plan.positions.sort_by_key(|p| p.at);
        Ok(id)
    })
    .unwrap()
}

/// The position counter of plan 1 of sequence `seq` in `project`, as stored.
fn counter_in(project: &Project, seq: ItemId) -> u32 {
    project.sequence(seq).and_then(|q| q.vertical.as_deref()).map(|set| set.plans[0].next_position_id).unwrap()
}

fn position_counter(s: &Session, seq: ItemId) -> u32 {
    counter_in(&s.project, seq)
}

/// The position mark of plan 1 of sequence `seq`.
fn position_mark(s: &Session, seq: ItemId) -> Option<PlanIdMarks> {
    s.vertical_ids.sets.get(&seq).and_then(|marks| marks.plans.get(&1)).copied()
}

fn decode_file(path: impl AsRef<Path>) -> Project {
    filmcraft_format::decode(&std::fs::read(path).unwrap()).unwrap().project
}

fn upgraded_toast(s: &mut Session) -> Option<String> {
    s.drain_events().into_iter().find_map(|e| match e {
        Event::Toast { message, .. } if message.contains("Upgraded") => Some(message),
        _ => None,
    })
}

/// Select the cut after the first V1 clip as a roll and start trimming it dynamically (sync locks off, as the trim tests
/// do).
fn start_dynamic_roll(s: &mut Session) {
    for t in ["V2", "V3", "A2", "A3"] {
        let _ = s.execute("timeline.setTrack", json!({"track": t, "syncLock": false}));
    }
    let clip = s.active_sequence().unwrap().video_tracks[0].items[0].id.0;
    s.execute("trim.selectEditPoint", json!({"clip": clip, "edge": "out", "kind": "roll"})).unwrap();
    s.execute("trim.shuttle", json!({"direction": "forward", "clock": 0.0})).unwrap();
    s.execute("trim.tick", json!({"clock": 0.5})).unwrap();
    assert!(s.trim_play.dynamic.is_some(), "a dynamic trim runs");
}

/// Start auto-save and the recovery journal with their data in `data`, the journal written 20 ms after a change.
fn start_journal(s: &mut Session, data: &Path) {
    let mut cfg = AutosaveConfig::new(data);
    cfg.journal_debounce = Duration::from_millis(20);
    s.start_autosave(cfg).unwrap();
}

/// Hand the session's state to the auto-save worker and wait until the recovery journal holds it.
fn wait_for_journal(s: &mut Session) {
    s.sync_persistence();
    let t0 = std::time::Instant::now();
    while s.persistence.as_ref().map(|p| p.status.journaled_revision) != Some(s.revision) {
        assert!(t0.elapsed() < Duration::from_secs(10), "timed out waiting for the journal");
        std::thread::sleep(Duration::from_millis(10));
        s.poll_persistence();
    }
}

/// Nudge the window of the first position of plan 1 left by `dx` through the plan-only commit path, as a drag sub-step
/// with gesture key `key`.
fn nudge_window(s: &mut Session, label: &str, key: Option<&str>, dx: f64) {
    let seq = active(s);
    s.edit_plan_only(seq, label, key, move |_, set, _| {
        if let Some(p) = set.as_deref_mut().and_then(|set| set.plans.first_mut()).and_then(|plan| plan.positions.first_mut()) {
            p.window.cx -= dx;
        }
        Ok(())
    })
    .unwrap();
}

/// Every step records `StepMeta {writer: Human, picture, run: None}`: an edit, a command folded from several steps and
/// a dynamic trim are picture steps, a plan-only commit is not; undo and redo carry the meta; `history.list` keeps the
/// labels.
#[test]
fn vertical_session_steps_record_their_writer_picture_and_run() {
    let picture = StepMeta { writer: Writer::Human, picture: true, run: None };
    let plan_only = StepMeta { writer: Writer::Human, picture: false, run: None };
    let mut s = demo();
    assert_eq!(s.writer, Writer::Human, "the desktop session writes as a person");
    // a command folded from several steps (default transitions on a selected clip) is one picture step
    let clip = s.active_sequence().unwrap().video_tracks[0].items[2].id.0;
    s.execute("timeline.select", json!({"clips": [clip]})).unwrap();
    let steps = s.history.undo.len();
    s.execute("trim.applyDefaultTransition", json!({})).unwrap();
    assert_eq!(s.history.undo.len(), steps + 1);
    assert_eq!(s.history.undo.last().unwrap().2, picture);
    // so is an edit
    s.execute("playhead.set", json!({"seconds": 2.0})).unwrap();
    s.execute("sequence.addEditAllTracks", json!({})).unwrap();
    assert_eq!(s.history.undo.last().unwrap().2, picture);
    // a plan-only commit is not
    let seq = create_plan(&mut s);
    let last = s.history.undo.last().unwrap();
    assert_eq!((last.label(), &last.2), ("Vertical: new plan 9:16", &plan_only), "a plan-only commit records picture: false");
    // undo and redo carry the meta
    s.undo();
    assert_eq!(s.history.redo.last().unwrap().2, plan_only);
    s.redo();
    assert_eq!(s.history.undo.last().unwrap().2, plan_only);
    assert!(s.project.sequence(seq).is_some_and(|q| q.vertical.is_some()));
    // a committed dynamic trim is a picture step
    let mut t = demo();
    start_dynamic_roll(&mut t);
    t.execute("trim.shuttleStop", json!({"clock": 1.0})).unwrap();
    let last = t.history.undo.last().unwrap();
    assert_eq!((last.label(), &last.2), ("Dynamic Rolling Edit", &picture));
    // history.list shows the labels, as before
    let listed = s.execute("history.list", json!({})).unwrap();
    let labels: Vec<&str> = s.history.undo.iter().map(|step| step.label()).collect();
    assert_eq!(listed["undo"], json!(labels));
    assert!(labels.contains(&"Vertical: new plan 9:16"));
}

/// T-N19 (mechanics): a plan-only step, its undo and its redo keep the picture revision; every other journaled edit,
/// its undo and redo, and a proxy switch advance it; reads change neither revision.
#[test]
fn vertical_session_t_n19_plan_only_steps_keep_the_picture_revision() {
    let mut s = demo();
    create_plan(&mut s);
    let (revision, picture) = (s.revision, s.picture_revision);
    add_cut(&mut s, 0);
    assert_eq!((s.revision, s.picture_revision), (revision + 1, picture), "a plan-only step moves the revision only");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!((s.revision, s.picture_revision), (revision + 3, picture), "so do its undo and redo");
    assert_eq!(s.picture_seen, s.revision, "every change was accounted for");
    s.execute("playhead.set", json!({"seconds": 2.0})).unwrap();
    for (id, params) in [("sequence.addEditAllTracks", json!({})), ("edit.undo", json!({})), ("edit.redo", json!({})), ("markers.add", json!({}))] {
        let (revision, picture) = (s.revision, s.picture_revision);
        s.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(s.revision > revision, "`{id}` is an edit");
        assert_eq!(s.picture_revision - picture, s.revision - revision, "`{id}` advances the picture revision with every revision");
        assert_eq!(s.picture_seen, s.revision);
    }
    // a change of what the media shows (proxies on) advances it too
    let picture = s.picture_revision;
    s.execute("prefs.set", json!({"key": "media.enableProxies", "value": true})).unwrap();
    assert!(s.picture_revision > picture, "switching to proxies changes what the frames show");
    // reads change nothing
    let (revision, picture) = (s.revision, s.picture_revision);
    s.execute("history.list", json!({})).unwrap();
    s.execute("state.inspect", json!({})).unwrap();
    assert_eq!((s.revision, s.picture_revision), (revision, picture));
}

/// T-N19: a drag of 20 merged plan-only sub-steps with one key and label is one undo step and keeps the picture
/// revision; another label, another key or another step in between starts a new step.
#[test]
fn vertical_session_t_n19_twenty_merged_sub_steps_make_one_undo_step() {
    let mut s = demo();
    let seq = create_plan(&mut s);
    add_cut(&mut s, 0);
    let cx = |s: &Session| s.project.sequence(seq).and_then(|q| q.vertical.as_deref()).map(|set| set.plans[0].positions[0].window.cx).unwrap();
    let nudge = |s: &mut Session, label: &str, key: Option<&str>| nudge_window(s, label, key, 1.0);
    let (steps, revision, picture, start) = (s.history.undo.len(), s.revision, s.picture_revision, cx(&s));
    for _ in 0..20 {
        nudge(&mut s, "Vertical: move window (#1)", Some("vertical.drag.1"));
    }
    assert_eq!(s.history.undo.len(), steps + 1, "one undo step for the whole drag");
    assert_eq!((s.revision, s.picture_revision), (revision + 20, picture), "no 16:9 frame changed");
    assert_eq!(cx(&s), start - 20.0);
    let last = s.history.undo.last().unwrap();
    assert_eq!((last.label(), last.2.picture), ("Vertical: move window (#1)", false));
    // one undo takes the whole drag back
    s.undo();
    assert_eq!((cx(&s), s.history.undo.len()), (start, steps));
    s.redo();
    // after an undo or redo the same key starts a new step
    nudge(&mut s, "Vertical: move window (#1)", Some("vertical.drag.1"));
    assert_eq!(s.history.undo.len(), steps + 2);
    // so do another label with the same key, and another key with the same label
    nudge(&mut s, "Vertical: zoom window (#1)", Some("vertical.drag.1"));
    assert_eq!(s.history.undo.len(), steps + 3);
    nudge(&mut s, "Vertical: zoom window (#1)", Some("vertical.drag.2"));
    assert_eq!(s.history.undo.len(), steps + 4);
    nudge(&mut s, "Vertical: zoom window (#1)", Some("vertical.drag.2"));
    assert_eq!(s.history.undo.len(), steps + 4, "the same key and label fold again");
    // and so does a call without a key
    nudge(&mut s, "Vertical: zoom window (#1)", None);
    nudge(&mut s, "Vertical: zoom window (#1)", Some("vertical.drag.2"));
    assert_eq!(s.history.undo.len(), steps + 6);
    assert_eq!(s.picture_revision, picture, "none of them changed a 16:9 frame");
}

/// A plan-only write that changes no data records nothing and keeps the revision (spec §7.5): a closure that changes
/// nothing, an empty set on a sequence without one (the planner reads no set as an empty one), a set whose counters were
/// only raised after an undo, and a gesture's sub-step that moves nothing, which leaves the gesture's one step. The
/// editor state is still taken.
#[test]
fn vertical_session_a_write_that_changes_nothing_records_nothing() {
    let mut s = demo();
    let seq = active(&s);
    let (steps, revision, project) = (s.history.undo.len(), s.revision, s.project.clone());
    s.edit_plan_only(seq, "Vertical: nothing", None, |_, _, _| Ok(())).unwrap();
    s.edit_plan_only(seq, "Vertical: nothing", None, |_, set, _| {
        *set = Some(Box::new(empty_vertical_set()));
        Ok(())
    })
    .unwrap();
    assert_eq!((s.history.undo.len(), s.revision), (steps, revision), "no step, no revision change");
    assert!(Arc::ptr_eq(&s.project, &project), "the project is not replaced");
    assert!(s.project.sequence(seq).is_some_and(|q| q.vertical.is_none()), "no empty set is stored");
    // after an undo the set handed in has its counters raised to the marks: that alone changes nothing
    create_plan(&mut s);
    add_cut(&mut s, 0);
    add_cut(&mut s, 24);
    s.undo();
    let (steps, revision, project) = (s.history.undo.len(), s.revision, s.project.clone());
    s.edit_plan_only(seq, "Vertical: nothing", None, |_, set, _| {
        assert_eq!(set.as_deref().map(|set| set.plans[0].next_position_id), Some(3), "raised to the mark");
        Ok(())
    })
    .unwrap();
    assert_eq!((s.history.undo.len(), s.revision, position_counter(&s, seq)), (steps, revision, 2));
    assert!(Arc::ptr_eq(&s.project, &project));
    // the editor state is taken all the same
    s.edit_plan_only(seq, "Vertical: select", None, |_, _, st| {
        st.vertical.plan = Some(1);
        Ok(())
    })
    .unwrap();
    assert_eq!((s.state.vertical.plan, s.history.undo.len(), s.revision), (Some(1), steps, revision));
    // a sub-step that moves nothing leaves the gesture's step, and the next one still folds into it
    nudge_window(&mut s, "Vertical: move window (#1)", Some("vertical.drag.1"), 1.0);
    let (steps, revision) = (s.history.undo.len(), s.revision);
    nudge_window(&mut s, "Vertical: move window (#1)", Some("vertical.drag.1"), 0.0);
    assert_eq!((s.history.undo.len(), s.revision), (steps, revision));
    nudge_window(&mut s, "Vertical: move window (#1)", Some("vertical.drag.1"), 1.0);
    assert_eq!((s.history.undo.len(), s.revision), (steps, revision + 1), "folded into the gesture's step");
}

/// A picture change folded into a plan-only step (the same merge key and label, `edit_merged`) makes it a picture step:
/// undoing or redoing it advances the picture revision.
#[test]
fn vertical_session_a_picture_change_folded_into_a_plan_only_step_makes_it_a_picture_step() {
    let mut s = demo();
    let seq = create_plan(&mut s);
    add_cut(&mut s, 0);
    let (label, key) = ("Vertical: move window (#1)", "vertical.drag.1");
    nudge_window(&mut s, label, Some(key), 1.0);
    let steps = s.history.undo.len();
    assert!(!s.history.undo.last().unwrap().2.picture);
    let picture = s.picture_revision;
    s.edit_merged(label, key, |p, _| {
        if let Some(item) = p.sequence_mut(seq).and_then(|q| q.video_tracks.first_mut()).and_then(|t| t.items.first_mut()) {
            item.enabled = !item.enabled;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(s.history.undo.len(), steps, "folded into the plan-only step");
    assert!(s.history.undo.last().unwrap().2.picture, "which is now a picture step");
    assert_eq!(s.picture_revision, picture + 1);
    s.undo();
    assert_eq!(s.picture_revision, picture + 2, "its undo changes a 16:9 frame");
    s.redo();
    assert_eq!(s.picture_revision, picture + 3, "and so does its redo");
}

/// A command folded from several steps (`collapse_history`) is a picture step when any of its steps is, whichever comes
/// first; one folded from plan-only steps alone stays plan-only.
#[test]
fn vertical_session_a_folded_step_is_a_picture_step_when_any_of_its_steps_is() {
    let mut s = demo();
    create_plan(&mut s);
    let n0 = s.history.undo.len();
    add_cut(&mut s, 0);
    s.edit("Rename", |p, _| {
        p.name = "Renamed".into();
        Ok(())
    })
    .unwrap();
    crate::clip_ops::collapse_history(&mut s, n0, "Folded");
    let last = s.history.undo.last().unwrap();
    assert_eq!((s.history.undo.len(), last.label(), last.2.picture), (n0 + 1, "Folded", true), "a plan-only step first, then a picture step");
    let picture = s.picture_revision;
    s.undo();
    assert_eq!(s.picture_revision, picture + 1);
    s.redo();
    let n0 = s.history.undo.len();
    add_cut(&mut s, 24);
    add_cut(&mut s, 48);
    crate::clip_ops::collapse_history(&mut s, n0, "Folded cuts");
    let last = s.history.undo.last().unwrap();
    assert_eq!((s.history.undo.len(), last.label(), last.2.picture), (n0 + 1, "Folded cuts", false));
    let picture = s.picture_revision;
    s.undo();
    assert_eq!(s.picture_revision, picture, "plan-only steps alone change no 16:9 frame");
}

/// Check that `id` replaced the project: the revision, the picture revision and the epoch advance, and the 16:9 frame
/// cache is cleared once.
fn replaced(s: &mut Session, id: &str, params: Value) {
    let (revision, picture, epoch, clears) = (s.revision, s.picture_revision, s.epoch, s.vertical_cache.clears());
    s.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
    assert!(s.revision > revision, "`{id}` advances the revision");
    assert!(s.picture_revision > picture, "`{id}` advances the picture revision");
    assert_eq!(s.epoch, epoch + 1, "`{id}` starts a new project epoch");
    assert_eq!(s.vertical_cache.clears(), clears + 1, "`{id}` clears the 16:9 frame cache");
    assert_eq!(s.picture_seen, s.revision);
}

/// T-N19: new, demo, open, close, new from template and a cancelled dynamic trim replace the project; an edit does not.
#[test]
fn vertical_session_t_n19_replacing_the_project_starts_a_new_epoch() {
    let dir = tmp_dir("ve-epoch");
    let path = dir.join("epoch.fcproj").to_string_lossy().into_owned();
    let mut s = Session::default();
    replaced(&mut s, "file.openDemoProject", json!({}));
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    replaced(&mut s, "file.open", json!({"path": path}));
    replaced(&mut s, "file.newProject", json!({"name": "Fresh"}));
    replaced(&mut s, "file.closeProject", json!({"force": true}));
    replaced(&mut s, "file.newProjectFromTemplate", json!({"template": path, "force": true}));
    replaced(&mut s, "file.openDemoProject", json!({}));
    start_dynamic_roll(&mut s);
    replaced(&mut s, "trim.cancelDynamic", json!({}));
    // an edit keeps the epoch and the cache
    let (epoch, clears) = (s.epoch, s.vertical_cache.clears());
    s.execute("playhead.set", json!({"seconds": 2.0})).unwrap();
    s.execute("sequence.addEditAllTracks", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!((s.epoch, s.vertical_cache.clears()), (epoch, clears));
    let _ = std::fs::remove_dir_all(&dir);
}

fn bump_directly(s: &mut Session, _: &Value) -> crate::Result<Value> {
    s.revision += 1;
    Ok(Value::Null)
}

/// T-N19: the `execute` safety net advances the picture revision after a command that bumps the revision directly,
/// past every helper; a change a helper accounted for is not counted twice.
#[test]
fn vertical_session_t_n19_the_execute_safety_net_catches_a_direct_revision_bump() {
    let spec = CommandSpec {
        id: "test.bumpRevisionDirectly",
        label: "Bump the revision directly",
        menu: &[],
        shortcut: None,
        params: "{}",
        enabled: crate::commands::always,
        run: bump_directly,
        journal: true,
    };
    let mut s = demo();
    let (revision, picture) = (s.revision, s.picture_revision);
    s.execute_spec(&spec, json!({})).unwrap();
    assert_eq!(s.revision, revision + 1);
    assert_eq!(s.picture_revision, picture + 1, "a revision change no helper accounted for refreshes the picture");
    assert_eq!(s.picture_seen, s.revision);
    s.execute("playhead.set", json!({"seconds": 2.0})).unwrap();
    let (revision, picture) = (s.revision, s.picture_revision);
    s.execute("sequence.addEditAllTracks", json!({})).unwrap();
    assert_eq!(s.picture_revision - picture, s.revision - revision, "an edit's own bump is not counted twice");
}

/// T-N19: a revision change made outside every command and helper is accounted for too, once: by the next commit, a
/// plan-only one included, or at the end of the next command.
#[test]
fn vertical_session_t_n19_a_change_outside_a_command_is_accounted_for_once() {
    let mut s = demo();
    let seq = create_plan(&mut s);
    let picture = s.picture_revision;
    s.revision += 1;
    s.edit_plan_only(seq, "Vertical: rename plan", None, |_, set, _| {
        if let Some(plan) = set.as_deref_mut().and_then(|set| set.plans.first_mut()) {
            plan.name = "Phone".into();
        }
        Ok(())
    })
    .unwrap();
    assert_eq!((s.picture_revision, s.picture_seen), (picture + 1, s.revision), "the earlier change may have changed a frame");
    s.revision += 1;
    s.execute("history.list", json!({})).unwrap();
    assert_eq!((s.picture_revision, s.picture_seen), (picture + 2, s.revision), "the next command settles it");
    s.execute("history.list", json!({})).unwrap();
    assert_eq!(s.picture_revision, picture + 2, "once");
}

/// T-N20 (engine half): a plain project is saved as v12 and opens with no upgrade toast and no backup on save; a project
/// with a vertical set is saved as v13, keeping its v12 file as a backup once; a v12 file holding a vertical set opens as
/// an upgrade.
#[test]
fn vertical_session_t_n20_the_schema_follows_the_content() {
    let dir = tmp_dir("ve-schema");
    let path = dir.join("plain.fcproj").to_string_lossy().into_owned();
    let mut s = demo();
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc["schema_version"], PLAIN, "a plain project is written in the schema every current build opens");
    let mut t = Session::default();
    let r = t.execute("file.open", json!({"path": path})).unwrap();
    assert_eq!((r["schemaVersion"].as_u64(), r["migrated"].as_bool()), (Some(u64::from(PLAIN)), Some(false)), "{r}");
    assert_eq!(upgraded_toast(&mut t), None, "no upgrade toast");
    assert!(t.execute("file.save", json!({})).unwrap()["backup"].is_null(), "no backup of a file that needs no upgrade");
    // a vertical set: the next save writes v13 and keeps the v12 file, once
    let seq = create_plan(&mut t);
    let v12 = std::fs::read(&path).unwrap();
    let r = t.execute("file.save", json!({})).unwrap();
    let backup = r["backup"].as_str().unwrap_or_else(|| panic!("a backup of the v12 file: {r}")).to_string();
    assert!(backup.ends_with(&format!("plain (schema v{PLAIN} backup).fcproj")), "{backup}");
    assert_eq!(std::fs::read(&backup).unwrap(), v12);
    let mut doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc["schema_version"], filmcraft_format::VERTICAL_SCHEMA);
    assert!(t.execute("file.save", json!({})).unwrap()["backup"].is_null(), "only the first save in the new schema keeps one");
    // a v12 file that holds a vertical set opens, as an upgrade
    doc["schema_version"] = json!(PLAIN);
    let odd = dir.join("odd.fcproj");
    std::fs::write(&odd, serde_json::to_vec(&doc).unwrap()).unwrap();
    let mut u = Session::default();
    let r = u.execute("file.open", json!({"path": odd.to_string_lossy()})).unwrap();
    assert_eq!(r["migrated"], true, "{r}");
    assert!(u.project.sequence(seq).is_some_and(|q| q.vertical.is_some()), "the set is read");
    let toast = upgraded_toast(&mut u).unwrap_or_default();
    assert!(toast.contains(&format!("from schema v{PLAIN} to v{}", filmcraft_format::VERTICAL_SCHEMA)), "{toast}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-N20 through a crash: a plain v12 file that gained a vertical set before the crash still has its v12 original kept
/// at the first save after recovery, which writes v13; and the id marks start from the recovery journal, which carries
/// the session's.
#[test]
fn vertical_session_t_n20_recovery_keeps_the_v12_file_and_starts_the_marks_from_the_journal() {
    let dir = tmp_dir("ve-recover");
    let data = dir.join("data");
    let path = dir.join("plain.fcproj").to_string_lossy().into_owned();
    let mut a = demo();
    a.execute("file.saveAs", json!({"path": path})).unwrap();
    let v12 = std::fs::read(&path).unwrap();
    let seq = create_plan(&mut a);
    for frame in [0, 24, 48] {
        add_cut(&mut a, frame);
    }
    a.undo();
    a.undo();
    start_journal(&mut a, &data);
    wait_for_journal(&mut a);
    a.persistence.take().unwrap().simulate_crash();
    // the next launch recovers the changes over the same file
    let mut b = Session::default();
    start_journal(&mut b, &data);
    b.execute("file.recover", json!({})).unwrap();
    assert_eq!((b.path.as_deref(), b.is_dirty()), (Some(path.as_str()), true));
    assert_eq!(b.loaded_schema, PLAIN, "the file on disk is still v12");
    assert_eq!(position_mark(&b, seq).map(|m| m.next_position_id), Some(4), "the marks start from the journal");
    assert_eq!(add_cut(&mut b, 72), 4, "ids 2 and 3 are never handed out again");
    let r = b.execute("file.save", json!({})).unwrap();
    let backup = r["backup"].as_str().unwrap_or_else(|| panic!("a backup of the v12 file: {r}")).to_string();
    assert!(backup.ends_with(&format!("plain (schema v{PLAIN} backup).fcproj")), "{backup}");
    assert_eq!(std::fs::read(&backup).unwrap(), v12);
    let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc["schema_version"], filmcraft_format::VERTICAL_SCHEMA);
    b.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-N20 with "Auto Save also saves the current project": the first auto-save in v13 over a v12 project file keeps the
/// v12 file, and the next manual save, finding the file in v13 already, keeps no backup: none named v12 holds v13 bytes.
#[test]
fn vertical_session_t_n20_auto_save_over_the_project_file_keeps_the_v12_file() {
    let dir = tmp_dir("ve-autosave-schema");
    let path = dir.join("plain.fcproj").to_string_lossy().into_owned();
    let backup = dir.join(format!("plain (schema v{PLAIN} backup).fcproj"));
    let mut s = demo();
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    let v12 = std::fs::read(&path).unwrap();
    start_journal(&mut s, &dir.join("data"));
    s.execute("prefs.set", json!({"key": "autoSave.saveCurrentProject", "value": true})).unwrap();
    create_plan(&mut s);
    s.sync_persistence();
    let r = s.execute("file.autoSaveNow", json!({})).unwrap();
    assert!(r["path"].is_string(), "{r}");
    assert!(!s.is_dirty(), "auto-save saved the project file");
    let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc["schema_version"], filmcraft_format::VERTICAL_SCHEMA);
    assert_eq!(std::fs::read(&backup).unwrap(), v12, "the v12 file is kept");
    std::fs::remove_file(&backup).unwrap();
    let r = s.execute("file.save", json!({})).unwrap();
    assert!(r["backup"].is_null(), "{r}");
    assert!(!backup.exists(), "the v13 file is not kept as a v12 backup");
    s.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

/// `EngineError::Invalid` carries the refusal's code and detail, and its text names the command and the code.
#[test]
fn vertical_session_invalid_carries_the_refusal_code_and_detail() {
    let detail = json!({"digest": "fnv1a64:0123456789abcdef"});
    let e = EngineError::invalid("vertical.setPosition", Refusal::new("stale_plan", "the plan changed since `expect` was read", detail.clone()));
    let EngineError::Invalid { cmd, code, message, detail: got } = &e else { panic!("not a structured refusal: {e}") };
    assert_eq!((cmd.as_str(), *code, message.as_str(), got), ("vertical.setPosition", "stale_plan", "the plan changed since `expect` was read", &detail));
    let text = e.to_string();
    assert!(text.contains("vertical.setPosition") && text.contains("stale_plan") && text.contains("the plan changed"), "{text}");
    assert!(!text.contains("FilmCraft"), "{text}");
}

/// The id no project item has.
const NO_SEQUENCE: ItemId = ItemId(u64::MAX);

fn plan_only_on_no_sequence(s: &mut Session, _: &Value) -> crate::Result<Value> {
    s.edit_plan_only(NO_SEQUENCE, "Vertical: rename plan", None, |_, _, _| Ok(Value::Null))
}

/// The plan-only commit path refuses a sequence that does not exist `not_found {sequence}` (spec §7.6), as a refusal of
/// the command that ran it, and changes nothing.
#[test]
fn vertical_session_a_missing_sequence_is_refused_not_found() {
    let mut s = demo();
    let (steps, revision) = (s.history.undo.len(), s.revision);
    let e = s.edit_plan_only(NO_SEQUENCE, "Vertical: rename plan", None, |_, _, _| Ok(())).unwrap_err();
    let EngineError::Invalid { code, detail, .. } = &e else { panic!("not a coded refusal: {e}") };
    assert_eq!((*code, detail), ("not_found", &json!({"sequence": u64::MAX})));
    let spec = CommandSpec {
        id: "test.planOnlyOnNoSequence",
        label: "Plan-only write on no sequence",
        menu: &[],
        shortcut: None,
        params: "{}",
        enabled: crate::commands::always,
        run: plan_only_on_no_sequence,
        journal: true,
    };
    let e = s.execute_spec(&spec, json!({})).unwrap_err();
    let EngineError::Invalid { cmd, code, .. } = &e else { panic!("not a coded refusal: {e}") };
    assert_eq!((cmd.as_str(), *code), ("test.planOnlyOnNoSequence", "not_found"), "named after the command that ran");
    assert!(e.to_string().contains("`test.planOnlyOnNoSequence` refused (not_found)"), "{e}");
    assert_eq!((s.history.undo.len(), s.revision), (steps, revision));
}

/// The id high-water marks: undo rolls a counter back but never its mark, so the working copy, the commit path and the
/// allocation API hand out ids above it; a cancelled dynamic trim keeps them.
#[test]
fn vertical_session_id_high_water_marks_outlive_undo() {
    let mut s = demo();
    let seq = create_plan(&mut s);
    assert_eq!([add_cut(&mut s, 0), add_cut(&mut s, 24), add_cut(&mut s, 48)], [1, 2, 3]);
    s.undo();
    s.undo();
    assert_eq!(position_counter(&s, seq), 2, "undo rolls the stored counter back");
    assert_eq!(position_mark(&s, seq), Some(PlanIdMarks { next_position_id: 4, next_record_id: 1 }), "but not the mark");
    let copy = s.vertical_working_copy(seq).unwrap();
    assert_eq!(copy.vertical.as_deref().map(|set| set.plans[0].next_position_id), Some(4), "the planner's working copy is raised");
    assert_eq!(position_counter(&s, seq), 2, "the project is not");
    assert_eq!(add_cut(&mut s, 72), 4, "ids 2 and 3 are never handed out again");
    s.undo();
    assert_eq!(position_mark(&s, seq).map(|m| m.next_position_id), Some(5));
    // the allocation API: above the counter and the mark, both of which move past the id
    let mut set = *s.vertical_working_copy(seq).and_then(|q| q.vertical).unwrap();
    assert_eq!(s.vertical_ids.allocate(seq, &mut set, IdCounter::Position(1)), Some(5));
    assert_eq!(set.plans[0].next_position_id, 6);
    assert_eq!(position_mark(&s, seq).map(|m| m.next_position_id), Some(6));
    assert_eq!(s.vertical_ids.allocate(seq, &mut set, IdCounter::Plan), Some(2));
    assert_eq!(s.vertical_ids.allocate(seq, &mut set, IdCounter::Part), Some(1));
    assert_eq!(s.vertical_ids.allocate(seq, &mut set, IdCounter::Region), Some(1));
    assert_eq!(s.vertical_ids.allocate(seq, &mut set, IdCounter::Annotation), Some(1));
    assert_eq!(s.vertical_ids.allocate(seq, &mut set, IdCounter::Record(1)), Some(1));
    assert_eq!(set.next_plan_id, 3);
    assert_eq!(s.vertical_ids.allocate(seq, &mut set, IdCounter::Position(9)), None, "no such plan");
    set.next_part_id = u32::MAX;
    assert_eq!(s.vertical_ids.allocate(seq, &mut set, IdCounter::Part), None, "a spent counter hands out nothing");
    // a cancelled dynamic trim puts back the project from before the trim, not the marks: the ids the edit hook handed
    // out while the trim ran (its live updates are edits) stay handed out
    start_dynamic_roll(&mut s);
    let during = position_mark(&s, seq).map(|m| m.next_position_id);
    assert!(during >= Some(6), "{during:?}");
    s.execute("trim.cancelDynamic", json!({})).unwrap();
    assert_eq!(position_counter(&s, seq), 2);
    assert_eq!(position_mark(&s, seq).map(|m| m.next_position_id), during);
    assert_eq!(Some(add_cut(&mut s, 96)), during);
}

/// A set created again after its creation was undone hands out no id twice (spec §2.6): the planner's working copy is a
/// new set with its counters at the marks; a set made from nothing that holds an id handed out before, and a set whose
/// counters are behind the marks (planned on the stored set after an undo), are refused with nothing changed.
#[test]
fn vertical_session_a_set_created_again_hands_out_no_id_twice() {
    let mut s = demo();
    let seq = create_plan(&mut s);
    for frame in [0, 24, 48] {
        add_cut(&mut s, frame);
    }
    for _ in 0..4 {
        s.undo();
    }
    assert!(s.project.sequence(seq).is_some_and(|q| q.vertical.is_none()), "the set is undone");
    let copy = s.vertical_working_copy(seq).and_then(|q| q.vertical).unwrap();
    assert_eq!((copy.next_plan_id, copy.plans.len()), (2, 0), "a new set above the marks");
    assert_eq!(*copy, s.new_vertical_set(seq));
    assert_eq!(VerticalSet { next_plan_id: 1, ..(*copy).clone() }, empty_vertical_set(), "otherwise empty");
    // a set made from nothing that holds plan id 1 again
    let (steps, revision, project) = (s.history.undo.len(), s.revision, s.project.clone());
    let e = s
        .edit_plan_only(seq, "Vertical: new plan 9:16", None, |_, set, _| {
            *set = Some(Box::new(one_plan_set()));
            Ok(())
        })
        .unwrap_err();
    assert!(e.to_string().contains("plan id 1 again"), "{e}");
    assert_eq!((s.history.undo.len(), s.revision), (steps, revision));
    assert!(Arc::ptr_eq(&s.project, &project), "nothing changed");
    // planned on the working copy, the new plan is plan 2, and its positions are its own
    s.edit_plan_only(seq, "Vertical: new plan 9:16", None, move |_, set, _| {
        let mut new = copy;
        new.plans.push(VerticalPlan::new(new.next_plan_id, "9:16", OutputSize { width: 1080, height: 1920 }));
        new.next_plan_id += 1;
        *set = Some(new);
        Ok(())
    })
    .unwrap();
    let plan = |s: &Session| s.project.sequence(seq).and_then(|q| q.vertical.as_deref()).map(|set| (set.plans[0].id, set.next_plan_id));
    assert_eq!(plan(&s), Some((2, 3)));
    assert_eq!([add_cut(&mut s, 0), add_cut(&mut s, 24)], [1, 2]);
    // a set planned on the stored set after an undo: its counters are behind the marks
    s.undo();
    let stale = s.project.sequence(seq).and_then(|q| q.vertical.clone()).unwrap();
    let e = s
        .edit_plan_only(seq, "Vertical: rename plan", None, move |_, set, _| {
            let mut stale = stale;
            stale.plans[0].name = "Phone".into();
            *set = Some(stale);
            Ok(())
        })
        .unwrap_err();
    assert!(e.to_string().contains("behind the session's high-water marks"), "{e}");
    assert_eq!(position_counter(&s, seq), 2);
}

/// Saving (Save As, Save a Copy) writes the high-water marks into the file and the in-memory project, with no history
/// step and no change of the dirty state.
#[test]
fn vertical_session_saving_writes_the_marks_with_no_step_and_no_dirty_change() {
    let dir = tmp_dir("ve-ids-save");
    let mut s = demo();
    let seq = create_plan(&mut s);
    for frame in [0, 24, 48] {
        add_cut(&mut s, frame);
    }
    s.undo();
    s.undo();
    let path = dir.join("ids.fcproj").to_string_lossy().into_owned();
    let (steps, revision) = (s.history.undo.len(), s.revision);
    s.execute("file.saveAs", json!({"path": path})).unwrap();
    assert_eq!(counter_in(&decode_file(&path), seq), 4, "the file carries the high-water mark");
    assert_eq!(position_counter(&s, seq), 4, "and so does the project in memory");
    assert_eq!((s.history.undo.len(), s.revision), (steps, revision), "no history step, no revision change");
    assert!(!s.is_dirty());
    // Save a Copy of a project with unsaved changes leaves them unsaved
    s.undo();
    assert_eq!(position_counter(&s, seq), 1);
    let copy = dir.join("copy.fcproj").to_string_lossy().into_owned();
    let (steps, revision) = (s.history.undo.len(), s.revision);
    s.execute("file.saveCopy", json!({"path": copy})).unwrap();
    assert_eq!(counter_in(&decode_file(&copy), seq), 4);
    assert_eq!((s.history.undo.len(), s.revision, s.is_dirty()), (steps, revision, true));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Auto-save and the recovery journal write the high-water marks too, and leave the session's snapshot as it is.
#[test]
fn vertical_session_auto_save_and_the_journal_write_the_marks() {
    let data = tmp_dir("ve-ids-autosave");
    let mut s = demo();
    let seq = create_plan(&mut s);
    for frame in [0, 24, 48] {
        add_cut(&mut s, frame);
    }
    s.undo();
    s.undo();
    start_journal(&mut s, &data);
    let r = s.execute("file.autoSaveNow", json!({})).unwrap();
    let auto = r["path"].as_str().unwrap_or_else(|| panic!("an auto-save of the unsaved project: {r}")).to_string();
    assert_eq!(counter_in(&decode_file(auto), seq), 4, "the auto-save carries the high-water mark");
    wait_for_journal(&mut s);
    let snapshot = s.persistence.as_ref().unwrap().session_dir.join("snapshot.fcproj");
    assert_eq!(counter_in(&decode_file(snapshot), seq), 4, "so does the recovery journal");
    assert_eq!(position_counter(&s, seq), 2, "the session's snapshot is left as it is");
    s.shutdown();
    let _ = std::fs::remove_dir_all(&data);
}

/// Opening (installing) a project starts the marks from its file.
#[test]
fn vertical_session_opening_a_project_starts_the_marks_from_its_file() {
    let dir = tmp_dir("ve-ids-open");
    let mut s = demo();
    let seq = create_plan(&mut s);
    for frame in [0, 24, 48, 72] {
        add_cut(&mut s, frame);
    }
    assert_eq!(position_mark(&s, seq).map(|m| m.next_position_id), Some(5));
    // the same project with plan 1 emptied and its counter back at the start, as a file holds it
    let mut p = (*s.project).clone();
    if let Some(plan) = p.sequence_mut(seq).and_then(|q| q.vertical.as_deref_mut()).and_then(|set| set.plans.first_mut()) {
        plan.positions.clear();
        plan.next_position_id = 1;
    }
    let file = dir.join("start.fcproj");
    std::fs::write(&file, filmcraft_format::encode(&p, false)).unwrap();
    s.execute("file.open", json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(position_mark(&s, seq).map(|m| m.next_position_id), Some(1), "the marks start from the file");
    assert_eq!(add_cut(&mut s, 0), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The clock of `when` fields: the system clock by default, a fixed or scripted one in tests.
#[test]
fn vertical_session_the_clock_is_injectable() {
    let mut s = Session::default();
    let now = s.now_unix();
    assert!((1_700_000_000..4_000_000_000).contains(&now), "the system clock reads Unix seconds: {now}");
    s.clock = Clock::fixed(1_790_000_000);
    assert_eq!(s.now_unix(), 1_790_000_000);
    let ticks = Arc::new(AtomicI64::new(10));
    let source = Arc::clone(&ticks);
    s.clock = Clock::from_fn(move || source.fetch_add(1, Ordering::Relaxed));
    assert_eq!((s.now_unix(), s.now_unix()), (10, 11));
    assert_eq!(ticks.load(Ordering::Relaxed), 12);
}

/// The media pool generation advances whenever what a source shows may change, and only then.
#[test]
fn vertical_session_the_media_pool_generation_counts_source_changes() {
    let pool = MediaPool::default();
    let rate = FrameRate::FPS_24;
    let source = || -> SharedSource { Arc::new(GeneratorSource::new(Generator::BlackVideo, 16, 16, rate, rate.tick_of(24))) };
    let g = pool.generation();
    pool.insert(ItemId(1), source());
    assert_eq!(pool.generation(), g + 1, "insert");
    pool.insert_keyed(ItemId(2), "generator:BlackVideo".into(), source());
    assert_eq!(pool.generation(), g + 2, "insert_keyed");
    pool.insert_file(ItemId(3), "/synthetic/black.mov", source());
    assert_eq!(pool.generation(), g + 3, "insert_file");
    pool.remove(ItemId(1));
    assert_eq!(pool.generation(), g + 4, "remove");
    pool.set_use_proxies(true);
    assert_eq!(pool.generation(), g + 5, "switching to proxies");
    pool.set_use_proxies(true);
    assert_eq!(pool.generation(), g + 5, "no switch, no change");
    pool.set_use_proxies(false);
    assert_eq!(pool.generation(), g + 6, "switching back");
    pool.clear();
    assert_eq!(pool.generation(), g + 7, "clear");
    let _ = (pool.cached(ItemId(2)), pool.use_proxies(), pool.open_sources(), pool.offline_status(ItemId(3)));
    assert_eq!(pool.generation(), g + 7, "reading changes nothing");
    pool.remove(ItemId(2));
    pool.clear();
    assert_eq!(pool.generation(), g + 7, "neither does removing or clearing what is not there");
}

/// The Vertical Edit preferences: the defaults of spec §5.14, dotted keys through `prefs.get` and `prefs.set`, hostile
/// values brought into range, the editor name's fallback, the default curve, persistence and an old file without them.
#[test]
fn vertical_session_preferences_round_trip_with_dotted_keys() {
    let d = VerticalPrefs::default();
    assert!(d.right_alt_as_alt && d.match_punctuation_by_position && d.require_watching);
    assert_eq!((d.watch_max_speed, d.flag_reaction_frames, d.live_switch_look_back, d.live_switch_look_ahead), (1.5, 8, 12, 3));
    assert_eq!((d.cut_reel_seconds, d.loop_seam_seconds, d.boundary_review_seconds), (1.0, 2.0, 2.0));
    assert_eq!((d.thresholds.upscale_amber, d.thresholds.upscale_red, d.thresholds.speed_amber, d.thresholds.speed_red), (2.0, 3.0, 0.14, 0.5));
    assert_eq!((d.edges.window_frames, d.edges.audio_floor_dbfs, d.edges.annotation_kinds.len()), (3, -45.0, 3));
    assert_eq!((d.editor_name.as_str(), d.default_curve()), ("", DEFAULT_CURVE));
    // until it is set, the editor name is the operating-system account name (trimmed, at most 64 characters)
    let account = ["USERNAME", "USER"].iter().filter_map(|k| std::env::var(k).ok()).map(|n| n.trim().to_string()).find(|n| !n.is_empty());
    let cut = |n: String| n.chars().take(64).collect::<String>().trim_end().to_string();
    assert_eq!(d.editor_name(), account.map(cut).unwrap_or_else(|| "Editor".into()));
    for (stored, read) in [(format!("  Dana {}  ", "y".repeat(80)), format!("Dana {}", "y".repeat(59))), (format!("{} b", "a".repeat(63)), "a".repeat(63))] {
        assert_eq!(VerticalPrefs { editor_name: stored, ..VerticalPrefs::default() }.editor_name(), read);
    }
    // dotted keys
    let mut s = Session::default();
    assert_eq!(s.execute("prefs.get", json!({"key": "vertical.watchMaxSpeed"})).unwrap(), json!(1.5));
    assert_eq!(s.execute("prefs.get", json!({"key": "vertical.thresholds.eyeTraceJump"})).unwrap(), json!(0.25));
    let keys = s.execute("prefs.get", json!({})).unwrap()["keys"].clone();
    assert!(keys.as_array().is_some_and(|k| k.iter().any(|k| k == "vertical.rightAltAsAlt")), "{keys}");
    let values = json!({
        "vertical.watchMaxSpeed": 2.0,
        "vertical.rightAltAsAlt": false,
        "vertical.editorName": "Dana Editor",
        "vertical.liveSwitchLookBack": 6,
        "vertical.defaultCurve": "linear"
    });
    s.execute("prefs.set", json!({"values": values})).unwrap();
    let v = &s.prefs.vertical;
    assert_eq!((v.watch_max_speed, v.right_alt_as_alt, v.editor_name(), v.live_switch_look_back), (2.0, false, "Dana Editor".to_string(), 6));
    assert_eq!(v.default_curve(), Curve::Linear);
    // hostile values are refused or brought into range
    assert!(s.execute("prefs.set", json!({"key": "vertical.flagReactionFrames", "value": -3})).is_err());
    assert!(s.execute("prefs.set", json!({"key": "vertical.rightAltAsAlt", "value": "yes"})).is_err());
    let hostile = json!({
        "vertical.watchMaxSpeed": 1e300,
        "vertical.liveSwitchLookAhead": 1_000_000,
        "vertical.cutReelSeconds": -2.0,
        "vertical.editorName": "x".repeat(100),
        "vertical.defaultCurve": "zigzag",
        "vertical.rampIn": 0.9,
        "vertical.thresholds.upscaleRed": -5.0,
        "vertical.edges.annotationKinds": ["speech", "Bad Kind!", "music", "speech"]
    });
    s.execute("prefs.set", json!({"values": hostile})).unwrap();
    let v = &s.prefs.vertical;
    assert_eq!((v.watch_max_speed, v.live_switch_look_ahead, v.cut_reel_seconds), (16.0, 250, 0.1));
    assert_eq!((v.editor_name.chars().count(), v.default_curve.as_str(), v.ramp_in, v.ramp_out), (64, "ramp", 0.2, 0.2));
    assert_eq!(v.default_curve(), DEFAULT_CURVE);
    assert_eq!(v.thresholds.upscale_red, 1.0);
    assert_eq!(v.edges.annotation_kinds, ["speech", "music"]);
    // persisted in the preferences file and read back; a file from before has the defaults
    let dir = tmp_dir("ve-prefs");
    let path = dir.join("preferences.json");
    s.prefs.save(&path).unwrap();
    assert_eq!(Preferences::load(&path).vertical, s.prefs.vertical);
    std::fs::write(&path, br#"{"version": 2, "timeline": {"snapPlayhead": false}}"#).unwrap();
    assert_eq!(Preferences::load(&path).vertical, VerticalPrefs::default());
    let _ = std::fs::remove_dir_all(&dir);
    // `prefs.reset` takes `vertical` as a category: every Vertical Edit preference back to its default, and nothing else
    s.execute("prefs.set", json!({"key": "autoSave.intervalMinutes", "value": 12})).unwrap();
    s.execute("prefs.reset", json!({"category": "vertical"})).unwrap();
    assert_eq!((&s.prefs.vertical, s.prefs.auto_save.interval_minutes), (&VerticalPrefs::default(), 12));
}

/// The picture edit points of the active sequence: the first frame shown of every item on a video track, past frame 0.
fn edit_frames(s: &Session) -> BTreeSet<i64> {
    let q = s.active_sequence().unwrap();
    let rate = q.settings.frame_rate;
    q.video_tracks.iter().flat_map(|t| &t.items).map(|i| rate.frame_at(Tick(i.start.0 - 1)) + 1).filter(|f| *f > 0).collect()
}

/// Change the vertical set of the active sequence through the plan-only commit path.
fn change_set(s: &mut Session, label: &str, change: impl FnOnce(&mut VerticalSet)) {
    let seq = active(s);
    s.edit_plan_only(seq, label, None, |_, set, _| {
        change(set.as_deref_mut().ok_or_else(|| EngineError::Other("no vertical set".into()))?);
        Ok(())
    })
    .unwrap();
}

/// `EditorState.vertical`: `fix_state` drops positions, regions and plans that no longer exist, walls that can no longer
/// be walls (a stored wall undone, a suppressed frame, an edit point while edit points are not walls, a frame no source
/// makes a wall, one off a frame start or outside the sequence), and everything when the active sequence has no
/// vertical set.
#[test]
fn vertical_session_the_selection_drops_what_no_longer_exists() {
    let mut s = demo();
    let seq = create_plan(&mut s);
    let (a, b) = (add_cut(&mut s, 0), add_cut(&mut s, 24));
    let (rate, end, edits) = (s.sequence_rate(), s.active_sequence().unwrap().duration(), edit_frames(&s));
    let edit = rate.tick_of(*edits.first().unwrap());
    let free: Vec<Tick> = (1_i64..).filter(|f| !edits.contains(f)).take(2).map(|f| rate.tick_of(f)).collect();
    let (wall, plain) = (free[0], free[1]);
    change_set(&mut s, "Vertical: protect logo, add a wall", |set| {
        set.regions.push(ProtectedRegion {
            id: 1,
            label: "logo".into(),
            from: rate.tick_of(0),
            to: rate.tick_of(48),
            rect: [10.0, 10.0, 100.0, 50.0],
            mode: RegionMode::KeepOut,
            origin: Origin::Manual,
            modified_by: Writer::Human,
        });
        set.walls.stored.push(StoredWall { at: wall, origin: WallOrigin::Manual, score: None });
    });
    let walls = vec![wall, Tick(-1), end, wall + Tick(1), edit, plain, edit];
    s.state.vertical = VerticalSelection { plan: Some(1), positions: vec![b, 77, a], walls, region: Some(1) };
    s.fix_state();
    assert_eq!(s.state.vertical, VerticalSelection { plan: Some(1), positions: vec![b, a], walls: vec![wall, edit], region: Some(1) });
    s.undo();
    assert_eq!(s.state.vertical.region, None, "the region is gone");
    assert_eq!(s.state.vertical.walls, [edit], "so is the stored wall; the edit point stays");
    assert_eq!(s.state.vertical.positions, [b, a]);
    // a suppressed frame is no wall, and neither is an edit point while edit points are not walls
    change_set(&mut s, "Vertical: suppress a wall", |set| set.walls.suppressed.push(edit));
    s.fix_state();
    assert!(s.state.vertical.walls.is_empty(), "the suppressed wall is dropped");
    s.undo();
    s.state.vertical.walls = vec![edit];
    s.fix_state();
    assert_eq!(s.state.vertical.walls, [edit], "an edit point is a wall again");
    change_set(&mut s, "Vertical: no edit-point walls", |set| set.walls.use_edit_points = false);
    s.fix_state();
    assert!(s.state.vertical.walls.is_empty(), "edit points are not walls now");
    s.undo();
    s.undo();
    assert_eq!(s.state.vertical.positions, [a], "position {b} is gone");
    s.state.vertical.plan = Some(9);
    s.fix_state();
    assert_eq!(s.state.vertical, VerticalSelection::default(), "plan 9 is not there");
    s.state.vertical = VerticalSelection { plan: Some(1), positions: vec![a], walls: vec![edit], region: None };
    s.undo();
    s.undo();
    assert!(s.project.sequence(seq).is_some_and(|q| q.vertical.is_none()));
    assert_eq!(s.state.vertical, VerticalSelection::default(), "no vertical set: nothing stays selected");
}

/// `EditorState.vertical` names each thing once, and belongs to the active sequence: when that sequence goes away, the
/// one that becomes active starts with nothing selected, even where the same ids exist in it.
#[test]
fn vertical_session_the_selection_names_each_thing_once_and_stays_with_its_sequence() {
    let mut s = demo();
    let main = create_plan(&mut s);
    let a = add_cut(&mut s, 0);
    let wall = s.sequence_rate().tick_of(*edit_frames(&s).first().unwrap());
    s.state.vertical = VerticalSelection { plan: Some(1), positions: vec![a, a], walls: vec![wall, wall], region: None };
    s.fix_state();
    assert_eq!(s.state.vertical, VerticalSelection { plan: Some(1), positions: vec![a], walls: vec![wall], region: None });
    s.execute("file.newSequence", json!({"name": "Second"})).unwrap();
    assert_ne!(active(&s), main);
    // plan 1 and position `a` exist in the main sequence too, which becomes active again when the new one is undone
    s.state.vertical = VerticalSelection { plan: Some(1), positions: vec![a], walls: vec![], region: None };
    s.undo();
    assert_eq!(active(&s), main);
    assert_eq!(s.state.vertical, VerticalSelection::default());
}

/// The three Vertical Edit shortcut scopes exist and take panel shortcuts.
#[test]
fn vertical_session_the_three_shortcut_scopes_exist() {
    for panel in ["Vertical Edit Overlay", "Vertical Monitor", "Vertical Edit"] {
        assert!(crate::shortcuts::PANELS.contains(&panel), "{panel} is a shortcut scope");
    }
    let mut s = Session::default();
    s.execute("shortcuts.set", json!({"command": "edit.undo", "keys": "F9", "panel": "Vertical Monitor"})).unwrap();
    assert!(s.shortcuts.for_command("edit.undo").iter().any(|b| b.panel.as_deref() == Some("Vertical Monitor") && b.keys == "F9"));
}
