//! The edit hook in the session (plan 63 §3.5, lane L2d; spec §4.8, §4.12, §4.13, §8.3, §10): every 16:9 edit carries
//! its framing. `Session::edit` and the merging branch of `Session::edit_merged` run the model's hook for every sequence
//! with a vertical set (a merged sub-step from its gesture's base, a dynamic trim's live update from the project before
//! the trim), with the picture digests cached per project snapshot and per picture item; the undo label gains the hook's
//! suffix, which takes no part in merging; the session validates the vertical sets after the hook. A person can re-cut a
//! framed sequence (spec §4.13) and remap its framing onto a re-delivered master (`vertical.remap`, spec §4.12).
//!
//! Tests: T-N17 (the hook against FilmCraft's real edits: a table of edits, random sequences of them, edits inside a
//! nest, and dynamic trims), T-N12 (the re-cut route and its cut-and-paste variant), T-N11 (remap end to end after a
//! relink, and its refusals), T-N15's remap retry, the hook audit (every picture edit runs the hook, no `vertical.*` write
//! does, and the engine changes its project only at the reviewed places), T-N19's hook half, T-M14's engine half, the
//! label suffix and its exclusion from merging, validation after the hook, I16 for a person, ids above the high-water
//! marks, and the cost structure of the digest caches (spec §4.8 "Cost"; its timing targets in §10 are reported, not
//! gated).
//!
//! The property never asks the code under test where a frame went: it reads what each frame shows from the timelines
//! alone (the leaf media item and the media time, through nests), and its bound is the model's `follow_bound`, two
//! corrected split bounds per edit (lane L1a round 4). A frame outside the bound is excused only by a flag the edit
//! raised: before each call a person marks the plan checked, which clears its flags. Every edit must also leave a valid
//! vertical set valid. The table, the random cases and the nest edits each run every sub-case and report every failure
//! together, so one build shows them all. Synthetic media only: generator sources in the session's pool.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use filmcraft_edit::Edge;
use filmcraft_media::Generator;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_project::vertical as v;
use filmcraft_project::{ClipId, ItemId, ItemKind, Label, MediaRef, ParamValue, Project, Sequence, SequenceSettings, TrackItem, TrackKind};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use serde_json::{Value, json};

use crate::panels::Level;
use crate::{EngineError, MediaPool, Session, VerticalHook};

// ---------- fixtures ----------

/// Every fixture's rate: 25p, so a frame is 10 160 640 000 ticks.
const RATE: FrameRate = FrameRate::FPS_25;

/// A 9:16 plan's output.
const NINE_SIXTEEN: v::OutputSize = v::OutputSize { width: 1080, height: 1920 };

/// Float noise allowed on top of a bound.
const NOISE: f64 = 1e-9;

/// How deep the oracle follows nested sequences: the renderer's nest depth.
const NEST_DEPTH: u32 = 8;

/// The tick of a frame.
fn at(frame: i64) -> Tick {
    RATE.tick_of(frame)
}

/// The frame of a tick.
fn fr(t: Tick) -> i64 {
    RATE.frame_at(t)
}

/// One call of an edit: a command and its parameters.
type Call = (&'static str, Value);

/// The synthetic media of a fixture: the master every clip is cut from, so that each of its frames shows once at most,
/// and a matte for the content an edit brings in.
#[derive(Clone, Copy)]
struct Media {
    master: ItemId,
    matte: ItemId,
}

/// Both media items of a fixture in `p`, registered in `pool`: 20 000 frames of 1920x1080 25p each, Bars and Tone
/// (picture and sound) and a colour matte (picture only).
fn media(p: &mut Project, pool: &MediaPool) -> Media {
    let source = |generator: Generator| GeneratorSource::new(generator, 1920, 1080, RATE, at(20_000));
    let master = crate::demo::add_generator(p, pool, source(Generator::BarsAndTone), "Master", Label::Iris, None);
    let matte = Generator::ColorMatte { color: [0.6, 0.2, 0.1, 1.0] };
    let matte = crate::demo::add_generator(p, pool, source(matte), "Insert", Label::Mango, None);
    Media { master, matte }
}

/// The settings of every fixture sequence: 1920x1080 at 25p.
fn settings() -> SequenceSettings {
    SequenceSettings { width: 1920, height: 1080, frame_rate: RATE, ..Default::default() }
}

/// A clip of project item `item` for a track of `kind`: frames `[start, start + frames)` showing the item from its frame
/// `source`, its automatic points resolved as a placed clip's are.
fn clip(p: &mut Project, item: ItemId, kind: TrackKind, start: i64, frames: i64, source: i64) -> TrackItem {
    let range = TimeRange::new(at(source), at(frames));
    let mut c = p.make_track_item(item, kind, at(start), range, RATE).expect("a clip of the item");
    for e in &mut c.effects {
        filmcraft_project::resolve_auto_points(e, (1920, 1080), (1920, 1080));
    }
    c
}

/// A cut at `frame` to the window `(cx, cy, h)`, made by a person.
fn cut(id: u32, frame: i64, (cx, cy, h): (f64, f64, f64)) -> v::Position {
    v::Position::new(id, at(frame), v::Window { cx, cy, h }, v::Arrival::Cut, v::Origin::Manual, v::Writer::Human)
}

/// A move of `frames` frames along `curve` arriving at `frame` at the window `(cx, cy, h)`, made by a person.
fn moving(id: u32, frame: i64, (cx, cy, h): (f64, f64, f64), frames: u32, curve: v::Curve) -> v::Position {
    let arrival = v::Arrival::Move { move_frames: frames, curve, split_of: None };
    v::Position::new(id, at(frame), v::Window { cx, cy, h }, arrival, v::Origin::Manual, v::Writer::Human)
}

/// Plan 1 "9:16" holding `positions` (sorted by frame), its counter past the largest id.
fn plan_of(positions: Vec<v::Position>) -> v::VerticalPlan {
    let mut plan = v::VerticalPlan::new(1, "9:16", NINE_SIXTEEN);
    plan.next_position_id = positions.iter().map(|p| p.id).max().unwrap_or(0) + 1;
    plan.positions = positions;
    plan
}

/// A vertical set holding `plan` (picture edit points are walls).
fn set_of(plan: v::VerticalPlan) -> v::VerticalSet {
    let mut set = crate::empty_vertical_set();
    set.plans.push(plan);
    set.next_plan_id = 2;
    set
}

/// Installs `p` in `s` as opening a file does (no history, the id marks from the project), with `seq` active and open.
fn install(s: &mut Session, p: Project, seq: ItemId) {
    s.project = Arc::new(p);
    s.history = crate::History { limit: 200, ..Default::default() };
    s.state.active_sequence = Some(seq);
    s.state.open_sequences = vec![seq];
    s.note_project_replaced();
}

/// The main fixture: sequence "Main" (V1 to V3, A1, A2) with four 150-frame clips of the master on V1, C1 to C4, each
/// linked to its sound on A1 (edit points at 150, 300 and 450, end 600), and a 30-frame overlay of the master on V2 over
/// frames `[300, 330)`; its plan is [`main_plan`].
struct Fixture {
    s: Session,
    seq: ItemId,
    /// C1 to C4 on V1.
    clips: [ClipId; 4],
    /// Their sounds on A1.
    sounds: [ClipId; 4],
    media: Media,
}

fn fixture() -> Fixture {
    let mut s = Session::default();
    let pool = Arc::clone(&s.media);
    let mut p = Project::new("Edit hook fixture");
    let media = media(&mut p, &pool);
    let seq = p.new_sequence("Main", settings(), 3, 2, None);
    let (mut clips, mut sounds) = ([ClipId::default(); 4], [ClipId::default(); 4]);
    for (k, (start, source)) in [(0, 1_000), (150, 3_000), (300, 5_000), (450, 7_000)].into_iter().enumerate() {
        let link = p.alloc_id();
        let mut picture = clip(&mut p, media.master, TrackKind::Video, start, 150, source);
        let mut sound = clip(&mut p, media.master, TrackKind::Audio, start, 150, source);
        (picture.link, sound.link) = (Some(link), Some(link));
        (clips[k], sounds[k]) = (picture.id, sound.id);
        let q = p.sequence_mut(seq).expect("Main");
        q.video_tracks[0].items.push(picture);
        q.audio_tracks[0].items.push(sound);
    }
    let overlay = clip(&mut p, media.master, TrackKind::Video, 300, 30, 19_000);
    let q = p.sequence_mut(seq).expect("Main");
    q.video_tracks[1].items.push(overlay);
    q.vertical = Some(Box::new(set_of(main_plan())));
    install(&mut s, p, seq);
    Fixture { s, seq, clips, sounds, media }
}

/// Main's plan: an opener on every edit point and on the overlay's end, moves along every family of curve (Ramp
/// included) with zooms and pure pans, and an exact split: #12 at 220 divides #5's 80-frame move in two pieces of 40.
fn main_plan() -> v::VerticalPlan {
    let mut plan = plan_of(vec![
        cut(1, 0, (960.0, 540.0, 1080.0)),
        moving(2, 60, (800.0, 540.0, 1000.0), 40, v::Curve::EaseInOut),
        moving(3, 120, (1100.0, 500.0, 900.0), 30, v::DEFAULT_CURVE),
        cut(4, 150, (700.0, 540.0, 1080.0)),
        moving(5, 260, (1000.0, 560.0, 800.0), 80, v::Curve::Linear),
        cut(6, 300, (960.0, 540.0, 1080.0)),
        cut(7, 330, (1250.0, 540.0, 1080.0)),
        moving(8, 420, (1200.0, 520.0, 960.0), 80, v::Curve::EaseIn),
        cut(9, 450, (900.0, 540.0, 1000.0)),
        moving(10, 560, (700.0, 520.0, 960.0), 60, v::Curve::Ramp { r_in: 0.3, r_out: 0.3 }),
        moving(11, 590, (760.0, 520.0, 960.0), 20, v::Curve::EaseOut),
    ]);
    let raster = v::Raster { width: 1920, height: 1080, rate: RATE, end_frame: 600 };
    let split = v::split_move(&plan, &raster, 220).expect("frame 220 lies inside #5's move");
    v::insert_split(&mut plan, &split, 12, v::Origin::Manual, v::Writer::Human);
    plan.next_position_id = 13;
    plan
}

/// The nest fixture: sequence "Inner" (V1: I1 `[0, 100)` and I2 `[100, 300)` of the master, a cut at 100) is nested in
/// "Parent" over `[150, 450)`, between P1 `[0, 150)` and P3 `[450, 600)`. Parent's plan moves across the nested cut (at
/// Parent's frame 250), which is no edit point of Parent; Inner has no vertical set.
struct Nest {
    s: Session,
    parent: ItemId,
    inner: ItemId,
    /// I1 and I2.
    inner_clips: [ClipId; 2],
    /// P1, the nest clip and P3.
    parent_clips: [ClipId; 3],
}

fn nest() -> Nest {
    let mut s = Session::default();
    let pool = Arc::clone(&s.media);
    let mut p = Project::new("Nest fixture");
    let media = media(&mut p, &pool);
    let inner = p.new_sequence("Inner", settings(), 1, 1, None);
    let (i1, i2) = (clip(&mut p, media.master, TrackKind::Video, 0, 100, 13_000), clip(&mut p, media.master, TrackKind::Video, 100, 200, 15_000));
    let inner_clips = [i1.id, i2.id];
    p.sequence_mut(inner).expect("Inner").video_tracks[0].items.extend([i1, i2]);
    let parent = p.new_sequence("Parent", settings(), 2, 1, None);
    let p1 = clip(&mut p, media.master, TrackKind::Video, 0, 150, 11_000);
    let nested = clip(&mut p, inner, TrackKind::Video, 150, 300, 0);
    let p3 = clip(&mut p, media.master, TrackKind::Video, 450, 150, 17_000);
    let parent_clips = [p1.id, nested.id, p3.id];
    let plan = plan_of(vec![
        cut(1, 0, (960.0, 540.0, 1080.0)),
        cut(2, 150, (700.0, 540.0, 1080.0)),
        moving(3, 300, (1000.0, 560.0, 800.0), 80, v::Curve::Linear),
        cut(4, 450, (900.0, 540.0, 1000.0)),
        moving(5, 560, (1200.0, 520.0, 960.0), 60, v::Curve::EaseInOut),
    ]);
    let q = p.sequence_mut(parent).expect("Parent");
    q.video_tracks[0].items.extend([p1, nested, p3]);
    q.vertical = Some(Box::new(set_of(plan)));
    install(&mut s, p, parent);
    Nest { s, parent, inner, inner_clips, parent_clips }
}

/// The one-clip master (T-N12, T-N11): sequence "Master" (V1, A1) holding one 1000-frame clip of the master from its
/// frame 0 (no edit point, so no shot is known), with `set` as its vertical data.
fn one_clip(set: v::VerticalSet) -> (Session, ItemId, Media) {
    let mut s = Session::default();
    let pool = Arc::clone(&s.media);
    let mut p = Project::new("One-clip master");
    let media = media(&mut p, &pool);
    let seq = p.new_sequence("Master", settings(), 1, 1, None);
    let only = clip(&mut p, media.master, TrackKind::Video, 0, 1000, 0);
    let q = p.sequence_mut(seq).expect("Master");
    q.video_tracks[0].items.push(only);
    q.vertical = Some(Box::new(set));
    install(&mut s, p, seq);
    (s, seq, media)
}

/// The re-cut's plan (T-N12): the hook is `[600, 700)`, opened by #5 and holding #6's move; every other frame is the
/// rest of the episode.
fn recut_plan() -> v::VerticalPlan {
    plan_of(vec![
        cut(1, 0, (960.0, 540.0, 1080.0)),
        moving(2, 100, (900.0, 540.0, 1000.0), 50, v::Curve::EaseInOut),
        cut(3, 300, (1100.0, 540.0, 1080.0)),
        moving(4, 500, (800.0, 500.0, 900.0), 100, v::DEFAULT_CURVE),
        cut(5, 600, (1000.0, 540.0, 1080.0)),
        moving(6, 680, (1150.0, 520.0, 960.0), 60, v::Curve::Linear),
        cut(7, 700, (960.0, 540.0, 1080.0)),
        moving(8, 900, (700.0, 540.0, 1000.0), 100, v::Curve::EaseOut),
    ])
}

/// The remap's set (T-N11): #3 is locked on frames the re-delivered master drops, and a person's wall stands at 760 on
/// #5.
fn remap_set() -> v::VerticalSet {
    let mut locked = cut(3, 320, (1100.0, 540.0, 1080.0));
    locked.locked = true;
    let mut set = set_of(plan_of(vec![
        cut(1, 0, (960.0, 540.0, 1080.0)),
        moving(2, 100, (900.0, 540.0, 1000.0), 50, v::Curve::EaseInOut),
        locked,
        moving(4, 500, (800.0, 500.0, 900.0), 100, v::DEFAULT_CURVE),
        cut(5, 760, (1000.0, 540.0, 1080.0)),
        moving(6, 900, (1150.0, 520.0, 960.0), 80, v::Curve::Linear),
    ]));
    set.walls.stored.push(v::StoredWall { at: at(760), origin: v::WallOrigin::Manual, score: None });
    set
}

/// The client re-delivers the master: the clip's media item now shows another file (synthetic: another generator) with
/// the same length, which the editor cannot compare with the old one. A person relinks it with `media.relink`, one
/// undoable edit of the media item through `Session::edit` (relink.rs, `apply`), which runs the edit hook; generator
/// media has no file for that command to link, so the test makes the same edit through `Session::edit` itself.
fn relink(s: &mut Session, item: ItemId) {
    let relinked = s.edit("Link Media", |p, _| {
        if let Some(ItemKind::Media(m)) = p.item_mut(item).map(|i| &mut i.kind) {
            m.media = MediaRef::Generator(Generator::ColorMatte { color: [0.1, 0.5, 0.2, 1.0] });
        }
        Ok(())
    });
    relinked.expect("the relink");
}

// ---------- helpers ----------

/// Runs a command that must succeed; its result.
fn ok(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, p.clone()) {
        Ok(r) => r,
        Err(e) => panic!("`{id}` {p}: {e}"),
    }
}

/// Runs a command that must be refused with a code; the code and its detail.
fn refused(s: &mut Session, id: &str, p: Value) -> (&'static str, Value) {
    match s.execute(id, p.clone()) {
        Err(EngineError::Invalid { code, detail, .. }) => (code, detail),
        other => panic!("`{id}` {p} should be refused with a code: {other:?}"),
    }
}

/// Sequence `seq` of `project`.
fn seq_in(project: &Project, seq: ItemId) -> &Sequence {
    project.sequence(seq).expect("the sequence")
}

/// The vertical set of sequence `seq` of `project`.
fn set_in(project: &Project, seq: ItemId) -> &v::VerticalSet {
    seq_in(project, seq).vertical.as_deref().expect("the sequence has a vertical set")
}

/// Plan 1 of sequence `seq` of `project`.
fn plan_in(project: &Project, seq: ItemId) -> &v::VerticalPlan {
    set_in(project, seq).plans.iter().find(|p| p.id == 1).expect("plan 1")
}

/// Position `id` of `plan`, if it holds one.
fn position(plan: &v::VerticalPlan, id: u32) -> Option<&v::Position> {
    plan.positions.iter().find(|p| p.id == id)
}

/// The label of the last undo step.
fn top_label(s: &Session) -> String {
    s.history.undo.last().map(|step| step.label().to_string()).unwrap_or_default()
}

/// The V1 clip of sequence `seq` that starts at `frame`.
fn clip_at(project: &Project, seq: ItemId, frame: i64) -> ClipId {
    let track = &seq_in(project, seq).video_tracks[0];
    track.items.iter().find(|i| i.start == at(frame)).map(|i| i.id).unwrap_or_else(|| panic!("a V1 clip starts at {frame}"))
}

/// A value as saved, byte for byte.
fn saved<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("it saves")
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

/// Runs `check`, which builds everything it touches and asserts; the message of its panic when it failed. The property
/// tests run every sub-case through it and fail once at the end with every message, so one build reports them all.
fn caught<T>(check: impl FnOnce() -> T) -> Result<T, String> {
    // nothing a failed check touched is used after its panic
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(check)).map_err(message_of)
}

/// The message of a caught panic: the text that `panic!` and `assert!` give it.
fn message_of(payload: Box<dyn std::any::Any + Send>) -> String {
    let text = payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|s| String::from(*s)));
    text.unwrap_or_else(|| String::from("a panic without a message"))
}

/// What a call that records nothing leaves as it was: the project, both revisions, the history and the hook's runs.
struct Unchanged {
    project: Arc<Project>,
    revisions: (u64, u64),
    steps: (usize, usize),
    runs: u64,
}

impl Unchanged {
    fn of(s: &Session) -> Self {
        let steps = (s.history.undo.len(), s.history.redo.len());
        Self { project: Arc::clone(&s.project), revisions: (s.revision, s.picture_revision), steps, runs: s.vertical_hook.runs }
    }

    fn check(&self, s: &Session, what: &str) {
        assert!(Arc::ptr_eq(&self.project, &s.project), "{what} changed the project");
        assert_eq!((s.revision, s.picture_revision), self.revisions, "{what} moved a revision");
        assert_eq!((s.history.undo.len(), s.history.redo.len()), self.steps, "{what} recorded a step");
        assert_eq!(s.vertical_hook.runs, self.runs, "{what} ran the edit hook");
    }
}

/// Sets the static value of parameter `param` of effect `effect` of `item`.
fn set_param(item: &mut TrackItem, effect: &str, param: &str, value: f64) {
    let p = item.effect_mut(effect).and_then(|e| e.param_mut(param)).expect("the parameter exists");
    p.value = ParamValue::Float(value);
}

/// A new instance of the effect `id`.
fn effect(id: &str) -> filmcraft_project::EffectInstance {
    filmcraft_project::find_effect(id).expect("a known effect").instance()
}

/// The marks `[from, to]` on the active sequence, then `cmd` (lift or extract).
fn marked(from: i64, to: i64, cmd: &'static str) -> Vec<Call> {
    vec![("markers.markIn", json!({"frame": from})), ("markers.markOut", json!({"frame": to})), (cmd, json!({}))]
}

/// The ids the edit hook added, by its report `report`, over every sequence and plan.
fn added_ids(report: Option<&v::FollowReport>) -> Vec<u32> {
    report.into_iter().flat_map(|r| &r.sequences).flat_map(|q| &q.plans).flat_map(|p| p.added.iter().copied()).collect()
}

/// The Vertical Edit's warnings in the Events log that name position `id`.
fn warnings_on(s: &Session, id: u32) -> Vec<String> {
    let name = format!("#{id} ");
    let lines = s.log.entries.iter().filter(|e| e.level == Level::Warning && e.source == "vertical" && e.message.contains(&name));
    lines.map(|e| e.message.clone()).collect()
}

// ---------- the oracle ----------

/// What a frame shows on top: the leaf media item and the media time it shows (ticks).
type Shown = (ItemId, i64);

/// What sequence `seq` of `project` shows at tick `t`: the topmost enabled clip of an enabled video track that shows
/// picture, followed into nested sequences (as deep as the renderer goes); where a nest shows nothing, the tracks below
/// it show through.
fn shown_at(project: &Project, seq: &Sequence, t: Tick, depth: u32) -> Option<Shown> {
    for track in seq.video_tracks.iter().rev().filter(|track| track.enabled) {
        let Some(item) = track.item_at(t).filter(|item| item.enabled) else { continue };
        let time = item.source_time_at(t);
        match project.item(item.item).map(|i| &i.kind) {
            Some(ItemKind::Sequence(nested)) if depth < NEST_DEPTH => {
                if let Some(found) = shown_at(project, nested, time, depth + 1) {
                    return Some(found);
                }
            }
            Some(ItemKind::Media(m)) if m.info.has_video() => return Some((item.item, time.0)),
            _ => {}
        }
    }
    None
}

/// What each frame of sequence `seq` of `project` shows.
fn frames_shown(project: &Project, seq: ItemId) -> Vec<Option<Shown>> {
    let q = seq_in(project, seq);
    (0..v::raster(q).end_frame).map(|f| shown_at(project, q, at(f), 0)).collect()
}

/// For each content: how many frames show it, and the first of them.
fn census(frames: &[Option<Shown>]) -> BTreeMap<Shown, (usize, i64)> {
    let mut out: BTreeMap<Shown, (usize, i64)> = BTreeMap::new();
    for (f, shown) in (0_i64..).zip(frames) {
        if let Some(key) = shown {
            out.entry(*key).or_insert((0, f)).0 += 1;
        }
    }
    out
}

/// The largest change of `cx`, `cy` or `h` between two windows.
fn change(a: &v::Window, b: &v::Window) -> f64 {
    (a.cx - b.cx).abs().max((a.cy - b.cy).abs()).max((a.h - b.h).abs())
}

/// Every flag the edit hook raises.
const FLAGS: [v::Flag; 3] = [v::Flag::PictureChanged, v::Flag::MoveShortened, v::Flag::TimeApproximate];

/// Whether position `id` of `after` carries one of `flags` that its namesake in `before` does not: a flag the edit
/// raised. Flags persist until a write to the position or `vertical.markChecked`, so one raised before excuses nothing.
fn raised_flag(before: &v::VerticalPlan, after: &v::VerticalPlan, id: u32, flags: &[v::Flag]) -> bool {
    let was = position(before, id).map(|p| p.flags.as_slice()).unwrap_or_default();
    position(after, id).is_some_and(|p| p.flags.iter().any(|f| flags.contains(f) && !was.contains(f)))
}

/// Whether a position that the sample after the edit names (the governing one, or the two ends of the move it lies in)
/// carries a flag the edit raised.
fn flagged(before: &v::VerticalPlan, after: &v::VerticalPlan, sample: &v::Sample) -> bool {
    [sample.position, sample.next].into_iter().flatten().any(|id| raised_flag(before, after, id, &FLAGS))
}

/// Whether the edit shortened the move a frame lay in before it (`sample`, on the old plan): its arrival gained
/// `MoveShortened`. The frames the move no longer covers hold the window before it, so the position that governs them
/// after the edit may carry no flag (spec §4.8 step 8 flags the shortened position).
fn shortened(before: &v::VerticalPlan, after: &v::VerticalPlan, sample: &v::Sample) -> bool {
    let arrival = sample.next.filter(|_| sample.phase == v::Phase::Move);
    arrival.is_some_and(|id| raised_flag(before, after, id, &[v::Flag::MoveShortened]))
}

/// What [`assert_follows`] compared: frames whose window stayed within the bound, and frames a flag excused.
#[derive(Debug, Default)]
struct Followed {
    kept: usize,
    flagged: usize,
}

/// T-N17's property between plan 1 of sequence `seq0` of `before` and plan 1 of sequence `seq1` of `after` (one sequence
/// across `edits` hook runs, or a sequence and its re-cut copy): every frame of `after` whose content (the leaf media and
/// the media time) `before` showed, each exactly once, shows within `edits` hook bounds the window that `before` showed
/// there (spec §4.8: two corrected split bounds per edit), in the same layout, or a position governing it carries a flag
/// the edits raised ([`flagged`]), or they shortened the move it lay in ([`shortened`]).
fn assert_follows(before: &Project, seq0: ItemId, after: &Project, seq1: ItemId, edits: u64, what: &str) -> Followed {
    let (q0, q1) = (seq_in(before, seq0), seq_in(after, seq1));
    let (r0, r1) = (v::raster(q0), v::raster(q1));
    let (p0, p1) = (plan_in(before, seq0), plan_in(after, seq1));
    let (old, new) = (frames_shown(before, seq0), frames_shown(after, seq1));
    let (was, now) = (census(&old), census(&new));
    let mut seen = Followed::default();
    for (g, key) in (0_i64..).zip(&new) {
        let Some(key) = key else { continue };
        let (Some(&(1, f)), Some(&(1, _))) = (was.get(key), now.get(key)) else { continue };
        let (a, b) = (v::window_at(p0, &r0, at(f)), v::window_at(p1, &r1, at(g)));
        let moved = change(&a.window, &b.window);
        let bound = v::follow_bound(p0, &r0, f) * edits.max(1) as f64;
        if moved <= bound + NOISE && a.layout == b.layout {
            seen.kept += 1;
            continue;
        }
        assert!(
            flagged(p0, p1, &b) || shortened(p0, p1, &a),
            "{what}: old frame {f} -> new frame {g} moved {moved} px (bound {bound}), and no flag the edit raised excuses it: {a:?} -> {b:?}"
        );
        seen.flagged += 1;
    }
    seen
}

/// What one call of an edit did.
#[derive(Debug)]
enum Outcome {
    /// The command refused: nothing changed.
    Refused,
    /// The command changed no project data (an editor-state command).
    Unchanged,
    /// One undo step: what the property compared.
    Applied(Followed),
}

/// Runs one call on `s` and checks it (T-N17). When plan 1 of sequence `planned` carries flags, a person first marks it
/// checked (`vertical.markChecked` clears them), so every flag the call leaves was raised by it. A refused call changes
/// nothing; a call that edits the project ran the edit hook, is one undo step, keeps the framing of plan 1
/// ([`assert_follows`]), leaves a valid vertical set valid, and its undo restores the project bit for bit (the 16:9 edit
/// and the framing in one snapshot) while its redo restores the edit with its framing.
fn edit_and_check(s: &mut Session, planned: ItemId, cmd: &str, p: &Value, what: &str) -> Outcome {
    if plan_in(&s.project, planned).positions.iter().any(|x| !x.flags.is_empty()) {
        ok(s, "vertical.markChecked", json!({"sequence": planned.0, "plan": 1}));
    }
    let (pre, steps, runs) = (Arc::clone(&s.project), s.history.undo.len(), s.vertical_hook.runs);
    if let Err(e) = s.execute(cmd, p.clone()) {
        assert!(Arc::ptr_eq(&pre, &s.project), "{what}: `{cmd}` {p} was refused ({e}) and changed the project");
        return Outcome::Refused;
    }
    let post = Arc::clone(&s.project);
    if Arc::ptr_eq(&pre, &post) {
        return Outcome::Unchanged;
    }
    let ran = s.vertical_hook.runs.saturating_sub(runs);
    assert!(ran >= 1, "{what}: `{cmd}` {p} edited the project and the edit hook did not run");
    assert_eq!(s.history.undo.len(), steps + 1, "{what}: `{cmd}` {p} is one undo step");
    let seen = assert_follows(&pre, planned, &post, planned, ran, &format!("{what}: `{cmd}` {p}"));
    // the hook re-checks every move against the walls after the edit (spec §4.8 steps 8 and 12): none holds a wall (I6)
    if v::validate(&pre, seq_in(&pre, planned)).is_empty() {
        let broken = v::validate(&post, seq_in(&post, planned));
        assert!(broken.is_empty(), "{what}: `{cmd}` {p} left the vertical set invalid: {broken:?}");
    }
    s.undo();
    assert!(Arc::ptr_eq(&pre, &s.project), "{what}: undoing `{cmd}` restores the 16:9 edit and the framing bit for bit");
    s.redo();
    assert!(Arc::ptr_eq(&post, &s.project), "{what}: redoing `{cmd}` restores the edit and its framing");
    Outcome::Applied(seen)
}

/// Runs every call of an edit with [`edit_and_check`]; each must apply or change nothing, and the last must apply. What
/// the property compared, over every call.
fn apply_edit(s: &mut Session, planned: ItemId, what: &str, calls: &[Call]) -> Followed {
    let (mut seen, mut last) = (Followed::default(), Outcome::Unchanged);
    for (cmd, p) in calls {
        last = edit_and_check(s, planned, cmd, p, what);
        match &last {
            Outcome::Refused => panic!("{what}: `{cmd}` {p} was refused"),
            Outcome::Applied(one) => {
                seen.kept += one.kept;
                seen.flagged += one.flagged;
            }
            Outcome::Unchanged => {}
        }
    }
    assert!(matches!(last, Outcome::Applied(_)), "{what}: the edit's last call did not apply: {last:?}");
    seen
}

// ---------- T-N17: the hook against FilmCraft's real edits ----------

/// FilmCraft's real edits on [`fixture`], each once (T-N17), with the frames each may leave to a flag: ripples across
/// tracks, razor, moves (insert and to another track), slide, slip, roll, trims of heads and tails (with the timeline's
/// commands and in trim mode), extract, lift, replace, speed changes, a frame hold, through edits, a transition, linked
/// audio and a framing change. Only the frame hold leaves frames to a flag, 19 of them: its cut at 240 is a wall inside
/// the move of #5 (the held piece is no through edit, spec §4.1), so step 8 of §4.8 shortens that move to start there,
/// flagged `MoveShortened`, and old frames 221 to 239 hold the window of #12 (the frame hold test below). A framing
/// change moves no window (it only flags), and every other edit keeps the window of every frame it still shows (spec
/// §4.8's guarantee; the frames of a new or an extended head show new content, which the property does not compare).
fn real_edits(f: &Fixture) -> Vec<(&'static str, usize, Vec<Call>)> {
    let [c1, c2, c3, c4] = f.clips.map(|c| c.0);
    let a2 = f.sounds[1].0;
    let matte = f.media.matte.0;
    let place = |frame: i64, frames: i64, insert: bool| json!({"item": matte, "track": "V1", "frame": frame, "insert": insert, "sourceIn": 0, "duration": at(frames).0});
    let point = |clip: u64, edge: &str, kind: &str| ("trim.selectEditPoint", json!({"clip": clip, "edge": edge, "kind": kind}));
    vec![
        ("ripple insert inside a move", 0, vec![("timeline.place", place(240, 20, true))]),
        ("ripple delete of a clip and its sound", 0, vec![("edit.rippleDelete", json!({"clips": [c2]}))]),
        ("ripple delete of the first clip: the overlay on V2 moves too", 0, vec![("edit.rippleDelete", json!({"clips": [c1]}))]),
        ("razor inside a move", 0, vec![("timeline.razor", json!({"time": at(240).0, "track": "V1"}))]),
        ("insert move to the head", 0, vec![("timeline.move", json!({"moves": [{"clip": c3, "track": "V1", "time": 0}], "insert": true}))]),
        ("move to another track", 0, vec![("timeline.move", json!({"moves": [{"clip": c4, "track": "V3", "time": at(450).0}]}))]),
        ("lift a clip, then close its gap", 0, vec![("edit.clear", json!({"clips": [c2]})), ("sequence.closeGap", json!({"track": "V1", "time": at(200).0}))]),
        ("slide", 0, vec![("timeline.slide", json!({"clip": c2, "deltaFrames": 12}))]),
        ("slip", 0, vec![("timeline.slip", json!({"clip": c2, "deltaFrames": -9}))]),
        ("roll", 0, vec![("timeline.roll", json!({"left": c1, "right": c2, "deltaFrames": -8}))]),
        ("ripple trim of a head", 0, vec![("timeline.trim", json!({"clip": c4, "edge": "in", "mode": "ripple", "deltaFrames": 10}))]),
        ("ripple trim of a tail", 0, vec![("timeline.trim", json!({"clip": c2, "edge": "out", "mode": "ripple", "deltaFrames": -25}))]),
        ("trim of a tail, leaving a gap", 0, vec![("timeline.trim", json!({"clip": c1, "edge": "out", "mode": "regular", "deltaFrames": -12}))]),
        ("trim mode: a ripple trim of a head", 0, vec![point(c4, "in", "ripple"), ("trim.nudge", json!({"frames": 6}))]),
        ("trim mode: a roll", 0, vec![point(c1, "out", "roll"), ("trim.nudge", json!({"frames": 4}))]),
        ("extract inside a move", 0, marked(200, 214, "sequence.extract")),
        ("lift inside a move", 0, marked(230, 239, "sequence.lift")),
        ("replace part of a clip", 0, vec![("timeline.place", place(460, 30, false))]),
        ("speed 200 % with ripple", 0, vec![("clip.speedDuration", json!({"clips": [c2], "speed": 200, "ripple": true}))]),
        ("speed 50 % with ripple", 0, vec![("clip.speedDuration", json!({"clips": [c4], "speed": 50, "ripple": true}))]),
        ("frame hold inside a move", 19, vec![("clip.frameHold", json!({"clips": [c2], "time": at(240).0}))]),
        ("through edit", 0, vec![("sequence.addEdit", json!({"time": at(390).0}))]),
        ("join a through edit", 0, vec![("timeline.razor", json!({"time": at(390).0, "track": "V1"})), ("sequence.joinThroughEdits", json!({"all": true}))]),
        ("video transition at a cut", 0, vec![("sequence.applyVideoTransition", json!({"clip": c2, "edge": "in", "frames": 10}))]),
        (
            "linked audio trimmed alone",
            0,
            vec![
                ("sequence.linkedSelection", json!({"on": false})),
                ("timeline.trim", json!({"clip": a2, "edge": "out", "mode": "regular", "deltaFrames": -10})),
            ],
        ),
        ("Motion changed: framing only", 0, vec![("effects.setParam", json!({"clip": c2, "effect": "motion", "param": "scale", "value": 120.0}))]),
    ]
}

#[test]
fn t_n17_every_real_edit_keeps_the_window_of_every_frame_it_still_shows_or_flags_it() {
    let edits = real_edits(&fixture());
    let mut failures = Vec::new();
    for (what, excused, calls) in edits {
        let checked = caught(move || {
            let mut f = fixture();
            let start = Arc::clone(&f.s.project);
            let seen = apply_edit(&mut f.s, f.seq, what, &calls);
            assert!(seen.kept > 0 && seen.flagged == excused, "{what}: {seen:?}, while {excused} frames may be left to a flag");
            while f.s.history.can_undo() {
                f.s.undo();
            }
            assert!(Arc::ptr_eq(&start, &f.s.project), "{what}: undoing it all restores the fixture bit for bit");
        });
        failures.extend(checked.err());
    }
    assert!(failures.is_empty(), "{} real edits failed:\n{}", failures.len(), failures.join("\n"));
}

/// A deterministic generator for the property tests (SplitMix64): the build runner passes no PROPTEST variables, so the
/// cases are fixed in code (spec §11).
struct Rng(u64);

impl Rng {
    fn draw(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A whole number in `[from, to)`; `from` when the range is empty.
    fn within(&mut self, from: i64, to: i64) -> i64 {
        let span = to.saturating_sub(from);
        if span <= 0 {
            return from;
        }
        from.saturating_add(i64::try_from(self.draw() % span.unsigned_abs()).unwrap_or(0))
    }

    /// A whole number in `[from, to)` or in `(-to, -from]`.
    fn signed(&mut self, from: i64, to: i64) -> i64 {
        let x = self.within(from, to);
        if self.coin() { -x } else { x }
    }

    fn coin(&mut self) -> bool {
        self.draw() & 1 == 1
    }

    /// One of `items`; None when there is none.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        let n = u64::try_from(items.len()).ok().filter(|n| *n > 0)?;
        items.get(usize::try_from(self.draw() % n).ok()?)
    }
}

/// The clips of V1 of the active sequence: id, first frame and end frame, in time order.
fn v1(s: &Session) -> Vec<(u64, i64, i64)> {
    let Some(track) = s.active_sequence().and_then(|q| q.video_tracks.first()) else { return Vec::new() };
    track.items.iter().map(|i| (i.id.0, fr(i.start), fr(i.end()))).collect()
}

/// The V1 clips of the active sequence longer than two frames.
fn long_clips(s: &Session) -> Vec<(u64, i64, i64)> {
    v1(s).into_iter().filter(|(_, a, b)| b - a > 2).collect()
}

/// The end frame of the active sequence.
fn end_of(s: &Session) -> i64 {
    s.active_sequence().map_or(0, |q| v::raster(q).end_frame)
}

/// A random instance of one kind of edit on the active sequence; None when its timeline has nothing to apply it to.
type Generate = fn(&Session, &Media, &mut Rng) -> Option<Vec<Call>>;

/// A clip of the matte placed at a random frame of the active sequence, inserted or over what is there.
fn gen_place(s: &Session, media: &Media, rng: &mut Rng, insert: bool) -> Option<Vec<Call>> {
    let end = end_of(s);
    if end <= 2 {
        return None;
    }
    let (frame, frames) = (rng.within(1, end), rng.within(5, 40));
    Some(vec![("timeline.place", json!({"item": media.matte.0, "track": "V1", "frame": frame, "insert": insert, "sourceIn": 0, "duration": at(frames).0}))])
}

fn gen_insert(s: &Session, media: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    gen_place(s, media, rng, true)
}

fn gen_replace(s: &Session, media: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    gen_place(s, media, rng, false)
}

fn gen_ripple_delete(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let (clip, ..) = *rng.pick(&v1(s))?;
    Some(vec![("edit.rippleDelete", json!({"clips": [clip]}))])
}

fn gen_razor(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let (_, a, b) = *rng.pick(&long_clips(s))?;
    Some(vec![("timeline.razor", json!({"time": at(rng.within(a + 1, b)).0, "track": "V1"}))])
}

fn gen_insert_move(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let clips = v1(s);
    let (clip, ..) = *rng.pick(&clips)?;
    let (_, to, _) = *rng.pick(&clips)?;
    Some(vec![("timeline.move", json!({"moves": [{"clip": clip, "track": "V1", "time": at(to).0}], "insert": true}))])
}

fn gen_close_gap(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let clips = v1(s);
    let head = clips.first().filter(|c| c.1 > 0).map(|c| (0, c.1));
    let gaps: Vec<(i64, i64)> = head.into_iter().chain(clips.windows(2).filter(|w| w[0].2 < w[1].1).map(|w| (w[0].2, w[1].1))).collect();
    let (a, b) = *rng.pick(&gaps)?;
    Some(vec![("sequence.closeGap", json!({"track": "V1", "time": at(a + (b - a) / 2).0}))])
}

fn gen_slide(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let clips = v1(s);
    let inner = clips.get(1..clips.len().saturating_sub(1))?;
    let (clip, ..) = *rng.pick(inner)?;
    Some(vec![("timeline.slide", json!({"clip": clip, "deltaFrames": rng.signed(1, 20)}))])
}

fn gen_slip(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let (clip, ..) = *rng.pick(&v1(s))?;
    Some(vec![("timeline.slip", json!({"clip": clip, "deltaFrames": rng.signed(1, 30)}))])
}

fn gen_roll(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let pairs: Vec<(u64, u64)> = v1(s).windows(2).filter(|w| w[0].2 == w[1].1).map(|w| (w[0].0, w[1].0)).collect();
    let (left, right) = *rng.pick(&pairs)?;
    Some(vec![("timeline.roll", json!({"left": left, "right": right, "deltaFrames": rng.signed(1, 20)}))])
}

fn gen_trim(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let (clip, ..) = *rng.pick(&v1(s))?;
    let edge = if rng.coin() { "in" } else { "out" };
    let mode = if rng.coin() { "ripple" } else { "regular" };
    Some(vec![("timeline.trim", json!({"clip": clip, "edge": edge, "mode": mode, "deltaFrames": rng.signed(1, 30)}))])
}

/// Marks a random range of up to 30 frames on the active sequence, then runs `cmd` on it.
fn gen_range(s: &Session, rng: &mut Rng, cmd: &'static str) -> Option<Vec<Call>> {
    let end = end_of(s);
    if end <= 4 {
        return None;
    }
    let from = rng.within(1, end - 2);
    let to = rng.within(from, (from + 30).min(end - 1));
    Some(marked(from, to, cmd))
}

fn gen_extract(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    gen_range(s, rng, "sequence.extract")
}

fn gen_lift(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    gen_range(s, rng, "sequence.lift")
}

fn gen_speed(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let (clip, ..) = *rng.pick(&v1(s))?;
    let speed = *rng.pick(&[50.0, 150.0, 200.0])?;
    Some(vec![("clip.speedDuration", json!({"clips": [clip], "speed": speed, "ripple": true}))])
}

fn gen_frame_hold(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let (clip, a, b) = *rng.pick(&long_clips(s))?;
    Some(vec![("clip.frameHold", json!({"clips": [clip], "time": at(rng.within(a + 1, b)).0}))])
}

fn gen_through_edit(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let (_, a, b) = *rng.pick(&long_clips(s))?;
    Some(vec![("sequence.addEdit", json!({"time": at(rng.within(a + 1, b)).0}))])
}

fn gen_join(_: &Session, _: &Media, _: &mut Rng) -> Option<Vec<Call>> {
    Some(vec![("sequence.joinThroughEdits", json!({"all": true}))])
}

fn gen_transition(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let incoming: Vec<u64> = v1(s).windows(2).filter(|w| w[0].2 == w[1].1).map(|w| w[1].0).collect();
    let clip = *rng.pick(&incoming)?;
    Some(vec![("sequence.applyVideoTransition", json!({"clip": clip, "edge": "in", "frames": rng.within(4, 13)}))])
}

fn gen_linked_audio(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let sounds: Vec<u64> = s.active_sequence()?.audio_tracks.first()?.items.iter().map(|i| i.id.0).collect();
    let clip = *rng.pick(&sounds)?;
    Some(vec![
        ("sequence.linkedSelection", json!({"on": false})),
        ("timeline.trim", json!({"clip": clip, "edge": "out", "mode": "regular", "deltaFrames": -rng.within(1, 20)})),
        ("sequence.linkedSelection", json!({"on": true})),
    ])
}

fn gen_motion(s: &Session, _: &Media, rng: &mut Rng) -> Option<Vec<Call>> {
    let (clip, ..) = *rng.pick(&v1(s))?;
    Some(vec![("effects.setParam", json!({"clip": clip, "effect": "motion", "param": "scale", "value": rng.within(80, 131)}))])
}

/// The kinds of real edit the random sequences draw from.
const GENERATORS: [(&str, Generate); 19] = [
    ("ripple insert", gen_insert),
    ("replace", gen_replace),
    ("ripple delete", gen_ripple_delete),
    ("razor", gen_razor),
    ("insert move", gen_insert_move),
    ("close gap", gen_close_gap),
    ("slide", gen_slide),
    ("slip", gen_slip),
    ("roll", gen_roll),
    ("trim", gen_trim),
    ("extract", gen_extract),
    ("lift", gen_lift),
    ("speed", gen_speed),
    ("frame hold", gen_frame_hold),
    ("through edit", gen_through_edit),
    ("join through edits", gen_join),
    ("transition", gen_transition),
    ("linked audio", gen_linked_audio),
    ("Motion", gen_motion),
];

/// The calls a generator makes around its real edit: the marks of a range, and the linked selection switched off and on.
const SETUP: [&str; 3] = ["markers.markIn", "markers.markOut", "sequence.linkedSelection"];

/// What T-N17's random cases drew: the real edits, the ones that applied, and the others with their outcome.
#[derive(Debug, Default)]
struct Drawn {
    drawn: u64,
    applied: u64,
    missed: Vec<String>,
}

#[test]
fn t_n17_random_sequences_of_real_edits_keep_the_framing_and_undo_restores_it() {
    const CASES: u64 = 12;
    let (mut all, mut failures) = (Drawn::default(), Vec::new());
    for case in 0..CASES {
        match caught(move || random_case(case)) {
            Ok(one) => {
                all.drawn += one.drawn;
                all.applied += one.applied;
                all.missed.extend(one.missed);
            }
            Err(failure) => failures.push(failure),
        }
    }
    assert!(failures.is_empty(), "{} of {CASES} random cases failed:\n{}", failures.len(), failures.join("\n"));
    // a real edit that is refused or changes nothing tests nothing: most of those drawn must apply
    assert!(all.applied * 2 >= all.drawn, "{} of {} real edits applied; the others: {:#?}", all.applied, all.drawn, all.missed);
}

/// T-N17's random case `case`: eight random real edits on a fresh [`fixture`], each call checked by [`edit_and_check`],
/// then every step undone, which restores the fixture bit for bit. What it drew: a step's real edit is its generator's
/// last call that sets no mark and switches no linked selection, and the calls around it must not be refused.
fn random_case(case: u64) -> Drawn {
    const STEPS: usize = 8;
    let mut f = fixture();
    let start = Arc::clone(&f.s.project);
    let mut rng = Rng(0x5EED_0D17_u64.wrapping_add(case.wrapping_mul(0x9E37_79B9)));
    let mut drawn = Drawn::default();
    for step in 0..STEPS {
        let Some(&(name, generate)) = rng.pick(&GENERATORS) else { continue };
        let Some(calls) = generate(&f.s, &f.media, &mut rng) else { continue };
        let what = format!("case {case}, step {step} ({name})");
        let real = calls.iter().rposition(|(cmd, _)| !SETUP.contains(cmd));
        drawn.drawn += 1;
        for (k, (cmd, p)) in calls.iter().enumerate() {
            let outcome = edit_and_check(&mut f.s, f.seq, cmd, p, &what);
            if Some(k) != real {
                assert!(!matches!(outcome, Outcome::Refused), "{what}: the preparing call `{cmd}` {p} was refused");
            } else if matches!(outcome, Outcome::Applied(_)) {
                drawn.applied += 1;
            } else {
                drawn.missed.push(format!("{what}: `{cmd}` {p}: {outcome:?}"));
            }
        }
    }
    while f.s.history.can_undo() {
        f.s.undo();
    }
    assert!(Arc::ptr_eq(&start, &f.s.project), "case {case}: undoing every step restores the fixture bit for bit");
    drawn
}

#[test]
fn t_n17_edits_inside_a_nest_are_followed_by_the_sequence_that_shows_it() {
    // the hook follows every sequence with a vertical set, not only the edited one: Parent follows edits of Inner
    let probe = nest();
    let ([i1, i2], [p1, ..]) = (probe.inner_clips.map(|c| c.0), probe.parent_clips.map(|c| c.0));
    let edits: [(&str, bool, Vec<Call>); 7] = [
        ("extract inside the nest", true, marked(140, 159, "sequence.extract")),
        ("slip inside the nest", true, vec![("timeline.slip", json!({"clip": i2, "deltaFrames": 10}))]),
        ("ripple delete inside the nest", true, vec![("edit.rippleDelete", json!({"clips": [i1]}))]),
        ("razor inside the nest", true, vec![("timeline.razor", json!({"time": at(200).0, "track": "V1"}))]),
        ("speed inside the nest", true, vec![("clip.speedDuration", json!({"clips": [i2], "speed": 200, "ripple": true}))]),
        ("razor of the nest clip in the parent", false, vec![("timeline.razor", json!({"time": at(250).0, "track": "V1"}))]),
        ("ripple delete before the nest in the parent", false, vec![("edit.rippleDelete", json!({"clips": [p1]}))]),
    ];
    let mut failures = Vec::new();
    for (what, inside, calls) in edits {
        let checked = caught(move || {
            let mut n = nest();
            if inside {
                ok(&mut n.s, "sequence.open", json!({"item": n.inner.0}));
            }
            apply_edit(&mut n.s, n.parent, what, &calls);
            if what == "extract inside the nest" {
                // Parent's frames 290 to 309 showed Inner's 140 to 159: the junction at 290 lies inside the nest clip,
                // where Parent has no edit point, so the hook stores a wall there
                let walls = &set_in(&n.s.project, n.parent).walls;
                let stored: Vec<i64> = walls.stored.iter().filter(|w| w.origin == v::WallOrigin::Reanchored).map(|w| fr(w.at)).collect();
                assert!(stored.contains(&290), "{what}: a reanchored wall at Parent's frame 290: {stored:?}");
            }
        });
        failures.extend(checked.err());
    }
    assert!(failures.is_empty(), "{} nest edits failed:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn t_n17_a_framing_change_that_makes_a_through_edit_a_wall_shortens_the_move_across_it_in_that_edit() {
    // random case 7: Add Edit at 578, inside #11's move (570 to 590), then a punch-out on the right piece. The pieces now
    // differ in Motion, so the through edit is a shot change (spec §4.1) and the framing change moved a wall: the hook
    // shortens the move to start at the wall and flags it in that same edit (W5, §4.8 step 8), so the set stays valid
    // (I6), and a lift far from it later finds nothing left to do there
    let mut f = fixture();
    let (seq, start) = (f.seq, Arc::clone(&f.s.project));
    apply_edit(&mut f.s, seq, "Add Edit inside #11's move", &[("timeline.razor", json!({"time": at(578).0, "track": "V1"}))]);
    assert_eq!(set_in(&f.s.project, seq), set_in(&start, seq), "a through edit is no wall and changes no framing");
    let (right, pre) = (clip_at(&f.s.project, seq, 578), Arc::clone(&f.s.project));
    ok(&mut f.s, "effects.setParam", json!({"clip": right.0, "effect": "motion", "param": "scale", "value": 87}));
    let post = Arc::clone(&f.s.project);
    let walls: Vec<i64> = v::walls(&post, seq_in(&post, seq)).iter().map(|w| fr(w.at)).collect();
    assert!(walls.contains(&578), "the pieces differ in Motion: a wall at 578: {walls:?}");
    let shortened = v::Arrival::Move { move_frames: 12, curve: v::Curve::EaseOut, split_of: None };
    let p11 = position(plan_in(&post, seq), 11).expect("#11");
    assert_eq!(p11.arrival, shortened, "#11's move starts at the wall");
    assert!(p11.flags.contains(&v::Flag::MoveShortened), "{:?}", p11.flags);
    let broken = v::validate(&post, seq_in(&post, seq));
    assert!(broken.is_empty(), "no move holds a wall: {broken:?}");
    // the old move's frames before the wall now hold #10's window and those after it follow the shorter move: frames 571
    // to 589 changed in this edit, each governed by a flagged position, and every other frame kept its window
    let seen = assert_follows(&pre, seq, &post, seq, 1, "a punch-out on the right piece");
    assert_eq!((seen.kept, seen.flagged), (581, 19), "{seen:?}");
    // the report (read before any undo, which clears it): #11 shortened, and #10 flagged, as its hold now reaches frame
    // 578, whose picture changed
    let report = f.s.hook_report().cloned().expect("the hook's report");
    let entry = report.sequences.iter().find(|q| q.sequence == seq).expect("Main's entry");
    let plan = entry.plans.iter().find(|p| p.plan == 1).expect("plan 1's entry");
    assert_eq!(plan.flagged, vec![10, 11], "{plan:?}");
    assert!(plan.moved.is_empty() && plan.removed.is_empty() && plan.added.is_empty(), "{plan:?}");
    let change = json!({"plan": 1, "position": 11, "frames": [20, 12]});
    assert!(entry.changes.iter().any(|c| c.code == "move_shortened" && c.detail == change), "{:?}", entry.changes);
    assert_eq!(top_label(&f.s), "Change Effect Parameter \u{b7} vertical: 2 flagged");
    // a lift far from the wall leaves #11 as the punch-out left it
    apply_edit(&mut f.s, seq, "a lift far from the wall", &marked(140, 154, "sequence.lift"));
    assert_eq!(position(plan_in(&f.s.project, seq), 11).map(|p| p.arrival), Some(shortened));
    while f.s.history.can_undo() {
        f.s.undo();
    }
    assert!(Arc::ptr_eq(&start, &f.s.project), "undoing it all restores the fixture bit for bit");
}

#[test]
fn t_n17_a_frame_hold_inside_a_move_is_a_wall_that_shortens_the_move_in_that_edit() {
    // Add Frame Hold at 240 cuts C2 there and holds the right piece on its first frame. That cut is no through edit
    // (the frame hold differs, spec §4.1), so it is a wall inside #5's move (220 to 260). The held piece maps by its
    // offset (§4.8 step 2), so the map runs on unbroken and no junction splits the move: step 8 shortens it to start at
    // the wall and flags it, and the set stays valid (I6)
    let mut f = fixture();
    let (seq, c2, pre) = (f.seq, f.clips[1].0, Arc::clone(&f.s.project));
    ok(&mut f.s, "clip.frameHold", json!({"clips": [c2], "time": at(240).0}));
    let post = Arc::clone(&f.s.project);
    let walls: Vec<i64> = v::walls(&post, seq_in(&post, seq)).iter().map(|w| fr(w.at)).collect();
    assert!(walls.contains(&240), "the held piece starts a shot: a wall at 240: {walls:?}");
    let shortened = v::Arrival::Move { move_frames: 20, curve: v::Curve::Linear, split_of: None };
    let p5 = position(plan_in(&post, seq), 5).expect("#5");
    assert_eq!(p5.arrival, shortened, "#5's move starts at the wall");
    assert!(p5.flags.contains(&v::Flag::MoveShortened), "{:?}", p5.flags);
    let broken = v::validate(&post, seq_in(&post, seq));
    assert!(broken.is_empty(), "no move holds a wall: {broken:?}");
    // old frames 221 to 239 still show their content, each once, and now hold #12's window: the 19 frames the table
    // leaves to the flag. The held frames show one media time 60 times, so they are not compared; every other frame
    // kept its window
    let seen = assert_follows(&pre, seq, &post, seq, 1, "a frame hold inside #5's move");
    assert_eq!((seen.kept, seen.flagged), (521, 19), "{seen:?}");
    // the report (read before any undo, which clears it): #5 flagged, its move shortened from 40 frames to 20, and no
    // position moved, removed or added
    let report = f.s.hook_report().cloned().expect("the hook's report");
    let entry = report.sequences.iter().find(|q| q.sequence == seq).expect("Main's entry");
    let plan = entry.plans.iter().find(|p| p.plan == 1).expect("plan 1's entry");
    assert!(plan.flagged.contains(&5), "{plan:?}");
    assert!(plan.moved.is_empty() && plan.removed.is_empty() && plan.added.is_empty(), "{plan:?}");
    let change = json!({"plan": 1, "position": 5, "frames": [40, 20]});
    assert!(entry.changes.iter().any(|c| c.code == "move_shortened" && c.detail == change), "{:?}", entry.changes);
    f.s.undo();
    assert!(Arc::ptr_eq(&pre, &f.s.project), "undo restores the fixture bit for bit");
}

// ---------- T-N17: dynamic trims ----------

/// One dynamic trim of the fixture: what it is, the locked position it removes, whether it runs backwards (J), its edit
/// point (`trim.selectEditPoint`), and the one edit of a given offset in frames that makes its change.
type DynamicCase = (&'static str, u32, bool, fn(&Fixture) -> Value, fn(&Fixture, i64) -> Call);

/// The roll of the C1/C2 cut.
fn roll_point(f: &Fixture) -> Value {
    json!({"clip": f.clips[0].0, "edge": "out", "kind": "roll"})
}

/// A roll of the C1/C2 cut by `frames`.
fn roll_by(f: &Fixture, frames: i64) -> Call {
    ("timeline.roll", json!({"left": f.clips[0].0, "right": f.clips[1].0, "deltaFrames": frames}))
}

/// The ripple trim of C2's tail.
fn ripple_point(f: &Fixture) -> Value {
    json!({"clip": f.clips[1].0, "edge": "out", "kind": "ripple"})
}

/// A ripple trim of C2's tail by `frames`.
fn ripple_by(f: &Fixture, frames: i64) -> Call {
    ("timeline.trim", json!({"clip": f.clips[1].0, "edge": "out", "mode": "ripple", "deltaFrames": frames}))
}

/// A dynamic trim (J/K/L in trim mode) of the edit points selected on `s`: L, or J when `reverse`, at clock 0, a live
/// update at each clock of `ticks` (seconds), then K at clock `stop`. K's result.
fn dynamic_trim(s: &mut Session, reverse: bool, ticks: &[f64], stop: f64) -> Value {
    let direction = if reverse { "reverse" } else { "forward" };
    ok(s, "trim.shuttle", json!({"direction": direction, "clock": 0.0}));
    for clock in ticks {
        ok(s, "trim.tick", json!({"clock": clock}));
    }
    ok(s, "trim.shuttleStop", json!({"clock": stop}))
}

#[test]
fn t_n17_a_dynamic_trim_is_one_gesture_and_commits_what_one_edit_of_its_offset_makes() {
    // each live update puts back the project from before the trim and trims it again: the hook follows every update from
    // there, so the step K commits is what one edit of the same offset makes, ids and report included, and the locked
    // position the trim removes is reported once, however many updates removed it
    let cases: [DynamicCase; 2] = [("a dynamic roll", 4, false, roll_point, roll_by), ("a dynamic ripple trim", 5, true, ripple_point, ripple_by)];
    for (what, locked, reverse, point, by) in cases {
        let lock = json!({"plan": 1, "positions": [locked], "locked": true});
        let mut f = fixture();
        ok(&mut f.s, "vertical.setLock", lock.clone());
        let (start, steps, edge) = (Arc::clone(&f.s.project), f.s.history.undo.len(), point(&f));
        ok(&mut f.s, "trim.selectEditPoint", edge);
        let r = dynamic_trim(&mut f.s, reverse, &[0.5, 1.0, 1.5], 2.0);
        assert_eq!(r["committed"], json!(true), "{what}: {r}");
        assert_eq!(f.s.history.undo.len(), steps + 1, "{what}: the trim is one undo step");
        let frames = r["offsetFrames"].as_i64().expect("the committed offset");
        let mut g = fixture();
        ok(&mut g.s, "vertical.setLock", lock);
        let (cmd, p) = by(&g, frames);
        ok(&mut g.s, cmd, p);
        assert_eq!(saved(seq_in(&f.s.project, f.seq)), saved(seq_in(&g.s.project, g.seq)), "{what}: K commits one edit of {frames} frames");
        assert!(!added_ids(g.s.hook_report()).is_empty(), "{what}: the edit adds a position: {:?}", g.s.hook_report());
        assert_eq!(f.s.hook_report(), g.s.hook_report(), "{what}: the same ids and the same report");
        let told = warnings_on(&f.s, locked);
        assert!(told.len() == 1 && told[0].starts_with("Dynamic trimming"), "{what}: told once, while the trim ran: {told:?}");
        assert_eq!(warnings_on(&g.s, locked).len(), 1, "{what}: the one edit tells it once");
        f.s.undo();
        assert!(Arc::ptr_eq(&start, &f.s.project), "{what}: undo restores the 16:9 edit and the framing bit for bit");
    }
}

#[test]
fn t_n17_a_cancelled_dynamic_trim_restores_the_framing_bit_for_bit_and_its_ids_stay_handed_out() {
    let mut f = fixture();
    ok(&mut f.s, "vertical.setLock", json!({"plan": 1, "positions": [4], "locked": true}));
    let (start, edge) = (Arc::clone(&f.s.project), roll_point(&f));
    ok(&mut f.s, "trim.selectEditPoint", edge);
    ok(&mut f.s, "trim.shuttle", json!({"direction": "forward", "clock": 0.0}));
    for clock in [0.5, 1.0, 2.0] {
        ok(&mut f.s, "trim.tick", json!({"clock": clock}));
    }
    // live: the roll of 50 frames is in the project, with the split the hook made in #12's move at C2's new head
    assert_eq!(added_ids(f.s.hook_report()), vec![13], "the live update's split");
    ok(&mut f.s, "trim.cancelDynamic", json!({}));
    assert!(Arc::ptr_eq(&start, &f.s.project), "Esc restores the 16:9 edit and the framing bit for bit");
    assert!(f.s.hook_report().is_none(), "nothing the trim did is reported now");
    // the same roll made afterwards hands out no id the trim handed out, and tells the person again
    let (cmd, p) = roll_by(&f, 50);
    ok(&mut f.s, cmd, p);
    assert_eq!(added_ids(f.s.hook_report()), vec![14], "id 13 went to the cancelled trim's split");
    assert_eq!(warnings_on(&f.s, 4).len(), 2, "the trim's live notice, then the roll's: {:?}", warnings_on(&f.s, 4));
}

#[test]
fn t_n17_a_dynamic_trim_that_comes_back_to_its_start_leaves_nothing_to_the_next_edit() {
    // L to 50 frames, J back to the start, K: nothing is committed. The next edit begins a gesture of its own: its ids
    // come after the ones the trim handed out, and it tells the person of the locked position it removes
    let mut f = fixture();
    ok(&mut f.s, "vertical.setLock", json!({"plan": 1, "positions": [4], "locked": true}));
    let (start, steps, edge) = (Arc::clone(&f.s.project), f.s.history.undo.len(), roll_point(&f));
    ok(&mut f.s, "trim.selectEditPoint", edge);
    ok(&mut f.s, "trim.shuttle", json!({"direction": "forward", "clock": 0.0}));
    ok(&mut f.s, "trim.tick", json!({"clock": 2.0}));
    assert_eq!(added_ids(f.s.hook_report()), vec![13], "the live update's split");
    ok(&mut f.s, "trim.shuttle", json!({"direction": "reverse", "clock": 2.0}));
    let r = ok(&mut f.s, "trim.shuttleStop", json!({"clock": 4.0}));
    assert_eq!(r["committed"], json!(false), "back at its start, K commits nothing: {r}");
    assert!(Arc::ptr_eq(&start, &f.s.project) && f.s.history.undo.len() == steps, "the project before the trim, and no step");
    assert!(f.s.hook_report().is_none(), "the live update the trim took back is not reported");
    let (cmd, p) = roll_by(&f, 50);
    ok(&mut f.s, cmd, p);
    assert_eq!(added_ids(f.s.hook_report()), vec![14], "id 13 went to the trim's live update");
    assert_eq!(warnings_on(&f.s, 4).len(), 2, "the trim's live notice, then the roll's: {:?}", warnings_on(&f.s, 4));
}

#[test]
fn t_n17_a_dynamic_trim_of_two_edit_points_commits_the_two_edits_one_after_the_other() {
    // each live update rolls the C1/C2 cut from the project before the trim, then the C3/C4 cut from that result: K
    // commits what two rolls of the same offset make one after the other, ids included, and each locked position the
    // trim removes is reported once
    let lock = json!({"plan": 1, "positions": [4, 9], "locked": true});
    let mut f = fixture();
    let [c1, _, c3, _] = f.clips.map(|c| c.0);
    ok(&mut f.s, "vertical.setLock", lock.clone());
    ok(&mut f.s, "trim.selectEditPoint", json!({"clip": c1, "edge": "out", "kind": "roll"}));
    ok(&mut f.s, "trim.selectEditPoint", json!({"clip": c3, "edge": "out", "kind": "roll", "add": true}));
    let r = dynamic_trim(&mut f.s, false, &[0.5, 1.0], 1.5);
    assert_eq!(r["committed"], json!(true), "{r}");
    let frames = r["offsetFrames"].as_i64().expect("the committed offset");
    let mut g = fixture();
    let [g1, g2, g3, g4] = g.clips.map(|c| c.0);
    ok(&mut g.s, "vertical.setLock", lock);
    ok(&mut g.s, "timeline.roll", json!({"left": g1, "right": g2, "deltaFrames": frames}));
    ok(&mut g.s, "timeline.roll", json!({"left": g3, "right": g4, "deltaFrames": frames}));
    assert_eq!(saved(seq_in(&f.s.project, f.seq)), saved(seq_in(&g.s.project, g.seq)), "two rolls of {frames} frames, ids included");
    assert_eq!(f.s.hook_report(), g.s.hook_report(), "the report of the second point is the second roll's");
    for id in [4, 9] {
        assert_eq!(warnings_on(&f.s, id).len(), 1, "#{id}: {:?}", warnings_on(&f.s, id));
    }
}

// ---------- T-N12: a vertical-only re-cut ----------

/// Each `(new, old)`: frame `new` of the re-cut (plan 1 of `seq1` in `after`) shows frame `old` of the original (plan 1 of
/// `seq0` in `before`) after a join; a position opens it with exactly the original's window there, on an effective wall
/// (frame 0 opens the sequence).
fn assert_openers(before: &Project, seq0: ItemId, after: &Project, seq1: ItemId, joins: &[(i64, i64)], what: &str) {
    let (q0, q1) = (seq_in(before, seq0), seq_in(after, seq1));
    let (r0, p0, p1) = (v::raster(q0), plan_in(before, seq0), plan_in(after, seq1));
    let walls: Vec<i64> = v::walls(after, q1).iter().map(|w| fr(w.at)).collect();
    for &(new, old) in joins {
        assert!(new == 0 || walls.contains(&new), "{what}: frame {new} is a wall: {walls:?}");
        let opener = p1.positions.iter().find(|p| fr(p.at) == new).unwrap_or_else(|| panic!("{what}: a position opens frame {new}"));
        let shown = v::window_at(p0, &r0, at(old));
        assert_eq!((opener.window, opener.layout), (shown.window, shown.layout), "{what}: the opener of frame {new} shows the original's frame {old}");
    }
}

/// Duplicates the one-clip master `seq` of `s` (Project panel ▸ Edit ▸ Duplicate), opens the copy and razors it at the
/// hook's first frame and at the frame after its last: through edits, which change no framing (spec §4.13 steps 1 and 2).
fn duplicate_and_razor(s: &mut Session, seq: ItemId) -> ItemId {
    ok(s, "project.select", json!({"items": [seq.0]}));
    let copy = ItemId(ok(s, "edit.duplicate", json!({}))["items"][0].as_u64().expect("the copy's id"));
    assert_eq!(set_in(&s.project, copy), set_in(&s.project, seq), "the copy carries the whole vertical set verbatim");
    ok(s, "sequence.open", json!({"item": copy.0}));
    for frame in [600, 700] {
        let before = set_in(&s.project, copy).clone();
        apply_edit(s, copy, "a razor", &[("timeline.razor", json!({"time": at(frame).0, "track": "V1"}))]);
        assert_eq!(set_in(&s.project, copy), &before, "a through edit is no wall and changes no framing");
    }
    copy
}

#[test]
fn t_n12_a_re_cut_by_duplicate_razor_insert_move_close_gap_and_ripple_delete_keeps_the_framing() {
    let (mut s, master, _) = one_clip(set_of(recut_plan()));
    let original = saved(seq_in(&s.project, master));
    let base = Arc::clone(&s.project);
    let copy = duplicate_and_razor(&mut s, master);
    // 3. the hook to the head with an insert move: one edit, and the clip keeps its id
    let hook = clip_at(&s.project, copy, 600);
    apply_edit(&mut s, copy, "insert move", &[("timeline.move", json!({"moves": [{"clip": hook.0, "track": "V1", "time": 0}], "insert": true}))]);
    assert_openers(&base, master, &s.project, copy, &[(0, 600), (100, 0), (800, 700)], "after the insert move");
    // 4. close the gap it left, and ripple-delete what the short leaves out
    apply_edit(&mut s, copy, "close the gap", &[("sequence.closeGap", json!({"track": "V1", "time": at(750).0}))]);
    assert_openers(&base, master, &s.project, copy, &[(0, 600), (100, 0), (700, 700)], "after closing the gap");
    let tail = clip_at(&s.project, copy, 700);
    apply_edit(&mut s, copy, "ripple delete", &[("edit.rippleDelete", json!({"clips": [tail.0]}))]);
    // the short: the hook first, then the episode's head, every frame with its window
    assert_eq!(v::raster(seq_in(&s.project, copy)).end_frame, 700);
    let seen = assert_follows(&base, master, &s.project, copy, 5, "the re-cut against the original");
    assert_eq!((seen.kept, seen.flagged), (700, 0), "every frame of the re-cut shows a frame of the original with its window");
    assert_openers(&base, master, &s.project, copy, &[(0, 600), (100, 0)], "the re-cut");
    // the original sequence and its plan were never touched
    assert_eq!(saved(seq_in(&s.project, master)), original, "the original sequence is bit-identical");
}

/// The positions of `plan` on frames `[from, to)` as `vertical.replaceRange` items moved to start at frame 0: what a person
/// copies in the Positions tab and pastes at a new head (spec §4.13, §5.7).
fn copied(plan: &v::VerticalPlan, from: i64, to: i64) -> Vec<Value> {
    let mut out = Vec::new();
    for p in plan.positions.iter().filter(|p| (from..to).contains(&fr(p.at))) {
        let mut item = json!({"frame": fr(p.at) - from, "cx": p.window.cx, "cy": p.window.cy, "h": p.window.h, "arrival": "cut"});
        if let v::Arrival::Move { move_frames, curve, .. } = p.arrival {
            let (name, shape) = match curve {
                v::Curve::Linear => ("linear", None),
                v::Curve::EaseIn => ("easeIn", None),
                v::Curve::EaseOut => ("easeOut", None),
                v::Curve::EaseInOut => ("easeInOut", None),
                v::Curve::Speeds { start, end } => ("speeds", Some(json!([start, end]))),
                v::Curve::Bezier { x1, y1, x2, y2 } => ("bezier", Some(json!([x1, y1, x2, y2]))),
                v::Curve::Ramp { r_in, r_out } => ("ramp", Some(json!([r_in, r_out]))),
            };
            item["arrival"] = json!("move");
            item["moveFrames"] = json!(move_frames);
            item["curve"] = json!(name);
            if let Some(shape) = shape {
                item[name] = shape;
            }
        }
        out.push(item);
    }
    out
}

#[test]
fn t_n12_cut_and_paste_is_new_content_that_reads_carried_until_the_copied_positions_are_pasted() {
    let (mut s, master, _) = one_clip(set_of(recut_plan()));
    let base = Arc::clone(&s.project);
    let copy = duplicate_and_razor(&mut s, master);
    let hook = clip_at(&s.project, copy, 600);
    ok(&mut s, "timeline.select", json!({"clips": [hook.0]}));
    apply_edit(&mut s, copy, "cut", &[("edit.cut", json!({}))]);
    apply_edit(&mut s, copy, "close the gap", &[("sequence.closeGap", json!({"track": "V1", "time": at(650).0}))]);
    ok(&mut s, "playhead.set", json!({"frame": 0}));
    apply_edit(&mut s, copy, "paste insert", &[("edit.pasteInsert", json!({}))]);
    // a pasted clip is a new clip: its frames are new content, and its shot reads carried
    let shots = v::shots(&s.project, seq_in(&s.project, copy), 1);
    let first = shots.first().expect("the copy's first shot");
    assert_eq!((first.from, first.to, first.status), (0, 100, v::ShotStatus::Carried), "the pasted hook carries the window after it");
    // pasting the copied positions at the new head restores the framing, frame for frame
    let positions = copied(plan_in(&base, master), 600, 700);
    ok(&mut s, "vertical.replaceRange", json!({"sequence": copy.0, "plan": 1, "fromFrame": 0, "toFrame": 100, "positions": positions}));
    let (r0, r1) = (v::raster(seq_in(&base, master)), v::raster(seq_in(&s.project, copy)));
    let (p0, p1) = (plan_in(&base, master), plan_in(&s.project, copy));
    for f in 0..100 {
        let (a, b) = (v::window_at(p0, &r0, at(600 + f)), v::window_at(p1, &r1, at(f)));
        assert_eq!((a.window, a.layout), (b.window, b.layout), "the copy's frame {f} shows the original's frame {}", 600 + f);
    }
    let shots = v::shots(&s.project, seq_in(&s.project, copy), 1);
    assert_eq!(shots.first().map(|shot| shot.status), Some(v::ShotStatus::Own), "the hook opens on its own position again");
}

// ---------- T-N11: re-delivered masters ----------

/// The remap of [`remap_set`]'s master: the re-delivered file drops the old frames `[300, 350)` and inserts 50 new frames
/// at 700.
fn remap_map() -> Value {
    let entry = |from: i64, to: i64, new_from: i64| json!({"fromFrame": from, "toFrame": to, "newFrom": new_from});
    json!([entry(0, 300, 0), entry(350, 750, 300), entry(750, 1000, 750)])
}

#[test]
fn t_n11_remap_moves_the_framing_onto_a_relinked_synthetic_master() {
    let (mut s, master, media) = one_clip(remap_set());
    // the relink is a 16:9 edit: the hook runs, finds no picture digest changed, and leaves the framing bit for bit
    let (pre, runs) = (Arc::clone(&s.project), s.vertical_hook.runs);
    relink(&mut s, media.master);
    assert_eq!(s.vertical_hook.runs, runs + 1, "the relink ran the edit hook");
    assert_eq!(saved(set_in(&s.project, master)), saved(set_in(&pre, master)), "the relink leaves the framing as it was");
    assert_eq!((top_label(&s).as_str(), s.hook_report().is_none()), ("Link Media", true), "no suffix and no report");
    let before = Arc::clone(&s.project);
    let expect = v::remap_digest(seq_in(&before, master));
    let p = json!({"map": remap_map(), "expect": expect});
    // a dry run is the real result and records nothing
    let unchanged = Unchanged::of(&s);
    let mut asked = p.clone();
    asked["dryRun"] = json!(true);
    let dry = ok(&mut s, "vertical.remap", asked);
    unchanged.check(&s, "a dry run of the remap");
    let r = ok(&mut s, "vertical.remap", p.clone());
    assert_eq!((&r["applied"], &r["changed"], &dry["applied"]), (&json!(true), &json!(true), &json!(false)), "{r}");
    assert_eq!(r["label"], json!("Vertical: remap (3 ranges, 50 frames removed, 50 frames inserted)"));
    let keys = ["dryRun", "applied", "label"];
    assert_eq!(without(&dry, &keys), without(&r, &keys), "the dry run's result is the real one");
    assert_eq!((&r["digestBefore"], &r["digest"]), (&json!(expect), &json!(v::remap_digest(seq_in(&s.project, master)))));
    // one plan-only undo step after the relink's: no edit hook ran and no 16:9 frame changed
    assert_eq!(s.history.undo.len(), 2);
    assert!(s.history.undo[0].2.picture && !s.history.undo[1].2.picture, "the relink is a picture step and the remap a plan-only one");
    assert_eq!((s.picture_revision, s.vertical_hook.runs), (unchanged.revisions.1, unchanged.runs));
    // every frame the map keeps shows its window within the hook's bound
    let (q0, q1) = (seq_in(&before, master), seq_in(&s.project, master));
    let (r0, r1, p0, p1) = (v::raster(q0), v::raster(q1), plan_in(&before, master), plan_in(&s.project, master));
    for (from, to, new_from) in [(0, 300, 0), (350, 750, 300), (750, 1000, 750)] {
        for f in from..to {
            let g = new_from + f - from;
            let (a, b) = (v::window_at(p0, &r0, at(f)), v::window_at(p1, &r1, at(g)));
            let (moved, bound) = (change(&a.window, &b.window), v::follow_bound(p0, &r0, f));
            assert!(moved <= bound + NOISE && a.layout == b.layout, "old frame {f} -> {g} moved {moved} px (bound {bound}): {a:?} -> {b:?}");
        }
    }
    // for a person, the locked position on the dropped frames goes with them, and the result says so (I16)
    assert!(position(p1, 3).is_none(), "the locked #3 lay on dropped frames");
    let removed = json!({"plan": 1, "position": 3, "frame": 320, "locked": true});
    let changes = r["changes"].as_array().expect("the changes");
    assert!(changes.iter().any(|c| c["code"] == json!("position_removed") && c["detail"] == removed), "{r}");
    // every junction is a wall (the editor cannot see the new master's cuts), and the person's wall moved with its picture
    let walls: Vec<i64> = v::walls(&s.project, q1).iter().map(|w| fr(w.at)).collect();
    for junction in [300, 700, 750, 760] {
        assert!(walls.contains(&junction), "a wall at {junction}: {walls:?}");
    }
    // undo restores the set bit for bit; a retry after the redo is refused by the required guard (T-N15)
    let after = Arc::clone(&s.project);
    s.undo();
    assert!(Arc::ptr_eq(&before, &s.project), "undo restores the framing before the remap");
    s.redo();
    assert!(Arc::ptr_eq(&after, &s.project));
    let (code, detail) = refused(&mut s, "vertical.remap", p);
    assert_eq!((code, &detail["digest"]), ("stale_plan", &json!(v::remap_digest(seq_in(&after, master)))), "the same remap twice is refused");
}

#[test]
fn t_n11_remap_refuses_a_bad_map_and_a_missing_or_stale_guard_and_changes_nothing() {
    let (mut s, master, _) = one_clip(remap_set());
    let expect = v::remap_digest(seq_in(&s.project, master));
    let with = |map: Value| json!({"map": map, "expect": expect});
    let entry = |from: i64, to: i64, new_from: i64| json!({"fromFrame": from, "toFrame": to, "newFrom": new_from});
    let too_many: Vec<Value> = (0..10_001).map(|k| entry(k, k + 1, k)).collect();
    let cases = [
        (json!({"map": [entry(0, 100, 0)]}), "out_of_range", json!({"key": "expect"})),
        (json!({"map": [entry(0, 100, 0)], "expect": "fnv1a64:0000000000000000"}), "stale_plan", json!({"digest": expect})),
        (with(json!([entry(500, 600, 500), entry(0, 100, 0)])), "bad_map", json!({"entry": 1, "reason": "unsorted"})),
        (with(json!([entry(0, 300, 0), entry(200, 400, 300)])), "bad_map", json!({"entry": 1, "reason": "overlapping"})),
        (with(json!([entry(0, 100, 950)])), "bad_map", json!({"entry": 0, "reason": "out_of_range"})),
        (with(json!([entry(-10, 100, 0)])), "bad_map", json!({"entry": 0, "reason": "out_of_range"})),
        (with(json!([entry(100, 100, 0)])), "bad_map", json!({"entry": 0, "reason": "empty"})),
        (with(json!({"fromFrame": 0})), "out_of_range", json!({"key": "map"})),
        (with(json!([5])), "out_of_range", json!({"key": "map", "item": 0})),
        (with(json!([{"fromFrame": 0, "toFrame": 100}])), "out_of_range", json!({"key": "newFrom", "index": 0})),
        (with(json!([{"fromFrame": 0, "toFrame": 100, "newFrom": 0, "bogus": 1}])), "out_of_range", json!({"key": "bogus", "index": 0})),
        (json!({"expect": expect}), "out_of_range", json!({"key": "map"})),
        (with(Value::Array(too_many)), "too_many", json!({"key": "map", "count": 10_001, "bound": 10_000})),
        (json!({"map": [], "expect": expect, "seconds": 1.0}), "use_frame", json!({"key": "seconds"})),
        (json!({"map": [], "expect": expect, "runId": "r1"}), "out_of_range", json!({"key": "runId"})),
        (json!({"sequence": 999_999, "map": [], "expect": expect}), "not_found", json!({"sequence": 999_999})),
    ];
    for (p, want, detail) in cases {
        let unchanged = Unchanged::of(&s);
        let what = format!("`vertical.remap` {p}");
        let (code, got) = refused(&mut s, "vertical.remap", p);
        assert_eq!(code, want, "{what}: {got}");
        for (key, value) in detail.as_object().into_iter().flatten() {
            assert_eq!(&got[key.as_str()], value, "{what}: `{key}` of {got}");
        }
        unchanged.check(&s, &what);
    }
    // the identity map changes nothing and records nothing
    let unchanged = Unchanged::of(&s);
    let r = ok(&mut s, "vertical.remap", with(json!([entry(0, 1000, 0)])));
    assert_eq!((&r["changed"], &r["applied"]), (&json!(false), &json!(false)), "{r}");
    unchanged.check(&s, "the identity remap");
}

#[test]
fn t_n15_a_retried_remap_of_a_set_without_plans_is_refused_stale_plan() {
    // walls and parts can come before any plan (a measurement applied, a part marked first): the remap moves them, so its
    // guard covers them, and a retry, which would move them again, is refused
    let mut set = crate::empty_vertical_set();
    set.walls.stored.push(v::StoredWall { at: at(400), origin: v::WallOrigin::Manual, score: None });
    set.parts.push(v::Part { id: 1, name: "P01".into(), from: at(400), to: at(600), audio: None, meta: v::PartMeta::default() });
    set.next_part_id = 2;
    let (mut s, master, _) = one_clip(set);
    let p = json!({"map": remap_map(), "expect": v::remap_digest(seq_in(&s.project, master))});
    let r = ok(&mut s, "vertical.remap", p.clone());
    assert_eq!((&r["applied"], &r["changed"]), (&json!(true), &json!(true)), "{r}");
    let moved = set_in(&s.project, master);
    assert!(moved.walls.stored.iter().any(|w| fr(w.at) == 350), "the wall moved with its picture: {:?}", moved.walls.stored);
    assert_eq!(moved.parts.iter().map(|x| (fr(x.from), fr(x.to))).collect::<Vec<_>>(), vec![(350, 550)], "and so did the part");
    let after = Arc::clone(&s.project);
    let (code, detail) = refused(&mut s, "vertical.remap", p);
    assert_eq!((code, &detail["digest"]), ("stale_plan", &json!(v::remap_digest(seq_in(&after, master)))), "the same remap twice is refused");
    assert!(Arc::ptr_eq(&after, &s.project), "and changes nothing");
}

// ---------- the hook audit ----------

/// The picture edits of the audit on [`fixture`], one or more per family of command that edits the 16:9 picture, each with
/// the calls that prepare it.
fn picture_edits(f: &Fixture) -> Vec<(Vec<Call>, Call)> {
    let [c1, c2, c3, c4] = f.clips.map(|c| c.0);
    let (a1, matte, seq) = (f.sounds[0].0, f.media.matte.0, f.seq.0);
    let select = |clip: u64| vec![("timeline.select", json!({"clips": [clip]}))];
    let copied_c1 = vec![("timeline.select", json!({"clips": [c1]})), ("edit.copy", json!({})), ("playhead.set", json!({"frame": 150}))];
    let marks = vec![("markers.markIn", json!({"frame": 200})), ("markers.markOut", json!({"frame": 229}))];
    let place = |insert: bool| json!({"item": matte, "track": "V1", "frame": 200, "insert": insert, "sourceIn": 0, "duration": at(25).0});
    vec![
        (vec![], ("timeline.move", json!({"moves": [{"clip": c3, "track": "V1", "time": 0}], "insert": true}))),
        (vec![], ("timeline.move", json!({"moves": [{"clip": c4, "track": "V3", "time": at(450).0}]}))),
        (vec![], ("timeline.trim", json!({"clip": c2, "edge": "out", "mode": "ripple", "deltaFrames": -10}))),
        (vec![], ("timeline.trim", json!({"clip": c2, "edge": "in", "mode": "regular", "deltaFrames": 5}))),
        (vec![], ("timeline.roll", json!({"left": c1, "right": c2, "deltaFrames": 5}))),
        (vec![], ("timeline.slip", json!({"clip": c2, "deltaFrames": 7}))),
        (vec![], ("timeline.slide", json!({"clip": c2, "deltaFrames": -6}))),
        (vec![], ("timeline.rateStretch", json!({"clip": c2, "edge": "out", "delta": at(-30).0}))),
        (vec![], ("timeline.razor", json!({"time": at(220).0, "track": "V1"}))),
        (vec![], ("timeline.place", place(true))),
        (vec![], ("timeline.place", place(false))),
        (vec![], ("timeline.setTrack", json!({"track": "V2", "enabled": false}))),
        (vec![], ("edit.rippleDelete", json!({"clips": [c2]}))),
        (vec![], ("edit.clear", json!({"clips": [c2]}))),
        (select(c3), ("edit.cut", json!({}))),
        (copied_c1.clone(), ("edit.paste", json!({}))),
        (copied_c1, ("edit.pasteInsert", json!({}))),
        (vec![("project.select", json!({"items": [seq]}))], ("edit.duplicate", json!({}))),
        (select(c1), ("edit.label", json!({"label": "Rose"}))),
        (vec![], ("clip.speedDuration", json!({"clips": [c2], "speed": 200, "ripple": true}))),
        (vec![], ("clip.frameHold", json!({"clips": [c2], "time": at(200).0}))),
        (vec![], ("clip.enable", json!({"clips": [c3]}))),
        (vec![], ("clip.link", json!({"clips": [c1, a1]}))),
        (select(c2), ("clip.nest", json!({"name": "Nested"}))),
        (select(c2), ("clip.scaleToFrameSize", json!({}))),
        (vec![], ("sequence.addEdit", json!({"time": at(220).0}))),
        (vec![], ("sequence.addEditAllTracks", json!({"time": at(230).0}))),
        (marks.clone(), ("sequence.lift", json!({}))),
        (marks, ("sequence.extract", json!({}))),
        (vec![("edit.clear", json!({"clips": [c2]}))], ("sequence.closeGap", json!({"track": "V1", "time": at(200).0}))),
        (vec![], ("sequence.applyVideoTransition", json!({"clip": c2, "edge": "in", "frames": 10}))),
        (vec![("timeline.razor", json!({"time": at(220).0, "track": "V1"}))], ("sequence.joinThroughEdits", json!({"all": true}))),
        (vec![], ("effects.setParam", json!({"clip": c2, "effect": "motion", "param": "scale", "value": 110.0}))),
        (vec![], ("effects.apply", json!({"clips": [c2], "effect": "crop"}))),
        (vec![], ("markers.add", json!({}))),
        (vec![], ("markers.markIn", json!({"frame": 10}))),
        (vec![("trim.selectEditPoint", json!({"clip": c1, "edge": "out", "kind": "roll"}))], ("trim.nudge", json!({"frames": 2}))),
    ]
}

/// The `vertical.*` writes of the audit on [`fixture`] (every write of plan 63 §3.2 but `applyMeasurement`, which needs a
/// measurement first), each with the calls that prepare it.
fn vertical_writes(f: &Fixture) -> Vec<(Vec<Call>, Call)> {
    let expect = v::remap_digest(seq_in(&f.s.project, f.seq));
    let part = json!({"add": [{"name": "P01", "fromFrame": 150, "toFrame": 300}]});
    let region = json!({"label": "logo", "fromFrame": 0, "toFrame": 600, "rect": [1700, 40, 160, 90], "mode": "keepOut"});
    let reviewed = vec![("vertical.setParts", part.clone()), ("vertical.setReview", json!({"plan": 1, "positions": [4, 12, 5], "reviewed": true}))];
    let remap = json!([{"fromFrame": 0, "toFrame": 300, "newFrom": 0}, {"fromFrame": 310, "toFrame": 600, "newFrom": 300}]);
    let replace = json!({"plan": 1, "fromFrame": 450, "toFrame": 500, "positions": [{"frame": 450, "cx": 880, "cy": 540, "h": 1000}]});
    vec![
        (vec![], ("vertical.create", json!({"name": "Square", "preset": "1:1"}))),
        (vec![], ("vertical.update", json!({"plan": 1, "name": "Phone"}))),
        (vec![], ("vertical.delete", json!({"plan": 1}))),
        (vec![], ("vertical.setPosition", json!({"plan": 1, "frame": 280, "cx": 1100}))),
        (vec![], ("vertical.retimePosition", json!({"plan": 1, "position": 9, "frame": 455}))),
        (vec![], ("vertical.deletePositions", json!({"plan": 1, "positions": [11]}))),
        (vec![], ("vertical.replaceRange", replace)),
        (vec![], ("vertical.setArrival", json!({"plan": 1, "positions": [11], "arrival": "cut"}))),
        (vec![], ("vertical.smoothThrough", json!({"plan": 1, "positions": [9, 10, 11]}))),
        (vec![], ("vertical.fitMove", json!({"plan": 1, "position": 10, "maxSpeed": 20.0}))),
        (vec![], ("vertical.swapWindows", json!({"plan": 1, "positions": [4, 9]}))),
        (vec![], ("vertical.setWalls", json!({"add": [470]}))),
        (vec![("vertical.deletePositions", json!({"plan": 1, "positions": [9]}))], ("vertical.seedShots", json!({"plan": 1, "mode": "carry"}))),
        (vec![], ("vertical.setParts", part)),
        (vec![], ("vertical.setRegions", json!({"add": [region]}))),
        (vec![], ("vertical.markChecked", json!({"plan": 1}))),
        (vec![], ("vertical.repair", json!({"plan": 1}))),
        (vec![], ("vertical.setAnnotations", json!({"add": [{"kind": "speech", "fromFrame": 0, "toFrame": 100}]}))),
        (vec![], ("vertical.setActiveArea", json!({"set": [{"fromFrame": 0, "toFrame": 600, "rect": [0, 140, 1920, 800]}]}))),
        (reviewed, ("vertical.approvePart", json!({"plan": 1, "part": 1}))),
        (vec![], ("vertical.setReview", json!({"plan": 1, "positions": [4], "reviewed": true}))),
        (vec![], ("vertical.setLock", json!({"plan": 1, "positions": [4], "locked": true}))),
        (vec![], ("vertical.flagAttention", json!({"plan": 1, "frame": 200, "note": "check the pan"}))),
        (vec![("vertical.flagAttention", json!({"plan": 1, "frame": 200}))], ("vertical.clearAttention", json!({"plan": 1, "frames": [200]}))),
        (vec![], ("vertical.remap", json!({"map": remap, "expect": expect}))),
    ]
}

#[test]
fn hook_audit_every_picture_edit_runs_the_hook_and_no_vertical_write_does() {
    let probe = fixture();
    // every edit of the 16:9 picture commits through the hooked paths: the hook ran for each step it recorded
    for (setup, (cmd, p)) in picture_edits(&probe) {
        let mut f = fixture();
        for (id, sp) in setup {
            ok(&mut f.s, id, sp);
        }
        let (steps, runs) = (f.s.history.undo.len(), f.s.vertical_hook.runs);
        ok(&mut f.s, cmd, p.clone());
        let recorded = u64::try_from(f.s.history.undo.len().saturating_sub(steps)).unwrap_or(u64::MAX);
        let ran = f.s.vertical_hook.runs.saturating_sub(runs);
        assert!(recorded >= 1, "`{cmd}` {p} records a step");
        assert!(ran >= recorded, "`{cmd}` {p}: {recorded} steps recorded, {ran} hook runs");
        assert!(f.s.history.undo.last().is_some_and(|step| step.2.picture), "`{cmd}` {p} records a picture step");
    }
    // no vertical write runs it: each commits through the plan-only path, and its step is plan-only
    let mut applied = 0;
    for (setup, (cmd, p)) in vertical_writes(&probe) {
        let mut f = fixture();
        for (id, sp) in setup {
            ok(&mut f.s, id, sp);
        }
        let (steps, runs) = (f.s.history.undo.len(), f.s.vertical_hook.runs);
        let r = f.s.execute(cmd, p.clone());
        assert_eq!(f.s.vertical_hook.runs, runs, "`{cmd}` {p} ran the edit hook");
        if f.s.history.undo.len() > steps {
            assert!(f.s.history.undo.last().is_some_and(|step| !step.2.picture), "`{cmd}` {p} records a plan-only step");
            applied += 1;
        } else {
            assert!(r.is_err() || r.as_ref().is_ok_and(|r| r["changed"] == json!(false)), "`{cmd}` {p}: {r:?}");
        }
    }
    assert!(applied >= 20, "the audit applied {applied} vertical writes");
    // and so does applying a measurement
    let mut f = fixture();
    let m = ok(&mut f.s, "vertical.measureCuts", json!({"fromFrame": 0, "toFrame": 20, "wait": true}));
    let runs = f.s.vertical_hook.runs;
    ok(&mut f.s, "vertical.applyMeasurement", json!({"measurement": m["measurement"]}));
    assert_eq!(f.s.vertical_hook.runs, runs, "applying a measurement runs no hook");
}

/// The text of the method of `text` that starts with `signature`, up to the next method of its impl.
fn method<'a>(text: &'a str, signature: &str) -> &'a str {
    let start = text.find(signature).unwrap_or_else(|| panic!("no `{signature}`"));
    let rest = &text[start + signature.len()..];
    let end = ["\n    pub fn ", "\n    fn ", "\n    pub(crate) fn "].iter().filter_map(|next| rest.find(*next)).min().unwrap_or(rest.len());
    &rest[..end]
}

/// The lines of the engine's own code that can change the session's project or its history, by source file: an
/// assignment to a `project` field, a mutable borrow of one, and an undo or redo step pushed or inserted, whatever the
/// receiver is called. Every source file of `src` is read but the tests (the `*_tests.rs` files and the modules lib.rs
/// declares under `#[cfg(test)]`); comment lines are no code.
fn change_sites() -> BTreeMap<String, Vec<String>> {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let lib = std::fs::read_to_string(src.join("lib.rs")).expect("lib.rs");
    let lines: Vec<&str> = lib.lines().map(str::trim).collect();
    let declared = lines.windows(2).filter(|pair| pair[0] == "#[cfg(test)]").filter_map(|pair| pair[1].strip_prefix("mod ")?.strip_suffix(';'));
    let tests: BTreeSet<String> = declared.map(|name| format!("{name}.rs")).collect();
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(&src).expect("the source folder") {
        let path = entry.expect("a source entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") || tests.contains(&name) {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a source file");
        let sites: Vec<String> = text.lines().map(str::trim).filter(|line| !line.starts_with("//") && changes_project(line)).map(str::to_string).collect();
        if !sites.is_empty() {
            out.insert(name, sites);
        }
    }
    out
}

/// Whether a line of code assigns to a `project` field, borrows one mutably, or pushes or inserts an undo or redo step.
fn changes_project(line: &str) -> bool {
    let borrows = line.match_indices("&mut ").any(|(i, m)| {
        let path: String = line[i + m.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
        path.ends_with(".project")
    });
    let steps = ["history.undo.push(", "history.redo.push(", "history.undo.insert(", "history.redo.insert("];
    line.contains(".project = ") || borrows || steps.iter().any(|step| line.contains(step))
}

#[test]
fn hook_audit_the_engine_changes_its_project_only_through_the_reviewed_paths() {
    let lib = include_str!("lib.rs");
    // the two commit paths of a 16:9 edit run the hook, and the plan-only path does not
    assert!(method(lib, "pub fn edit<R>(").contains("self.run_edit_hook("), "Session::edit runs the edit hook");
    assert!(method(lib, "pub fn edit_merged<R>(").contains("self.run_edit_hook("), "the merging branch of Session::edit_merged runs it");
    assert!(!method(lib, "pub fn edit_plan_only<R>(").contains("run_edit_hook"), "a plan-only commit never runs it");
    assert_eq!(lib.matches("self.run_edit_hook(").count(), 2, "no other place runs the hook");
    // every other line of the engine that changes the session's project or its history, in every source file but the
    // tests and whatever the receiver is called, as reviewed against the hook
    let reviewed: [(&str, &[&str]); 7] = [
        // collapse_history folds steps that ran the hook; closing the project replaces it (note_project_replaced)
        (
            "clip_ops.rs",
            &["s.history.undo.push(crate::Step(label.to_string(), first, meta));", r#"s.project = std::sync::Arc::new(Project::new("Untitled"));"#],
        ),
        // a finished background import puts in the media's information, which changes no picture digest; a new and the
        // demo project replace it; Save writes the project's name and the id marks; installing a project replaces it
        (
            "commands.rs",
            &[
                "s.project = std::sync::Arc::new(project);",
                "s.project = std::sync::Arc::new(filmcraft_project::Project::new(&name));",
                "s.project = std::sync::Arc::new(p);",
                "name_project(std::sync::Arc::make_mut(&mut s.project), name);",
                "s.vertical_ids.raise(std::sync::Arc::make_mut(&mut s.project));",
                "s.project = std::sync::Arc::new(proj);",
            ],
        ),
        // the source graphics' sync edits graphic layers, outside the hook by design (spec §4.8)
        ("graphic_templates.rs", &["let p = Arc::make_mut(&mut s.project);"]),
        // the session's own commits: edit, the merging branch of edit_merged, edit_plan_only, undo and redo
        (
            "lib.rs",
            &[
                "let old = std::mem::replace(&mut self.project, Arc::new(p));",
                "self.history.undo.push(Step(hooked.label(label), old, meta));",
                "self.project = Arc::new(p);",
                "let old = std::mem::replace(&mut self.project, Arc::new(p));",
                "self.history.undo.push(Step(label.to_string(), old, meta));",
                "let cur = std::mem::replace(&mut self.project, prev);",
                "self.history.redo.push(Step(label.clone(), cur, meta));",
                "let cur = std::mem::replace(&mut self.project, next);",
                "self.history.undo.push(Step(label.clone(), cur, meta));",
            ],
        ),
        // a new project from a template replaces it
        ("project_tools.rs", &["s.project = std::sync::Arc::new(proj);"]),
        // a dynamic trim puts back the project before it for each live update and after a refused one, then commits its
        // one step or puts that project back; a cancel puts it back too (note_project_replaced)
        (
            "trim.rs",
            &[
                "s.project = d.before.clone();",
                "s.project = d.before.clone();",
                "s.project = d.before;",
                "s.history.undo.push(crate::Step(label.to_string(), d.before, meta));",
                "s.project = d.before;",
            ],
        ),
        // the validity store's weak reference to the snapshot it validated, not the session's project
        ("vertical.rs", &["known.project = Weak::clone(&now);"]),
    ];
    let reviewed: BTreeMap<String, Vec<String>> =
        reviewed.iter().map(|(file, lines)| (String::from(*file), lines.iter().copied().map(String::from).collect())).collect();
    assert_eq!(change_sites(), reviewed, "a new place that changes the session's project must be reviewed against the edit hook");
    // no vertical command commits through a hooked path
    for (name, text) in [("vertical.rs", include_str!("vertical.rs")), ("vertical_measure.rs", include_str!("vertical_measure.rs"))] {
        for call in [".edit(", "edit_merged(", "edit_sequence(", "edit_sequence_as("] {
            assert!(!text.contains(call), "{name} commits through `{call}`");
        }
    }
}

// ---------- T-N19 and T-M14: gestures ----------

#[test]
fn t_n19_twenty_merged_plan_only_sub_steps_run_no_hook() {
    let mut f = fixture();
    let runs = f.s.vertical_hook.runs;
    ok(&mut f.s, "markers.add", json!({}));
    assert_eq!(f.s.vertical_hook.runs, runs + 1, "a 16:9 edit runs the edit hook once");
    // a drag of 20 merged sub-steps through the plan-only path runs none, nor do its undo and redo
    let (runs, steps, picture) = (f.s.vertical_hook.runs, f.s.history.undo.len(), f.s.picture_revision);
    for k in 1..=20 {
        ok(&mut f.s, "vertical.setPosition", json!({"plan": 1, "frame": 150, "cx": 700 + k, "merge": "drag:4"}));
    }
    assert_eq!(f.s.vertical_hook.runs, runs, "no plan-only sub-step runs the hook");
    assert_eq!((f.s.history.undo.len(), f.s.picture_revision), (steps + 1, picture), "the drag is one plan-only step");
    f.s.undo();
    f.s.redo();
    assert_eq!(f.s.vertical_hook.runs, runs, "nor do the drag's undo and redo");
}

#[test]
fn t_m14_a_trim_drag_through_edit_merged_that_returns_to_its_start_leaves_the_plan_bit_identical() {
    let mut f = fixture();
    let (seq, c2, a2) = (f.seq, f.clips[1], f.sounds[1]);
    let start = set_in(&f.s.project, seq).clone();
    let steps = f.s.history.undo.len();
    // one sub-step of a ripple trim drag of C2's tail (with its sound), as the Timeline makes it: a frame at a time
    let drag = |s: &mut Session, frames: i64| {
        s.edit_sequence_as("Ripple Trim", Some("ve-hook:trim-drag"), move |q, ctx, _| {
            filmcraft_edit::ripple_trim_group(q, &[c2, a2], Edge::Out, at(frames), ctx)?;
            Ok(())
        })
        .expect("a sub-step of the drag");
    };
    for _ in 0..45 {
        drag(&mut f.s, -1);
    }
    // mid-drag the plan follows the trim from the gesture's base: #5 at 260 lay on the frames the trim took away, and
    // the overlay's opener #6 moved with the overlay
    let mid = plan_in(&f.s.project, seq);
    assert!(position(mid, 5).is_none(), "#5 at 260 lies past C2's end at 255");
    assert_eq!(position(mid, 6).map(|p| fr(p.at)), Some(255), "the overlay's opener follows the overlay");
    for _ in 0..45 {
        drag(&mut f.s, 1);
    }
    assert_eq!(f.s.history.undo.len(), steps + 1, "the drag is one undo step");
    assert_eq!(set_in(&f.s.project, seq), &start, "back at its start, the plan is bit-identical");
    assert_eq!(saved(set_in(&f.s.project, seq)), saved(&start));
}

#[test]
fn t_m14_a_trim_drag_keeps_its_base_when_the_project_is_replaced_between_its_sub_steps() {
    // a finished background import, a save or the source graphics' sync can put a new snapshot of the same project in the
    // session between two sub-steps of a drag: the drag still follows from where it began, so its return is bit-identical
    let mut f = fixture();
    let (seq, c2, a2) = (f.seq, f.clips[1], f.sounds[1]);
    let (start, steps) = (set_in(&f.s.project, seq).clone(), f.s.history.undo.len());
    let drag = |s: &mut Session, frames: i64| {
        s.edit_sequence_as("Ripple Trim", Some("ve-hook:trim-drag"), move |q, ctx, _| {
            filmcraft_edit::ripple_trim_group(q, &[c2, a2], Edge::Out, at(frames), ctx)?;
            Ok(())
        })
        .expect("a sub-step of the drag");
    };
    for _ in 0..45 {
        drag(&mut f.s, -1);
    }
    assert!(position(plan_in(&f.s.project, seq), 5).is_none(), "#5 at 260 lies past C2's end at 255");
    // what `Arc::make_mut` does to a snapshot that something else holds: the same project in a new allocation
    f.s.project = Arc::new((*f.s.project).clone());
    for _ in 0..45 {
        drag(&mut f.s, 1);
    }
    assert_eq!(f.s.history.undo.len(), steps + 1, "the drag is one undo step");
    assert_eq!(set_in(&f.s.project, seq), &start, "back at its start, the plan is bit-identical: #5 is back");
    assert_eq!(top_label(&f.s), "Ripple Trim", "with no suffix: from its start, the drag changed nothing");
}

#[test]
fn a_merged_gesture_that_began_with_a_plan_only_sub_step_keeps_it() {
    let mut f = fixture();
    let (seq, c1, a1) = (f.seq, f.clips[0], f.sounds[0]);
    let (label, key) = ("Vertical: move window (#4)", "ve-hook:mixed");
    f.s.edit_plan_only(seq, label, Some(key), |_, set, _| {
        if let Some(p) = set.as_deref_mut().and_then(|set| set.plans.first_mut()).and_then(|plan| plan.positions.iter_mut().find(|p| p.id == 4)) {
            p.window.cx = 720.0;
        }
        Ok(())
    })
    .expect("a plan-only sub-step");
    // a picture sub-step with the same key and label folds into the step and follows the picture from where the
    // plan-only sub-step left the plan
    f.s.edit_sequence_as(label, Some(key), move |q, _, _| {
        filmcraft_edit::ripple_delete_items(q, &[c1, a1])?;
        Ok(())
    })
    .expect("a picture sub-step");
    assert_eq!(f.s.history.undo.len(), 1, "one step");
    assert!(f.s.history.undo[0].2.picture, "the picture sub-step made the gesture a picture step");
    let p4 = position(plan_in(&f.s.project, seq), 4).expect("#4");
    assert_eq!((fr(p4.at), p4.window.cx), (0, 720.0), "#4 opens C2 at its new first frame, with the window the drag gave it");
}

// ---------- the label, validation, I16 and ids ----------

#[test]
fn the_undo_label_carries_the_hook_s_suffix_which_never_stops_a_gesture_from_merging() {
    let mut f = fixture();
    let [_, c2, c3, _] = f.clips.map(|c| c.0);
    ok(&mut f.s, "edit.rippleDelete", json!({"clips": [c2]}));
    let suffix = f.s.hook_report().and_then(v::FollowReport::label_suffix).expect("the ripple moved positions");
    assert!(suffix.starts_with("\u{b7} vertical: ") && suffix.contains(" moved"), "{suffix}");
    assert_eq!(top_label(&f.s), format!("Ripple Delete {suffix}"));
    let listed = ok(&mut f.s, "history.list", json!({}));
    assert_eq!(listed["undo"].as_array().and_then(|undo| undo.last()), Some(&json!(format!("Ripple Delete {suffix}"))));
    // an edit that changes no vertical data has no suffix
    ok(&mut f.s, "markers.add", json!({}));
    assert_eq!((top_label(&f.s).as_str(), f.s.hook_report().is_none()), ("Add Marker", true));
    // a gesture: the suffix takes no part in the merge comparison, and the step's label follows its last sub-step
    let steps = f.s.history.undo.len();
    let scale = |value: f64| json!({"clip": c3, "effect": "motion", "param": "scale", "value": value, "merge": true});
    let drag = |s: &mut Session, value: f64| ok(s, "effects.setParam", scale(value));
    drag(&mut f.s, 110.0);
    let first = top_label(&f.s);
    assert!(first.starts_with("Change Effect Parameter \u{b7} vertical: "), "a Motion change flags the positions over the clip: {first}");
    drag(&mut f.s, 120.0);
    assert_eq!(f.s.history.undo.len(), steps + 1, "the second sub-step folds into the first, whose label carries the suffix");
    let suffix = f.s.hook_report().and_then(v::FollowReport::label_suffix).expect("the positions over C3 are flagged");
    assert_eq!(top_label(&f.s), format!("Change Effect Parameter {suffix}"));
}

#[test]
fn the_session_validates_the_vertical_sets_right_after_the_hook() {
    let mut f = fixture();
    ok(&mut f.s, "vertical.inspect", json!({"plan": 1}));
    let current = |s: &Session| {
        let known = s.vertical_store.validity.get(&f.seq).expect("the sequence was validated");
        std::ptr::eq(known.project.as_ptr(), Arc::as_ptr(&s.project)) && known.violations.is_empty()
    };
    assert!(current(&f.s), "a read validated the installed project");
    ok(&mut f.s, "edit.rippleDelete", json!({"clips": [f.clips[1].0]}));
    assert!(current(&f.s), "the 16:9 edit's snapshot was validated after the hook, before any read");
    for value in [110.0, 120.0] {
        ok(&mut f.s, "effects.setParam", json!({"clip": f.clips[2].0, "effect": "motion", "param": "scale", "value": value, "merge": true}));
        assert!(current(&f.s), "and so is every merged sub-step's");
    }
}

#[test]
fn i16_a_person_s_ripple_removes_a_locked_position_with_its_picture_and_reports_it() {
    let mut f = fixture();
    ok(&mut f.s, "vertical.setLock", json!({"plan": 1, "positions": [5], "locked": true}));
    ok(&mut f.s, "edit.rippleDelete", json!({"clips": [f.clips[1].0]}));
    // the person's edit applies: the clip is gone, and the locked position went with its picture
    assert!(seq_in(&f.s.project, f.seq).find_item(f.clips[1]).is_none(), "the ripple applied");
    assert!(position(plan_in(&f.s.project, f.seq), 5).is_none(), "the locked #5 lay on the deleted clip");
    // and it is reported: in the hook's report, in the step's label and in the Events log
    let report = f.s.hook_report().cloned().expect("the hook's report");
    let entry = report.sequences.iter().find(|q| q.sequence == f.seq).expect("Main's entry");
    let plan = entry.plans.iter().find(|p| p.plan == 1).expect("plan 1's entry");
    assert!(plan.removed.contains(&5) && plan.locked.contains(&5), "{plan:?}");
    let removed = json!({"plan": 1, "position": 5, "frame": 260, "locked": true});
    assert!(entry.changes.iter().any(|c| c.code == "position_removed" && c.detail == removed), "{:?}", entry.changes);
    assert!(top_label(&f.s).contains(" removed"), "{}", top_label(&f.s));
    let warnings: Vec<&str> = f.s.log.entries.iter().filter(|e| e.level == Level::Warning && e.source == "vertical").map(|e| e.message.as_str()).collect();
    assert!(warnings.iter().any(|m| m.contains("locked") && m.contains("#5")), "the Events log tells the person: {warnings:?}");
    // undo brings both back
    f.s.undo();
    assert!(position(plan_in(&f.s.project, f.seq), 5).is_some_and(|p| p.locked));
}

#[test]
fn the_hook_hands_out_ids_above_the_session_s_high_water_marks() {
    let mut f = fixture();
    // a position made and undone: its id stays handed out (the marks live outside undo)
    let r = ok(&mut f.s, "vertical.setPosition", json!({"plan": 1, "frame": 280, "cx": 1100}));
    assert_eq!(r["added"], json!([13]));
    f.s.undo();
    assert_eq!(plan_in(&f.s.project, f.seq).next_position_id, 13, "undo rolled the counter back");
    // an extract inside #12's move: the hook splits the move exactly on both sides of the junction
    for (cmd, p) in marked(200, 209, "sequence.extract") {
        ok(&mut f.s, cmd, p);
    }
    let added = added_ids(f.s.hook_report());
    assert!(!added.is_empty(), "the hook added exact splits");
    assert!(added.iter().all(|id| *id >= 14), "id 13 was handed out before its undo: {added:?}");
}

// ---------- the cost of the hook: digests cached per snapshot and per item ----------

#[test]
fn an_edit_digests_only_its_own_result_and_unchanged_items_reuse_their_hash() {
    let mut f = fixture();
    let counts = |s: &Session| (s.vertical_hook.runs, s.vertical_hook.snapshots_digested, s.vertical_hook.items_hashed);
    let (runs, digested, hashed) = counts(&f.s);
    // the first edit digests the installed project and its own result; Main's five picture clips are hashed once
    ok(&mut f.s, "markers.add", json!({}));
    assert_eq!(counts(&f.s), (runs + 1, digested + 2, hashed + 5), "the first edit");
    // later edits find the snapshot before them in the cache, and no item changed
    for (k, frame) in [(1, 10), (2, 20), (3, 30), (4, 40)] {
        ok(&mut f.s, "markers.add", json!({"time": at(frame).0}));
        assert_eq!(counts(&f.s), (runs + 1 + k, digested + 2 + k, hashed + 5), "edit {k}");
    }
    // a snapshot undo brings back is still in the cache
    f.s.undo();
    ok(&mut f.s, "markers.add", json!({"time": at(100).0}));
    assert_eq!(counts(&f.s), (runs + 6, digested + 7, hashed + 5), "an edit after an undo");
    // a framing change hashes the one item it changed, a ripple none, and a razor its new clip
    ok(&mut f.s, "effects.setParam", json!({"clip": f.clips[1].0, "effect": "motion", "param": "scale", "value": 110.0}));
    assert_eq!(counts(&f.s), (runs + 7, digested + 8, hashed + 6), "a Motion change of C2");
    ok(&mut f.s, "edit.rippleDelete", json!({"clips": [f.clips[3].0]}));
    assert_eq!(counts(&f.s), (runs + 8, digested + 9, hashed + 6), "a ripple delete of C4");
    ok(&mut f.s, "timeline.razor", json!({"time": at(220).0, "track": "V1"}));
    assert_eq!(counts(&f.s), (runs + 9, digested + 10, hashed + 7), "a razor of C2");
}

#[test]
fn an_edit_that_changes_no_picture_digest_copies_no_base_to_raise_its_ids() {
    let mut f = fixture();
    // a position made and undone leaves the session's id marks ahead of the plan's counter
    ok(&mut f.s, "vertical.setPosition", json!({"plan": 1, "frame": 280, "cx": 1100}));
    f.s.undo();
    let (raises, plan) = (f.s.vertical_hook.raises, plan_in(&f.s.project, f.seq).clone());
    for frame in [10, 20, 30] {
        ok(&mut f.s, "markers.add", json!({"time": at(frame).0}));
    }
    assert_eq!(f.s.vertical_hook.raises, raises, "a marker changes no picture digest: no project is copied to raise ids");
    assert_eq!(plan_in(&f.s.project, f.seq), &plan, "and the plan is as it was");
    // a drag of the picture copies its base once, at its first sub-step
    for value in [110.0, 120.0, 130.0] {
        ok(&mut f.s, "effects.setParam", json!({"clip": f.clips[1].0, "effect": "motion", "param": "scale", "value": value, "merge": true}));
    }
    assert_eq!(f.s.vertical_hook.raises, raises + 1, "one copy for the whole drag");
    // an edit that hands out ids copies its base, and its ids come after the undone position's
    for (cmd, p) in marked(200, 209, "sequence.extract") {
        ok(&mut f.s, cmd, p);
    }
    assert_eq!(f.s.vertical_hook.raises, raises + 2, "the extract copies its base");
    let added = added_ids(f.s.hook_report());
    assert!(!added.is_empty() && added.iter().all(|id| *id >= 14), "id 13 was handed out before its undo: {added:?}");
}

/// The model's layout and framing digests of sequence `seq` of `p`.
fn model_digests(p: &Project, seq: ItemId) -> (String, String) {
    (v::layout_digest(p, seq_in(p, seq)), v::framing_digest(p, seq_in(p, seq)))
}

#[test]
fn the_cached_digests_agree_with_the_model_s_and_each_snapshot_is_digested_once() {
    let f = fixture();
    let (base, seq, c2) = (Arc::clone(&f.s.project), f.seq, f.clips[1]);
    let mutations: [(&str, fn(&mut TrackItem)); 11] = [
        ("Motion scale", |i| set_param(i, "motion", "scale", 120.0)),
        ("Motion switched off", |i| i.effect_mut("motion").expect("Motion").enabled = false),
        ("opacity, which frames nothing", |i| set_param(i, "opacity", "opacity", 50.0)),
        ("a blur, which frames nothing", |i| i.effects.push(effect("gaussian_blur"))),
        ("a crop", |i| i.effects.push(effect("crop"))),
        ("a crop switched off", |i| {
            let mut crop = effect("crop");
            crop.enabled = false;
            i.effects.push(crop);
        }),
        ("a horizontal flip (Transform)", |i| i.effects.push(effect("horizontal_flip"))),
        ("time remapping", |i| set_param(i, "time_remap", "speed", 50.0)),
        ("Scale to Frame Size", |i| i.scale_to_frame = true),
        ("the clip switched off", |i| i.enabled = false),
        ("the clip ten frames later", |i| i.start += at(10)),
    ];
    let mut hook = VerticalHook::default();
    let (layout0, framing0) = model_digests(&base, seq);
    let ours0 = hook.digests(&base).sequences.get(&seq).cloned().expect("Main's digests");
    assert_eq!(ours0.layout, layout0, "the layout digest is the model's");
    for (what, mutate) in mutations {
        let mut p = (*base).clone();
        mutate(p.sequence_mut(seq).and_then(|q| q.find_item_mut(c2)).map(|(_, i)| i).expect("C2"));
        let p = Arc::new(p);
        let (layout, framing) = model_digests(&p, seq);
        let ours = hook.digests(&p).sequences.get(&seq).cloned().expect("Main's digests");
        assert_eq!(ours.layout, layout, "{what}: the layout digest is the model's");
        assert_eq!(ours.framing != ours0.framing, framing != framing0, "{what}: the cached framing digest changes when the model's does");
        assert_eq!(hook.digests(&base).sequences.get(&seq), Some(&ours0), "{what}: the base snapshot's digests are kept");
    }
    assert_eq!(hook.snapshots_digested, 12, "each snapshot was digested once: the base and the eleven edits");
}
