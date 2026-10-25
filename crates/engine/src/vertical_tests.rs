//! Vertical Edit's engine commands and reads (plan 63 §3.2 and §3.4, lane L2b): the command table, strict parameters
//! (unknown keys, `seconds`, times off a frame start, part timecodes, hostile values: T-M9's engine half), every refusal
//! with its detail, the result envelope of spec §7.2, dry runs, no-ops, merged gestures guarded by `expectRange`,
//! `changedFrames`, `strict` and `echo`, retries (T-N15), ids (T-M13), approvals (T-N2, T-M18, T-M27), annotations and
//! active areas (T-N14), the reads (`list`, `inspect`, `windows` with its stored golden T-M6, `measure`, `fit`,
//! `storyboard`), the shot-change measurement and its application (spec §4.2, §4.4, T-M34), the 16:9 frame cache and
//! the exact-frame helper (spec §6.3, T-N19's cache half), the scope audit (T-N21), validation and repair, the watched
//! store, the selection, `goTo`, and two agreements between lanes (the through-edit rule, the caption anchor).
//!
//! Synthetic media only: colour mattes from the media generators.

use std::sync::Arc;
use std::time::{Duration, Instant};

use filmcraft_media::Generator;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_project::vertical::{
    self as v, ActiveArea, Arrival, CaptionAnchor, Curve, FrameRange, Layout, Origin, OutputSize, Part, PartMeta, Position, SplitOf, VerticalPlan, VerticalSet,
    WallOrigin, Window, Writer,
};
use filmcraft_project::{ItemId, Label, MulticamSel, Project, Sequence, SequenceSettings, TrackItem, TrackKind};
use filmcraft_render::{FrameCache16x9, FrameIdentity, FrameKey16x9, Image, VerticalRenderInputs, VerticalRenderOptions, render_vertical};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use serde_json::{Value, json};

use crate::vertical_cache::VerticalFrameCache;
use crate::vertical_frames::{exact_frame, exact_frame_rgba8, frame_identity};
use crate::{Clock, EngineError, MediaPool, Session, VerticalSelection};

// ---------- fixtures ----------

/// Every fixture's frame rate: 25p, so a frame is 10 160 640 000 ticks.
const RATE: FrameRate = FrameRate::FPS_25;

/// The output of a 9:16 plan.
const NINE_SIXTEEN: OutputSize = OutputSize { width: 1080, height: 1920 };

/// One clip of a fixture: the colour of its matte (8-bit sRGB) and its length in frames.
type Clip = ([u8; 3], i64);

/// The framing fixture's clips: four 100-frame mattes, so picture edit points at 100, 200 and 300 and an end at 400.
const FOUR: [Clip; 4] = [([40, 72, 200], 100), ([200, 40, 72], 100), ([72, 200, 40], 100), ([104, 104, 104], 100)];

/// The measurement fixture's clips: hard cuts at 100, 200 and 300, and at 400 a change of one channel by one histogram bin
/// (a frame difference of about 0.17: below the cut threshold 0.285 of sensitivity 50, above its half), 500 frames.
const CUTS: [Clip; 5] = [([40, 72, 200], 100), ([200, 40, 72], 100), ([72, 200, 40], 100), ([104, 104, 104], 100), ([104, 104, 120], 100)];

/// The burst fixture's clips, 320 frames, in the colours of [`CUTS`]' first clips: a 4-frame flash shot at 97 (hard cuts
/// at 97 and 101, frame differences about 0.51 and 0.83), a hard cut at 156 and one at 200 (0.83), and a 3-frame shot at
/// 250 (cuts at 250 and 253, about 0.84 and 0.51).
const BURST: [Clip; 7] =
    [([40, 72, 200], 97), ([200, 40, 72], 4), ([72, 200, 40], 55), ([200, 40, 72], 44), ([72, 200, 40], 50), ([40, 72, 200], 3), ([200, 40, 72], 67)];

/// The flicker fixture's clips, 200 frames: at 98 a change of one channel by one histogram bin (about 0.17), at 101 a
/// hard cut (about 0.62).
const FLICKER: [Clip; 3] = [([104, 104, 104], 98), ([104, 104, 120], 3), ([40, 72, 200], 99)];

/// The tick of a frame.
fn at(frame: i64) -> Tick {
    RATE.tick_of(frame)
}

/// A colour matte showing exactly `rgb`: the generator truncates `c · 255`, so each level is given at its centre.
fn matte(rgb: [u8; 3]) -> Generator {
    let level = |c: u8| (f32::from(c) + 0.5) / 255.0;
    Generator::ColorMatte { color: [level(rgb[0]), level(rgb[1]), level(rgb[2]), 1.0] }
}

/// A project with one `w`×`h` 25p sequence "Main" holding `clips` back to back on V1 (colour mattes from the media
/// generators, registered in `pool`) and `set` as its vertical data; the sequence's id.
fn project_with(pool: &MediaPool, w: u32, h: u32, clips: &[Clip], set: Option<VerticalSet>) -> (Project, ItemId) {
    let shown: Vec<(Generator, i64)> = clips.iter().map(|&(rgb, frames)| (matte(rgb), frames)).collect();
    project_showing(pool, w, h, &shown, set)
}

/// [`project_with`] for clips of any generator, each one a generator and its length in frames. A matte is named
/// "Matte k", any other generator by its label and `k`.
fn project_showing(pool: &MediaPool, w: u32, h: u32, clips: &[(Generator, i64)], set: Option<VerticalSet>) -> (Project, ItemId) {
    let mut p = Project::new("Vertical fixture");
    let settings = SequenceSettings { width: w, height: h, frame_rate: RATE, ..Default::default() };
    let seq = p.new_sequence("Main", settings, 2, 1, None);
    let mut start = 0;
    for (k, (generator, frames)) in clips.iter().enumerate() {
        let frames = *frames;
        let name = match generator {
            Generator::ColorMatte { .. } => format!("Matte {k}"),
            other => format!("{} {k}", other.label()),
        };
        let src = GeneratorSource::new(generator.clone(), w, h, RATE, at(frames));
        let item = crate::demo::add_generator(&mut p, pool, src, &name, Label::Iris, None);
        let range = TimeRange::new(Tick::ZERO, at(frames));
        let mut clip = p.make_track_item(item, TrackKind::Video, at(start), range, RATE).expect("a generator makes a clip");
        for e in &mut clip.effects {
            filmcraft_project::resolve_auto_points(e, (w, h), (w, h));
        }
        p.sequence_mut(seq).expect("the sequence").video_tracks[0].items.push(clip);
        start += frames;
    }
    p.sequence_mut(seq).expect("the sequence").vertical = set.map(Box::new);
    (p, seq)
}

/// Install `p` in `s` as opening a file does (no history, the id marks from the project), with `seq` active and open.
fn install(s: &mut Session, p: Project, seq: ItemId) {
    s.project = Arc::new(p);
    s.history = crate::History { limit: 200, ..Default::default() };
    s.state.active_sequence = Some(seq);
    s.state.open_sequences = vec![seq];
    s.note_project_replaced();
}

/// A session holding [`project_with`]'s project.
fn session_with(w: u32, h: u32, clips: &[Clip], set: Option<VerticalSet>) -> (Session, ItemId) {
    let mut s = Session::default();
    let (p, seq) = project_with(&s.media, w, h, clips, set);
    install(&mut s, p, seq);
    (s, seq)
}

/// Change the sequence `seq` of the project of `s` outside the commands (a file made by hand) and install it again.
fn rebuild(s: &mut Session, seq: ItemId, f: impl FnOnce(&mut Sequence)) {
    let mut p = (*s.project).clone();
    f(p.sequence_mut(seq).expect("the sequence"));
    install(s, p, seq);
}

/// A cut at `frame` to the window `(cx, cy, h)`, made by a person.
fn cut(id: u32, frame: i64, cx: f64, cy: f64, h: f64) -> Position {
    Position::new(id, at(frame), Window { cx, cy, h }, Arrival::Cut, Origin::Manual, Writer::Human)
}

/// A move of `frames` frames along `curve` arriving at `frame` at the window `(cx, cy, h)`, made by a person.
fn moved(id: u32, frame: i64, window: (f64, f64, f64), frames: u32, curve: Curve) -> Position {
    let (cx, cy, h) = window;
    let arrival = Arrival::Move { move_frames: frames, curve, split_of: None };
    Position::new(id, at(frame), Window { cx, cy, h }, arrival, Origin::Manual, Writer::Human)
}

/// A vertical set with plan 1 "9:16" of output `out` holding `positions`.
fn set_for(out: OutputSize, positions: Vec<Position>) -> VerticalSet {
    let mut set = crate::empty_vertical_set();
    let mut plan = VerticalPlan::new(1, "9:16", out);
    plan.next_position_id = positions.iter().map(|p| p.id).max().unwrap_or(0) + 1;
    plan.positions = positions;
    set.plans.push(plan);
    set.next_plan_id = 2;
    set
}

/// A vertical set with plan 1 "9:16" (1080×1920) holding `positions`.
fn set_with(positions: Vec<Position>) -> VerticalSet {
    set_for(NINE_SIXTEEN, positions)
}

/// The framing fixture's plan: #1 a cut at 0 (centred, full height), #2 a cut at 100, #3 a 30-frame ease in-out move
/// arriving at 140 (h 1000), #4 a 30-frame Ramp 20/20 move arriving at 180 (h 900), #5 a cut at 200, #6 a cut at 300.
/// Every shot opens on its own position, and #2 to #4 walk through shot 2.
fn framing_positions() -> Vec<Position> {
    vec![
        cut(1, 0, 960.0, 540.0, 1080.0),
        cut(2, 100, 700.0, 540.0, 1080.0),
        moved(3, 140, (900.0, 540.0, 1000.0), 30, Curve::EaseInOut),
        moved(4, 180, (1100.0, 540.0, 900.0), 30, v::DEFAULT_CURVE),
        cut(5, 200, 1000.0, 540.0, 1080.0),
        cut(6, 300, 960.0, 540.0, 1080.0),
    ]
}

/// The framing fixture: [`FOUR`] at 1920×1080 25p with plan 1 of [`framing_positions`].
fn framing() -> (Session, ItemId) {
    session_with(1920, 1080, &FOUR, Some(set_with(framing_positions())))
}

/// The measurement fixture: [`CUTS`] at 640×360 25p, with no vertical data.
fn measuring() -> (Session, ItemId) {
    session_with(640, 360, &CUTS, None)
}

/// The frame fixture's set: plan 1 (180×320) with #1 at 0 (centred, full height) and #2, a cut at 50 to a smaller window.
fn small_set() -> VerticalSet {
    set_for(OutputSize { width: 180, height: 320 }, vec![cut(1, 0, 160.0, 90.0, 180.0), cut(2, 50, 100.0, 90.0, 160.0)])
}

/// The frame fixture at 320×180 25p: a 50-frame matte, then 50 frames of Bars and Tone, with [`small_set`]: cheap exact
/// frames. #2 frames the bars, whose columns differ, so a pan of its window shows other pixels (on a matte every window
/// shows the same ones).
fn small() -> (Session, ItemId) {
    let mut s = Session::default();
    let clips = [(matte([200, 40, 72]), 50), (Generator::BarsAndTone, 50)];
    let (p, seq) = project_showing(&s.media, 320, 180, &clips, Some(small_set()));
    install(&mut s, p, seq);
    (s, seq)
}

/// The master fixture: a 640×360 sequence "Master" whose one clip nests the sequence "Main" of [`CUTS`]' first three
/// mattes (cuts at 100 and 200 inside the nest). The master has no picture edit point, so its shots are not known until a
/// measurement is applied. `set` is the master's vertical data.
fn master(set: Option<VerticalSet>) -> (Session, ItemId) {
    let mut s = Session::default();
    let (mut p, source) = project_with(&s.media, 640, 360, &CUTS[..3], None);
    let settings = SequenceSettings { width: 640, height: 360, frame_rate: RATE, ..Default::default() };
    let main = p.new_sequence("Master", settings, 1, 1, None);
    let range = TimeRange::new(Tick::ZERO, at(300));
    let mut nest = p.make_track_item(source, TrackKind::Video, Tick::ZERO, range, RATE).expect("a sequence nests");
    for e in &mut nest.effects {
        filmcraft_project::resolve_auto_points(e, (640, 360), (640, 360));
    }
    let q = p.sequence_mut(main).expect("the master");
    q.video_tracks[0].items.push(nest);
    q.vertical = set.map(Box::new);
    install(&mut s, p, main);
    (s, main)
}

/// A framing fixture whose plan 1 breaks I1: positions #2 and #3 are stored out of order.
fn damaged() -> (Session, ItemId) {
    let mut positions = framing_positions();
    positions.swap(1, 2);
    session_with(1920, 1080, &FOUR, Some(set_with(positions)))
}

// ---------- helpers ----------

/// Run a command that must succeed; its result.
fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, p.clone()) {
        Ok(r) => r,
        Err(e) => panic!("`{id}` {p}: {e}"),
    }
}

/// The code and detail of a refused call (`EngineError::Invalid`).
fn refusal_of(id: &str, r: crate::Result<Value>) -> (&'static str, Value) {
    match r {
        Err(EngineError::Invalid { code, detail, .. }) => (code, detail),
        other => panic!("`{id}` should be refused with a code: {other:?}"),
    }
}

/// Run a command that must be refused with a code; the code and its detail.
fn refused(s: &mut Session, id: &str, p: Value) -> (&'static str, Value) {
    let r = s.execute(id, p);
    refusal_of(id, r)
}

/// Assert that the object `actual` holds every key of `expected` with its value.
fn contains(actual: &Value, expected: &Value, what: &str) {
    for (key, want) in expected.as_object().into_iter().flatten() {
        assert_eq!(&actual[key.as_str()], want, "{what}: `{key}` of {actual}");
    }
}

/// `value` without the keys `keys`.
fn without(value: &Value, keys: &[&str]) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        for key in keys {
            object.remove(*key);
        }
    }
    value
}

/// What a call that records nothing must leave as it was: the project, both revisions and the history.
struct Snapshot {
    project: Arc<Project>,
    revision: u64,
    picture: u64,
    undo: usize,
    redo: usize,
}

impl Snapshot {
    fn of(s: &Session) -> Self {
        Self { project: s.project.clone(), revision: s.revision, picture: s.picture_revision, undo: s.history.undo.len(), redo: s.history.redo.len() }
    }

    fn unchanged(&self, s: &Session, what: &str) {
        assert!(Arc::ptr_eq(&self.project, &s.project), "{what} changed the project");
        assert_eq!((s.revision, s.picture_revision), (self.revision, self.picture), "{what} moved a revision");
        assert_eq!((s.history.undo.len(), s.history.redo.len()), (self.undo, self.redo), "{what} recorded a step");
    }
}

/// Run a command that must be refused `code` with at least `detail`, and change nothing.
fn expect_refusal(s: &mut Session, id: &str, p: Value, code: &str, detail: Value) {
    let before = Snapshot::of(s);
    let what = format!("`{id}` {p}");
    let (got, d) = refused(s, id, p);
    assert_eq!(got, code, "{what}: {d}");
    contains(&d, &detail, &what);
    before.unchanged(s, &what);
}

/// The vertical set of sequence `seq`.
fn set_of(s: &Session, seq: ItemId) -> &VerticalSet {
    s.project.sequence(seq).and_then(|q| q.vertical.as_deref()).expect("the sequence has a vertical set")
}

/// Plan `id` of sequence `seq`.
fn plan_in(s: &Session, seq: ItemId, id: u32) -> &VerticalPlan {
    set_of(s, seq).plans.iter().find(|p| p.id == id).expect("the plan exists")
}

/// Position `id` of plan 1 of sequence `seq`.
fn position(s: &Session, seq: ItemId, id: u32) -> &Position {
    plan_in(s, seq, 1).positions.iter().find(|p| p.id == id).expect("the position exists")
}

/// The plan digest of plan `id` of sequence `seq`.
fn plan_digest(s: &Session, seq: ItemId, id: u32) -> String {
    v::plan_digest(s.project.sequence(seq).expect("the sequence"), plan_in(s, seq, id))
}

/// The range digest of `[from, to)` of plan `plan` of sequence `seq`.
fn range_digest(s: &Session, seq: ItemId, plan: u32, from: i64, to: i64) -> String {
    v::range_digest(&s.project, s.project.sequence(seq).expect("the sequence"), plan_in(s, seq, plan), from, to)
}

/// An id a result names.
fn id_of(value: &Value) -> u32 {
    u32::try_from(value.as_u64().expect("an id")).expect("an id is a u32")
}

/// The frames of a list of `{frame, score}`.
fn frames_of(list: &Value) -> Vec<i64> {
    list.as_array().into_iter().flatten().filter_map(|c| c["frame"].as_i64()).collect()
}

/// The items of a list of `{frame, score}` whose frame lies in `[from, to)`.
fn within(list: &Value, from: i64, to: i64) -> Value {
    let inside = |c: &&Value| c["frame"].as_i64().is_some_and(|f| (from..to).contains(&f));
    Value::Array(list.as_array().into_iter().flatten().filter(inside).cloned().collect())
}

/// Read `vertical.jobResult` until the job's state is `state` (at most 60 s); the last reply.
fn wait_for(s: &mut Session, job: u64, state: &str) -> Value {
    let t0 = Instant::now();
    loop {
        let r = ok(s, "vertical.jobResult", json!({"job": job}));
        if r["state"] == json!(state) {
            return r;
        }
        assert_eq!(r["state"], json!("running"), "job {job}");
        assert!(t0.elapsed() < Duration::from_secs(60), "job {job} did not reach `{state}`: {r}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A media source with a gate in front of its frames: while the gate is closed, a frame asked of it waits (at most a
/// minute), so a measurement job over it is still running whatever the machine's speed until the test opens the gate.
struct Gated {
    inner: filmcraft_media::SharedSource,
    closed: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}

impl Gated {
    /// Put a gate, open, in front of the source of the item that clip `index` of V1 of sequence `seq` shows.
    fn install(s: &Session, seq: ItemId, index: usize) -> Arc<Gated> {
        let item = s.project.sequence(seq).expect("the sequence").video_tracks[0].items[index].item;
        let inner = s.media.full_res_source(&s.project, item, &*s.services).expect("the clip's source");
        let gate = Arc::new(Gated { inner, closed: std::sync::Mutex::new(false), changed: std::sync::Condvar::new() });
        s.media.insert(item, gate.clone());
        gate
    }

    fn set(&self, closed: bool) {
        *self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = closed;
        self.changed.notify_all();
    }
}

impl filmcraft_media::MediaSource for Gated {
    fn info(&self) -> &filmcraft_media::MediaInfo {
        self.inner.info()
    }

    fn video_frame(&self, req: filmcraft_media::FrameRequest) -> filmcraft_media::Result<Arc<filmcraft_frame::VideoFrame>> {
        let t0 = Instant::now();
        let mut closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while *closed && t0.elapsed() < Duration::from_secs(60) {
            closed = match self.changed.wait_timeout(closed, Duration::from_millis(50)) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        drop(closed);
        self.inner.video_frame(req)
    }

    fn audio(&self, start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<filmcraft_frame::AudioBuffer> {
        self.inner.audio(start, frames, sample_rate)
    }
}

/// Start a dynamic roll trim of the out edge of V1's first clip of the active sequence (L and a tick, as lane L2a's
/// session tests do).
fn dynamic_roll(s: &mut Session) {
    for track in ["V2", "V3", "A2", "A3"] {
        let _ = s.execute("timeline.setTrack", json!({"track": track, "syncLock": false}));
    }
    let clip = s.active_sequence().expect("the active sequence").video_tracks[0].items[0].id.0;
    ok(s, "trim.selectEditPoint", json!({"clip": clip, "edge": "out", "kind": "roll"}));
    ok(s, "trim.shuttle", json!({"direction": "forward", "clock": 0.0}));
    ok(s, "trim.tick", json!({"clock": 0.5}));
    assert!(s.trim_play.dynamic.is_some(), "a dynamic trim runs");
}

/// The calls a case runs on its fixture first.
type Setup = Vec<(&'static str, Value)>;

/// Lock positions `ids` of plan 1.
fn lock(ids: &[u32]) -> Setup {
    vec![("vertical.setLock", json!({"plan": 1, "positions": ids, "locked": true}))]
}

/// A manual wall at 125, inside #3's move: the move is shortened to start there and flagged `MoveShortened`.
fn shortened() -> Setup {
    vec![("vertical.setWalls", json!({"add": [125], "onConflict": "shorten"}))]
}

/// Part P01 over shots 2 and 3, `[100, 300)`.
fn with_part() -> Setup {
    vec![("vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 100, "toFrame": 300}]}))]
}

/// Part P01 with every position in it reviewed.
fn reviewed_part() -> Setup {
    let mut calls = with_part();
    calls.push(("vertical.setReview", json!({"plan": 1, "positions": [2, 3, 4, 5], "reviewed": true})));
    calls
}

/// Shot 4 without its opener: #6 deleted, so shot 4 is carried.
fn carried() -> Setup {
    vec![("vertical.deletePositions", json!({"plan": 1, "positions": [6]}))]
}

/// An attention mark at 150.
fn flagged() -> Setup {
    vec![("vertical.flagAttention", json!({"plan": 1, "frame": 150, "note": "look again"}))]
}

// ---------- the command table and its parameters ----------

/// The writes of plan 63 §3.2 (`vertical.remap` is lane L2d's, M2 Stage 3).
const WRITES: [&str; 26] = [
    "vertical.create",
    "vertical.update",
    "vertical.delete",
    "vertical.setPosition",
    "vertical.retimePosition",
    "vertical.deletePositions",
    "vertical.replaceRange",
    "vertical.setArrival",
    "vertical.smoothThrough",
    "vertical.fitMove",
    "vertical.swapWindows",
    "vertical.setWalls",
    "vertical.applyMeasurement",
    "vertical.seedShots",
    "vertical.setParts",
    "vertical.setRegions",
    "vertical.markChecked",
    "vertical.repair",
    "vertical.setAnnotations",
    "vertical.setActiveArea",
    "vertical.approvePart",
    "vertical.setReview",
    "vertical.setLock",
    "vertical.flagAttention",
    "vertical.clearAttention",
    "vertical.remap",
];

/// The editor-state commands and the reads of plan 63 §3.2: never journaled, never an undo step.
const UNJOURNALED: [&str; 11] = [
    "vertical.select",
    "vertical.goTo",
    "vertical.noteWatched",
    "vertical.list",
    "vertical.inspect",
    "vertical.windows",
    "vertical.measure",
    "vertical.fit",
    "vertical.storyboard",
    "vertical.measureCuts",
    "vertical.jobResult",
];

/// Commands of M4 and M5, and v1.1's replaced names: none is registered in M2.
const LATER: [&str; 19] = [
    "vertical.takeReference",
    "vertical.conformCheck",
    "vertical.partEdges",
    "vertical.edgeProperties",
    "vertical.setDeliveryInfo",
    "vertical.deliveries",
    "vertical.staleDeliveries",
    "vertical.verifyDelivery",
    "vertical.queueParts",
    "vertical.coverFrame",
    "vertical.exportBreakList",
    "vertical.setBoxTracks",
    "vertical.setAgentOverwrite",
    "vertical.pauseAgentWrites",
    "vertical.revertToProposal",
    "vertical.revertToReviewed",
    "vertical.acceptOverwrite",
    "vertical.detectWalls",
    "vertical.detectResult",
];

#[test]
fn every_kept_command_is_registered_with_its_journal_flag() {
    for id in WRITES {
        let spec = crate::find_command(id).unwrap_or_else(|| panic!("`{id}` is registered"));
        assert!(spec.journal, "`{id}` is journaled");
    }
    for id in UNJOURNALED {
        let spec = crate::find_command(id).unwrap_or_else(|| panic!("`{id}` is registered"));
        assert!(!spec.journal, "`{id}` records nothing");
    }
    for id in LATER {
        assert!(crate::find_command(id).is_none(), "`{id}` is not a command of M2");
    }
    let registered = crate::command_specs().iter().filter(|c| c.id.starts_with("vertical.")).count();
    assert_eq!(registered, WRITES.len() + UNJOURNALED.len());
    let params = |id: &str| crate::find_command(id).map(|c| c.params).unwrap_or_default();
    for key in ["\"merge\"", "\"ramp\"", "\"timecode\"", "\"expectRange\"", "\"focus\"", "\"fromLabel\""] {
        assert!(params("vertical.setPosition").contains(key), "setPosition documents {key}");
    }
    for key in ["subjects", "confidence", "risk", "runId"] {
        assert!(!params("vertical.setPosition").contains(key), "setPosition takes no `{key}` in M2");
    }
    for key in ["\"echo\"", "\"strict\"", "\"expectWalls\""] {
        assert!(params("vertical.replaceRange").contains(key), "replaceRange documents {key}");
    }
    assert!(!params("vertical.replaceRange").contains("overwriteReviewed"));
    assert!(params("vertical.measureCuts").contains("\"wait\""));
    assert!(params("vertical.measure").contains("\"baseline\"") && params("vertical.measure").contains("\"safeZone\""));
    assert!(params("vertical.windows").contains("\"fields\""));
    assert!(params("vertical.noteWatched").contains("\"frames\"") && params("vertical.noteWatched").contains("\"pass\""));
}

#[test]
fn unknown_keys_and_run_ids_are_refused_by_every_command() {
    let (mut s, _) = framing();
    for id in WRITES.iter().chain(&UNJOURNALED) {
        for key in ["bogus", "runId"] {
            let before = Snapshot::of(&s);
            let (code, detail) = refused(&mut s, id, json!({ key: 1 }));
            assert_eq!((code, &detail["key"]), ("out_of_range", &json!(key)), "`{id}` refuses `{key}`: {detail}");
            before.unchanged(&s, id);
        }
    }
    // the parameters are an object
    for id in ["vertical.setPosition", "vertical.list", "vertical.measureCuts"] {
        let (code, detail) = refused(&mut s, id, json!([1, 2]));
        assert_eq!((code, &detail["key"]), ("out_of_range", &json!("params")), "`{id}`: {detail}");
    }
}

#[test]
fn seconds_are_refused_use_frame() {
    let (mut s, _) = framing();
    let calls = [
        ("vertical.setPosition", json!({"plan": 1, "seconds": 2.0, "cx": 960})),
        ("vertical.retimePosition", json!({"plan": 1, "position": 3, "seconds": 5.5})),
        ("vertical.flagAttention", json!({"plan": 1, "seconds": 1.0})),
        ("vertical.goTo", json!({"plan": 1, "to": "nextWall", "seconds": 1.0})),
        ("vertical.windows", json!({"plan": 1, "fromFrame": 0, "toFrame": 10, "seconds": 0.5})),
        ("vertical.measureCuts", json!({"seconds": 1.0})),
    ];
    for (id, p) in calls {
        expect_refusal(&mut s, id, p, "use_frame", json!({"key": "seconds"}));
    }
}

#[test]
fn a_time_off_a_frame_start_is_bad_time_with_the_floor_and_nearest_frames() {
    let (mut s, seq) = framing();
    let sp = "vertical.setPosition";
    let (t, frame) = (at(250).0, RATE.frame_duration().0);
    let floor = json!({"floorFrame": 250, "nearestFrame": 250});
    expect_refusal(&mut s, sp, json!({"plan": 1, "time": t + 1, "cx": 1000}), "bad_time", floor);
    let late = (t + frame * 3 / 4).to_string();
    let nearest = json!({"floorFrame": 250, "nearestFrame": 251});
    expect_refusal(&mut s, sp, json!({"plan": 1, "time": late, "cx": 1000}), "bad_time", nearest);
    expect_refusal(&mut s, sp, json!({"plan": 1, "frame": 250, "time": t, "cx": 1000}), "out_of_range", json!({"key": "frame"}));
    expect_refusal(&mut s, sp, json!({"plan": 1, "frame": 250.5, "cx": 1000}), "out_of_range", json!({"key": "frame"}));
    // a time on a frame start, as an integer or a string of ticks, names that frame; an integral float is a frame
    let r = ok(&mut s, sp, json!({"plan": 1, "time": t.to_string(), "cx": 1000}));
    assert_eq!(position(&s, seq, id_of(&r["added"][0])).at, at(250));
    let r = ok(&mut s, sp, json!({"plan": 1, "time": at(255).0, "cx": 1000}));
    assert_eq!(position(&s, seq, id_of(&r["added"][0])).at, at(255));
    let r = ok(&mut s, sp, json!({"plan": 1, "frame": 260.0, "cx": 1010}));
    assert_eq!(position(&s, seq, id_of(&r["added"][0])).at, at(260));
}

#[test]
fn a_timecode_names_a_frame_and_may_carry_a_part_prefix() {
    let (mut s, seq) = framing();
    rebuild(&mut s, seq, |q| {
        q.start_timecode = 90_000;
        let set = q.vertical.as_deref_mut().expect("the set");
        set.parts.push(Part { id: 1, name: "P01".into(), from: at(100), to: at(300), audio: None, meta: PartMeta::default() });
        set.next_part_id = 2;
    });
    let sp = "vertical.setPosition";
    let r = ok(&mut s, sp, json!({"plan": 1, "timecode": "P01 00:00:06:00", "cx": 1000}));
    assert_eq!(position(&s, seq, id_of(&r["added"][0])).at, at(250), "P01 starts at frame 100: 6 s later is frame 250");
    let r = ok(&mut s, sp, json!({"plan": 1, "timecode": "01:00:13:00", "cx": 1010}));
    assert_eq!(position(&s, seq, id_of(&r["added"][0])).at, at(325), "the sequence starts at 01:00:00:00");
    expect_refusal(&mut s, sp, json!({"plan": 1, "timecode": "P99 00:00:01:00", "cx": 1000}), "not_found", json!({"part": "P99"}));
    expect_refusal(&mut s, sp, json!({"plan": 1, "timecode": "P01 ab:cd", "cx": 1000}), "bad_time", json!({}));
    expect_refusal(&mut s, sp, json!({"plan": 1, "timecode": "00:59:59:00", "cx": 1000}), "out_of_range", json!({"key": "frame"}));
}

#[test]
fn t_m9_hostile_values_are_refused_without_a_panic() {
    let (mut s, _) = framing();
    let sp = "vertical.setPosition";
    let many: Vec<u32> = (0..10_001).collect();
    let frames: Vec<i64> = (1..10_002).collect();
    let long = "x".repeat(100_000);
    let cases = [
        (sp, json!({"plan": 1, "frame": 250, "cx": "NaN"}), "bad_number", json!({"key": "cx"})),
        (sp, json!({"plan": 1, "frame": 250, "cx": "1e309"}), "bad_number", json!({"key": "cx"})),
        (sp, json!({"plan": 1, "frame": 250, "h": "-inf"}), "bad_number", json!({"key": "h"})),
        (sp, json!({"plan": 1, "frame": 250, "cx": true}), "bad_number", json!({"key": "cx"})),
        (sp, json!({"plan": 1, "frame": 250, "cx": 1e300}), "out_of_range", json!({"key": "cx"})),
        (sp, json!({"plan": 1, "frame": -5, "cx": 900}), "out_of_range", json!({"key": "frame"})),
        (sp, json!({"plan": 1, "frame": 1_000_000_000_000_000_i64, "cx": 900}), "out_of_range", json!({"key": "frame"})),
        (sp, json!({"plan": 4_294_967_296_u64, "frame": 250}), "out_of_range", json!({"key": "plan"})),
        (sp, json!({"plan": -1, "frame": 250}), "out_of_range", json!({"key": "plan"})),
        (sp, json!({"plan": "1", "frame": 250}), "out_of_range", json!({"key": "plan"})),
        (sp, json!({"plan": 1, "frame": 250, "label": long}), "too_many", json!({"key": "label"})),
        (sp, json!({"plan": 1, "frame": 250, "tags": 5}), "out_of_range", json!({"key": "tags"})),
        (sp, json!({"plan": 1, "frame": 250, "curve": "wobbly", "moveFrames": 10}), "out_of_range", json!({"key": "curve"})),
        (sp, json!({"plan": 1, "frame": 250, "ramp": [0.7, 0.6], "moveFrames": 10}), "bad_curve", json!({"axis": "ramp"})),
        (sp, json!({"plan": 1, "frame": 250, "bezier": [1, 0, 0, 1], "moveFrames": 10}), "bad_curve", json!({"axis": "slope"})),
        (sp, json!({"plan": 1, "frame": 250, "bezier": [0.5, 0.5], "moveFrames": 10}), "out_of_range", json!({"key": "bezier"})),
        ("vertical.deletePositions", json!({"plan": 1, "positions": many}), "too_many", json!({"key": "positions", "count": 10_001, "bound": 10_000})),
        ("vertical.setWalls", json!({"add": frames}), "too_many", json!({"key": "add"})),
    ];
    for (id, p, code, detail) in cases {
        let before = Snapshot::of(&s);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute(id, p.clone())));
        let r = r.unwrap_or_else(|_| panic!("`{id}` {p} panicked"));
        let (got, d) = refusal_of(id, r);
        assert_eq!(got, code, "`{id}` {p}: {d}");
        contains(&d, &detail, id);
        before.unchanged(&s, id);
    }
}

#[test]
fn a_missing_sequence_or_plan_is_not_found() {
    let (mut s, seq) = framing();
    let media = s.project.sequence(seq).expect("the sequence").video_tracks[0].items[0].item.0;
    let p = json!({"sequence": 999_999, "plan": 1, "frame": 10});
    expect_refusal(&mut s, "vertical.setPosition", p, "not_found", json!({"sequence": 999_999}));
    expect_refusal(&mut s, "vertical.inspect", json!({"sequence": media, "plan": 1}), "not_found", json!({"sequence": media}));
    let calls = [
        ("vertical.inspect", json!({"plan": 9})),
        ("vertical.windows", json!({"plan": 9, "fromFrame": 0, "toFrame": 10})),
        ("vertical.setPosition", json!({"plan": 9, "frame": 10})),
        ("vertical.measure", json!({"plan": 9})),
    ];
    for (id, p) in calls {
        expect_refusal(&mut s, id, p, "not_found", json!({"plan": 9}));
    }
    // without a sequence open, a call that names none finds none
    let mut empty = Session::default();
    let (code, _) = refused(&mut empty, "vertical.list", json!({}));
    assert_eq!(code, "not_found");
}

// ---------- writes: the envelope, dry runs, no-ops, gestures ----------

#[test]
fn a_write_returns_the_envelope_of_spec_7_2() {
    let (mut s, seq) = framing();
    let before = plan_digest(&s, seq, 1);
    let (revision, picture, steps) = (s.revision, s.picture_revision, s.history.undo.len());
    let r = ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 250, "cx": 1100}));
    assert_eq!((&r["applied"], &r["changed"], &r["dryRun"]), (&json!(true), &json!(true), &json!(false)), "{r}");
    assert_eq!((&r["sequence"], &r["plan"]), (&json!(seq.0), &json!(1)));
    assert_eq!(r["digestBefore"], json!(before));
    assert_eq!(r["digest"], json!(plan_digest(&s, seq, 1)));
    assert_ne!(r["digest"], r["digestBefore"]);
    assert_eq!(r["label"], json!("Vertical: new position (#7)"));
    assert_eq!((&r["added"], &r["updated"], &r["removed"]), (&json!([7]), &json!([]), &json!([])));
    assert_eq!(r["changedFrames"], json!([[250, 300]]), "frames 250 to 299 show the new window");
    assert_eq!(r["clamped"], json!([]));
    assert!(r["issues"].is_array(), "{r}");
    // one plan-only undo step: the revision moved, the picture revision did not
    assert_eq!(s.history.undo.len(), steps + 1);
    let step = s.history.undo.last().expect("the step");
    assert_eq!((step.label(), step.2.picture), ("Vertical: new position (#7)", false));
    assert_eq!((s.revision, s.picture_revision), (revision + 1, picture));
    // a person's direct edit of one position: reviewed, by a person
    let p = position(&s, seq, 7);
    let window = Window { cx: 1100.0, cy: 540.0, h: 1080.0 };
    assert_eq!((p.window, p.reviewed, p.modified_by, p.origin), (window, true, Writer::Human, Origin::Manual));
}

/// One write of a table: the calls that prepare [`framing`], then the write.
type Case = (Setup, &'static str, Value);

/// One case per write of plan 63 §3.2 but `remap`, `applyMeasurement` and `repair` (which need their own fixtures), each
/// changing something on [`framing`].
fn changing_writes() -> Vec<Case> {
    let items = json!([{"frame": 200, "cx": 990, "cy": 540, "h": 1080}, {"frame": 250, "cx": 1050, "cy": 540, "h": 1000}]);
    let region = json!({"label": "logo", "fromFrame": 0, "toFrame": 400, "rect": [1700, 40, 160, 90], "mode": "keepOut"});
    vec![
        (Vec::new(), "vertical.create", json!({"name": "Square", "preset": "1:1"})),
        (Vec::new(), "vertical.update", json!({"plan": 1, "name": "Phone"})),
        (Vec::new(), "vertical.delete", json!({"plan": 1})),
        (Vec::new(), "vertical.setPosition", json!({"plan": 1, "frame": 250, "cx": 1100})),
        (Vec::new(), "vertical.retimePosition", json!({"plan": 1, "position": 6, "frame": 310})),
        (Vec::new(), "vertical.deletePositions", json!({"plan": 1, "positions": [6]})),
        (Vec::new(), "vertical.replaceRange", json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": items})),
        (Vec::new(), "vertical.setArrival", json!({"plan": 1, "positions": [3], "arrival": "move", "moveFrames": 20, "curve": "linear"})),
        (Vec::new(), "vertical.smoothThrough", json!({"plan": 1, "positions": [2, 3, 4]})),
        (Vec::new(), "vertical.fitMove", json!({"plan": 1, "position": 3, "maxSpeed": 20.0})),
        (Vec::new(), "vertical.swapWindows", json!({"plan": 1, "positions": [5, 6]})),
        (Vec::new(), "vertical.setWalls", json!({"add": [{"frame": 250, "score": 0.5}]})),
        (carried(), "vertical.seedShots", json!({"plan": 1, "mode": "carry"})),
        (Vec::new(), "vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 100, "toFrame": 300}]})),
        (Vec::new(), "vertical.setRegions", json!({"add": [region]})),
        (shortened(), "vertical.markChecked", json!({"plan": 1, "positions": [3]})),
        (Vec::new(), "vertical.setAnnotations", json!({"add": [{"kind": "speech", "fromFrame": 0, "toFrame": 100, "label": "char:dana"}]})),
        (Vec::new(), "vertical.setActiveArea", json!({"set": [{"fromFrame": 0, "toFrame": 400, "rect": [0, 140, 1920, 800]}]})),
        (reviewed_part(), "vertical.approvePart", json!({"plan": 1, "part": 1, "note": "ok"})),
        (Vec::new(), "vertical.setReview", json!({"plan": 1, "positions": [2], "reviewed": true})),
        (Vec::new(), "vertical.setLock", json!({"plan": 1, "positions": [2], "locked": true})),
        (Vec::new(), "vertical.flagAttention", json!({"plan": 1, "frame": 150, "note": "check the pan"})),
        (flagged(), "vertical.clearAttention", json!({"plan": 1, "frames": [150]})),
    ]
}

/// A framing fixture prepared by `setup`, with a fixed clock.
fn prepared(setup: &[(&'static str, Value)]) -> Session {
    let (mut s, _) = framing();
    s.clock = Clock::fixed(1_793_000_000);
    for (id, p) in setup {
        ok(&mut s, id, p.clone());
    }
    s
}

#[test]
fn a_dry_run_returns_the_real_result_and_records_nothing() {
    for (setup, id, p) in changing_writes() {
        let (mut dry, mut real) = (prepared(&setup), prepared(&setup));
        let mut asked = p.clone();
        asked["dryRun"] = json!(true);
        let before = Snapshot::of(&dry);
        let d = ok(&mut dry, id, asked);
        before.unchanged(&dry, id);
        let r = ok(&mut real, id, p);
        assert_eq!((&d["dryRun"], &d["applied"], &d["label"]), (&json!(true), &json!(false), &Value::Null), "`{id}`: {d}");
        assert_eq!((&r["dryRun"], &r["applied"], &r["changed"]), (&json!(false), &json!(true), &json!(true)), "`{id}`: {r}");
        assert!(r["label"].is_string(), "`{id}`: {r}");
        let keys = ["dryRun", "applied", "label"];
        assert_eq!(without(&d, &keys), without(&r, &keys), "`{id}`: the dry run's result is the real one");
    }
}

#[test]
fn a_write_that_changes_nothing_records_nothing() {
    let (mut s, _) = framing();
    let calls = [
        ("vertical.setPosition", json!({"plan": 1, "frame": 100, "cx": 700, "cy": 540, "h": 1080})),
        ("vertical.update", json!({"plan": 1, "name": "9:16"})),
        ("vertical.setWalls", json!({})),
        ("vertical.markChecked", json!({"plan": 1})),
        ("vertical.setRegions", json!({})),
        ("vertical.clearAttention", json!({"plan": 1, "frames": [5]})),
        ("vertical.setLock", json!({"plan": 1, "positions": [2], "locked": false})),
        ("vertical.setReview", json!({"plan": 1, "positions": [2], "reviewed": false})),
        ("vertical.seedShots", json!({"plan": 1, "mode": "carry"})),
        ("vertical.setArrival", json!({"plan": 1, "positions": [2], "arrival": "cut"})),
        ("vertical.deletePositions", json!({"plan": 1, "positions": [99]})),
    ];
    for (id, p) in calls {
        let before = Snapshot::of(&s);
        let r = ok(&mut s, id, p);
        assert_eq!((&r["changed"], &r["applied"], &r["label"]), (&json!(false), &json!(false), &Value::Null), "`{id}`: {r}");
        before.unchanged(&s, id);
    }
    let r = ok(&mut s, "vertical.deletePositions", json!({"plan": 1, "positions": [99]}));
    assert_eq!((&r["deleted"], &r["missing"]), (&json!([]), &json!([99])));
}

#[test]
fn merged_sub_steps_are_one_undo_step_and_stale_range_stops_the_gesture() {
    let (mut s, seq) = framing();
    let (steps, picture) = (s.history.undo.len(), s.picture_revision);
    let mut digest = range_digest(&s, seq, 1, 100, 200);
    for k in 1..=20 {
        let guard = json!({"fromFrame": 100, "toFrame": 200, "digest": digest});
        let p = json!({"plan": 1, "frame": 140, "cx": 900 + 2 * k, "merge": "drag:3", "expectRange": guard});
        let r = ok(&mut s, "vertical.setPosition", p);
        let fixed = (&json!("Vertical: move window (#3)"), &Value::Null, &Value::Null);
        assert_eq!((&r["label"], &r["digestBefore"], &r["digest"]), fixed, "sub-step {k}: {r}");
        digest = r["rangeDigest"].as_str().expect("a guarded write returns the range's new digest").to_string();
        assert_eq!(digest, range_digest(&s, seq, 1, 100, 200));
    }
    assert_eq!(s.history.undo.len(), steps + 1, "the drag is one undo step");
    assert_eq!(s.picture_revision, picture, "no 16:9 frame changed");
    assert_eq!(position(&s, seq, 3).window.cx, 940.0);
    // another write to the dragged shot stops the drag: its next sub-step is refused with the range's current digest
    ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 180, "cx": 1110}));
    let current = range_digest(&s, seq, 1, 100, 200);
    let guard = json!({"fromFrame": 100, "toFrame": 200, "digest": digest});
    let p = json!({"plan": 1, "frame": 140, "cx": 950, "merge": "drag:3", "expectRange": guard});
    expect_refusal(&mut s, "vertical.setPosition", p, "stale_range", json!({"fromFrame": 100, "toFrame": 200, "digest": current}));
    assert_eq!(s.history.undo.len(), steps + 2);
    s.undo();
    assert_eq!(position(&s, seq, 3).window.cx, 940.0, "undo takes the other write back");
    s.undo();
    assert_eq!(position(&s, seq, 3).window.cx, 900.0, "one more undo takes the whole drag back");
}

#[test]
fn changed_frames_strict_and_echo() {
    let (mut s, _) = framing();
    let rr = "vertical.replaceRange";
    let item = json!({"frame": 200, "cx": 990, "cy": 540, "h": 1080});
    let r = ok(&mut s, rr, json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": [item]}));
    assert_eq!(r["changedFrames"], json!([[200, 300]]), "frames 200 to 299 show the new window");
    assert!(r.get("positions").is_none(), "positions are echoed only when asked");
    // strict: a write whose changed frames leave its range is refused with them, and changes nothing
    let item = json!({"frame": 100, "cx": 720, "cy": 540, "h": 1080});
    let p = json!({"plan": 1, "fromFrame": 100, "toFrame": 120, "strict": true, "positions": [item]});
    expect_refusal(&mut s, rr, p, "range_exceeded", json!({"changedFrames": [[100, 140]], "range": [100, 120]}));
    let r = ok(&mut s, rr, json!({"plan": 1, "fromFrame": 100, "toFrame": 120, "positions": [item]}));
    assert_eq!(r["changedFrames"], json!([[100, 140]]), "the move out of #2 starts from its new window");
    // echo: the changed and added positions in the read shape
    let items = json!([{"frame": 200, "cx": 980, "cy": 540, "h": 1080}, {"frame": 250, "cx": 1050, "cy": 540, "h": 1000}]);
    let r = ok(&mut s, rr, json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "echo": true, "positions": items}));
    let echoed = r["positions"].as_array().expect("echo returns positions");
    let ids: Vec<u64> = echoed.iter().filter_map(|p| p["id"].as_u64()).collect();
    assert_eq!(ids, [5, 7]);
    let added = echoed.get(1).expect("the new position");
    let want = json!({"id": 7, "frame": 250, "tick": at(250).0.to_string(), "cx": 1050.0, "cy": 540.0, "h": 1000.0, "tag": "JUMP", "reviewed": false});
    contains(added, &want, "the echoed position");
}

#[test]
fn stale_guards_name_the_current_digest() {
    let (mut s, seq) = framing();
    let zero = "fnv1a64:0000000000000000";
    let current = plan_digest(&s, seq, 1);
    let walls = v::walls_digest(set_of(&s, seq));
    let effective = v::effective_walls_digest(&s.project, s.project.sequence(seq).expect("the sequence"));
    let (data, parts, regions) = (v::data_digest(set_of(&s, seq)), v::parts_digest(set_of(&s, seq)), v::regions_digest(set_of(&s, seq)));
    expect_refusal(&mut s, "vertical.update", json!({"plan": 1, "name": "x", "expect": zero}), "stale_plan", json!({"digest": current}));
    expect_refusal(&mut s, "vertical.setWalls", json!({"add": [250], "expect": zero}), "stale_plan", json!({"digest": walls}));
    expect_refusal(&mut s, "vertical.setWalls", json!({"add": [250], "expectWalls": zero}), "stale_walls", json!({"digest": effective}));
    expect_refusal(&mut s, "vertical.setAnnotations", json!({"remove": [1], "expect": zero}), "stale_plan", json!({"digest": data}));
    expect_refusal(&mut s, "vertical.setActiveArea", json!({"clear": [[0, 10]], "expect": zero}), "stale_plan", json!({"digest": data}));
    expect_refusal(&mut s, "vertical.setParts", json!({"remove": [1], "expect": zero}), "stale_plan", json!({"digest": parts}));
    expect_refusal(&mut s, "vertical.setRegions", json!({"remove": [1], "expect": zero}), "stale_plan", json!({"digest": regions}));
    // the digests `vertical.list` returns are the guards' current values
    let list = ok(&mut s, "vertical.list", json!({}));
    let read = (&list["wallsDigest"], &list["effectiveWallsDigest"], &list["dataDigest"], &list["partsDigest"], &list["regionsDigest"]);
    assert_eq!(read, (&json!(walls), &json!(effective), &json!(data), &json!(parts), &json!(regions)));
    assert_eq!(list["plans"][0]["digest"], json!(current));
    ok(&mut s, "vertical.update", json!({"plan": 1, "name": "Phone", "expect": current}));
}

#[test]
fn create_refuses_a_sequence_with_non_square_pixels() {
    let (mut s, seq) = session_with(1920, 1080, &FOUR, None);
    rebuild(&mut s, seq, |q| q.settings.par = (2, 1));
    expect_refusal(&mut s, "vertical.create", json!({}), "unsupported_pixel_aspect", json!({"par": [2, 1]}));
}

/// One refusal of a table: the calls that prepare [`framing`], the call, its code and the detail it holds.
type RefusedCase = (Setup, &'static str, Value, &'static str, Value);

/// Each command's refusals on [`framing`], with the detail that fixes the call.
fn refusals() -> Vec<RefusedCase> {
    let none = Vec::new;
    let zero = "fnv1a64:0000000000000000";
    let (sp, rt, rr, sa, st) = ("vertical.setPosition", "vertical.retimePosition", "vertical.replaceRange", "vertical.setArrival", "vertical.smoothThrough");
    let items: Vec<Value> = (0..2001).map(|k| json!({"frame": 200 + k % 100, "cx": 960, "cy": 540, "h": 1080})).collect();
    let twice = json!([{"frame": 250, "cx": 900, "cy": 540, "h": 1080}, {"frame": 250, "cx": 910, "cy": 540, "h": 1080}]);
    let fit_first = vec![(sp, json!({"plan": 1, "frame": 0, "layout": "fit"}))];
    let p01 = vec![("vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 0, "toFrame": 100}]}))];
    let keep_out = |rect: [i64; 4], mode: &str| json!({"add": [{"label": "logo", "fromFrame": 0, "toFrame": 400, "rect": rect, "mode": mode}]});
    let unmet = json!({"position": 3, "maxMoveFrames": 40, "alternative": {"arrival": "cut"}});
    vec![
        (none(), "vertical.create", json!({"name": "9:16"}), "plan_exists", json!({"id": 1})),
        (none(), "vertical.create", json!({"width": 1081, "height": 1920}), "invalid_output_size", json!({"width": 1081, "height": 1920})),
        (none(), "vertical.create", json!({"width": 1080}), "invalid_output_size", json!({"width": 1080, "height": null})),
        (none(), "vertical.create", json!({"width": 4320, "height": 7680}), "size_limit", json!({"minHeight": 1920.0, "maxHeight": 1080.0})),
        (none(), "vertical.create", json!({"preset": "16:9"}), "out_of_range", json!({"key": "preset"})),
        (none(), "vertical.create", json!({"filter": "lanczos"}), "out_of_range", json!({"key": "filter"})),
        (none(), "vertical.update", json!({"plan": 1, "width": 1080, "height": 1080}), "aspect_change", json!({"width": 1080, "height": 1080})),
        (
            none(),
            "vertical.update",
            json!({"plan": 1, "width": 2160, "height": 3840}),
            "size_limit",
            json!({"minHeight": 960.0, "maxHeight": 1080.0, "positions": [4]}),
        ),
        (lock(&[4]), "vertical.update", json!({"plan": 1, "width": 2160, "height": 3840, "clamp": true}), "locked", json!({"ids": [4]})),
        (none(), "vertical.update", json!({"plan": 1, "expect": zero}), "stale_plan", json!({})),
        (none(), "vertical.update", json!({"plan": 1, "name": ""}), "out_of_range", json!({"key": "name"})),
        (none(), "vertical.update", json!({"plan": 1, "captions": {"maxLines": 5}}), "out_of_range", json!({"key": "maxLines"})),
        (none(), "vertical.update", json!({"plan": 1, "startingWindow": {"cx": 100, "cy": 540, "h": 1080}}), "outside_picture", json!({})),
        (none(), "vertical.update", json!({"plan": 1, "fitBackground": "white"}), "out_of_range", json!({"key": "fitBackground"})),
        (none(), "vertical.delete", json!({"plan": 1, "expect": zero}), "stale_plan", json!({})),
        (none(), "vertical.delete", json!({"plan": 9}), "not_found", json!({"plan": 9})),
        (none(), sp, json!({"plan": 1}), "out_of_range", json!({"key": "frame"})),
        (none(), sp, json!({"plan": 1, "frame": 120, "arrival": "cut"}), "split_conflict", json!({})),
        (none(), sp, json!({"plan": 1, "frame": 250, "cx": 100}), "outside_picture", json!({"frame": 250, "cx": [303.75, 1616.25], "cy": [540.0, 540.0]})),
        (none(), sp, json!({"plan": 1, "frame": 250, "h": 300}), "size_limit", json!({"minHeight": 480.0, "maxHeight": 1080.0})),
        (lock(&[2]), sp, json!({"plan": 1, "frame": 100, "cx": 720}), "locked", json!({"ids": [2]})),
        (none(), sp, json!({"plan": 1, "frame": 250, "fromLabel": "Nobody"}), "label_not_found", json!({"label": "Nobody"})),
        (none(), sp, json!({"plan": 1, "frame": 250, "fromPosition": 99}), "not_found", json!({"position": 99})),
        (none(), sp, json!({"plan": 1, "frame": 250, "cx": 1000, "fromPosition": 2}), "out_of_range", json!({"key": "fromLabel"})),
        (none(), sp, json!({"plan": 1, "frame": 250, "arrival": "move", "moveFrames": 1}), "move_too_short", json!({"frames": 1, "minimum": 2})),
        (
            none(),
            sp,
            json!({"plan": 1, "frame": 250, "arrival": "move", "moveFrames": 80}),
            "move_overlaps",
            json!({"position": 7, "earliestStart": 200, "maxMoveFrames": 50}),
        ),
        (none(), sp, json!({"plan": 1, "frame": 250, "captionAnchor": "middle"}), "out_of_range", json!({"key": "captionAnchor"})),
        (none(), sp, json!({"plan": 1, "frame": 250, "color": "puce"}), "out_of_range", json!({"key": "color"})),
        (none(), sp, json!({"plan": 1, "frame": 250, "layout": "stretch"}), "out_of_range", json!({"key": "layout"})),
        (none(), sp, json!({"plan": 1, "frame": 250, "focus": [5000, 10]}), "outside_picture", json!({"key": "focus"})),
        (none(), sp, json!({"plan": 1, "frame": 250, "h": 1080, "zoom": 50}), "out_of_range", json!({"key": "zoom"})),
        (
            none(),
            sp,
            json!({"plan": 1, "frame": 250, "expectRange": {"fromFrame": 200, "toFrame": 300, "digest": zero}}),
            "stale_range",
            json!({"fromFrame": 200, "toFrame": 300}),
        ),
        (none(), sp, json!({"plan": 1, "frame": 250, "expectRange": {"fromFrame": 300, "toFrame": 200, "digest": zero}}), "out_of_range", json!({})),
        (none(), sp, json!({"plan": 1, "frame": 250, "expectRange": {"fromFrame": 200, "toFrame": 300}}), "out_of_range", json!({"key": "digest"})),
        (none(), rt, json!({"plan": 1, "position": 3, "frame": 190}), "would_reorder", json!({"position": 3, "frame": 190, "allowed": [101, 179]})),
        (none(), rt, json!({"plan": 1, "position": 4, "frame": 165}), "move_overlaps", json!({"position": 4, "earliestStart": 140, "maxMoveFrames": 25})),
        (none(), rt, json!({"plan": 1, "position": 99, "frame": 10}), "not_found", json!({"position": 99})),
        (none(), rt, json!({"plan": 1, "frame": 10}), "out_of_range", json!({"key": "position"})),
        (
            vec![("vertical.setWalls", json!({"add": [120], "onConflict": "shorten"}))],
            rt,
            json!({"plan": 1, "position": 3, "frame": 125}),
            "move_crosses_wall",
            json!({"position": 3, "wall": 120, "maxMoveFrames": 5}),
        ),
        (lock(&[2]), "vertical.deletePositions", json!({"plan": 1, "positions": [2]}), "locked", json!({"ids": [2]})),
        (fit_first, "vertical.deletePositions", json!({"plan": 1, "positions": [2]}), "layout_move", json!({"ids": [3]})),
        (
            none(),
            rr,
            json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": [{"frame": 310, "cx": 900, "cy": 540, "h": 1080}]}),
            "out_of_range",
            json!({"key": "frame", "index": 0}),
        ),
        (none(), rr, json!({"plan": 1, "fromFrame": 300, "toFrame": 200, "positions": []}), "out_of_range", json!({"key": "fromFrame"})),
        (none(), rr, json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": [], "expectWalls": zero}), "stale_walls", json!({})),
        (
            none(),
            rr,
            json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": [{"frame": 250, "cx": 900}]}),
            "out_of_range",
            json!({"key": "cx", "index": 0}),
        ),
        (none(), rr, json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": twice}), "position_exists", json!({"frame": 250})),
        (none(), rr, json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": items}), "too_many", json!({"key": "positions", "bound": 2000})),
        (
            none(),
            rr,
            json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": [{"frame": 250, "cx": 900, "bogus": 1}]}),
            "out_of_range",
            json!({"index": 0}),
        ),
        (none(), sa, json!({"plan": 1, "positions": [5], "arrival": "move", "moveFrames": 10}), "move_crosses_wall", json!({"position": 5, "wall": 200})),
        (
            none(),
            sa,
            json!({"plan": 1, "positions": [2], "arrival": "move", "moveFrames": 200}),
            "move_overlaps",
            json!({"position": 2, "earliestStart": 0, "maxMoveFrames": 100}),
        ),
        (none(), sa, json!({"plan": 1, "positions": [99], "arrival": "cut"}), "not_found", json!({"position": 99})),
        (none(), sa, json!({"plan": 1, "positions": [3], "arrival": "move", "moveFrames": 30, "ramp": [0.7, 0.6]}), "bad_curve", json!({"axis": "ramp"})),
        (none(), sa, json!({"plan": 1, "positions": [3]}), "out_of_range", json!({"key": "arrival"})),
        (none(), st, json!({"plan": 1, "positions": [4, 5, 6]}), "move_crosses_wall", json!({"wall": 200})),
        (none(), st, json!({"plan": 1, "positions": [2, 3]}), "out_of_range", json!({"count": 2, "minimum": 3})),
        (lock(&[3]), st, json!({"plan": 1, "positions": [2, 3, 4]}), "locked", json!({"ids": [3]})),
        (none(), "vertical.fitMove", json!({"plan": 1, "position": 3, "maxSpeed": 6.048}), "speed_limit_unmet", unmet),
        (none(), "vertical.fitMove", json!({"plan": 1, "position": 1, "maxSpeed": 10.0}), "out_of_range", json!({"key": "position"})),
        (none(), "vertical.fitMove", json!({"plan": 1, "position": 3, "maxSpeed": 0}), "bad_number", json!({"key": "maxSpeed"})),
        (none(), "vertical.fitMove", json!({"plan": 1, "position": 3}), "out_of_range", json!({"key": "maxSpeed"})),
        (none(), "vertical.swapWindows", json!({"plan": 1, "positions": [2]}), "out_of_range", json!({"key": "positions", "count": 1, "required": 2})),
        (none(), "vertical.swapWindows", json!({"plan": 1, "positions": [2, 99]}), "not_found", json!({"position": 99})),
        (none(), "vertical.setWalls", json!({"add": [{"frame": 120}]}), "would_split_move", json!({"plan": 1, "position": 3, "wall": 120})),
        (none(), "vertical.setWalls", json!({"add": [0]}), "bad_wall", json!({"frame": 0})),
        (none(), "vertical.setWalls", json!({"move": [{"from": 100, "to": 105}]}), "bad_wall", json!({"frame": 100})),
        (none(), "vertical.setWalls", json!({"move": [{"from": 150, "to": 155}]}), "not_found", json!({"frame": 150})),
        (none(), "vertical.setWalls", json!({"promote": [150]}), "not_found", json!({"candidate": 150})),
        (none(), "vertical.setWalls", json!({"remove": [100]}), "bad_wall", json!({"frame": 100})),
        (none(), "vertical.setWalls", json!({"onConflict": "maybe"}), "out_of_range", json!({"key": "onConflict"})),
        (none(), "vertical.setWalls", json!({"analysed": [[300, 200]]}), "bad_wall", json!({"range": [300, 200]})),
        (none(), "vertical.seedShots", json!({"plan": 1, "mode": "magic"}), "out_of_range", json!({"key": "mode"})),
        (none(), "vertical.seedShots", json!({"plan": 1}), "out_of_range", json!({"key": "mode"})),
        (none(), "vertical.seedShots", json!({"plan": 1, "mode": "carry", "fromFrame": 300, "toFrame": 100}), "out_of_range", json!({"key": "fromFrame"})),
        (p01, "vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 100, "toFrame": 300}]}), "part_exists", json!({"id": 1})),
        (none(), "vertical.setParts", json!({"add": [{"name": "", "fromFrame": 0, "toFrame": 100}]}), "bad_part", json!({"index": 0})),
        (none(), "vertical.setParts", json!({"add": [{"name": "P02", "fromFrame": 120, "toFrame": 300}]}), "would_split_move", json!({"plan": 1, "wall": 120})),
        (none(), "vertical.setParts", json!({"update": [{"part": 9, "name": "P09"}]}), "not_found", json!({"part": 9})),
        (none(), "vertical.setRegions", keep_out([1900, 40, 160, 90], "keepOut"), "outside_picture", json!({"key": "rect", "index": 0})),
        (none(), "vertical.setRegions", keep_out([10, 10, 0, 90], "keepIn"), "bad_number", json!({"key": "rect", "index": 0})),
        (none(), "vertical.setRegions", keep_out([10, 10, 40, 90], "hide"), "out_of_range", json!({"key": "mode"})),
        (none(), "vertical.setRegions", json!({"update": [{"region": 9, "label": "x"}]}), "not_found", json!({"region": 9})),
        (none(), "vertical.markChecked", json!({"plan": 1, "positions": [99]}), "not_found", json!({"position": 99})),
        (none(), "vertical.repair", json!({"plan": 9}), "not_found", json!({"plan": 9})),
        (none(), "vertical.setAnnotations", json!({"add": [{"kind": "Speech!", "fromFrame": 0, "toFrame": 10}]}), "bad_annotation", json!({"index": 0})),
        (none(), "vertical.setAnnotations", json!({"update": [{"id": 9, "label": "x"}]}), "not_found", json!({"annotation": 9})),
        (none(), "vertical.setActiveArea", json!({"set": [{"fromFrame": 0, "toFrame": 400, "rect": [0, 0, 2000, 1080]}]}), "bad_area", json!({"index": 0})),
        (none(), "vertical.setActiveArea", json!({"set": [{"fromFrame": 300, "toFrame": 100, "rect": [0, 0, 100, 100]}]}), "bad_area", json!({"index": 0})),
        (with_part(), "vertical.approvePart", json!({"plan": 1, "part": 1}), "not_reviewed", json!({"part": 1, "shots": [2, 3]})),
        (none(), "vertical.approvePart", json!({"plan": 1, "part": 9}), "not_found", json!({"part": 9})),
        (none(), "vertical.setReview", json!({"plan": 1, "positions": [99], "reviewed": true}), "not_found", json!({"position": 99})),
        (none(), "vertical.setReview", json!({"plan": 1, "positions": [2]}), "out_of_range", json!({"key": "reviewed"})),
        (none(), "vertical.setLock", json!({"plan": 1, "positions": [99], "locked": true}), "not_found", json!({"position": 99})),
        (none(), "vertical.flagAttention", json!({"plan": 1, "frame": 400}), "out_of_range", json!({"key": "frame"})),
        (none(), "vertical.flagAttention", json!({"plan": 1, "frame": 10, "note": "n".repeat(1001)}), "too_many", json!({"key": "note"})),
        (none(), "vertical.clearAttention", json!({"plan": 1, "frames": (0..10_001).collect::<Vec<i64>>()}), "too_many", json!({"key": "frames"})),
        (none(), "vertical.noteWatched", json!({"plan": 1, "shot": 9, "frames": [[0, 10]]}), "not_found", json!({"shot": 9})),
        (none(), "vertical.noteWatched", json!({"plan": 1, "shot": 2, "frames": [[50, 60]]}), "out_of_range", json!({"key": "frames"})),
        (none(), "vertical.select", json!({"plan": 9}), "not_found", json!({"plan": 9})),
        (none(), "vertical.select", json!({"plan": 1, "positions": [99]}), "not_found", json!({"position": 99})),
        (none(), "vertical.select", json!({"plan": 1, "walls": [150]}), "bad_wall", json!({"frame": 150})),
        (none(), "vertical.goTo", json!({"plan": 1, "to": "sideways"}), "out_of_range", json!({"key": "to"})),
        (none(), "vertical.goTo", json!({"to": "nextPosition"}), "out_of_range", json!({"key": "plan"})),
        (none(), "vertical.windows", json!({"plan": 1, "fromFrame": 10, "toFrame": 5}), "out_of_range", json!({"key": "fromFrame"})),
        (none(), "vertical.windows", json!({"plan": 1, "fromFrame": 0, "toFrame": 500}), "out_of_range", json!({"key": "toFrame"})),
        (none(), "vertical.windows", json!({"plan": 1, "fromFrame": 0, "toFrame": 10, "step": 0}), "out_of_range", json!({"key": "step"})),
        (none(), "vertical.windows", json!({"plan": 1, "fromFrame": 0, "toFrame": 10, "fields": ["x", "bogus"]}), "out_of_range", json!({"key": "fields"})),
        (none(), "vertical.measure", json!({"plan": 1, "thresholds": {"lookRoomRatio": 0.3}}), "out_of_range", json!({"key": "lookRoomRatio"})),
        (none(), "vertical.measure", json!({"plan": 1, "regions": [{"space": "media", "id": "a", "boxes": []}]}), "out_of_range", json!({"key": "space"})),
        (none(), "vertical.measure", json!({"plan": 1, "regions": [{"source": "boxTracks"}]}), "out_of_range", json!({"key": "source"})),
        (none(), "vertical.measure", json!({"plan": 1, "safeZone": "tiktok"}), "not_found", json!({"safeZone": "tiktok"})),
        (none(), "vertical.measure", json!({"plan": 1, "baseline": "zoomed"}), "out_of_range", json!({"key": "baseline"})),
        (none(), "vertical.fit", json!({"plan": 1, "frame": 250}), "out_of_range", json!({"key": "regions"})),
        (none(), "vertical.fit", json!({"plan": 1, "frame": 250, "regions": [{"source": "captions"}]}), "out_of_range", json!({"key": "source"})),
        (none(), "vertical.storyboard", json!({"plan": 1, "part": 9}), "not_found", json!({"part": 9})),
        (none(), "vertical.measureCuts", json!({"fromFrame": 300, "toFrame": 100}), "out_of_range", json!({"key": "fromFrame"})),
        (none(), "vertical.measureCuts", json!({"sensitivity": 101}), "out_of_range", json!({"key": "sensitivity"})),
        (none(), "vertical.measureCuts", json!({"minShotFrames": 0}), "out_of_range", json!({"key": "minShotFrames"})),
        (none(), "vertical.measureCuts", json!({"minShotFrames": 1}), "out_of_range", json!({"key": "minShotFrames", "minimum": 2})),
        (none(), "vertical.measureCuts", json!({"candidateFloor": 2.0}), "out_of_range", json!({"key": "candidateFloor"})),
        (none(), "vertical.jobResult", json!({"job": 999}), "not_found", json!({"job": 999})),
        (none(), "vertical.jobResult", json!({}), "out_of_range", json!({"key": "job"})),
        (none(), "vertical.applyMeasurement", json!({"measurement": "m999"}), "not_found", json!({"measurement": "m999"})),
        (none(), "vertical.applyMeasurement", json!({}), "out_of_range", json!({"key": "measurement"})),
    ]
}

#[test]
fn every_command_refuses_with_its_code_and_detail() {
    for (setup, id, p, code, detail) in refusals() {
        let (mut s, _) = framing();
        for (sid, sp) in setup {
            ok(&mut s, sid, sp);
        }
        expect_refusal(&mut s, id, p, code, detail);
    }
}

/// One case per write of plan 63 §3.2 but `applyMeasurement` and `repair` (run twice in their own tests), each changing
/// something on [`framing`] the first time; `swapWindows` carries the guard its retry safety needs, and `remap` its
/// required one (lane L2d), so the retry of either is refused `stale_plan`.
fn retried_writes() -> Vec<Case> {
    let (digest, remapped) = {
        let (s, seq) = framing();
        (plan_digest(&s, seq, 1), v::remap_digest(s.project.sequence(seq).expect("the sequence")))
    };
    let map = json!([{"fromFrame": 0, "toFrame": 100, "newFrom": 0}, {"fromFrame": 110, "toFrame": 400, "newFrom": 100}]);
    let moved_wall = vec![("vertical.setWalls", json!({"add": [250]}))];
    let region = json!({"label": "logo", "fromFrame": 0, "toFrame": 400, "rect": [1700, 40, 160, 90], "mode": "keepOut"});
    vec![
        (Vec::new(), "vertical.create", json!({"name": "Second"})),
        (Vec::new(), "vertical.update", json!({"plan": 1, "name": "Phone"})),
        (Vec::new(), "vertical.delete", json!({"plan": 1})),
        (Vec::new(), "vertical.setPosition", json!({"plan": 1, "frame": 250, "cx": 1100})),
        (Vec::new(), "vertical.retimePosition", json!({"plan": 1, "position": 6, "frame": 310})),
        (Vec::new(), "vertical.deletePositions", json!({"plan": 1, "positions": [6]})),
        (
            Vec::new(),
            "vertical.replaceRange",
            json!({"plan": 1, "fromFrame": 200, "toFrame": 300, "positions": [{"frame": 200, "cx": 990, "cy": 540, "h": 1080}]}),
        ),
        (Vec::new(), "vertical.setArrival", json!({"plan": 1, "positions": [3], "arrival": "move", "moveFrames": 20, "curve": "linear"})),
        (Vec::new(), "vertical.smoothThrough", json!({"plan": 1, "positions": [2, 3, 4]})),
        (Vec::new(), "vertical.fitMove", json!({"plan": 1, "position": 3, "maxSpeed": 20.0})),
        (Vec::new(), "vertical.swapWindows", json!({"plan": 1, "positions": [5, 6], "expect": digest})),
        (Vec::new(), "vertical.setWalls", json!({"add": [250]})),
        (moved_wall, "vertical.setWalls", json!({"move": [{"from": 250, "to": 260}]})),
        (carried(), "vertical.seedShots", json!({"plan": 1, "mode": "carry"})),
        (Vec::new(), "vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 100, "toFrame": 300}]})),
        (Vec::new(), "vertical.setRegions", json!({"add": [region]})),
        (shortened(), "vertical.markChecked", json!({"plan": 1, "positions": [3]})),
        (Vec::new(), "vertical.setAnnotations", json!({"add": [{"kind": "speech", "fromFrame": 0, "toFrame": 100, "label": "char:dana"}]})),
        (Vec::new(), "vertical.setActiveArea", json!({"set": [{"fromFrame": 0, "toFrame": 400, "rect": [0, 140, 1920, 800]}]})),
        (reviewed_part(), "vertical.approvePart", json!({"plan": 1, "part": 1, "note": "ok"})),
        (Vec::new(), "vertical.setReview", json!({"plan": 1, "positions": [2], "reviewed": true})),
        (Vec::new(), "vertical.setLock", json!({"plan": 1, "positions": [2], "locked": true})),
        (Vec::new(), "vertical.flagAttention", json!({"plan": 1, "frame": 150, "note": "look"})),
        (flagged(), "vertical.clearAttention", json!({"plan": 1, "frames": [150]})),
        (Vec::new(), "vertical.remap", json!({"map": map, "expect": remapped})),
    ]
}

#[test]
fn t_n15_every_write_run_twice_changes_once() {
    // a deleted plan is not found again: the retry changes nothing, as every guard refusal
    let guards = ["stale_plan", "stale_range", "plan_exists", "part_exists", "not_found"];
    for (setup, id, p) in retried_writes() {
        let mut s = prepared(&setup);
        let first = ok(&mut s, id, p.clone());
        assert_eq!(first["changed"], json!(true), "`{id}` changes something the first time: {first}");
        let before = Snapshot::of(&s);
        match s.execute(id, p.clone()) {
            Ok(again) => assert_eq!(again["changed"], json!(false), "`{id}` run again: {again}"),
            Err(EngineError::Invalid { code, .. }) => assert!(guards.contains(&code), "`{id}` run again: {code}"),
            Err(e) => panic!("`{id}` run again: {e}"),
        }
        before.unchanged(&s, id);
        if id.ends_with("Parts") || id.ends_with("Regions") || id.ends_with("Annotations") {
            let again = ok(&mut s, id, p);
            assert_eq!(again["ids"], first["ids"], "`{id}`: an identical add returns the existing id");
        }
    }
}

// ---------- ids, approvals, annotations and areas ----------

#[test]
fn t_m13_an_id_is_never_handed_out_twice() {
    let (mut s, seq) = framing();
    let r = ok(&mut s, "vertical.create", json!({"name": "Second"}));
    assert_eq!(r["plan"], json!(2));
    s.undo();
    let r = ok(&mut s, "vertical.create", json!({"name": "Third"}));
    assert_eq!(r["plan"], json!(3), "undo rolled the counter back; the session's mark did not");
    let digest = plan_digest(&s, seq, 1);
    let r = ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 250, "cx": 1100}));
    assert_eq!(r["added"], json!([7]));
    let range = range_digest(&s, seq, 1, 200, 300);
    s.undo();
    let r = ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 260, "cx": 1100}));
    assert_eq!(r["added"], json!([8]));
    // a caller still holding id 7 from before the undo is stopped by its guard, never sent to another position
    let p = json!({"plan": 1, "frame": 250, "cx": 1150, "expectRange": {"fromFrame": 200, "toFrame": 300, "digest": range}});
    let (code, _) = refused(&mut s, "vertical.setPosition", p);
    assert_eq!(code, "stale_range");
    let (code, _) = refused(&mut s, "vertical.deletePositions", json!({"plan": 1, "positions": [7], "expect": digest}));
    assert_eq!(code, "stale_plan");
    // save and open keep the high-water marks
    let dir = crate::media_test_util::tmp_dir("vertical-ids");
    let path = dir.join("ids.fcproj").to_string_lossy().to_string();
    ok(&mut s, "file.saveAs", json!({"path": path}));
    ok(&mut s, "file.open", json!({"path": path}));
    let seq = s.state.active_sequence.expect("the opened project's sequence");
    let r = ok(&mut s, "vertical.create", json!({"sequence": seq.0, "name": "Fourth"}));
    assert_eq!(r["plan"], json!(4));
    let r = ok(&mut s, "vertical.setPosition", json!({"sequence": seq.0, "plan": 1, "frame": 270, "cx": 1100}));
    assert_eq!(r["added"], json!([9]));
}

#[test]
fn a_set_created_again_after_its_undo_equals_the_first_but_for_its_ids() {
    // the first create plans on the planner's own empty set (the sequence has no set and no marks); once it is undone, the
    // working copy holds the engine's `empty_vertical_set` raised to the marks (lane L2a): both give the same set but ids
    let (mut s, seq) = session_with(1920, 1080, &FOUR, None);
    ok(&mut s, "vertical.create", json!({"name": "9:16"}));
    let first = set_of(&s, seq).clone();
    s.undo();
    assert!(s.project.sequence(seq).expect("the sequence").vertical.is_none(), "the undo took the set away");
    let r = ok(&mut s, "vertical.create", json!({"name": "9:16"}));
    assert_eq!(r["plan"], json!(2), "the undone plan's id is not handed out again");
    let mut again = set_of(&s, seq).clone();
    again.plans[0].id = first.plans[0].id;
    again.next_plan_id = first.next_plan_id;
    assert_eq!(again, first);
}

#[test]
fn t_n2_approve_part_records_who_when_the_note_and_both_digests() {
    let (mut s, seq) = framing();
    s.clock = Clock::fixed(1_793_000_000);
    s.prefs.vertical.editor_name = "Dana Levi".into();
    let r = ok(&mut s, "vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 100, "toFrame": 300}]}));
    assert_eq!(r["ids"], json!([1]));
    let status = |s: &mut Session| ok(s, "vertical.list", json!({}))["plans"][0]["parts"][0]["status"].clone();
    assert_eq!(status(&mut s), json!("open"));
    ok(&mut s, "vertical.setReview", json!({"plan": 1, "shotFrame": 100, "reviewed": true}));
    ok(&mut s, "vertical.setReview", json!({"plan": 1, "shotFrame": 200, "reviewed": true}));
    assert_eq!(status(&mut s), json!("reviewed"));
    let r = ok(&mut s, "vertical.approvePart", json!({"plan": 1, "part": 1, "note": "ready for air"}));
    assert_eq!(r["label"], json!("Vertical: approve part P01"));
    let plan = plan_in(&s, seq, 1);
    let approval = plan.part_approvals.first().expect("the approval is stored");
    assert_eq!((approval.by.as_str(), approval.when, approval.note.as_str()), ("Dana Levi", 1_793_000_000, "ready for air"));
    let q = s.project.sequence(seq).expect("the sequence");
    let part = set_of(&s, seq).parts.first().expect("the part");
    assert_eq!(approval.framing_digest, v::range_digest(&s.project, q, plan, 100, 300));
    assert_eq!(approval.output_digest, v::output_digest(&s.project, q, plan, part));
    assert_eq!(status(&mut s), json!("approved"));
    let inspected = ok(&mut s, "vertical.inspect", json!({"plan": 1}));
    contains(&inspected["parts"][0], &json!({"id": 1, "name": "P01", "status": "approved"}), "the part");
    contains(&inspected["parts"][0]["approval"], &json!({"by": "Dana Levi", "when": 1_793_000_000, "note": "ready for air"}), "the approval");
    // a framing change inside the part, and its undo (T-M27)
    ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 250, "cx": 1100}));
    assert_eq!(status(&mut s), json!("changedSinceApproval"));
    ok(&mut s, "edit.undo", json!({}));
    assert_eq!(status(&mut s), json!("approved"), "undo restores the approval");
    // a change of the output alone
    ok(&mut s, "vertical.update", json!({"plan": 1, "captions": {"maxLines": 3}}));
    assert_eq!(status(&mut s), json!("outputChangedSinceApproval"));
}

#[test]
fn t_n14_annotations_and_active_areas_refuse_retry_and_dry_run_under_the_data_digest() {
    let (mut s, seq) = framing();
    let data = |s: &Session| v::data_digest(set_of(s, seq));
    let zero = "fnv1a64:0000000000000000";
    let add = json!({"add": [{"kind": "speech", "fromFrame": 0, "toFrame": 100, "label": "char:dana", "score": 0.9}]});
    let r = ok(&mut s, "vertical.setAnnotations", add.clone());
    assert_eq!((&r["changed"], &r["ids"]), (&json!(true), &json!([1])));
    let before = Snapshot::of(&s);
    let r = ok(&mut s, "vertical.setAnnotations", add);
    assert_eq!((&r["changed"], &r["ids"]), (&json!(false), &json!([1])), "an identical add is its own retry");
    before.unchanged(&s, "a retried add");
    // a dry run reports the id it would hand out and keeps nothing
    let dry = json!({"add": [{"kind": "laughter", "fromFrame": 120, "toFrame": 140}], "dryRun": true});
    let r = ok(&mut s, "vertical.setAnnotations", dry);
    assert_eq!((&r["changed"], &r["ids"], &r["applied"]), (&json!(true), &json!([2]), &json!(false)));
    before.unchanged(&s, "a dry run");
    // `expect` is the data digest
    let current = data(&s);
    expect_refusal(&mut s, "vertical.setAnnotations", json!({"remove": [1], "expect": zero}), "stale_plan", json!({"digest": current}));
    ok(&mut s, "vertical.setAnnotations", json!({"update": [{"id": 1, "label": "char:noa"}], "expect": current}));
    let nan = json!({"add": [{"kind": "speech", "fromFrame": 0, "toFrame": 10, "score": "NaN"}]});
    expect_refusal(&mut s, "vertical.setAnnotations", nan, "bad_number", json!({"key": "score"}));
    let outside = json!({"replace": {"kind": "speech", "fromFrame": 0, "toFrame": 100, "set": [{"fromFrame": 50, "toFrame": 150}]}});
    expect_refusal(&mut s, "vertical.setAnnotations", outside, "bad_annotation", json!({"index": 0}));
    let replace = json!({"replace": {"kind": "speech", "fromFrame": 0, "toFrame": 400, "set": [{"fromFrame": 10, "toFrame": 60, "label": "char:dana"}]}});
    let r = ok(&mut s, "vertical.setAnnotations", replace);
    assert_eq!(r["label"], json!("Vertical: annotations speech 00:00:00:00\u{2013}00:00:16:00"));
    // the active area: set, retry, read, refuse, clear; it is in the data digest
    let area = json!({"set": [{"fromFrame": 0, "toFrame": 400, "rect": [0, 140, 1920, 800]}]});
    let before_area = data(&s);
    let r = ok(&mut s, "vertical.setActiveArea", area.clone());
    assert_eq!((&r["changed"], &r["label"]), (&json!(true), &json!("Vertical: active area")));
    assert_ne!(data(&s), before_area);
    let before = Snapshot::of(&s);
    let r = ok(&mut s, "vertical.setActiveArea", area);
    assert_eq!(r["changed"], json!(false));
    before.unchanged(&s, "a retried area");
    let windows = ok(&mut s, "vertical.windows", json!({"plan": 1, "fromFrame": 10, "toFrame": 11, "fields": ["active"]}));
    assert_eq!(windows["active"], json!([[0.0, 140.0, 1920.0, 800.0]]));
    let current = data(&s);
    expect_refusal(&mut s, "vertical.setActiveArea", json!({"clear": [[0, 400]], "expect": zero}), "stale_plan", json!({"digest": current}));
    ok(&mut s, "vertical.setActiveArea", json!({"clear": [[0, 400]], "expect": current}));
    assert!(set_of(&s, seq).active_area.is_empty());
}

// ---------- reads ----------

#[test]
fn list_reports_plans_parts_and_the_digests() {
    let (mut s, seq) = framing();
    ok(&mut s, "vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 100, "toFrame": 300}]}));
    let r = ok(&mut s, "vertical.list", json!({}));
    assert_eq!(r["sequence"], json!(seq.0));
    let plan = &r["plans"][0];
    let want = json!({"id": 1, "name": "9:16", "output": {"width": 1080, "height": 1920}, "aspect": "9:16", "positions": 6, "valid": true});
    contains(plan, &want, "plan 1");
    assert_eq!(plan["digest"], json!(plan_digest(&s, seq, 1)));
    assert_eq!(plan["issues"]["error"], json!(0), "{plan}");
    assert_eq!(plan["parts"], json!([{"part": 1, "status": "open"}]));
    assert_eq!(r["parts"], json!([{"id": 1, "name": "P01", "fromFrame": 100, "toFrame": 300}]));
    // a sequence without vertical data lists no plan, with the digests of nothing
    let (mut t, _) = measuring();
    let r = ok(&mut t, "vertical.list", json!({}));
    let empty = crate::empty_vertical_set();
    assert_eq!((&r["plans"], &r["wallsDigest"], &r["dataDigest"]), (&json!([]), &json!(v::walls_digest(&empty)), &json!(v::data_digest(&empty))));
}

#[test]
fn inspect_reports_the_plan_its_shots_and_positions_with_their_derived_values() {
    let (mut s, seq) = framing();
    let r = ok(&mut s, "vertical.inspect", json!({"plan": 1}));
    let plan = json!({"plan": 1, "name": "9:16", "output": {"width": 1080, "height": 1920}, "aspect": "9:16", "filter": "catmullRom"});
    contains(&r, &plan, "inspect");
    contains(&r, &json!({"fitBackground": "blur", "startingWindow": null, "limits": {"minHeight": 480.0, "maxHeight": 1080.0}}), "inspect");
    assert_eq!(r["digest"], json!(plan_digest(&s, seq, 1)));
    let walls: Vec<i64> = r["walls"].as_array().expect("walls").iter().filter_map(|w| w["frame"].as_i64()).collect();
    assert_eq!(walls, [100, 200, 300]);
    assert_eq!(r["walls"][0]["sources"], json!(["edit"]));
    let positions = r["positions"].as_array().expect("positions");
    assert_eq!(positions.len(), 6);
    let third = &positions[2];
    let placed = json!({"id": 3, "frame": 140, "tick": "1422489600000", "timecode": "00:00:05:15", "cx": 900.0, "cy": 540.0, "h": 1000.0});
    contains(third, &placed, "position #3");
    let arrival = json!({"layout": "fill", "arrival": "move", "moveFrames": 30, "curve": "easeInOut", "tag": "MOVE", "shot": 2});
    contains(third, &arrival, "position #3");
    let derived = json!({"moving": [111, 140], "static": [140, 151], "holdFrames": 11, "reviewed": false, "locked": false});
    contains(third, &derived, "position #3");
    contains(third, &json!({"origin": "manual", "modifiedBy": "human", "upscale": 1.92}), "position #3");
    assert!(third["peakSpeed"].as_f64().is_some_and(|speed| speed > 0.0), "{third}");
    assert_eq!((&positions[3]["curve"], &positions[3]["ramp"]), (&json!("ramp"), &json!([0.2, 0.2])));
    let shot = json!({"number": 2, "fromFrame": 100, "toFrame": 200, "status": "own", "opener": 2, "positions": [2, 3, 4], "reviewed": 0});
    contains(&r["shots"][1], &shot, "shot 2");
    contains(&r["shots"][1], &json!({"watched": false, "watch": null, "attention": []}), "shot 2");
    let session = (&r["library"], &r["unanchoredSinceMeasurement"], &r["shotsUnknown"]);
    assert_eq!(session, (&json!([]), &json!([]), &json!([])));
    assert!(r["issues"].is_array());
    // the stored set as stored: the fields agents write are there, empty in manual use
    let stored = json!({"boxTracks": [], "agentOverwrite": null, "overwritten": [], "partAudio": {"fadeInFrames": 2, "fadeOutFrames": 4}});
    contains(&r, &stored, "inspect");
    // a file written by a later build, with a box track: its summary, without its keys
    rebuild(&mut s, seq, |q| {
        let key = |frame: i64| v::KeyBox { at: at(frame), rect: [100.0, 100.0, 200.0, 300.0], facing: None, gone: false };
        let track = v::BoxTrack {
            id: "char:dana".into(),
            label: "Dana".into(),
            kind: "face".into(),
            keys: vec![key(0), key(50)],
            origin: Origin::Agent,
            modified_by: Writer::Agent,
        };
        q.vertical.as_deref_mut().expect("the set").box_tracks.push(track);
    });
    let r = ok(&mut s, "vertical.inspect", json!({"plan": 1}));
    let track = json!({"id": "char:dana", "label": "Dana", "kind": "face", "keys": 2, "origin": "agent", "modifiedBy": "agent"});
    assert_eq!(r["boxTracks"], json!([track]));
    // a range narrows the positions and shots
    let r = ok(&mut s, "vertical.inspect", json!({"plan": 1, "fromFrame": 100, "toFrame": 200}));
    let ids: Vec<u64> = r["positions"].as_array().expect("positions").iter().filter_map(|p| p["id"].as_u64()).collect();
    assert_eq!(ids, [2, 3, 4]);
    let shots: Vec<u64> = r["shots"].as_array().expect("shots").iter().filter_map(|p| p["number"].as_u64()).collect();
    assert_eq!(shots, [2]);
}

#[test]
fn windows_returns_the_exact_window_of_every_frame_in_columns() {
    let (mut s, seq) = framing();
    let r = ok(&mut s, "vertical.windows", json!({"plan": 1, "fromFrame": 105, "toFrame": 150, "step": 5}));
    let rows = v::frame_rows(&s.project, s.project.sequence(seq).expect("the sequence"), 1, 105, 150, 5);
    assert_eq!(rows.len(), 9);
    assert_eq!((&r["fromFrame"], &r["toFrame"], &r["step"], &r["plan"]), (&json!(105), &json!(150), &json!(5), &json!(1)));
    let column = |f: fn(&v::FrameRow) -> Value| Value::Array(rows.iter().map(f).collect());
    assert_eq!(r["frame"], column(|row| json!(row.frame)));
    assert_eq!(r["tick"], column(|row| json!(row.tick.0.to_string())));
    assert_eq!(r["x"], column(|row| json!(row.rect.x)));
    assert_eq!(r["y"], column(|row| json!(row.rect.y)));
    assert_eq!(r["w"], column(|row| json!(row.rect.w)));
    assert_eq!(r["h"], column(|row| json!(row.rect.h)));
    assert_eq!(r["cx"], column(|row| json!(row.window.cx)));
    assert_eq!(r["cy"], column(|row| json!(row.window.cy)));
    assert_eq!(r["position"], column(|row| json!(row.position)));
    assert_eq!(r["u"], column(|row| json!(row.u)));
    assert_eq!(r["e"], column(|row| json!(row.e)));
    assert_eq!(r["speed"], column(|row| json!(row.speed)));
    assert_eq!(r["upscale"], column(|row| json!(row.upscale)));
    let states: String = rows.iter().map(|row| row.state).collect();
    assert_eq!((&r["state"], &r["layout"]), (&json!(states), &json!("fffffffff")), "`f` is fill, `F` fit");
    assert!(r.get("focus").is_none() && r.get("active").is_none(), "focus and active only when named");
    // `fields` picks the columns; the frames are always there
    let r = ok(&mut s, "vertical.windows", json!({"plan": 1, "fromFrame": 105, "toFrame": 110, "fields": ["cx", "focus", "active"]}));
    let mut keys: Vec<&str> = r.as_object().expect("an object").keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["active", "cx", "digest", "focus", "frame", "fromFrame", "plan", "sequence", "step", "toFrame"]);
    assert_eq!(r["active"][0], json!([0.0, 0.0, 1920.0, 1080.0]), "without an active area, the frame");
}

#[test]
fn windows_reads_at_most_two_million_numbers_in_one_reply() {
    // a 400 000-frame sequence (4 h 26 min at 25p): its rows are refused before any is built
    let (mut s, _) = session_with(1920, 1080, &[([104, 104, 104], 400_000)], Some(set_with(vec![cut(1, 0, 960.0, 540.0, 1080.0)])));
    let all = ["tick", "x", "y", "w", "h", "cx", "cy", "layout", "state", "position", "u", "e", "speed", "upscale", "focus", "active"];
    let every = json!({"plan": 1, "fromFrame": 0, "toFrame": 400_000, "fields": all});
    // a row: the frame, 14 one-number fields, a focus point (2) and an active rect (4)
    expect_refusal(&mut s, "vertical.windows", every, "too_many", json!({"key": "frames", "count": 400_000, "bound": 95_238}));
    let default = json!({"plan": 1, "fromFrame": 0, "toFrame": 400_000});
    expect_refusal(&mut s, "vertical.windows", default, "too_many", json!({"key": "frames", "count": 400_000, "bound": 133_333}));
    // a step reads fewer rows
    let r = ok(&mut s, "vertical.windows", json!({"plan": 1, "fromFrame": 0, "toFrame": 400_000, "step": 400, "fields": ["cx"]}));
    assert_eq!(r["frame"].as_array().map(Vec::len), Some(1000));
}

/// T-M6's fixed plan on one 300-frame clip: every curve, zooms in and out, a pair of exact split pieces, a fit position, a
/// focus point and an active area.
fn golden_set() -> VerticalSet {
    let piece = |offset: u32| Arrival::Move { move_frames: 15, curve: Curve::Ramp { r_in: 0.3, r_out: 0.3 }, split_of: Some(SplitOf { offset, total: 30 }) };
    let mut eased = moved(5, 120, (700.0, 600.0, 800.0), 25, Curve::EaseInOut);
    eased.focus = Some([700.0, 600.0]);
    let mut fit = cut(11, 260, 960.0, 540.0, 1080.0);
    fit.layout = Layout::Fit;
    let positions = vec![
        cut(1, 0, 960.0, 540.0, 1080.0),
        moved(2, 30, (800.0, 540.0, 1080.0), 20, Curve::Linear),
        moved(3, 60, (1000.0, 500.0, 900.0), 25, Curve::EaseIn),
        moved(4, 90, (1100.0, 540.0, 1000.0), 25, Curve::EaseOut),
        eased,
        moved(6, 150, (900.0, 540.0, 1080.0), 25, Curve::Speeds { start: 0.5, end: 1.5 }),
        moved(7, 180, (1300.0, 540.0, 960.0), 25, Curve::Bezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 }),
        moved(8, 210, (1000.0, 540.0, 700.0), 25, v::DEFAULT_CURVE),
        Position::new(9, at(225), Window { cx: 900.0, cy: 540.0, h: 900.0 }, piece(0), Origin::Manual, Writer::Human),
        Position::new(10, at(240), Window { cx: 800.0, cy: 540.0, h: 1080.0 }, piece(15), Origin::Manual, Writer::Human),
        fit,
        cut(12, 280, 960.0, 540.0, 1080.0),
    ];
    let mut set = set_with(positions);
    let area = ActiveArea { from: at(100), to: at(200), rect: [0.0, 140.0, 1920.0, 800.0], origin: Origin::Manual, modified_by: Writer::Human };
    set.active_area.push(area);
    set
}

#[test]
fn t_m6_the_windows_of_a_fixed_plan_equal_the_stored_golden_bit_for_bit() {
    let (mut s, seq) = session_with(1920, 1080, &[([104, 104, 104], 300)], Some(golden_set()));
    let fields = ["tick", "x", "y", "w", "h", "cx", "cy", "layout", "state", "position", "u", "e", "speed", "upscale", "focus", "active"];
    let mut r = ok(&mut s, "vertical.windows", json!({"plan": 1, "fromFrame": 0, "toFrame": 300, "fields": fields}));
    assert_eq!(r["digest"], json!(plan_digest(&s, seq, 1)));
    // the digest and the sequence id are pinned elsewhere; the golden holds the windows
    let object = r.as_object_mut().expect("windows returns an object");
    object.remove("digest");
    object.remove("sequence");
    let actual = serde_json::to_string_pretty(&r).expect("JSON") + "\n";
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let golden = manifest.join("tests/goldens/vertical_windows.json");
    let stored = std::fs::read_to_string(&golden).map(|text| text.replace('\r', ""));
    if stored.as_deref().ok() != Some(actual.as_str()) {
        let out = manifest.join("../../target/vertical-golden");
        std::fs::create_dir_all(&out).expect("the golden output folder");
        let written = out.join("windows.actual.json");
        std::fs::write(&written, actual.as_bytes()).expect("the actual windows");
        panic!("vertical.windows differs from {} (or it is missing); the actual JSON is {}", golden.display(), written.display());
    }
}

#[test]
fn measure_reports_the_model_measures_and_findings_only_for_passed_thresholds() {
    let (mut s, seq) = framing();
    let oracle = |s: &Session, opts: &v::MeasureOpts| {
        let measures = v::measure(&s.project, s.project.sequence(seq).expect("the sequence"), 1, opts).expect("the measures");
        serde_json::to_value(measures).expect("JSON")
    };
    let plain = ok(&mut s, "vertical.measure", json!({"plan": 1}));
    let want = oracle(&s, &v::MeasureOpts::default());
    for key in ["moves", "junctions", "jumps", "breaks", "shots", "labels", "parts", "regions", "totals", "findings"] {
        assert_eq!(plain[key], want[key], "{key}");
    }
    assert_eq!(plain["moves"].as_array().map(Vec::len), Some(2));
    assert_eq!(plain["findings"], json!([]), "no threshold, no finding");
    let fast = ok(&mut s, "vertical.measure", json!({"plan": 1, "thresholds": {"maxSpeed": 1.0}}));
    let thresholds = v::Thresholds { max_speed: Some(1.0), ..Default::default() };
    assert_eq!(fast["findings"], oracle(&s, &v::MeasureOpts { thresholds, ..Default::default() })["findings"]);
    assert!(fast["findings"].as_array().expect("findings").iter().any(|f| f["code"] == json!("speed_over")));
    // an inline region in sequence px, the protected regions and the caption boxes, against the union safe zone
    let logo = json!({"label": "logo", "fromFrame": 0, "toFrame": 400, "rect": [1700, 40, 160, 90], "mode": "keepOut"});
    ok(&mut s, "vertical.setRegions", json!({"add": [logo]}));
    ok(&mut s, "captions.add", json!({"text": "Hello there", "time": at(210).0, "durationSeconds": 2.0}));
    let boxes = json!([{"frame": 0, "rect": [800, 1000, 400, 80]}, {"frame": 399, "rect": [800, 1000, 400, 80]}]);
    let regions = json!([{"id": "lower third", "boxes": boxes, "interpolate": true}, {"source": "protected"}, {"source": "captions"}]);
    let r = ok(&mut s, "vertical.measure", json!({"plan": 1, "regions": regions, "safeZone": "union"}));
    let q = s.project.sequence(seq).expect("the sequence");
    let range = TimeRange::from_bounds(Tick::ZERO, at(400));
    let caption = |b: &filmcraft_captions::burn::CaptionBox| v::CaptionBox {
        cue: b.cue.0,
        from_frame: v::shown_frame(RATE, b.from),
        to_frame: v::shown_frame(RATE, b.to),
        rect: b.rect.map(f64::from),
    };
    let captions: Vec<v::CaptionBox> = filmcraft_render::vertical::caption_boxes(q, plan_in(&s, seq, 1), range, None).iter().map(caption).collect();
    assert_eq!(captions.len(), 1, "one cue is burned");
    let lower = |frame| v::RegionBox { frame, rect: [800.0, 1000.0, 400.0, 80.0] };
    let input = v::RegionInput { id: "lower third".into(), label: String::new(), boxes: vec![lower(0), lower(399)], interpolate: true };
    let opts = v::MeasureOpts { regions: vec![input], protected: true, captions, safe_zone: Some([120.0, 288.0, 768.0, 960.0]), ..Default::default() };
    let want = oracle(&s, &opts);
    assert_eq!((&r["regions"], &r["findings"]), (&want["regions"], &want["findings"]));
    assert_eq!(r["regions"].as_array().map(Vec::len), Some(2), "the inline region and the protected one");
    // the union and its mirror (spec §5.14): safe from x 192 to 888 on 1080, whichever side a platform puts its buttons
    let mirror = crate::vertical::mirrored([120.0, 288.0, 768.0, 960.0], 1080.0);
    assert_eq!(mirror, [192.0, 288.0, 696.0, 960.0]);
    let r = ok(&mut s, "vertical.measure", json!({"plan": 1, "regions": regions, "safeZone": "unionMirrored"}));
    let want = oracle(&s, &v::MeasureOpts { safe_zone: Some(mirror), ..opts });
    assert_eq!((&r["regions"], &r["findings"]), (&want["regions"], &want["findings"]));
    let r = ok(&mut s, "vertical.measure", json!({"plan": 1, "baseline": "centreCrop", "series": true}));
    assert!(r["baseline"].is_object() && r["series"]["frames"].is_array(), "{r}");
    let r = ok(&mut s, "vertical.measure", json!({"plan": 1, "safeZone": [100, 200, 880, 1500]}));
    assert!(r["regions"].is_array());
}

#[test]
fn fit_returns_the_window_for_regions_and_for_the_eye_trace() {
    let (mut s, seq) = framing();
    let oracle = |s: &Session, req: &v::FitRequest| {
        let fitted = v::fit::fit(&s.project, s.project.sequence(seq).expect("the sequence"), 1, req).expect("the fit");
        serde_json::to_value(fitted).expect("JSON")
    };
    let r = ok(&mut s, "vertical.fit", json!({"plan": 1, "frame": 250, "regions": [{"space": "sequence", "rect": [800, 300, 200, 300]}]}));
    let want = oracle(&s, &v::FitRequest { frame: 250, regions: vec![[800.0, 300.0, 200.0, 300.0]], ..Default::default() });
    for key in ["window", "fits", "visible", "limitedBy"] {
        assert_eq!(r[key], want[key], "{key}: {r}");
    }
    let r = ok(&mut s, "vertical.fit", json!({"plan": 1, "frame": 250, "regions": [{"rect": [800, 300, 200, 300]}], "zoom": 120}));
    assert_eq!(r["window"]["h"], json!(900.0), "zoom 120 % of the largest window");
    // Match eye-trace: focus points on both sides of the cut at 200
    ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 180, "focus": [1050, 500]}));
    ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 200, "focus": [1000, 520]}));
    let r = ok(&mut s, "vertical.fit", json!({"plan": 1, "frame": 200, "eyeTrace": {"fromFrame": 199}}));
    let trace = v::EyeTrace { from_frame: 199, from_point: None, to_point: None };
    assert_eq!(r["window"], oracle(&s, &v::FitRequest { frame: 200, eye_trace: Some(trace), ..Default::default() })["window"]);
    // the protected regions at the frame are regions too
    let logo = json!({"label": "logo", "fromFrame": 0, "toFrame": 400, "rect": [1500, 40, 160, 90], "mode": "keepIn"});
    ok(&mut s, "vertical.setRegions", json!({"add": [logo]}));
    let r = ok(&mut s, "vertical.fit", json!({"plan": 1, "frame": 250, "regions": [{"source": "protected"}]}));
    assert_eq!(r["window"], oracle(&s, &v::FitRequest { frame: 250, regions: vec![[1500.0, 40.0, 160.0, 90.0]], ..Default::default() })["window"]);
}

#[test]
fn storyboard_lists_each_shot_with_its_opener_and_its_first_and_last_windows() {
    let (mut s, _) = framing();
    let r = ok(&mut s, "vertical.storyboard", json!({"plan": 1}));
    let shots = r["shots"].as_array().expect("shots");
    assert_eq!(shots.len(), 4);
    let shot = &shots[1];
    let span = json!({"number": 2, "fromFrame": 100, "toFrame": 200, "lastFrame": 199, "sources": ["edit"], "status": "own", "positions": [2, 3, 4]});
    contains(shot, &span, "shot 2");
    let review = json!({"reviewed": 0, "flagged": 0, "watched": false, "attention": [], "opener": 2, "openerLabel": null});
    contains(shot, &review, "shot 2");
    assert_eq!(shot["firstWindow"], json!({"cx": 700.0, "cy": 540.0, "h": 1080.0}));
    assert_eq!(shot["lastWindow"], json!({"cx": 1100.0, "cy": 540.0, "h": 900.0}));
    ok(&mut s, "vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 100, "toFrame": 300}]}));
    let r = ok(&mut s, "vertical.storyboard", json!({"plan": 1, "part": 1}));
    let numbers: Vec<u64> = r["shots"].as_array().expect("shots").iter().filter_map(|shot| shot["number"].as_u64()).collect();
    assert_eq!(numbers, [2, 3]);
}

#[test]
fn t_m18_a_part_s_edges_are_walls_in_the_engine_s_reads() {
    // a part from 150 to 250, where the picture has no edit point
    let (mut s, _) = framing();
    ok(&mut s, "vertical.setParts", json!({"add": [{"name": "P01", "fromFrame": 150, "toFrame": 250}]}));
    let r = ok(&mut s, "vertical.inspect", json!({"plan": 1}));
    let walls: Vec<(Option<i64>, Value)> = r["walls"].as_array().expect("walls").iter().map(|w| (w["frame"].as_i64(), w["sources"].clone())).collect();
    let want = [(100_i64, "edit"), (150, "part"), (200, "edit"), (250, "part"), (300, "edit")].map(|(frame, source)| (Some(frame), json!([source])));
    assert_eq!(walls, want);
    let starts: Vec<Option<i64>> = r["shots"].as_array().expect("shots").iter().map(|shot| shot["fromFrame"].as_i64()).collect();
    assert_eq!(starts, [0_i64, 100, 150, 200, 250, 300].map(Some), "the part's edges open shots");
    let board = ok(&mut s, "vertical.storyboard", json!({"plan": 1, "part": 1}));
    let span = |shot: &Value| (shot["fromFrame"].as_i64(), shot["toFrame"].as_i64());
    let spans: Vec<(Option<i64>, Option<i64>)> = board["shots"].as_array().expect("shots").iter().map(span).collect();
    assert_eq!(spans, [(Some(150), Some(200)), (Some(200), Some(250))]);
    assert_eq!(ok(&mut s, "vertical.goTo", json!({"to": "nextWall", "frame": 120}))["frame"], json!(150));
    ok(&mut s, "vertical.select", json!({"walls": [150, 250]}));
    assert_eq!(s.state.vertical.walls, [at(150), at(250)]);
}

// ---------- the shot-change measurement ----------

#[test]
fn measure_cuts_finds_the_cuts_and_possible_cuts_and_records_nothing() {
    let (mut s, seq) = measuring();
    let before = Snapshot::of(&s);
    let journal = s.journal.len();
    let r = ok(&mut s, "vertical.measureCuts", json!({"wait": true}));
    before.unchanged(&s, "a measurement");
    assert_eq!(s.journal.len(), journal, "a measurement is not journaled");
    assert!(s.project.sequence(seq).expect("the sequence").vertical.is_none(), "nothing is written into the project");
    let id = r["measurement"].as_str().expect("a measurement id").to_string();
    assert!(id.starts_with('m'), "{id}");
    let threshold = f64::from(filmcraft_render::scene::SceneOptions { sensitivity: 50.0, min_shot_frames: 6 }.threshold());
    assert_eq!((&r["sequence"], &r["analysed"], &r["sensitivity"], &r["minShotFrames"]), (&json!(seq.0), &json!([0, 500]), &json!(50.0), &json!(6)));
    assert_eq!((&r["threshold"], &r["candidateFloor"]), (&json!(threshold), &json!(threshold / 2.0)));
    assert_eq!(frames_of(&r["cuts"]), [100, 200, 300]);
    assert_eq!(frames_of(&r["candidates"]), [400], "a one-bin change is a possible cut, not a cut");
    assert_eq!(r["dropped"], json!([]));
    for cut in r["cuts"].as_array().expect("cuts") {
        assert!(cut["score"].as_f64().is_some_and(|score| score >= threshold), "{cut}");
    }
    let score = r["candidates"][0]["score"].as_f64().expect("a score");
    assert!((threshold / 2.0..threshold).contains(&score), "{score}");
    assert!(r.get("scores").is_none(), "per-frame scores only when asked");
    assert_eq!(s.vertical_store.measurements.ids(seq), [id]);
    // walls come only from a write
    let effective = v::effective_walls_digest(&s.project, s.project.sequence(seq).expect("the sequence"));
    assert_eq!(ok(&mut s, "vertical.list", json!({}))["effectiveWallsDigest"], json!(effective));
}

/// Assert that each range run on `s` finds what the full run `full` (asked with `scores`) finds in it: the same cuts,
/// dropped changes, possible cuts and per-frame scores.
fn ranges_equal_the_full_run(s: &mut Session, full: &Value, ranges: &[[i64; 2]]) {
    let scores = full["scores"].as_array().expect("per-frame scores").clone();
    for &[from, to] in ranges {
        let r = ok(s, "vertical.measureCuts", json!({"wait": true, "scores": true, "fromFrame": from, "toFrame": to}));
        assert_eq!(r["analysed"], json!([from, to]));
        for key in ["cuts", "dropped", "candidates"] {
            assert_eq!(r[key], within(&full[key], from, to), "the {key} of [{from}, {to})");
        }
        let (a, b) = (usize::try_from(from).expect("a frame"), usize::try_from(to).expect("a frame"));
        assert_eq!(r["scores"], Value::Array(scores[a..b].to_vec()), "the scores of [{from}, {to})");
    }
}

#[test]
fn a_range_measurement_equals_the_full_run_restricted() {
    let (mut s, _) = measuring();
    let full = ok(&mut s, "vertical.measureCuts", json!({"wait": true, "scores": true}));
    assert_eq!(full["scores"].as_array().map(Vec::len), Some(500));
    ranges_equal_the_full_run(&mut s, &full, &[[150, 260], [350, 450], [0, 250], [250, 500], [95, 105], [100, 200]]);
}

#[test]
fn a_range_with_close_changes_and_walls_near_its_edges_equals_the_full_run_restricted() {
    // the clip boundaries are no walls; a person's wall at 152
    let (mut s, _) = session_with(640, 360, &BURST, None);
    ok(&mut s, "vertical.setWalls", json!({"useEditPoints": false, "add": [152]}));
    let full = ok(&mut s, "vertical.measureCuts", json!({"wait": true, "scores": true}));
    assert_eq!(frames_of(&full["cuts"]), [101, 200, 250], "of two changes closer than 6 frames, the stronger is the cut");
    assert_eq!(frames_of(&full["dropped"]), [97, 156, 253], "97 and 253 are outranked, 156 is 4 frames after the wall at 152");
    assert_eq!(frames_of(&full["candidates"]), [97, 156, 253], "a dropped change stays a possible cut");
    // ranges that start or end between two close changes, or hold the wall near their start
    let ranges = [[99, 320], [96, 98], [98, 157], [150, 260], [153, 201], [0, 100], [101, 102], [249, 251], [251, 254], [0, 320]];
    ranges_equal_the_full_run(&mut s, &full, &ranges);
}

#[test]
fn re_measuring_after_an_apply_finds_what_a_measurement_before_it_finds() {
    // a one-bin change at 98 and a hard cut 3 frames later; the clip boundaries are no walls
    let fixture = || {
        let (mut s, _) = session_with(640, 360, &FLICKER, None);
        ok(&mut s, "vertical.setWalls", json!({"useEditPoints": false}));
        s
    };
    let mut s = fixture();
    let m = ok(&mut s, "vertical.measureCuts", json!({"wait": true, "sensitivity": 100}));
    assert_eq!(frames_of(&m["cuts"]), [101], "the stronger change is the cut");
    assert_eq!(frames_of(&m["dropped"]), [98], "the one-bin change 3 frames before it is outranked");
    ok(&mut s, "vertical.applyMeasurement", json!({"measurement": m["measurement"]}));
    // the applied wall at 101 changes no later measurement, at any sensitivity
    for sensitivity in [0, 50, 100] {
        let again = ok(&mut s, "vertical.measureCuts", json!({"wait": true, "sensitivity": sensitivity}));
        let before = ok(&mut fixture(), "vertical.measureCuts", json!({"wait": true, "sensitivity": sensitivity}));
        assert_eq!(again["cuts"], before["cuts"], "sensitivity {sensitivity}");
        assert_eq!(frames_of(&again["cuts"]), [101], "sensitivity {sensitivity}");
    }
}

#[test]
fn t_m34_min_shot_frames_holds_against_every_wall_in_every_run() {
    let (mut s, _) = measuring();
    ok(&mut s, "vertical.setWalls", json!({"useEditPoints": false, "add": [97, 203]}));
    let full = ok(&mut s, "vertical.measureCuts", json!({"wait": true, "scores": true}));
    assert_eq!(frames_of(&full["cuts"]), [300]);
    assert_eq!(frames_of(&full["dropped"]), [100, 200], "100 is 3 frames after the wall at 97, 200 3 frames before the wall at 203");
    // the walls just before and after a range count as the walls inside it do
    ranges_equal_the_full_run(&mut s, &full, &[[98, 200], [150, 203], [0, 150], [97, 101], [100, 500], [199, 204]]);
    let r = ok(&mut s, "vertical.measureCuts", json!({"wait": true, "fromFrame": 98, "toFrame": 200, "minShotFrames": 2}));
    assert_eq!(frames_of(&r["cuts"]), [100], "3 frames from the wall is a shot of 2 frames or more");
}

#[test]
fn a_live_measurement_is_a_job_with_progress_and_a_result() {
    let (mut s, seq) = measuring();
    let started = ok(&mut s, "vertical.measureCuts", json!({}));
    let job = started["job"].as_u64().expect("a job id");
    assert_eq!(started["frames"], json!(500));
    let listed = ok(&mut s, "jobs.list", json!({}));
    assert!(listed.as_array().expect("jobs").iter().any(|j| j["id"] == json!(job)), "jobs.list shows it: {listed}");
    let done = wait_for(&mut s, job, "done");
    assert_eq!(done["progress"], json!({"done": 500, "total": 500}));
    assert_eq!(done["result"]["measurement"], started["measurement"]);
    let waited = ok(&mut s, "vertical.measureCuts", json!({"wait": true}));
    for key in ["analysed", "threshold", "cuts", "candidates", "dropped"] {
        assert_eq!(done["result"][key], waited[key], "{key}");
    }
    assert_eq!(s.vertical_store.measurements.ids(seq).len(), 2);
}

#[test]
fn a_cancelled_measurement_keeps_nothing_and_a_running_one_blocks_another() {
    let (mut s, seq) = measuring();
    // the job waits at its first frame until the gate opens: it is running, however fast the machine
    let gate = Gated::install(&s, seq, 0);
    gate.set(true);
    let started = ok(&mut s, "vertical.measureCuts", json!({}));
    let job = started["job"].as_u64().expect("a job id");
    let id = started["measurement"].clone();
    expect_refusal(&mut s, "vertical.measureCuts", json!({"wait": true}), "measure_running", json!({"job": job}));
    expect_refusal(&mut s, "vertical.applyMeasurement", json!({"measurement": id}), "measure_running", json!({"job": job}));
    assert_eq!(ok(&mut s, "vertical.jobResult", json!({"job": job}))["state"], json!("running"));
    ok(&mut s, "jobs.cancel", json!({"job": job}));
    gate.set(false);
    let r = wait_for(&mut s, job, "cancelled");
    assert_eq!(r["result"], Value::Null);
    assert!(s.vertical_store.measurements.ids(seq).is_empty(), "a cancelled measurement is not kept");
    expect_refusal(&mut s, "vertical.applyMeasurement", json!({"measurement": id}), "not_found", json!({"measurement": id}));
}

#[test]
fn a_cancelled_dynamic_trim_keeps_the_measurements_their_jobs_and_the_watched_shots() {
    let (mut s, seq) = framing();
    let gate = Gated::install(&s, seq, 0);
    ok(&mut s, "vertical.noteWatched", json!({"plan": 1, "shot": 2, "frames": [[100, 200]]}));
    let kept = ok(&mut s, "vertical.measureCuts", json!({"wait": true}))["measurement"].clone();
    gate.set(true);
    let job = ok(&mut s, "vertical.measureCuts", json!({}))["job"].as_u64().expect("a job id");
    // Esc in a dynamic trim puts back the project from before the trim: the same project, its history and its life
    dynamic_roll(&mut s);
    ok(&mut s, "trim.cancelDynamic", json!({}));
    assert_eq!(ok(&mut s, "vertical.jobResult", json!({"job": job}))["state"], json!("running"), "the measurement still runs");
    assert_eq!(ok(&mut s, "vertical.inspect", json!({"plan": 1}))["shots"][1]["watched"], json!(true), "the watched shot is kept");
    ok(&mut s, "vertical.applyMeasurement", json!({"measurement": kept, "dryRun": true}));
    gate.set(false);
    let done = wait_for(&mut s, job, "done");
    ok(&mut s, "vertical.applyMeasurement", json!({"measurement": done["result"]["measurement"], "dryRun": true}));
    assert_eq!(s.vertical_store.measurements.ids(seq).len(), 2);
    // another project is another life: the store forgets the old one and stops its jobs
    gate.set(true);
    let job = ok(&mut s, "vertical.measureCuts", json!({}))["job"].as_u64().expect("a job id");
    ok(&mut s, "file.newProject", json!({}));
    let stopped = s.jobs.iter().find(|j| j.id == job).is_some_and(|j| j.progress.cancel.load(std::sync::atomic::Ordering::Relaxed));
    gate.set(false);
    assert!(stopped, "the old project's measurement is stopped");
    let store = &s.vertical_store;
    assert!(store.measurements.pending.is_empty() && store.measurements.by_sequence.is_empty() && store.watched.is_empty());
}

#[test]
fn applying_a_measurement_writes_detected_walls_and_possible_cuts() {
    let (mut s, seq) = measuring();
    let id = ok(&mut s, "vertical.measureCuts", json!({"wait": true}))["measurement"].clone();
    let dry = ok(&mut s, "vertical.applyMeasurement", json!({"measurement": id, "dryRun": true}));
    let r = ok(&mut s, "vertical.applyMeasurement", json!({"measurement": id}));
    let keys = ["dryRun", "applied", "label"];
    assert_eq!(without(&dry, &keys), without(&r, &keys), "the dry run's result is the real one");
    assert_eq!(r["label"], json!("Vertical: apply measured cuts 00:00:00:00\u{2013}00:00:20:00 (+3, 1 possible)"));
    let walls = &set_of(&s, seq).walls;
    let stored: Vec<(i64, WallOrigin)> = walls.stored.iter().map(|w| (RATE.frame_at(w.at), w.origin)).collect();
    assert_eq!(stored, [(100, WallOrigin::Detected), (200, WallOrigin::Detected), (300, WallOrigin::Detected)]);
    let candidates: Vec<i64> = walls.candidates.iter().map(|c| RATE.frame_at(c.at)).collect();
    assert_eq!(candidates, [400]);
    assert_eq!(walls.analysed, [FrameRange { from: at(0), to: at(500) }]);
    // the same measurement again restates the same walls (T-N15)
    let before = Snapshot::of(&s);
    let again = ok(&mut s, "vertical.applyMeasurement", json!({"measurement": id}));
    assert_eq!(again["changed"], json!(false));
    before.unchanged(&s, "applying a measurement again");
    // a person's wall and a deleted detected wall survive a new measurement (T-M34)
    ok(&mut s, "vertical.setWalls", json!({"add": [450], "remove": [200]}));
    let id = ok(&mut s, "vertical.measureCuts", json!({"wait": true}))["measurement"].clone();
    let r = ok(&mut s, "vertical.applyMeasurement", json!({"measurement": id}));
    assert_eq!(r["label"], json!("Vertical: apply measured cuts 00:00:00:00\u{2013}00:00:20:00 (+2, 1 possible)"));
    let walls = &set_of(&s, seq).walls;
    let stored: Vec<(i64, WallOrigin)> = walls.stored.iter().map(|w| (RATE.frame_at(w.at), w.origin)).collect();
    assert_eq!(stored, [(100, WallOrigin::Detected), (300, WallOrigin::Detected), (450, WallOrigin::Manual)]);
    let suppressed: Vec<i64> = walls.suppressed.iter().map(|t| RATE.frame_at(*t)).collect();
    assert_eq!(suppressed, [200], "a detected wall a person deleted stays suppressed");
}

#[test]
fn a_measurement_goes_stale_when_the_picture_or_the_media_changes() {
    let (mut s, seq) = measuring();
    let picture = |s: &Session| crate::vertical_measure::picture_digest(&s.project, seq);
    let id = ok(&mut s, "vertical.measureCuts", json!({"wait": true}))["measurement"].clone();
    let (measured, generation) = (picture(&s), s.media.generation());
    // a marker changes no picture: the measurement still applies
    ok(&mut s, "markers.add", json!({}));
    ok(&mut s, "vertical.applyMeasurement", json!({"measurement": id, "dryRun": true}));
    // a 16:9 edit changes the picture: the detail names what changed, with the measured and the current values
    ok(&mut s, "playhead.set", json!({"frame": 50}));
    ok(&mut s, "sequence.addEditAllTracks", json!({}));
    let (digests, generations) = (json!({"measured": measured, "current": picture(&s)}), json!({"measured": generation, "current": generation}));
    let detail = json!({"measurement": id, "changed": ["picture"], "pictureDigest": digests, "mediaGeneration": generations});
    expect_refusal(&mut s, "vertical.applyMeasurement", json!({"measurement": id}), "stale_measurement", detail);
    // and so does a change of the media pool
    let id = ok(&mut s, "vertical.measureCuts", json!({"wait": true}))["measurement"].clone();
    ok(&mut s, "vertical.applyMeasurement", json!({"measurement": id, "dryRun": true}));
    s.media.insert(ItemId(999_999), Arc::new(GeneratorSource::new(matte([10, 20, 30]), 640, 360, RATE, at(10))));
    let generations = json!({"measured": generation, "current": generation + 1});
    let detail = json!({"measurement": id, "changed": ["media"], "mediaGeneration": generations});
    expect_refusal(&mut s, "vertical.applyMeasurement", json!({"measurement": id}), "stale_measurement", detail);
}

#[test]
fn applying_a_measurement_keeps_cuts_inside_locked_moves_as_possible_cuts() {
    let out = OutputSize { width: 180, height: 320 };
    let positions = vec![cut(1, 0, 320.0, 180.0, 360.0), moved(2, 120, (400.0, 180.0, 300.0), 40, Curve::EaseInOut)];
    let (mut s, seq) = master(Some(set_for(out, positions)));
    let m = ok(&mut s, "vertical.measureCuts", json!({"wait": true}));
    assert_eq!(frames_of(&m["cuts"]), [100, 200], "the cuts inside the nest are measured");
    let id = m["measurement"].clone();
    // an unlocked move: refused by default, shortened when the caller says so
    let split = json!({"plan": 1, "position": 2, "wall": 100});
    expect_refusal(&mut s, "vertical.applyMeasurement", json!({"measurement": id}), "would_split_move", split);
    let r = ok(&mut s, "vertical.applyMeasurement", json!({"measurement": id, "onConflict": "shorten", "dryRun": true}));
    assert_eq!(r["changes"][0]["code"], json!("move_shortened"), "{r}");
    // a locked move keeps its frames: the cut inside it stays a possible cut, with the info issue
    ok(&mut s, "vertical.setLock", json!({"plan": 1, "positions": [2], "locked": true}));
    let r = ok(&mut s, "vertical.applyMeasurement", json!({"measurement": id, "onConflict": "shorten"}));
    let blocked = r["issues"].as_array().expect("issues").iter().find(|i| i["code"] == json!("wall_blocked_by_locked")).cloned();
    assert_eq!(blocked.map(|i| i["detail"].clone()), Some(json!({"plan": 1, "position": 2, "frame": 100})));
    let walls = &set_of(&s, seq).walls;
    let stored: Vec<i64> = walls.stored.iter().map(|w| RATE.frame_at(w.at)).collect();
    let candidates: Vec<i64> = walls.candidates.iter().map(|c| RATE.frame_at(c.at)).collect();
    assert_eq!(stored, [200]);
    assert_eq!(candidates, [100]);
}

#[test]
fn applying_a_measurement_lists_the_positions_made_while_shots_were_unknown() {
    let (mut s, _) = master(Some(set_for(OutputSize { width: 180, height: 320 }, Vec::new())));
    let inspect = |s: &mut Session| ok(s, "vertical.inspect", json!({"plan": 1}));
    assert_eq!(inspect(&mut s)["shotsUnknown"], json!([[0, 300]]), "a master without edit points: shots are not known");
    let sp = "vertical.setPosition";
    ok(&mut s, sp, json!({"plan": 1, "frame": 0, "cx": 320, "cy": 180, "h": 360}));
    let off = ok(&mut s, sp, json!({"plan": 1, "frame": 95, "cx": 300}))["added"][0].clone();
    ok(&mut s, sp, json!({"plan": 1, "frame": 200, "cx": 340}));
    let m = ok(&mut s, "vertical.measureCuts", json!({"wait": true}));
    ok(&mut s, "vertical.applyMeasurement", json!({"measurement": m["measurement"]}));
    let r = inspect(&mut s);
    assert_eq!(r["shotsUnknown"], json!([]));
    assert_eq!(r["unanchoredSinceMeasurement"], json!([off]), "only #2 is off a wall: #3 landed on the measured cut at 200");
    // a position made once shots are known is not listed; one moved onto a wall leaves the list
    ok(&mut s, sp, json!({"plan": 1, "frame": 150, "cx": 330}));
    ok(&mut s, "vertical.retimePosition", json!({"plan": 1, "position": off, "frame": 100}));
    assert_eq!(inspect(&mut s)["unanchoredSinceMeasurement"], json!([]));
}

#[test]
fn the_session_keeps_the_last_four_measurements_of_each_sequence() {
    let (mut s, seq) = measuring();
    let mut ids = Vec::new();
    for _ in 0..5 {
        let r = ok(&mut s, "vertical.measureCuts", json!({"wait": true, "toFrame": 50}));
        ids.push(r["measurement"].as_str().expect("an id").to_string());
    }
    assert_eq!(s.vertical_store.measurements.ids(seq), &ids[1..]);
    expect_refusal(&mut s, "vertical.applyMeasurement", json!({"measurement": ids[0]}), "not_found", json!({"measurement": ids[0]}));
    ok(&mut s, "vertical.applyMeasurement", json!({"measurement": ids[4], "dryRun": true}));
}

// ---------- the 16:9 frame cache and the exact frame ----------

/// The key of frame `frame` of sequence 1 at full size in a fixed picture.
fn key(frame: i64) -> FrameKey16x9 {
    FrameKey16x9::new(FrameIdentity { epoch: 1, picture_revision: 1, media_generation: 0 }, ItemId(1), frame, 1.0, false)
}

/// A 2×2 frame of grey `level`.
fn frame_image(level: f32) -> Arc<Image> {
    Arc::new(Image::filled(2, 2, [level, level, level, 1.0]))
}

#[test]
fn the_frame_cache_keeps_the_four_most_recently_used_frames() {
    let cache = VerticalFrameCache::default();
    assert_eq!((cache.capacity(), cache.len()), (crate::vertical_cache::CAPACITY, 0));
    for frame in 0..4 {
        cache.put(key(frame), frame_image(0.1));
    }
    assert_eq!(cache.len(), 4);
    assert!(cache.get(&key(0)).is_some(), "frame 0 is held, and now the most recently used");
    cache.put(key(4), frame_image(0.2));
    assert_eq!(cache.len(), 4);
    assert!(!cache.contains(&key(1)), "the least recently used frame is dropped");
    for frame in [0, 2, 3, 4] {
        assert!(cache.contains(&key(frame)), "frame {frame} is held");
    }
    // storing a key again replaces its frame
    cache.put(key(4), frame_image(0.9));
    assert_eq!(cache.len(), 4);
    assert_eq!(cache.get(&key(4)).map(|i| i.px[0]), Some(0.9));
    assert!(cache.get(&key(1)).is_none());
    assert_eq!((cache.hits(), cache.misses(), cache.puts()), (2, 1, 6));
    // a key differs in every field
    let identity = FrameIdentity { epoch: 1, picture_revision: 1, media_generation: 0 };
    cache.put(key(7), frame_image(0.3));
    let others = [
        FrameKey16x9::new(FrameIdentity { epoch: 2, ..identity }, ItemId(1), 7, 1.0, false),
        FrameKey16x9::new(FrameIdentity { picture_revision: 2, ..identity }, ItemId(1), 7, 1.0, false),
        FrameKey16x9::new(FrameIdentity { media_generation: 1, ..identity }, ItemId(1), 7, 1.0, false),
        FrameKey16x9::new(identity, ItemId(2), 7, 1.0, false),
        FrameKey16x9::new(identity, ItemId(1), 8, 1.0, false),
        FrameKey16x9::new(identity, ItemId(1), 7, 0.5, false),
        FrameKey16x9::new(identity, ItemId(1), 7, 1.0, true),
    ];
    for other in others {
        assert!(cache.get(&other).is_none(), "{other:?} is another frame");
    }
    assert!(cache.get(&key(7)).is_some());
    // clearing forgets everything
    let clears = cache.clears();
    cache.clear();
    assert_eq!((cache.len(), cache.clears()), (0, clears + 1));
    // a frame rendered while media data was still loading is incomplete: never kept
    filmcraft_media::pending::mark();
    cache.put(key(5), frame_image(0.5));
    let _ = filmcraft_media::pending::take();
    assert!(!cache.contains(&key(5)), "a pending frame is not kept");
    assert_eq!(VerticalFrameCache::with_capacity(0).capacity(), 1);
}

#[test]
fn replacing_the_project_clears_the_frame_cache() {
    let (mut s, seq) = small();
    exact_frame(&s, seq, 1, at(10), false).expect("the exact frame");
    assert_eq!(s.vertical_cache.len(), 1);
    ok(&mut s, "file.newProject", json!({}));
    assert!(s.vertical_cache.is_empty(), "a new project starts with an empty cache");
}

#[test]
fn the_exact_frame_is_render_vertical_through_full_resolution_sources() {
    let (s, seq) = small();
    let t = at(60);
    let img = exact_frame(&s, seq, 1, t, false).expect("the exact frame");
    let sources = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let inputs = VerticalRenderInputs { sources: &sources, captions: false, working_output: false, cache: None, identity: FrameIdentity::default() };
    let direct = render_vertical(&s.project, seq, 1, t, VerticalRenderOptions::default(), &inputs).expect("the direct render");
    assert_eq!((img.w, img.h), (180, 320), "the plan's output size");
    assert_eq!(img, direct, "the helper is render_vertical through full-resolution sources");
    let (w, h, bytes) = exact_frame_rgba8(&s, seq, 1, t, false).expect("the RGBA8 frame");
    assert_eq!((w, h, bytes), (direct.w, direct.h, direct.over_black_rgba8()));
    let identity = FrameIdentity { epoch: s.epoch, picture_revision: s.picture_revision, media_generation: s.media.generation() };
    assert_eq!(frame_identity(&s), identity);
    assert!(s.vertical_cache.contains(&FrameKey16x9::new(identity, seq, 60, 1.0, false)), "the 16:9 frame is cached");
    for (seq, plan, code) in [(seq, 9, "not_found"), (ItemId(999_999), 1, "not_found")] {
        match exact_frame(&s, seq, plan, t, false) {
            Err(EngineError::Invalid { code: got, .. }) => assert_eq!(got, code),
            other => panic!("sequence {seq:?} plan {plan}: {other:?}"),
        }
    }
    let (t2, seq2) = damaged();
    match exact_frame(&t2, seq2, 1, at(10), false) {
        Err(EngineError::Invalid { code, .. }) => assert_eq!(code, "invalid_plan"),
        other => panic!("a damaged plan: {other:?}"),
    }
}

#[test]
fn t_n19_a_plan_only_edit_renders_no_16x9_frame_and_another_project_gets_no_hit() {
    let (mut s, seq) = small();
    let cache = s.vertical_cache.clone();
    let t = at(60);
    let first = exact_frame(&s, seq, 1, t, false).expect("the exact frame");
    assert_eq!((cache.puts(), cache.misses(), cache.hits()), (1, 1, 0), "the first frame renders its 16:9 frame once");
    // 20 merged sub-steps of a drag: each frame after them is a hit; only the resample runs again
    for k in 1..=20 {
        ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 50, "cx": 100 + k, "merge": "drag:2"}));
        exact_frame(&s, seq, 1, t, false).expect("the exact frame");
    }
    assert_eq!((cache.puts(), cache.hits()), (1, 20), "a nudge causes zero 16:9 renders");
    assert_eq!(position(&s, seq, 2).window.cx, 120.0, "the drag's last sub-step holds");
    // and validates nothing again: each write left the store the validity of the snapshot it made
    let known = s.vertical_store.validity.get(&seq).expect("the sequence's validity");
    assert!(std::ptr::eq(known.project.as_ptr(), Arc::as_ptr(&s.project)) && known.violations.is_empty());
    // #2 frames the bars: its window, 20 px further right, shows other columns
    let moved = exact_frame(&s, seq, 1, t, false).expect("the exact frame");
    assert_ne!(moved.px, first.px, "the window moved");
    // the same frame of another project whose sequence has the same id is never a hit
    let identity = frame_identity(&s);
    let key_a = FrameKey16x9::new(identity, seq, 60, 1.0, false);
    assert!(cache.contains(&key_a));
    let (other, other_seq) = project_with(&s.media, 320, 180, &[([72, 200, 40], 100)], Some(small_set()));
    assert_eq!(other_seq, seq, "item ids restart in every project");
    install(&mut s, other, other_seq);
    assert!(cache.is_empty(), "replacing the project clears the cache");
    assert_ne!(FrameKey16x9::new(frame_identity(&s), seq, 60, 1.0, false), key_a, "the epoch is part of the key");
    let puts = cache.puts();
    let b = exact_frame(&s, seq, 1, t, false).expect("project B's frame");
    assert_eq!(cache.puts(), puts + 1, "project B renders its own 16:9 frame");
    assert_ne!(b.px, moved.px);
}

// ---------- validation, the watched store, the selection, goTo ----------

#[test]
fn a_damaged_plan_is_never_evaluated_until_it_is_repaired() {
    let (mut s, _) = damaged();
    let list = ok(&mut s, "vertical.list", json!({}));
    assert_eq!(list["plans"][0]["valid"], json!(false), "{list}");
    assert!(list["plans"][0]["issues"]["error"].as_u64().is_some_and(|n| n >= 1), "{list}");
    let calls = [
        ("vertical.inspect", json!({"plan": 1})),
        ("vertical.windows", json!({"plan": 1, "fromFrame": 0, "toFrame": 10})),
        ("vertical.measure", json!({"plan": 1})),
        ("vertical.measure", json!({"plan": 1, "regions": [{"source": "captions"}]})),
        ("vertical.fit", json!({"plan": 1, "frame": 10, "regions": [{"rect": [800, 300, 200, 300]}]})),
        ("vertical.storyboard", json!({"plan": 1})),
        ("vertical.setPosition", json!({"plan": 1, "frame": 250, "cx": 1100})),
    ];
    for (id, p) in calls {
        let (code, detail) = refused(&mut s, id, p);
        assert_eq!(code, "invalid_plan", "`{id}`");
        let violations = detail["violations"].as_array().expect("the violations");
        assert!(violations.iter().any(|violation| violation["invariant"] == json!("I1")), "`{id}`: {detail}");
    }
    // a person's marks still work on it
    ok(&mut s, "vertical.setLock", json!({"plan": 1, "positions": [6], "locked": true}));
    ok(&mut s, "vertical.setLock", json!({"plan": 1, "positions": [6], "locked": false}));
    // repair: a dry run, then for real; undo brings the damage back (validation after undo), redo the repair
    let dry = ok(&mut s, "vertical.repair", json!({"plan": 1, "dryRun": true}));
    assert_eq!((&dry["changed"], &dry["applied"]), (&json!(true), &json!(false)));
    let r = ok(&mut s, "vertical.repair", json!({"plan": 1}));
    assert_eq!(r["label"], json!("Vertical: repair plan 9:16"));
    ok(&mut s, "vertical.inspect", json!({"plan": 1}));
    let again = ok(&mut s, "vertical.repair", json!({"plan": 1}));
    assert_eq!(again["changed"], json!(false), "repairing a valid plan is a no-op (T-N15)");
    ok(&mut s, "edit.undo", json!({}));
    assert_eq!(refused(&mut s, "vertical.inspect", json!({"plan": 1})).0, "invalid_plan");
    ok(&mut s, "edit.redo", json!({}));
    ok(&mut s, "vertical.inspect", json!({"plan": 1}));
}

#[test]
fn opening_a_file_with_a_damaged_plan_reads_it_as_invalid() {
    let (mut s, _) = damaged();
    let dir = crate::media_test_util::tmp_dir("vertical-damaged");
    let path = dir.join("damaged.fcproj").to_string_lossy().to_string();
    ok(&mut s, "file.saveAs", json!({"path": path}));
    let mut t = Session::default();
    ok(&mut t, "file.open", json!({"path": path}));
    assert_eq!(ok(&mut t, "vertical.list", json!({}))["plans"][0]["valid"], json!(false));
    assert_eq!(refused(&mut t, "vertical.inspect", json!({"plan": 1})).0, "invalid_plan");
}

#[test]
fn note_watched_records_what_playback_showed_and_resets_when_the_shot_changes() {
    let (mut s, seq) = framing();
    let watched = |s: &mut Session, shot: usize| ok(s, "vertical.inspect", json!({"plan": 1}))["shots"][shot]["watched"].clone();
    let note = |s: &mut Session, shot: u32, frames: Value| ok(s, "vertical.noteWatched", json!({"plan": 1, "shot": shot, "frames": frames}));
    let before = Snapshot::of(&s);
    let journal = s.journal.len();
    note(&mut s, 2, json!([[100, 198]]));
    before.unchanged(&s, "noting watched frames");
    assert_eq!(s.journal.len(), journal, "noteWatched is not journaled");
    assert_eq!(watched(&mut s, 1), json!(true), "98 of 100 frames: two dropped frames are allowed");
    assert_eq!(ok(&mut s, "vertical.storyboard", json!({"plan": 1}))["shots"][1]["watched"], json!(true));
    // inspect names the range digest the record holds for (spec §7.4)
    let record = json!({"digest": range_digest(&s, seq, 1, 100, 200), "frames": 98, "pass": null});
    assert_eq!(ok(&mut s, "vertical.inspect", json!({"plan": 1}))["shots"][1]["watch"], record);
    note(&mut s, 3, json!([[200, 297]]));
    assert_eq!(watched(&mut s, 2), json!(false), "97 of 100 frames is not watched");
    note(&mut s, 3, json!([297, 298, 299]));
    assert_eq!(watched(&mut s, 2), json!(true));
    // a change elsewhere keeps it, and so does a change of the next shot's opener, a cut on the wall that ends the shot
    ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 350, "cx": 1000}));
    ok(&mut s, "vertical.setPosition", json!({"plan": 1, "frame": 200, "cx": 1010}));
    assert_eq!((watched(&mut s, 1), watched(&mut s, 2)), (json!(true), json!(false)), "the opener at 200 is shot 3's");
    // a change of the shot made while paused resets it: the frames noted before are not the new framing's
    let opener = |s: &mut Session, cx: f64| ok(s, "vertical.setPosition", json!({"plan": 1, "frame": 100, "cx": cx}));
    opener(&mut s, 710.0);
    assert_eq!(watched(&mut s, 1), json!(false), "the shot's range digest changed");
    note(&mut s, 2, json!([[160, 200]]));
    assert_eq!(watched(&mut s, 1), json!(false), "40 frames since the change");
    // a person's live edit during a playback pass keeps the pass: the shot is watched when the pass ends (spec §5.10)
    let in_pass = |s: &mut Session, frames: Value, pass: u64| ok(s, "vertical.noteWatched", json!({"plan": 1, "shot": 2, "frames": frames, "pass": pass}));
    in_pass(&mut s, json!([[100, 150]]), 7);
    opener(&mut s, 720.0);
    let r = in_pass(&mut s, json!([[150, 200]]), 7);
    assert_eq!((&r["frames"], &r["watched"], &r["pass"]), (&json!(100), &json!(true), &json!(7)));
    assert_eq!(watched(&mut s, 1), json!(true), "the record holds for the new framing");
    // an edit, then a new pass: the edit was made while paused
    opener(&mut s, 730.0);
    assert_eq!(in_pass(&mut s, json!([[100, 120]]), 8)["frames"], json!(20));
    // the same pass after a picture change (a 16:9 edit advances the picture revision): not a live edit of the framing
    in_pass(&mut s, json!([[120, 150]]), 8);
    s.picture_revision += 1;
    opener(&mut s, 740.0);
    assert_eq!(in_pass(&mut s, json!([[150, 200]]), 8)["frames"], json!(50));
    let p = json!({"plan": 1, "shot": 2, "frames": [[100, 110]], "pass": -1});
    expect_refusal(&mut s, "vertical.noteWatched", p, "out_of_range", json!({"key": "pass"}));
}

#[test]
fn select_sets_the_vertical_selection_and_checks_what_it_names() {
    let (mut s, _) = framing();
    let before = Snapshot::of(&s);
    let r = ok(&mut s, "vertical.select", json!({"plan": 1, "positions": [2, 3], "walls": [100, 200]}));
    before.unchanged(&s, "a selection");
    let want = VerticalSelection { plan: Some(1), positions: vec![2, 3], walls: vec![at(100), at(200)], region: None };
    assert_eq!(s.state.vertical, want);
    contains(&r, &json!({"plan": 1, "positions": [2, 3], "walls": [100, 200], "region": null}), "the selection");
    ok(&mut s, "vertical.select", json!({}));
    assert_eq!(s.state.vertical, VerticalSelection::default(), "omitted keys select nothing");
    ok(&mut s, "vertical.setRegions", json!({"add": [{"label": "logo", "fromFrame": 0, "toFrame": 400, "rect": [10, 10, 40, 40], "mode": "keepIn"}]}));
    ok(&mut s, "vertical.select", json!({"region": 1}));
    assert_eq!(s.state.vertical.region, Some(1));
    expect_refusal(&mut s, "vertical.select", json!({"region": 9}), "not_found", json!({"region": 9}));
}

#[test]
fn fix_state_keeps_only_effective_walls_selected() {
    let (mut s, seq) = framing();
    // Add Edit at 50 on every track: a through edit, an edit point that is no shot change
    ok(&mut s, "playhead.set", json!({"frame": 50}));
    ok(&mut s, "sequence.addEditAllTracks", json!({}));
    let walls: Vec<i64> = v::walls(&s.project, s.project.sequence(seq).expect("the sequence")).iter().map(|w| RATE.frame_at(w.at)).collect();
    assert_eq!(walls, [100, 200, 300], "a through edit is no wall");
    s.state.vertical = VerticalSelection { plan: Some(1), positions: vec![2], walls: vec![at(50), at(100)], region: None };
    s.fix_state();
    assert_eq!(s.state.vertical.walls, [at(100)]);
}

#[test]
fn go_to_moves_the_playhead_to_the_next_position_wall_break_or_mark() {
    let (mut s, _) = framing();
    let go = |s: &mut Session, to: &str| ok(s, "vertical.goTo", json!({"plan": 1, "to": to}))["frame"].clone();
    let before = Snapshot::of(&s);
    assert_eq!(go(&mut s, "nextPosition"), json!(100));
    assert_eq!(s.playhead(), at(100));
    before.unchanged(&s, "goTo");
    assert_eq!(go(&mut s, "nextPosition"), json!(140));
    assert_eq!(go(&mut s, "prevPosition"), json!(100));
    assert_eq!(go(&mut s, "nextWall"), json!(200));
    assert_eq!(go(&mut s, "prevWall"), json!(100));
    assert_eq!(go(&mut s, "nextBreak"), json!(200), "#3 and #4 arrive by moves: the next break is #5's cut");
    assert_eq!(go(&mut s, "prevBreak"), json!(100));
    assert_eq!(go(&mut s, "nextAttention"), Value::Null);
    assert_eq!(s.playhead(), at(100), "nothing to go to leaves the playhead");
    ok(&mut s, "vertical.flagAttention", json!({"plan": 1, "frame": 250}));
    assert_eq!(go(&mut s, "nextAttention"), json!(250));
    assert_eq!(ok(&mut s, "vertical.goTo", json!({"plan": 1, "to": "nextWall", "frame": 250}))["frame"], json!(300), "from a given frame");
}

// ---------- the scope audit and the agreements between lanes ----------

#[test]
fn t_n21_the_command_table_runs_no_model_and_only_measure_cuts_analyses_pixels() {
    let sources = [
        ("vertical.rs", include_str!("vertical.rs")),
        ("vertical_measure.rs", include_str!("vertical_measure.rs")),
        ("vertical_frames.rs", include_str!("vertical_frames.rs")),
        ("vertical_cache.rs", include_str!("vertical_cache.rs")),
    ];
    let allowed = ["std", "serde", "serde_json", "crate", "super", "web_time"];
    for (name, text) in sources {
        for line in text.lines().map(str::trim).filter(|l| l.starts_with("use ")) {
            let root = line.trim_start_matches("use ").split("::").next().unwrap_or_default();
            assert!(allowed.contains(&root) || root.starts_with("filmcraft_"), "{name}: {line}");
        }
        for word in ["filmcraft_speech", "Transcriber", "onnx", "candle", "whisper"] {
            assert!(!text.contains(word), "{name} uses {word}");
        }
    }
    let commands = include_str!("vertical.rs");
    for call in ["render_sequence", "render_vertical", "video_frame", "SceneDetector", "vertical_frames::"] {
        assert!(!commands.contains(call), "the command module renders and analyses nothing itself: `{call}`");
    }
    let measure = include_str!("vertical_measure.rs");
    assert!(measure.contains("render_sequence(") && measure.contains("scene::"), "measureCuts analyses the composited program with the detector");
    // measuring records nothing and makes no wall; a write does
    let (mut s, seq) = measuring();
    let effective = |s: &Session| v::effective_walls_digest(&s.project, s.project.sequence(seq).expect("the sequence"));
    let walls = effective(&s);
    let before = Snapshot::of(&s);
    let m = ok(&mut s, "vertical.measureCuts", json!({"wait": true}));
    before.unchanged(&s, "measureCuts");
    assert_eq!(effective(&s), walls);
    ok(&mut s, "vertical.setWalls", json!({"useEditPoints": false}));
    let walls = effective(&s);
    ok(&mut s, "vertical.applyMeasurement", json!({"measurement": m["measurement"]}));
    assert_ne!(effective(&s), walls, "applying the measurement writes its walls");
}

#[test]
fn the_wall_rule_agrees_with_filmcraft_edit_on_through_edits() {
    let mut p = Project::new("pairs");
    let pool = MediaPool::default();
    let src = |rgb: [u8; 3]| GeneratorSource::new(matte(rgb), 64, 36, RATE, at(1000));
    let a = crate::demo::add_generator(&mut p, &pool, src([1, 2, 3]), "a", Label::Iris, None);
    let b = crate::demo::add_generator(&mut p, &pool, src([4, 5, 6]), "b", Label::Iris, None);
    let base = p.make_track_item(a, TrackKind::Video, at(20), TimeRange::new(at(10), at(100)), RATE).expect("a clip");
    let variations: [fn(&mut TrackItem); 6] = [
        |_| {},
        |item| item.speed = 2.0,
        |item| item.speed = 0.5,
        |item| item.reverse = true,
        |item| item.frame_hold = Some(at(5)),
        |item| item.multicam = Some(MulticamSel { enabled: true, angle: 1 }),
    ];
    let (mut through, mut cuts) = (0_usize, 0_usize);
    for left_variation in variations {
        for right_variation in variations {
            for media in [a, b] {
                for slack in [-3, -2, -1, 0, 1, 2, 3, 1000] {
                    for gap in [0, 1] {
                        let mut left = base.clone();
                        left_variation(&mut left);
                        let mut right = base.clone();
                        right_variation(&mut right);
                        right.item = media;
                        right.start = left.end() + Tick(gap);
                        let consumed = right.source_out() - right.source_in;
                        right.source_in = if left.reverse { left.source_in + Tick(slack) - consumed } else { left.source_out() + Tick(slack) };
                        let edit = filmcraft_edit::through::is_through_edit(&left, &right);
                        assert_eq!(v::is_through_edit(&left, &right), edit, "{left:?} | {right:?}");
                        through += usize::from(edit);
                        cuts += usize::from(!edit);
                    }
                }
            }
        }
    }
    assert!(through > 0 && cuts > 0, "both kinds of pair are covered: {through} through edits, {cuts} cuts");
}

#[test]
fn the_governing_position_agrees_with_the_caption_anchor_rule() {
    let mut positions = framing_positions();
    positions[0].caption_anchor = Some(CaptionAnchor::Top);
    positions[2].caption_anchor = Some(CaptionAnchor::Bottom);
    positions[4].caption_anchor = Some(CaptionAnchor::Top);
    let mut late = positions.clone();
    late[0].at = at(10);
    let (p, seq) = project_with(&MediaPool::default(), 1920, 1080, &FOUR, None);
    let raster = v::raster(p.sequence(seq).expect("the sequence"));
    for list in [positions, late] {
        let mut set = set_with(list);
        set.plans[0].captions.anchor = CaptionAnchor::Top;
        let plan = &set.plans[0];
        for frame in 0..raster.end_frame {
            let anchor = filmcraft_render::vertical::cue_anchor(plan, &raster, at(frame));
            let governing = v::governing(plan, &raster, frame).and_then(|id| plan.positions.iter().find(|p| p.id == id));
            assert_eq!(anchor, governing.and_then(|p| p.caption_anchor).unwrap_or(plan.captions.anchor), "frame {frame}");
        }
    }
}
