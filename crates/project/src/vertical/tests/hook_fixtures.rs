//! Fixtures of the edit-hook and remap tests (lane L1c, M2 stage 2). `filmcraft-project` cannot call FilmCraft's edit
//! commands (`filmcraft-edit` depends on it), so each test edits a copy of the project as the command would, on video
//! track 0 of a 25p sequence whose clips sit on the frame grid, and knows on its own where every old frame went: the oracle
//! map, a `Vec<Option<i64>>` indexed by old frame (None: the frame's content is gone). The bound is computed here again from
//! lane L1a's corrected split bound (round 4, `tests/splits.rs`), not taken from the code under test. Synthetic data only.

use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaSource};
use filmcraft_time::{Tick, TimeRange};

use super::fixtures::{Fx, RATE, tk};
use crate::vertical::*;
use crate::{ClipId, ItemId, ItemKind, Label, MediaClip, MediaRef, ParamValue, Project, Sequence, Track, TrackItem, TrackKind};

/// Float noise allowed on top of a bound.
pub const NOISE: f64 = 1e-9;

/// The frame of a tick at the fixtures' rate.
pub fn fr(t: Tick) -> i64 {
    RATE.frame_at(t)
}

/// A second synthetic matte in `project`: inserted and replacing clips show it.
pub fn add_matte(project: &mut Project, name: &str) -> ItemId {
    let generator = Generator::ColorMatte { color: [0.6, 0.2, 0.1, 1.0] };
    let source = GeneratorSource::new(generator.clone(), 1920, 1080, RATE, tk(400_000));
    let clip = MediaClip {
        media: MediaRef::Generator(generator),
        info: source.info().clone(),
        interpret: Default::default(),
        mark_in: None,
        mark_out: None,
        markers: Vec::new(),
        offline: false,
        proxy: None,
        identity: None,
    };
    project.add_item(name, Label::Mango, ItemKind::Media(clip), None)
}

/// A nested 25p 1920x1080 sequence in `fx`'s project whose first video track holds one clip of `fx`'s matte, `frames` long
/// from source frame `source`. Returns the nested sequence and its clip.
pub fn add_nest(fx: &mut Fx, frames: i64, source: i64) -> (ItemId, ClipId) {
    let settings = crate::SequenceSettings { frame_rate: RATE, ..Default::default() };
    let nest = fx.project.new_sequence("Nest", settings, 1, 0, None);
    let media = fx.media;
    let clip = fx.make_item(media, tk(0), frames, source);
    let id = clip.id;
    video_mut(&mut fx.project, nest, 0).items.push(clip);
    (nest, id)
}

/// Sequence `seq` of `project`.
pub fn seq_of(project: &Project, seq: ItemId) -> &Sequence {
    project.sequence(seq).expect("the sequence exists")
}

/// Video track `track` of sequence `seq` of `project`, to edit.
pub fn video_mut(project: &mut Project, seq: ItemId, track: usize) -> &mut Track {
    project.sequence_mut(seq).and_then(|s| s.video_tracks.get_mut(track)).expect("the video track exists")
}

/// The video track item `clip` of sequence `seq`, to edit.
pub fn clip_mut(project: &mut Project, seq: ItemId, clip: ClipId) -> &mut TrackItem {
    project.sequence_mut(seq).and_then(|s| s.video_tracks.iter_mut().find_map(|t| t.item_mut(clip))).expect("the clip exists")
}

/// The ids of the clips of video track 0 of sequence `seq`, in time order.
pub fn clip_ids(project: &Project, seq: ItemId) -> Vec<ClipId> {
    seq_of(project, seq).video_tracks.first().map(|t| t.items.iter().map(|i| i.id).collect::<Vec<ClipId>>()).unwrap_or_default()
}

/// A copy of `base` edited by `edit`.
pub fn edited(base: &Project, edit: impl FnOnce(&mut Project)) -> Project {
    let mut project = base.clone();
    edit(&mut project);
    project
}

/// Whether two projects are the same as saved, byte for byte. Not `PartialEq`: a clip's default Motion anchor is a NaN
/// sentinel (the clip's centre), so a project with a clip never equals even its own clone.
pub fn same_project(a: &Project, b: &Project) -> bool {
    let saved = |p: &Project| serde_json::to_string(p).expect("a project saves");
    saved(a) == saved(b)
}

/// The vertical set of sequence `seq` of `project`.
pub fn set_in(project: &Project, seq: ItemId) -> &VerticalSet {
    seq_of(project, seq).vertical.as_deref().expect("the sequence has a vertical set")
}

/// Plan `plan` of sequence `seq` of `project`.
pub fn plan_in(project: &Project, seq: ItemId, plan: u32) -> &VerticalPlan {
    set_in(project, seq).plans.iter().find(|p| p.id == plan).expect("the plan exists")
}

/// Position `id` of a plan, if it holds one.
pub fn find(plan: &VerticalPlan, id: u32) -> Option<&Position> {
    plan.positions.iter().find(|p| p.id == id)
}

/// The frames of a plan's positions, in stored order.
pub fn frames_of(plan: &VerticalPlan) -> Vec<i64> {
    plan.positions.iter().map(|p| fr(p.at)).collect()
}

/// The ids of a plan's positions, in stored order.
pub fn ids_of(plan: &VerticalPlan) -> Vec<u32> {
    plan.positions.iter().map(|p| p.id).collect()
}

/// The frames of the stored walls of sequence `seq` with `origin`.
pub fn stored_walls(project: &Project, seq: ItemId, origin: WallOrigin) -> Vec<i64> {
    set_in(project, seq).walls.stored.iter().filter(|w| w.origin == origin).map(|w| fr(w.at)).collect()
}

/// The frames of the effective walls of sequence `seq`.
pub fn wall_frames_of(project: &Project, seq: ItemId) -> Vec<i64> {
    walls(project, seq_of(project, seq)).iter().map(|w| fr(w.at)).collect()
}

/// A person's annotation of `kind` over frames `[from, to)`.
pub fn annotation(id: u32, kind: &str, from: i64, to: i64) -> Annotation {
    Annotation { id, kind: kind.into(), from: tk(from), to: tk(to), label: String::new(), score: None, origin: Origin::Manual, modified_by: Writer::Human }
}

/// A box-track key at `frame`.
pub fn key(frame: i64) -> KeyBox {
    KeyBox { at: tk(frame), rect: [700.0, 300.0, 200.0, 240.0], facing: None, gone: false }
}

/// The box track `char:dana` holding `keys`.
pub fn dana(keys: Vec<KeyBox>) -> BoxTrack {
    BoxTrack { id: "char:dana".into(), label: String::new(), kind: String::new(), keys, origin: Origin::Manual, modified_by: Writer::Human }
}

/// A person's attention mark at `frame`.
pub fn mark(frame: i64) -> AttentionMark {
    AttentionMark { at: tk(frame), by: "Dana Editor".into(), when: 1_790_000_100, note: String::new() }
}

/// Approves part `part` in plan `plan` of `fx` with both of its digests as they are now, as `vertical.approvePart` stores
/// them.
pub fn approve(fx: &mut Fx, plan: u32, part: u32) {
    let (seq, current) = (fx.seq().clone(), plan_in(&fx.project, fx.seq, plan).clone());
    let found = set_in(&fx.project, fx.seq).parts.iter().find(|p| p.id == part).expect("the part exists").clone();
    let framing_digest = range_digest(&fx.project, &seq, &current, fr(found.from), fr(found.to));
    let output = output_digest(&fx.project, &seq, &current, &found);
    let approval = PartApproval { part, framing_digest, output_digest: output, by: "Dana Editor".into(), when: 1_790_000_000, note: String::new() };
    fx.plan_mut(plan).part_approvals.push(approval);
}

/// Sets a static parameter of an effect of `item`.
pub fn set_param(item: &mut TrackItem, effect: &str, param: &str, value: ParamValue) {
    let p = item.effect_mut(effect).and_then(|e| e.param_mut(param)).expect("the parameter exists");
    p.value = value;
}

/// The edit hook run as a person; a refusal fails the test.
pub fn follow(before: &Project, after: &mut Project) -> FollowReport {
    match follow_picture(before, after, PERSON) {
        Ok(report) => report,
        Err(e) => panic!("the hook refused a person's edit: {} {} {}", e.code, e.message, e.detail),
    }
}

/// What the report says about plan `plan` of sequence `seq` (empty when the hook did not change it).
pub fn plan_report(report: &FollowReport, seq: ItemId, plan: u32) -> PlanFollow {
    let found = report.sequences.iter().filter(|s| s.sequence == seq).flat_map(|s| &s.plans).find(|p| p.plan == plan);
    found.cloned().unwrap_or(PlanFollow { plan, ..PlanFollow::default() })
}

/// The changes the report lists for sequence `seq` with `code`.
pub fn report_changes<'a>(report: &'a FollowReport, seq: ItemId, code: &str) -> Vec<&'a Change> {
    report.sequences.iter().filter(|s| s.sequence == seq).flat_map(|s| &s.changes).filter(|c| c.code == code).collect()
}

/// Ripple-deletes frames `[from, to)` of video track 0: clips after the range move left, a clip that holds the whole range
/// is razored (its right part is a new clip), and clips inside it go.
pub fn ripple_delete(project: &mut Project, seq: ItemId, from: i64, to: i64) {
    let cut = to - from;
    let items = std::mem::take(&mut video_mut(project, seq, 0).items);
    let mut out = Vec::new();
    for mut item in items {
        let (s, e) = (fr(item.start), fr(item.end()));
        if e <= from {
            out.push(item);
        } else if s >= to {
            item.start = tk(s - cut);
            out.push(item);
        } else if s < from && e > to {
            let mut right = item.clone();
            right.id = ClipId(project.alloc_id());
            right.start = tk(from);
            right.source_in = Tick(item.source_in.0 + tk(to - s).0);
            right.duration = tk(e - to);
            item.duration = tk(from - s);
            out.push(item);
            out.push(right);
        } else if s < from {
            item.duration = tk(from - s);
            out.push(item);
        } else if e > to {
            item.source_in = Tick(item.source_in.0 + tk(to - s).0);
            item.start = tk(from);
            item.duration = tk(e - to);
            out.push(item);
        }
    }
    video_mut(project, seq, 0).items = out;
}

/// Ripple-inserts a clip of `media`, `frames` long from source frame `source`, at frame `at` of video track 0: clips at or
/// after it move right, and a clip that holds `at` is razored there (its right part is a new clip). Returns the new clip.
pub fn ripple_insert(project: &mut Project, seq: ItemId, at: i64, media: ItemId, frames: i64, source: i64) -> ClipId {
    let items = std::mem::take(&mut video_mut(project, seq, 0).items);
    let mut out = Vec::new();
    for mut item in items {
        let (s, e) = (fr(item.start), fr(item.end()));
        if e <= at {
            out.push(item);
        } else if s >= at {
            item.start = tk(s + frames);
            out.push(item);
        } else {
            let mut right = item.clone();
            right.id = ClipId(project.alloc_id());
            right.start = tk(at + frames);
            right.source_in = Tick(item.source_in.0 + tk(at - s).0);
            right.duration = tk(e - at);
            item.duration = tk(at - s);
            out.push(item);
            out.push(right);
        }
    }
    let new = project.make_track_item(media, TrackKind::Video, tk(at), TimeRange::new(tk(source), tk(frames)), RATE).expect("the media exists");
    let id = new.id;
    out.push(new);
    out.sort_by_key(|i| i.start);
    video_mut(project, seq, 0).items = out;
    id
}

/// Razors video track 0 at frame `at` (Add Edit): the clip holding it ends there and a new clip continues its media.
/// Returns the new clip.
pub fn razor(project: &mut Project, seq: ItemId, at: i64) -> ClipId {
    let id = ClipId(project.alloc_id());
    let track = video_mut(project, seq, 0);
    let k = track.items.iter().position(|i| fr(i.start) < at && at < fr(i.end())).expect("a clip holds the frame");
    let mut right = track.items[k].clone();
    let s = fr(right.start);
    right.id = id;
    right.start = tk(at);
    right.source_in = Tick(right.source_in.0 + tk(at - s).0);
    right.duration = Tick(track.items[k].end().0 - tk(at).0);
    track.items[k].duration = tk(at - s);
    track.items.insert(k + 1, right);
    id
}

/// A ripple trim of clip `clip`'s tail by `by` frames (positive extends it with its own media, negative shortens it): the
/// clips after it move by `by`.
pub fn trim_tail(project: &mut Project, seq: ItemId, clip: ClipId, by: i64) {
    let track = video_mut(project, seq, 0);
    let end = track.items.iter().find(|i| i.id == clip).map(|i| fr(i.end())).expect("the clip exists");
    for item in &mut track.items {
        if item.id == clip {
            item.duration = Tick(item.duration.0 + tk(by).0);
        } else if fr(item.start) >= end {
            item.start = Tick(item.start.0 + tk(by).0);
        }
    }
}

/// A ripple trim of clip `clip`'s head: its first `by` frames go, it keeps its start, and the clips after it move left.
pub fn trim_head(project: &mut Project, seq: ItemId, clip: ClipId, by: i64) {
    let track = video_mut(project, seq, 0);
    let end = track.items.iter().find(|i| i.id == clip).map(|i| fr(i.end())).expect("the clip exists");
    for item in &mut track.items {
        if item.id == clip {
            item.source_in = Tick(item.source_in.0 + tk(by).0);
            item.duration = Tick(item.duration.0 - tk(by).0);
        } else if fr(item.start) >= end {
            item.start = Tick(item.start.0 - tk(by).0);
        }
    }
}

/// A roll of the edit between clip `left` and clip `right` that follows it: `by` frames move from `left`'s tail to
/// `right`'s head, which shows `by` more frames of its own media. Nothing else moves.
pub fn roll(project: &mut Project, seq: ItemId, left: ClipId, right: ClipId, by: i64) {
    for item in &mut video_mut(project, seq, 0).items {
        if item.id == left {
            item.duration = Tick(item.duration.0 - tk(by).0);
        } else if item.id == right {
            item.start = Tick(item.start.0 - tk(by).0);
            item.source_in = Tick(item.source_in.0 - tk(by).0);
            item.duration = Tick(item.duration.0 + tk(by).0);
        }
    }
}

/// Swaps clip `index` of video track 0 with the clip after it, as an insert move does: both keep their ids.
pub fn swap(project: &mut Project, seq: ItemId, index: usize) {
    let track = video_mut(project, seq, 0);
    let start = track.items[index].start;
    let second = track.items[index + 1].duration;
    track.items[index + 1].start = start;
    track.items[index].start = start + second;
    track.sort();
}

/// Replaces clip `index` of video track 0 by a new clip of `media` over the same frames, from source frame `source`.
pub fn replace(project: &mut Project, seq: ItemId, index: usize, media: ItemId, source: i64) {
    let track = video_mut(project, seq, 0);
    let (start, duration) = (track.items[index].start, track.items[index].duration);
    let new = project.make_track_item(media, TrackKind::Video, start, TimeRange::new(tk(source), duration), RATE).expect("the media exists");
    video_mut(project, seq, 0).items[index] = new;
}

/// Plays clip `clip` at `speed`, `frames` long, from the same source in: the clips after it move with its end.
pub fn retime(project: &mut Project, seq: ItemId, clip: ClipId, speed: f64, frames: i64) {
    let track = video_mut(project, seq, 0);
    let (end, old) = track.items.iter().find(|i| i.id == clip).map(|i| (fr(i.end()), fr(i.duration))).expect("the clip exists");
    for item in &mut track.items {
        if item.id == clip {
            item.speed = speed;
            item.duration = tk(frames);
        } else if fr(item.start) >= end {
            item.start = Tick(item.start.0 + tk(frames - old).0);
        }
    }
}

/// The oracle of a ripple delete of `[x, y)` on a timeline of `end` frames.
pub fn deleted(end: i64, x: i64, y: i64) -> Vec<Option<i64>> {
    (0..end)
        .map(|f| {
            if f < x {
                Some(f)
            } else if f < y {
                None
            } else {
                Some(f - (y - x))
            }
        })
        .collect()
}

/// The oracle of a ripple insert of `n` frames at `x` on a timeline of `end` frames.
pub fn inserted(end: i64, x: i64, n: i64) -> Vec<Option<i64>> {
    (0..end).map(|f| Some(if f < x { f } else { f + n })).collect()
}

/// The oracle of `first` then `second` (`second` indexed by the frames `first` gives).
pub fn then(first: &[Option<i64>], second: &[Option<i64>]) -> Vec<Option<i64>> {
    first.iter().map(|g| g.and_then(|g| usize::try_from(g).ok().and_then(|g| second.get(g).copied().flatten()))).collect()
}

/// The largest change of `cx`, `cy` or `h` between two windows.
pub fn change(a: &Window, b: &Window) -> f64 {
    (a.cx - b.cx).abs().max((a.cy - b.cy).abs()).max((a.h - b.h).abs())
}

/// Two split bounds of a move from `from` to `to` (lane L1a round 4): `2 (delta + travel delta_h / (8 h_lo))`, with the
/// clamped `delta = 0.001 + 0.00025 max(1, OW / OH)`, `delta_h = 0.001`, `travel` the move's largest change of `cx`, `cy` or
/// `h`, `h_lo` its smaller end height, and no travel term for a pure pan. It covers the unclamped case (0.0005) too.
pub fn two_splits(from: &Window, to: &Window, out: OutputSize) -> f64 {
    let aspect = f64::from(out.width) / f64::from(out.height);
    let delta = 0.001 + 0.00025 * aspect.max(1.0);
    let tilt = if from.h == to.h { 0.0 } else { 0.001 / from.h.min(to.h) };
    2.0 * (delta + change(from, to) * tilt / 8.0)
}

/// The hook's bound at `frame` of `plan`: 0 where the window holds, two split bounds of the move inside a move.
pub fn bound_at(plan: &VerticalPlan, r: &Raster, frame: i64) -> f64 {
    let s = window_at(plan, r, r.rate.tick_of(frame));
    if s.phase != Phase::Move {
        return 0.0;
    }
    let ends = (s.position.and_then(|id| find(plan, id)), s.next.and_then(|id| find(plan, id)));
    let (Some(from), Some(to)) = ends else { return f64::INFINITY };
    two_splits(&from.window, &to.window, plan.output)
}

/// Asserts that every old frame `f` that `map` keeps shows at `map[f]` in `after` the layout and, within [`bound_at`] of the
/// old plan, the window it showed in `before`.
pub fn assert_follows(before: &Project, after: &Project, seq: ItemId, plan: u32, map: &[Option<i64>], what: &str) {
    let (r0, r1) = (raster(seq_of(before, seq)), raster(seq_of(after, seq)));
    let (p0, p1) = (plan_in(before, seq, plan), plan_in(after, seq, plan));
    for (f, g) in (0_i64..).zip(map) {
        let Some(g) = *g else { continue };
        let (a, b) = (window_at(p0, &r0, r0.rate.tick_of(f)), window_at(p1, &r1, r1.rate.tick_of(g)));
        let (moved, bound) = (change(&a.window, &b.window), bound_at(p0, &r0, f));
        assert!(moved <= bound + NOISE, "{what}: old frame {f} -> {g} moved {moved} px, bound {bound} ({:?} -> {:?})", a.window, b.window);
        assert_eq!(a.layout, b.layout, "{what}: old frame {f} -> {g}");
    }
}
