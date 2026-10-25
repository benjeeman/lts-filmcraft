//! The edit hook (spec section 4.8): the Vertical Edit follows every edit of the 16:9 picture.
//!
//! [`follow_picture`] runs after every journaled 16:9 edit (the engine calls it from `Session::edit` and from the merging
//! branch of `Session::edit_merged`, lane L2d) for every sequence that has a vertical set in `before` and still exists in
//! `after`. It always derives `after`'s vertical data from `before`'s, so a merged gesture's sub-step, followed from the
//! gesture's base, depends only on the base and the sub-step: a drag that returns to its start leaves the plan bit-identical.
//!
//! Two digests per sequence decide how much work an edit needs: the [`layout_digest`] (which frames show which picture)
//! and the [`framing_digest`] (Motion, time remapping and the geometry effects of the picture clips). Neither is the range
//! digest that an approval calls its framing digest. Both unchanged: nothing to do. Only the framing digest changed: no
//! frame moves, and the positions whose frames show a clip whose framing changed are flagged `PictureChanged`. The layout
//! changed, or a framing change moved a wall (a through edit whose pieces now differ in geometry is a shot change, section
//! 4.1, so a move may now hold a wall): the full algorithm, which works on intervals, never frame by frame (section 4.8,
//! steps 1-12). A set that broke an invariant before the edit is followed too, and the result reports what it still breaks.
//!
//! Each timeline is cut into runs that show one leaf clip through one chain of clips (nests and multicam clips are followed
//! to the renderer's nest depth, and where a nest shows nothing the tracks below it show through). It also lists the
//! picture it holds without showing it on top: the clips under the top one, the clips of switched-off tracks and
//! switched-off clips, and each video track of a nest (a multicam clip's other angles). The map matches old runs to new
//! ones by the media time each frame shows (a frame-held clip by its offset), in four passes: the same chain of clips, the
//! same leaf clip, the same media on the same top track, the same media on any track. The first three passes then run
//! again over the hidden picture for the old frames still unmatched, so content that an overlay, a switched-off track or
//! clip, or another multicam angle now hides (or that comes back from one) keeps its positions, flagged `PictureChanged`.
//! Runs without picture then follow their neighbours. A matched piece is exact (whole frames, the same timing), snapped
//! (the same timing on another frame grid or at a sub-frame offset: each old frame goes to the new frame whose media time
//! is nearest, ties to the earlier) or retimed (speed, direction or hold changed: the nearest frames too). A snapped or
//! retimed piece is found by bisection at its two ends and maps a frame on demand, so neither the work nor the memory of
//! an edit grows with its pieces' lengths. Old frames left over were removed; new frames left over are new content.
//!
//! Where the map breaks (a junction), every move is split exactly at the surviving frames on both sides, every split
//! computed on the plan before the edit, and the old content after a junction gets an exact anchor holding the window
//! that governed it (old content that ends at a junction before the first position, which showed that position's
//! window, gets one on its last frame), so a removed governing position, or content now preceded by other content,
//! keeps its framing. A junction whose picture does not continue gets a stored wall of origin `reanchored` where no
//! wall stands and no suppression keeps it free, so no move crosses it. Positions and every frame-based datum then move
//! through the map, and removed content takes its data with it (reported). An opener, and a stored wall, suppression or
//! candidate on its edit point, follows its clip onto an extended head (the opener flagged), new content inherits the
//! window before it, moves that now overlap their predecessor, hold a wall or touch a `fit` position are shortened
//! (`MoveShortened`), the anchors that change no frame and stand on no wall are dropped, and `PictureChanged` and
//! `TimeApproximate` flag what is not exact (a position kept where another landed on its frame too is flagged when the
//! two showed different windows). Video transitions are compared at every level a run shows. A frame of the same aspect
//! rescales every window and rect; another aspect leaves them and records on each plan the frame its windows were made on
//! (`VerticalPlan::framed_on`): the error `frame_changed` then holds until a person repairs the plan or the frame returns.
//! The engine caches the digests per project snapshot ([`picture_digests`], [`follow_picture_with`]).
//!
//! Ids the hook hands out come from `before`'s counters, never below the ids the plan holds. The session keeps id
//! high-water marks outside undo (spec section 2.6), so the engine passes a `before` whose counters are raised to them
//! (`VerticalIdMarks::raised`); in a merged gesture, the base raised with the marks as they stood when the gesture began.
//!
//! The guarantee: every new frame that shows content the old timeline showed keeps its window within [`follow_bound`] of
//! the old plan at the old frame (0 where the window holds; two split bounds inside a move: lane L1a's corrected split
//! bound `delta + delta_h travel / (8 h_lo)`, so not the spec's flat 2 delta). Extended heads, retimed content and
//! shortened moves are the exceptions, and each is flagged.
//!
//! For a person (the only caller in M2-M4) a locked position on removed content is removed, a shortened move and an inexact
//! remap are applied, and each is reported (I16). An agent caller's `locked` refusal is M5's (plan 63, D-12), behind the
//! same signature. `vertical.remap` ([`plan_remap`](super::remap::plan_remap)) carries a set through a caller's map with
//! the same machinery (`Mapping`, `carry_set`).

use std::collections::BTreeMap;

use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick};
use serde::Serialize;
use serde_json::json;

use super::canon::{clamp_window, floor_g, q};
use super::derive::{Scene, raster};
use super::digest::digest_of;
use super::eval::{centred_largest, move_span, window_at};
use super::output_digest::NEST_DEPTH;
use super::plan::{alloc, merge_ranges};
use super::types::{
    ActiveArea, Annotation, Arrival, AttentionMark, Caller, Candidate, Change, ConformReference, Curve, Flag, FrameRange, Issue, KeyBox, Layout, Level, Origin,
    OutputSize, OverwrittenRange, Part, Phase, Position, ProtectedRegion, Raster, Refusal, SplitOf, StoredWall, VE_MAX_FRAMES, VerticalPlan, VerticalSet,
    WallOrigin, Window, Writer,
};
use super::validate::{Violation, plan_only_violations, set_violations};
use super::walls::{is_geometry, shown_frame, shows_picture, walls, walls_in};
use crate::{ClipId, EffectInstance, ItemId, ItemKind, MulticamSel, Project, Sequence, Track, TrackId, TrackItem, Transition, TransitionId};

/// What the edit hook did to one plan. Each list holds position ids, sorted ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanFollow {
    pub plan: u32,
    /// Positions that moved with their picture: their frame changed.
    pub moved: Vec<u32>,
    /// Positions deleted: their picture is gone, or another position landed on their frame first.
    pub removed: Vec<u32>,
    /// Positions that gained a flag (`PictureChanged`, `MoveShortened`, `TimeApproximate`).
    pub flagged: Vec<u32>,
    /// Positions the hook added (origin `reanchored`): exact splits and anchors at junctions, openers.
    pub added: Vec<u32>,
    /// Locked positions the hook removed, shortened or remapped inexactly: done and reported for a person (I16); what an
    /// agent caller is refused for (M5).
    pub locked: Vec<u32>,
}

impl PlanFollow {
    /// Whether the report has nothing to say about the plan.
    fn is_empty(&self) -> bool {
        self.moved.is_empty() && self.removed.is_empty() && self.flagged.is_empty() && self.added.is_empty() && self.locked.is_empty()
    }
}

/// What the edit hook did to one sequence's vertical set.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceFollow {
    pub sequence: ItemId,
    /// The plans the hook changed, in plan order.
    pub plans: Vec<PlanFollow>,
    /// Everything else it did, in order: positions removed (`position_removed {plan, position, frame, locked}`), stored
    /// walls, suppressions, candidates, keys, marks and reference frames dropped with removed content (a suppression is
    /// dropped too when it would land on a junction, which is always a wall: `suppression_dropped {frame}`), walls stored
    /// at junctions (`wall_stored {frame}`), parts, regions, annotations and ranges removed, the approvals of a removed
    /// part (`approval_removed {plan, part}`), active areas removed or cut back where they would overlap (`area_removed
    /// {old}`, `area_trimmed {old, frames, from}`), moves shortened, and what no id was left for (`ids_exhausted {kind}`).
    pub changes: Vec<Change>,
    /// What the edit raised: `frame_changed` (error, per plan) when the frame changed and a plan's windows belong to a
    /// frame of another aspect, and `invalid_plan` (error) for a set that breaks an invariant after the edit.
    pub issues: Vec<Issue>,
}

/// The edit hook's result for one 16:9 edit: one entry per sequence whose vertical data it changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FollowReport {
    pub sequences: Vec<SequenceFollow>,
}

impl FollowReport {
    /// How many positions moved with their picture, over every sequence and plan.
    pub fn moved(&self) -> usize {
        self.count(|p| p.moved.len())
    }

    /// How many positions were removed.
    pub fn removed(&self) -> usize {
        self.count(|p| p.removed.len())
    }

    /// How many positions gained a flag.
    pub fn flagged(&self) -> usize {
        self.count(|p| p.flagged.len())
    }

    /// How many positions the hook added.
    pub fn added(&self) -> usize {
        self.count(|p| p.added.len())
    }

    /// Whether the hook changed nothing.
    pub fn is_empty(&self) -> bool {
        self.sequences.is_empty()
    }

    /// The suffix of the edit's undo label, `· vertical: 12 moved, 1 removed, 2 flagged` (the counts that are not zero, in
    /// the order moved, removed, flagged, added), or None when no position changed. The engine appends it after a space
    /// and leaves it out of `edit_merged`'s label comparison.
    pub fn label_suffix(&self) -> Option<String> {
        let counts = [(self.moved(), "moved"), (self.removed(), "removed"), (self.flagged(), "flagged"), (self.added(), "added")];
        let parts: Vec<String> = counts.into_iter().filter(|(n, _)| *n > 0).map(|(n, what)| format!("{n} {what}")).collect();
        (!parts.is_empty()).then(|| format!("\u{b7} vertical: {}", parts.join(", ")))
    }

    /// The sum of `of` over every plan of every sequence.
    fn count(&self, of: impl Fn(&PlanFollow) -> usize) -> usize {
        self.sequences.iter().flat_map(|s| &s.plans).map(of).sum()
    }
}

/// A sequence's two digests: its picture layout and its framing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SequenceDigests {
    pub layout: String,
    pub framing: String,
}

/// The digests of a project snapshot that the hook compares: one entry per sequence with a vertical set. The engine
/// computes them once per snapshot (an `Arc<Project>`) and keeps them while the snapshot lives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PictureDigests {
    pub sequences: BTreeMap<ItemId, SequenceDigests>,
}

/// The layout digest of `seq` (spec section 4.8): its raster (frame size, rate and end frame) and, for each video track in
/// order, its id, its enabled flag, its transitions and each enabled picture clip's id, project item, start, duration,
/// source in, speed, direction, frame hold, time remapping, Scale to Frame Size and multicam angle, with both digests of
/// the nested or multicam sequence it shows (to the renderer's nest depth). FNV-1a 64, printed `fnv1a64:` and 16 hex
/// digits. The end frame is in it because the frames a plan may use end there.
pub fn layout_digest(project: &Project, seq: &Sequence) -> String {
    Nests::default().layout(project, seq, 0)
}

/// The framing digest of `seq` (spec section 4.8; not an approval's range digest): for each video track in order, its id
/// and enabled flag, and for each enabled picture clip its id, its Motion effect (parameters, keyframes, enabled flag),
/// its time remapping, its enabled effects of the Transform and Distort categories plus Crop and Auto Reframe, keyframes
/// included, and Scale to Frame Size.
pub fn framing_digest(project: &Project, seq: &Sequence) -> String {
    digest_of(&framing_view(project, seq))
}

/// The layout and framing digests of every sequence of `project` that has a vertical set.
pub fn picture_digests(project: &Project) -> PictureDigests {
    let mut nests = Nests::default();
    let mut out = PictureDigests::default();
    for item in project.items.values() {
        if let Some(seq) = item.as_sequence().filter(|s| s.vertical.is_some()) {
            out.sequences.insert(item.id, nests.both(project, seq));
        }
    }
    out
}

/// The edit hook: follows the 16:9 edit from `before` to `after` for every sequence with a vertical set, writing `after`'s
/// vertical data. Never panics; the person's edit is never refused.
///
/// The contract with the engine: `after`'s vertical data of those sequences is replaced by what the hook derives from
/// `before`'s, so no command that commits through `Session::edit` may change vertical data (every vertical write commits
/// through `Session::edit_plan_only`, which runs no hook). The ids it hands out come from `before`'s counters: the engine
/// passes a `before` whose counters it raised to the session's high-water marks.
pub fn follow_picture(before: &Project, after: &mut Project, caller: Caller) -> Result<FollowReport, Refusal> {
    let (old, new) = (picture_digests(before), picture_digests(after));
    follow_picture_with(before, &old, after, &new, caller)
}

/// [`follow_picture`] with the digests of both snapshots already known (the engine's per-snapshot cache). A sequence
/// missing from either is digested in the call.
pub fn follow_picture_with(
    before: &Project,
    before_digests: &PictureDigests,
    after: &mut Project,
    after_digests: &PictureDigests,
    caller: Caller,
) -> Result<FollowReport, Refusal> {
    let mut report = FollowReport::default();
    let planned: Vec<ItemId> = before.items.values().filter(|i| i.as_sequence().is_some_and(|s| s.vertical.is_some())).map(|i| i.id).collect();
    for id in planned {
        let Some(followed) = follow_one(before, before_digests, after, after_digests, id, caller) else { continue };
        if let Some(seq) = after.sequence_mut(id) {
            seq.vertical = Some(Box::new(followed.set));
        }
        report.sequences.extend(followed.entry);
    }
    Ok(report)
}

/// The most one edit may change a frame inside the move from `from` to `to` of a plan with output `out`: two split bounds
/// `2 (delta + travel delta_h / (8 h_lo))`, with `delta = 0.001 + 0.00025 max(1, OW / OH)` (the clamped case, which covers
/// the unclamped 0.0005), `delta_h = 0.001`, `travel` the move's largest change of `cx`, `cy` or `h`, `h_lo` its smaller end
/// height, and no travel term for a pure pan (lane L1a round 4; spec sections 3.6 and 4.8).
pub fn hook_bound(from: &Window, to: &Window, out: OutputSize) -> f64 {
    let aspect = f64::from(out.width) / f64::from(out.height);
    let delta = 0.001 + 0.00025 * aspect.max(1.0);
    let tilt = if from.h == to.h { 0.0 } else { 0.001 / from.h.min(to.h) };
    let travel = (from.cx - to.cx).abs().max((from.cy - to.cy).abs()).max((from.h - to.h).abs());
    2.0 * (delta + travel * tilt / 8.0)
}

/// The bound of the edit hook at `frame` of `plan`: 0 where the window holds (a hold, before the first position, a fit
/// frame); inside a move, [`hook_bound`] of the positions it departs from and arrives at.
pub fn follow_bound(plan: &VerticalPlan, r: &Raster, frame: i64) -> f64 {
    let s = window_at(plan, r, r.rate.tick_of(frame));
    if s.phase != Phase::Move {
        return 0.0;
    }
    let find = |id: Option<u32>| id.and_then(|id| plan.positions.iter().find(|p| p.id == id));
    match (find(s.position), find(s.next)) {
        (Some(from), Some(to)) => hook_bound(&from.window, &to.window, plan.output),
        _ => f64::INFINITY,
    }
}

/// The digests of the nested and multicam sequences already computed, by project item and nesting depth.
#[derive(Default)]
struct Nests(BTreeMap<(ItemId, u32), [String; 2]>);

impl Nests {
    /// Both digests of a top-level sequence.
    fn both(&mut self, project: &Project, seq: &Sequence) -> SequenceDigests {
        SequenceDigests { layout: self.layout(project, seq, 0), framing: framing_digest(project, seq) }
    }

    /// The layout digest of `seq` shown at nesting `depth`.
    fn layout(&mut self, project: &Project, seq: &Sequence, depth: u32) -> String {
        let mut tracks = Vec::with_capacity(seq.video_tracks.len());
        for track in &seq.video_tracks {
            let mut items = Vec::new();
            for item in track.items.iter().filter(|i| i.enabled && shows_picture(project, i)) {
                let nested = self.nested(project, item.item, depth);
                items.push(ItemLayout::of(item, nested));
            }
            tracks.push(TrackLayout { id: track.id, enabled: track.enabled, transitions: &track.transitions, items });
        }
        digest_of(&LayoutView { raster: raster(seq), tracks })
    }

    /// Both digests of the sequence that project item `item` is, one level below `depth`; None for any other item and past
    /// the nest depth.
    fn nested(&mut self, project: &Project, item: ItemId, depth: u32) -> Option<[String; 2]> {
        let seq = project.sequence(item)?;
        let inner = depth.checked_add(1).filter(|d| *d <= NEST_DEPTH)?;
        if let Some(found) = self.0.get(&(item, inner)) {
            return Some(found.clone());
        }
        let both = [self.layout(project, seq, inner), framing_digest(project, seq)];
        self.0.insert((item, inner), both.clone());
        Some(both)
    }
}

/// What a layout digest covers.
#[derive(Serialize)]
struct LayoutView<'a> {
    raster: Raster,
    tracks: Vec<TrackLayout<'a>>,
}

/// One video track as far as which frames show what.
#[derive(Serialize)]
struct TrackLayout<'a> {
    id: TrackId,
    enabled: bool,
    transitions: &'a [Transition],
    items: Vec<ItemLayout<'a>>,
}

/// An enabled picture clip as far as which frames show what: its timing, its time remapping, Scale to Frame Size, its
/// multicam angle and both digests of the sequence it nests.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemLayout<'a> {
    id: ClipId,
    item: ItemId,
    start: Tick,
    duration: Tick,
    source_in: Tick,
    speed: f64,
    reverse: bool,
    frame_hold: Option<Tick>,
    time_remap: Option<&'a EffectInstance>,
    scale_to_frame: bool,
    multicam: Option<MulticamSel>,
    nested: Option<[String; 2]>,
}

impl<'a> ItemLayout<'a> {
    fn of(i: &'a TrackItem, nested: Option<[String; 2]>) -> Self {
        Self {
            id: i.id,
            item: i.item,
            start: i.start,
            duration: i.duration,
            source_in: i.source_in,
            speed: i.speed,
            reverse: i.reverse,
            frame_hold: i.frame_hold,
            time_remap: i.effect("time_remap"),
            scale_to_frame: i.scale_to_frame,
            multicam: i.multicam,
            nested,
        }
    }
}

/// What a framing digest covers.
#[derive(Serialize)]
struct FramingView<'a> {
    tracks: Vec<TrackFraming<'a>>,
}

/// One video track's order, enabled flag and the framing of its enabled picture clips.
#[derive(Serialize)]
struct TrackFraming<'a> {
    id: TrackId,
    enabled: bool,
    items: Vec<(ClipId, ClipFraming<'a>)>,
}

/// A clip's framing: its Motion (enabled or not), its time remapping, its enabled geometry effects (the Transform and
/// Distort categories, Crop, Auto Reframe) and Scale to Frame Size.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipFraming<'a> {
    motion: Option<&'a EffectInstance>,
    time_remap: Option<&'a EffectInstance>,
    geometry: Vec<&'a EffectInstance>,
    scale_to_frame: bool,
}

impl<'a> ClipFraming<'a> {
    fn of(item: &'a TrackItem) -> Self {
        let geometry = item.effects.iter().filter(|e| e.enabled && is_geometry(e)).collect();
        Self { motion: item.effect("motion"), time_remap: item.effect("time_remap"), geometry, scale_to_frame: item.scale_to_frame }
    }
}

/// The framing view of a sequence.
fn framing_view<'a>(project: &Project, seq: &'a Sequence) -> FramingView<'a> {
    let mut tracks = Vec::with_capacity(seq.video_tracks.len());
    for track in &seq.video_tracks {
        let items = track.items.iter().filter(|i| i.enabled && shows_picture(project, i)).map(|i| (i.id, ClipFraming::of(i))).collect();
        tracks.push(TrackFraming { id: track.id, enabled: track.enabled, items });
    }
    FramingView { tracks }
}

/// One sequence followed: its new vertical set and the report's entry for it, if the hook changed anything.
struct Followed {
    set: VerticalSet,
    entry: Option<SequenceFollow>,
}

/// Follows sequence `id`; None when it has no vertical set in `before` or no longer exists in `after`.
fn follow_one(before: &Project, old: &PictureDigests, after: &Project, new: &PictureDigests, id: ItemId, caller: Caller) -> Option<Followed> {
    let (old_seq, new_seq) = (before.sequence(id)?, after.sequence(id)?);
    let set = old_seq.vertical.as_deref()?;
    let a = old.sequences.get(&id).cloned().unwrap_or_else(|| Nests::default().both(before, old_seq));
    let b = new.sequences.get(&id).cloned().unwrap_or_else(|| Nests::default().both(after, new_seq));
    if a == b {
        return Some(Followed { set: set.clone(), entry: None });
    }
    let hook = Hook { before, old_seq, after, new_seq, set, id, writer: caller.writer };
    Some(if a.layout == b.layout { hook.framing_only().unwrap_or_else(|| hook.full()) } else { hook.full() })
}

/// One sequence of an edit, before and after.
struct Hook<'a> {
    before: &'a Project,
    old_seq: &'a Sequence,
    after: &'a Project,
    new_seq: &'a Sequence,
    /// The vertical set before the edit.
    set: &'a VerticalSet,
    id: ItemId,
    writer: Writer,
}

impl Hook<'_> {
    /// The full algorithm. A set that broke an invariant before the edit is followed too: the map needs no valid position
    /// and the evaluator is total, so a damaged plan (or one whose windows a frame of another aspect left outside) keeps
    /// following its picture, and the result reports what it still breaks (`invalid_plan`).
    fn full(&self) -> Followed {
        let (old, new) = (Timeline::of(self.before, self.old_seq), Timeline::of(self.after, self.new_seq));
        let map = Mapping::new(Matcher::new(&old, &new).run(), old.end, new.end);
        let picture = self.picture(&old, &new, &map);
        let (r0, r1) = (raster(self.old_seq), raster(self.new_seq));
        let carry = Carry { project: self.after, seq: self.new_seq, old: self.set, r0, r1, map: &map, picture: Some(&picture), writer: self.writer };
        let carried = carry_set(&carry);
        let changed = carried.set != *self.set || !carried.issues.is_empty();
        let entry = changed.then_some(SequenceFollow { sequence: self.id, plans: carried.plans, changes: carried.changes, issues: carried.issues });
        Followed { set: carried.set, entry }
    }

    /// What the hook knows about the edit's picture besides the map (spec section 4.8, steps 5 and 9): where the picture
    /// continues across a junction; the new frames whose picture changed (a changed framing of the clips shown or of the
    /// picture under them, hidden picture that now shows or picture that is now hidden, a video transition changed at any
    /// level); the old walls; the extended heads.
    fn picture(&self, old: &Timeline, new: &Timeline, map: &Mapping) -> Picture {
        let old_walls = walls(self.before, self.old_seq).iter().map(|w| old.rate.frame_at(w.at)).collect();
        let continues = map.joins.iter().copied().filter(|g| continuous(new, *g)).collect();
        let mut changed = Vec::new();
        for p in &map.pieces {
            let Some((oi, ni)) = p.segs else { continue };
            let (Some(a), Some(b)) = (old.run(oi), new.run(ni)) else { continue };
            // a piece matched through hidden picture shows on top what it did not before, or no longer shows it there
            let hidden = oi >= old.segs.len() || ni >= new.segs.len();
            if hidden || a.framing() != b.framing() {
                changed.push(p.claim);
            }
        }
        changed.extend(hidden_framing_changed(old, new));
        let (was, now) = (placed_transitions(self.before, self.old_seq, old), placed_transitions(self.after, self.new_seq, new));
        changed.extend(transitions_changed(&was, &now, map));
        let changed = merge_ranges(changed);
        let heads = extended_heads(old, new, map);
        Picture { continues, changed, old_walls, heads, old_leaves: old.leaves(), new_leaves: new.leaves() }
    }

    /// The fast path of a framing-only change: no frame moves, and the positions whose spans show a clip whose framing
    /// changed (on top, or under the clip on top) are flagged `PictureChanged`. None when the change moved a wall, or when
    /// the two timelines do not show the same clips on the same frames (then the full algorithm runs).
    fn framing_only(&self) -> Option<Followed> {
        // a geometry change can make a through edit a shot change, or a plain through edit again (spec section 4.1): a
        // move may then hold a wall, which the full algorithm's re-check shortens and flags (step 8)
        let (was, now) = (walls_in(self.before, self.old_seq, Some(self.set)), walls_in(self.after, self.new_seq, Some(self.set)));
        if !was.iter().map(|w| w.at).eq(now.iter().map(|w| w.at)) {
            return None;
        }
        let (old, new) = (Timeline::of(self.before, self.old_seq), Timeline::of(self.after, self.new_seq));
        if old.segs.len() != new.segs.len() || old.end != new.end {
            return None;
        }
        let mut changed = Vec::new();
        for (a, b) in old.segs.iter().zip(&new.segs) {
            if a.from != b.from || a.to != b.to || !same_clips(a, b) {
                return None;
            }
            if a.framing() != b.framing() {
                changed.push([a.from, a.to]);
            }
        }
        changed.extend(hidden_framing_changed(&old, &new));
        let changed = merge_ranges(changed);
        let r = raster(self.new_seq);
        let mut set = self.set.clone();
        let mut plans = Vec::new();
        for plan in &mut set.plans {
            let spans = plan_spans(plan, &r);
            let mut flagged = Vec::new();
            for (p, span) in plan.positions.iter_mut().zip(spans) {
                if meets(&changed, span) && add_flag(p, Flag::PictureChanged) {
                    flagged.push(p.id);
                }
            }
            if !flagged.is_empty() {
                flagged.sort_unstable();
                plans.push(PlanFollow { plan: plan.id, flagged, ..PlanFollow::default() });
            }
        }
        let entry = (set != *self.set).then(|| SequenceFollow { sequence: self.id, plans, ..SequenceFollow::default() });
        Some(Followed { set, entry })
    }
}

/// The violations of a set as `invalid_plan` errors (spec section 2.8): one per plan, and one for the set's own data.
fn invalid_issues(violations: Vec<Violation>) -> Vec<Issue> {
    let mut by_plan: BTreeMap<Option<u32>, Vec<Violation>> = BTreeMap::new();
    for v in violations {
        by_plan.entry(v.plan).or_default().push(v);
    }
    let mut out = Vec::with_capacity(by_plan.len());
    for (plan, list) in by_plan {
        out.push(Issue { code: "invalid_plan", level: Level::Error, plan, frames: None, detail: json!({"violations": list}) });
    }
    out
}

/// The spans of a plan's positions (spec section 2.7: moving and static frames; the first position's also covers the
/// frames before it), as frame ranges.
fn plan_spans(plan: &VerticalPlan, r: &Raster) -> Vec<[i64; 2]> {
    let rate = r.rate;
    let moves: Vec<Option<i64>> = (0..plan.positions.len()).map(|k| move_span(plan, r, k).map(|m| m.start_frame)).collect();
    let mut out = Vec::with_capacity(plan.positions.len());
    for (k, p) in plan.positions.iter().enumerate() {
        let f = rate.frame_at(p.at);
        let from = match moves.get(k).copied().flatten() {
            _ if k == 0 => 0,
            Some(s) => s.saturating_add(1).min(f),
            None => f,
        };
        let next = match (plan.positions.get(k + 1), moves.get(k + 1).copied().flatten()) {
            (Some(_), Some(s)) => s.saturating_add(1),
            (Some(n), None) => rate.frame_at(n.at),
            (None, _) => r.end_frame,
        };
        out.push([from, next.max(f)]);
    }
    out
}

/// Whether `span` meets one of the sorted, apart `ranges`.
fn meets(ranges: &[[i64; 2]], [a, b]: [i64; 2]) -> bool {
    let i = ranges.partition_point(|r| r[1] <= a);
    ranges.get(i).is_some_and(|r| r[0] < b.max(a.saturating_add(1)))
}

/// Adds `flag` to a position; whether it is new there.
fn add_flag(p: &mut Position, flag: Flag) -> bool {
    if p.flags.contains(&flag) {
        return false;
    }
    p.flags.push(flag);
    true
}

/// Media-time slack when comparing what two frames show, in ticks, as `filmcraft-edit`'s `MEDIA_SLACK`.
const SLACK: i128 = 2;

/// The media time `chain` shows at top-level tick `t`: each clip's map in turn, holds included.
fn shown_time(chain: &[&TrackItem], t: Tick) -> Tick {
    chain.iter().fold(t, |t, item| item.frame_hold.unwrap_or_else(|| ClipTime::of(item).at(t)))
}

/// The time `chain` maps top-level tick `t` to with every hold ignored: what frames are matched by, so a frame-held clip
/// maps by its offset.
fn key_time(chain: &[&TrackItem], t: Tick) -> Tick {
    chain.iter().fold(t, |t, item| ClipTime::of(item).at(t))
}

/// How one clip maps the time of its track to the media time it shows, a frame hold left out: its start, duration,
/// source in, speed and direction.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipTime {
    start: i64,
    duration: i64,
    source_in: i64,
    speed: f64,
    reverse: bool,
}

impl ClipTime {
    fn of(item: &TrackItem) -> Self {
        Self { start: item.start.0, duration: item.duration.0, source_in: item.source_in.0, speed: item.speed, reverse: item.reverse }
    }

    /// The media time the clip shows at tick `t` of its track, ignoring a frame hold: `TrackItem::moving_source_time_at`
    /// in its operation order, saturating.
    fn at(&self, t: Tick) -> Tick {
        let speed = self.speed.abs();
        let scaled = (t.0.saturating_sub(self.start) as f64 * speed).round() as i64;
        if self.reverse {
            let consumed = (self.duration as f64 * speed).round() as i64;
            Tick(self.source_in.saturating_add(consumed).saturating_sub(scaled).saturating_sub(1))
        } else {
            Tick(self.source_in.saturating_add(scaled))
        }
    }
}

/// The time map of a chain of clips, the top clip first, held by a snapped or retimed piece so that it maps its frames on
/// demand.
#[derive(Clone, Debug, PartialEq)]
struct TimeMap(Vec<ClipTime>);

impl TimeMap {
    fn of(chain: &[&TrackItem]) -> Self {
        Self(chain.iter().copied().map(ClipTime::of).collect())
    }

    /// The time the chain maps top-level tick `t` to with every hold ignored, as [`key_time`].
    fn key(&self, t: Tick) -> Tick {
        self.0.iter().fold(t, |t, clip| clip.at(t))
    }
}

/// Whether every tick of `rate` starts a frame at a whole multiple of its frame duration (every standard rate does).
fn linear(rate: FrameRate) -> bool {
    let r = rate.sane();
    (i128::from(TICKS_PER_SECOND) * i128::from(r.den)) % i128::from(r.num) == 0
}

/// The smallest frame of `[lo, hi)` for which `past` holds, or `hi`; `past` is false and then true along the frames.
fn first_frame(mut lo: i64, mut hi: i64, past: impl Fn(i64) -> bool) -> i64 {
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if past(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// One run of frames of a timeline: the frames `[from, to)` show one leaf clip through one chain of clips (the top
/// clip first), or no picture (an empty chain).
struct Seg<'a> {
    from: i64,
    to: i64,
    chain: Vec<&'a TrackItem>,
    /// The track of each clip of the chain in its own sequence, the top clip's first.
    path: Vec<TrackId>,
    /// The media the leaf shows (a subclip's media item), or the leaf's project item.
    media: Option<ItemId>,
}

impl Seg<'_> {
    /// A run without picture.
    fn blank([from, to]: [i64; 2]) -> Self {
        Self { from, to, chain: Vec::new(), path: Vec::new(), media: None }
    }

    /// The top clip's track.
    fn track(&self) -> Option<TrackId> {
        self.path.first().copied()
    }

    /// The ids of the chain's clips, the top clip first.
    fn ids(&self) -> Vec<ClipId> {
        self.chain.iter().map(|i| i.id).collect()
    }

    /// The leaf clip: the media or subclip clip whose picture the frames show.
    fn leaf(&self) -> Option<ClipId> {
        self.chain.last().map(|i| i.id)
    }

    /// The key time of top frame `f` on `rate`: the media time the chain maps it to with every hold ignored.
    fn key(&self, rate: FrameRate, f: i64) -> i128 {
        i128::from(key_time(&self.chain, rate.tick_of(f)).0)
    }

    /// The media time the chain shows at top frame `f` on `rate`, holds included (the chain's maps go on past the run).
    fn shown(&self, rate: FrameRate, f: i64) -> i128 {
        i128::from(shown_time(&self.chain, rate.tick_of(f)).0)
    }

    /// The digest of the chain's framing: each clip's Motion, time remapping, enabled geometry effects and Scale to Frame
    /// Size.
    fn framing(&self) -> String {
        let views: Vec<ClipFraming> = self.chain.iter().copied().map(ClipFraming::of).collect();
        digest_of(&views)
    }

    /// How the chain plays its media: per clip its speed, direction and frame hold.
    fn timing(&self) -> Vec<(u64, bool, Option<Tick>)> {
        self.chain.iter().map(|i| (i.speed.abs().to_bits(), i.reverse, i.frame_hold)).collect()
    }

    /// Whether every clip of the chain plays at 100 %.
    fn unit(&self) -> bool {
        self.chain.iter().all(|i| i.speed.abs() == 1.0)
    }

    /// The direction of the key time along the frames: -1 when an odd number of the chain's clips play reversed.
    fn sigma(&self) -> i128 {
        self.chain.iter().fold(1, |s, i| if i.reverse { -s } else { s })
    }
}

/// Whether two runs show the same chain of clips from the same top track (two runs without picture included).
fn same_clips(a: &Seg, b: &Seg) -> bool {
    a.track() == b.track() && a.chain.len() == b.chain.len() && a.chain.iter().zip(&b.chain).all(|(x, y)| x.id == y.id)
}

/// The media a clip shows: a subclip's media item, or the clip's own project item.
fn media_of(project: &Project, item: &TrackItem) -> ItemId {
    project.resolve_media(item.item).map_or(item.item, |(media, _, _)| media)
}

/// A timeline cut into runs (spec section 4.8, step 1), with its rate and end frame, and the picture it holds without
/// necessarily showing it.
struct Timeline<'a> {
    rate: FrameRate,
    end: i64,
    /// The runs the frames show on top, covering `[0, end)` in order.
    segs: Vec<Seg<'a>>,
    /// The picture the timeline holds, shown on top or not: every picture clip of its video tracks, enabled or not, and
    /// a nest by each of its video tracks (a multicam clip's every angle). Runs with picture only, in track order; they
    /// overlap the runs on top and each other.
    hidden: Vec<Seg<'a>>,
}

impl<'a> Timeline<'a> {
    /// The runs of `seq`'s frames `[0, end)`: one sweep over the clips of the enabled video tracks, the top track first,
    /// nests and multicam clips followed to the renderer's nest depth; neighbouring runs of one chain are one run. Then the
    /// picture it holds ([`Walk::layers`]).
    fn of(project: &'a Project, seq: &'a Sequence) -> Self {
        let r = raster(seq);
        let end = r.end_frame.max(0);
        let walk = Walk { project, rate: r.rate };
        let mut runs = Vec::new();
        walk.cover(seq, &[], &[], Pick::Below(seq.video_tracks.len()), [0, end], &mut runs);
        let mut segs: Vec<Seg<'a>> = Vec::with_capacity(runs.len());
        for seg in runs {
            match segs.last_mut() {
                Some(last) if last.to == seg.from && same_clips(last, &seg) => last.to = seg.to,
                _ => segs.push(seg),
            }
        }
        Self { rate: r.rate, end, segs, hidden: walk.layers(seq, end) }
    }

    /// The run holding frame `f`.
    fn at(&self, f: i64) -> Option<&Seg<'a>> {
        let i = self.segs.partition_point(|s| s.to <= f);
        self.segs.get(i).filter(|s| s.from <= f)
    }

    /// Run `i`, counting the runs on top first and the hidden ones after them.
    fn run(&self, i: usize) -> Option<&Seg<'a>> {
        match i.checked_sub(self.segs.len()) {
            None => self.segs.get(i),
            Some(h) => self.hidden.get(h),
        }
    }

    /// How many runs [`Timeline::run`] counts.
    fn count(&self) -> usize {
        self.segs.len().saturating_add(self.hidden.len())
    }

    /// The leaf clip of every run.
    fn leaves(&self) -> Vec<([i64; 2], Option<ClipId>)> {
        self.segs.iter().map(|s| ([s.from, s.to], s.leaf())).collect()
    }
}

/// Which video tracks of a sequence a level looks at: the enabled ones below an index, or one multicam angle's track,
/// enabled or not.
#[derive(Clone, Copy)]
enum Pick {
    Below(usize),
    Only(usize),
}

/// The tracks a level looks at, the top one first.
fn picked(seq: &Sequence, pick: Pick) -> Vec<&Track> {
    match pick {
        Pick::Only(i) => seq.video_tracks.get(i).into_iter().collect(),
        Pick::Below(n) => seq.video_tracks.iter().take(n).filter(|t| t.enabled).rev().collect(),
    }
}

/// The tracks a nest shows: an enabled multicam clip only its angle's track (none for an angle without video), any other
/// nest every enabled track.
fn nest_pick(item: &TrackItem, nested: &Sequence) -> Pick {
    match item.multicam_angle(nested) {
        Some(angle) => nested.angle_video_track_index(angle).map_or(Pick::Below(0), Pick::Only),
        None => Pick::Below(nested.video_tracks.len()),
    }
}

/// What a timeline's runs are built from: the project (which items show picture) and the top sequence's rate.
struct Walk<'a> {
    project: &'a Project,
    rate: FrameRate,
}

impl<'a> Walk<'a> {
    /// The time at the level below `chain` that top frame `f` shows.
    fn time(&self, chain: &[&TrackItem], f: i64) -> Tick {
        shown_time(chain, self.rate.tick_of(f))
    }

    /// Appends the runs of the top frames `run` at the level of `seq`, below the clips of `chain` (on the tracks `path`):
    /// the frames are cut where a clip of the level's tracks starts or ends, and each piece is filled by [`Walk::fill`].
    fn cover(&self, seq: &'a Sequence, chain: &[&'a TrackItem], path: &[TrackId], pick: Pick, run: [i64; 2], out: &mut Vec<Seg<'a>>) {
        let tracks = picked(seq, pick);
        let mut edges = Vec::new();
        for item in tracks.iter().flat_map(|t| &t.items) {
            edges.push(item.start);
            edges.push(Tick(item.start.0.saturating_add(item.duration.0)));
        }
        edges.sort_unstable();
        edges.dedup();
        let mut from = run[0];
        for cut in self.crossings(chain, run, &edges).into_iter().chain([run[1]]) {
            if cut > from {
                self.fill(chain, path, &tracks, [from, cut], out);
                from = cut;
            }
        }
    }

    /// The picture `seq`'s video tracks hold over the frames `[0, end)`, shown on top or not, the top track first: each
    /// picture clip, enabled or not, on any track, enabled or not; a nest by each of its video tracks (a multicam clip's
    /// every angle), followed further as [`Walk::cover`] follows nests. Runs without picture are left out.
    fn layers(&self, seq: &'a Sequence, end: i64) -> Vec<Seg<'a>> {
        let mut out = Vec::new();
        for track in seq.video_tracks.iter().rev() {
            for item in track.items.iter().filter(|i| shows_picture(self.project, i)) {
                let stop = Tick(item.start.0.saturating_add(item.duration.0));
                let run = [shown_frame(self.rate, item.start).max(0), shown_frame(self.rate, stop).min(end)];
                if run[0] >= run[1] {
                    continue;
                }
                match self.project.item(item.item).map(|p| &p.kind) {
                    Some(ItemKind::Sequence(nested)) => {
                        for k in 0..nested.video_tracks.len() {
                            self.cover(nested, &[item], &[track.id], Pick::Only(k), run, &mut out);
                        }
                    }
                    _ => out.push(Seg { from: run[0], to: run[1], chain: vec![item], path: vec![track.id], media: Some(media_of(self.project, item)) }),
                }
            }
        }
        out.retain(|s| !s.chain.is_empty());
        out
    }

    /// The first frames of `run` at which the time below `chain` reaches one of the sorted `edges`, sorted. That time is
    /// monotone in the frame (every clip's map is), so each is found by bisection.
    fn crossings(&self, chain: &[&TrackItem], [a, b]: [i64; 2], edges: &[Tick]) -> Vec<i64> {
        if b <= a {
            return Vec::new();
        }
        let (first, last) = (self.time(chain, a), self.time(chain, b - 1));
        let rising = first <= last;
        let (lo, hi) = if rising { (first, last) } else { (last, first) };
        // along rising times a clip starts or ends at the first frame at or past an edge; along falling ones, below it
        let past = |f: i64, edge: Tick| {
            let t = self.time(chain, f);
            if rising { t >= edge } else { t < edge }
        };
        let mut out = Vec::new();
        for &edge in edges.iter().filter(|e| **e > lo && **e <= hi) {
            let f = first_frame(a, b, |f| past(f, edge));
            if f > a && f < b {
                out.push(f);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Appends the runs of `run`, over which no clip of `tracks` starts or ends: the topmost enabled picture clip there,
    /// followed into a nest; where a nest shows nothing the tracks below it show through; no picture when no track shows
    /// one.
    fn fill(&self, chain: &[&'a TrackItem], path: &[TrackId], tracks: &[&'a Track], run: [i64; 2], out: &mut Vec<Seg<'a>>) {
        let t = self.time(chain, run[0]);
        for (n, &tr) in tracks.iter().enumerate() {
            let Some(item) = tr.item_at(t).filter(|i| i.enabled && shows_picture(self.project, i)) else { continue };
            let mut deeper = chain.to_vec();
            deeper.push(item);
            let mut route = path.to_vec();
            route.push(tr.id);
            let Some(ItemKind::Sequence(nested)) = self.project.item(item.item).map(|p| &p.kind) else {
                out.push(Seg { from: run[0], to: run[1], chain: deeper, path: route, media: Some(media_of(self.project, item)) });
                return;
            };
            let mut inner = Vec::new();
            if deeper.len() <= NEST_DEPTH as usize {
                self.cover(nested, &deeper, &route, nest_pick(item, nested), run, &mut inner);
            } else {
                inner.push(Seg::blank(run));
            }
            let below = tracks.get(n + 1..).unwrap_or_default();
            for seg in inner {
                if seg.chain.is_empty() {
                    self.fill(chain, path, below, [seg.from, seg.to], out);
                } else {
                    out.push(seg);
                }
            }
            return;
        }
        out.push(Seg::blank(run));
    }
}

/// How old frames are matched to new ones (spec section 4.8, step 2), in order of preference.
#[derive(Clone, Copy)]
enum Pass {
    /// The same chain of clips.
    Chain,
    /// The same leaf clip.
    Leaf,
    /// The same media on the same top track.
    Track,
    /// The same media on any track.
    Media,
}

/// What a pass matches runs by.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Chain(Vec<ClipId>),
    Leaf(ClipId),
    Track(ItemId, TrackId),
    Media(ItemId),
}

impl Pass {
    /// The key of a run with picture.
    fn key(self, s: &Seg) -> Option<Key> {
        let leaf = s.leaf()?;
        Some(match self {
            Pass::Chain => Key::Chain(s.chain.iter().map(|i| i.id).collect()),
            Pass::Leaf => Key::Leaf(leaf),
            Pass::Track => Key::Track(s.media?, s.track()?),
            Pass::Media => Key::Media(s.media?),
        })
    }
}

/// The matching of an old timeline's frames to a new one's.
struct Matcher<'m> {
    old: &'m Timeline<'m>,
    new: &'m Timeline<'m>,
    /// Old frames matched so far.
    matched: Ranges,
    /// New frames shown by the old frames matched so far.
    claimed: Ranges,
    pieces: Vec<Piece>,
}

impl<'m> Matcher<'m> {
    fn new(old: &'m Timeline<'m>, new: &'m Timeline<'m>) -> Self {
        Self { old, new, matched: Ranges::default(), claimed: Ranges::default(), pieces: Vec::new() }
    }

    /// The pieces of the map: the four passes over the runs on top; then, for the old frames still unmatched, the first
    /// three over the hidden picture too (content an overlay, a switched-off track or clip, or another multicam angle now
    /// hides, or that comes back from one; the same media on another track is not looked for there); then the runs without
    /// picture.
    fn run(mut self) -> Vec<Piece> {
        for pass in [Pass::Chain, Pass::Leaf, Pass::Track, Pass::Media] {
            self.pass(pass, false);
        }
        for pass in [Pass::Chain, Pass::Leaf, Pass::Track] {
            self.pass(pass, true);
        }
        self.blanks();
        self.pieces
    }

    /// One pass: every old run with frames left is matched to the new runs of its key, exact candidates first (the
    /// smallest shift first), then by nearest media time (the nearest runs first). With `hidden` the runs are those on
    /// top and the hidden ones ([`Timeline::run`]), and pairs of two runs on top, which the passes over them tried, are left
    /// out.
    fn pass(&mut self, pass: Pass, hidden: bool) {
        let (old, new) = (self.old, self.new);
        let (olds, news) = if hidden { (old.count(), new.count()) } else { (old.segs.len(), new.segs.len()) };
        let mut by_key: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
        for ni in 0..news {
            if let Some(key) = new.run(ni).and_then(|s| pass.key(s)) {
                by_key.entry(key).or_default().push(ni);
            }
        }
        let mut exact = Vec::new();
        let mut nearest = Vec::new();
        for oi in 0..olds {
            let Some(o) = old.run(oi) else { continue };
            let Some(key) = pass.key(o) else { continue };
            if self.matched.free([o.from, o.to]).is_empty() {
                continue;
            }
            for &ni in by_key.get(&key).into_iter().flatten() {
                if hidden && oi < old.segs.len() && ni < new.segs.len() {
                    continue;
                }
                let Some(n) = new.run(ni) else { continue };
                match self.shift(o, n) {
                    Some(d) => exact.push((d.saturating_abs(), o.from, oi, ni, d)),
                    None => nearest.push((n.from.saturating_sub(o.from).saturating_abs(), o.from, oi, ni)),
                }
            }
        }
        exact.sort_unstable();
        nearest.sort_unstable();
        for (_, _, oi, ni, d) in exact {
            self.accept_exact(oi, ni, d);
        }
        for (_, _, oi, ni) in nearest {
            self.accept_nearest(oi, ni);
        }
    }

    /// The whole-frame shift that shows every frame of `o` where `n` shows its media, when both runs play it alike at 100 %
    /// on the same frame grid (a clip moved, rippled or trimmed); None otherwise. Their key times are then `sigma * tick +
    /// constant`, so the shift follows from one frame of each; both ends of the overlap are checked.
    fn shift(&self, o: &Seg, n: &Seg) -> Option<i64> {
        let rate = self.old.rate;
        if rate != self.new.rate || !linear(rate) || !o.unit() || o.timing() != n.timing() {
            return None;
        }
        let sigma = o.sigma();
        let tick = |f: i64| i128::from(rate.tick_of(f).0);
        let ko = o.key(rate, o.from) - sigma * tick(o.from);
        let kn = n.key(rate, n.from) - sigma * tick(n.from);
        let ticks = sigma * (ko - kn);
        let duration = i128::from(rate.sane().frame_duration().0);
        if duration <= 0 || ticks % duration != 0 {
            return None;
        }
        let d = i64::try_from(ticks / duration).ok()?;
        let (lo, hi) = (o.from.max(n.from.saturating_sub(d)), o.to.min(n.to.saturating_sub(d)));
        let same = |f: i64| (n.key(rate, f.saturating_add(d)) - o.key(rate, f)).abs() <= SLACK;
        (lo < hi && same(lo) && same(hi - 1)).then_some(d)
    }

    /// Accepts, as exact pieces, the frames of old run `oi` that new run `ni` shows `d` frames later and that no earlier
    /// piece took.
    fn accept_exact(&mut self, oi: usize, ni: usize, d: i64) {
        let (old, new) = (self.old, self.new);
        let (Some(o), Some(n)) = (old.run(oi), new.run(ni)) else { return };
        let span = [o.from.max(n.from.saturating_sub(d)), o.to.min(n.to.saturating_sub(d))];
        for [x, y] in self.matched.free(span) {
            for [u, v] in self.claimed.free([x.saturating_add(d), y.saturating_add(d)]) {
                self.take(Piece { segs: Some((oi, ni)), ..Piece::exact([u.saturating_sub(d), v.saturating_sub(d)], d) });
            }
        }
    }

    /// Accepts the frames of old run `oi` whose media new run `ni` shows, each at the new frame whose media time is nearest
    /// (ties to the earlier), among the new frames no earlier piece took: snapped pieces when both runs play their media
    /// alike, retimed pieces when not. A frame further than half a frame step from every new frame is not shown there. A
    /// run of such frames that the new run shows exactly, on the same grid with one whole-frame shift, is exact. Each free
    /// part of the new run takes, from each run of free old frames, the frames whose media times lie within its own: one
    /// run, since both time maps are monotone, found by bisection ([`Near::window`]). Nothing is done per frame.
    fn accept_nearest(&mut self, oi: usize, ni: usize) {
        let (old, new) = (self.old, self.new);
        let (Some(o), Some(n)) = (old.run(oi), new.run(ni)) else { return };
        let parts = self.claimed.free([n.from, n.to]);
        let todo = self.matched.free([o.from, o.to]);
        if parts.is_empty() || todo.is_empty() {
            return;
        }
        let alike = o.timing() == n.timing();
        let grid = alike && old.rate == new.rate;
        let (was, now) = (TimeMap::of(&o.chain), TimeMap::of(&n.chain));
        let mut used = Ranges::default();
        let mut pieces = Vec::new();
        for part in parts {
            let near = Near { old: was.clone(), new: now.clone(), r0: old.rate, r1: new.rate, part };
            let Some([a, b]) = near.window([o.from, o.to]) else { continue };
            for &[x, y] in &todo {
                for range in used.free([x.max(a), y.min(b)]) {
                    used.insert(range);
                    pieces.extend(Piece::near(range, near.clone(), grid, alike, (oi, ni)));
                }
            }
        }
        for piece in pieces {
            self.take(piece);
        }
    }

    /// Records an accepted piece.
    fn take(&mut self, piece: Piece) {
        self.matched.insert(piece.old);
        self.claimed.insert(piece.claim);
        self.pieces.push(piece);
    }

    /// The new frame of old frame `f`, among the pieces accepted so far.
    fn mapped(&self, f: i64) -> Option<i64> {
        self.pieces.iter().find_map(|p| p.map(f))
    }

    /// The runs without picture, matched by their neighbours: the frames after a matched frame continue after its new
    /// frame, the frames before one end before its new frame, as far as the new timeline has free frames without picture
    /// there; frames with neither keep their own frames when those are free and without picture.
    fn blanks(&mut self) {
        let old = self.old;
        for o in old.segs.iter().filter(|s| s.chain.is_empty()) {
            for [x, y] in self.matched.free([o.from, o.to]) {
                let mut x = x;
                let mut y = y;
                if let Some(p) = self.mapped(x - 1) {
                    let n = self.blank_after(p + 1, y - x);
                    if n > 0 {
                        self.take(Piece::exact([x, x + n], p + 1 - x));
                        x += n;
                    }
                }
                if let Some(q) = self.mapped(y).filter(|_| x < y) {
                    let n = self.blank_before(q, y - x);
                    if n > 0 {
                        self.take(Piece::exact([y - n, y], q - y));
                        y -= n;
                    }
                }
                let n = if x < y { self.blank_after(x, y - x) } else { 0 };
                if n > 0 {
                    self.take(Piece::exact([x, x + n], 0));
                }
            }
        }
    }

    /// How many new frames from `g` on (at most `most`) are free and show no picture.
    fn blank_after(&self, g: i64, most: i64) -> i64 {
        let Some(seg) = self.new.at(g).filter(|s| s.chain.is_empty()) else { return 0 };
        let free = self.claimed.free([g, seg.to]);
        free.first().filter(|r| r[0] == g).map_or(0, |r| (r[1] - g).min(most))
    }

    /// How many new frames just before `g` (at most `most`) are free and show no picture.
    fn blank_before(&self, g: i64, most: i64) -> i64 {
        let Some(seg) = self.new.at(g - 1).filter(|s| s.chain.is_empty()) else { return 0 };
        let free = self.claimed.free([seg.from, g]);
        free.last().filter(|r| r[1] == g).map_or(0, |r| (g - r[0]).min(most))
    }
}

/// The frame of `frames` (half-open; its end frame too when `boundary`) whose key time is nearest `m`, ties to the earlier
/// frame, with its distance. A run's key times are monotone, so it is found by bisection.
fn nearest(key: impl Fn(i64) -> i128, [u, v]: [i64; 2], m: i128, boundary: bool) -> Option<(i64, i128)> {
    let last = if boundary { v } else { v.checked_sub(1)? };
    if last < u {
        return None;
    }
    let rising = key(u) <= key(last);
    let g = first_frame(u, last.saturating_add(1), |g| if rising { key(g) >= m } else { key(g) <= m });
    let mut best: Option<(i64, i128)> = None;
    for at in [g.saturating_sub(1), g].into_iter().filter(|at| (u..=last).contains(at)) {
        let d = (key(at) - m).abs();
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((at, d));
        }
    }
    best
}

/// The key-time step around frame `g`: the larger of the steps to the frames on either side.
fn step(key: &impl Fn(i64) -> i128, g: i64) -> i128 {
    let k = key(g);
    (k - key(g.saturating_sub(1))).abs().max((key(g.saturating_add(1)) - k).abs())
}

/// Sorted, apart, half-open frame ranges.
#[derive(Clone, Debug, Default)]
struct Ranges(Vec<[i64; 2]>);

impl Ranges {
    /// Adds `[a, b)`, merged with the ranges it meets or touches.
    fn insert(&mut self, [a, b]: [i64; 2]) {
        if a >= b {
            return;
        }
        // the ranges `[lo, hi)` meet or touch `[a, b)`: none ends before `a`, none starts after `b`
        let lo = self.0.partition_point(|r| r[1] < a);
        let hi = self.0.partition_point(|r| r[0] <= b).max(lo);
        let from = self.0.get(lo).filter(|_| lo < hi).map_or(a, |r| r[0].min(a));
        let to = hi.checked_sub(1).and_then(|i| self.0.get(i)).filter(|_| lo < hi).map_or(b, |r| r[1].max(b));
        self.0.drain(lo..hi);
        self.0.insert(lo, [from, to]);
    }

    /// The parts of `[a, b)` that no range covers, in order.
    fn free(&self, [a, b]: [i64; 2]) -> Vec<[i64; 2]> {
        let mut out = Vec::new();
        let mut from = a;
        for &[x, y] in self.0.iter().skip(self.0.partition_point(|r| r[1] <= a)) {
            if x >= b {
                break;
            }
            if x > from {
                out.push([from, x]);
            }
            from = from.max(y);
        }
        if from < b {
            out.push([from, b]);
        }
        out
    }
}

/// How a piece of the map shows its old frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PieceKind {
    /// Every old frame on the frame a whole-frame shift away, showing the same media and timing.
    Exact,
    /// The same timing on another frame grid, or at a sub-frame offset: each old frame on the nearest new frame.
    Snapped,
    /// Speed, direction or hold changed: each old frame on the nearest new frame.
    Retimed,
}

/// One piece of a map: a run of old frames and the new frames that show them.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    /// The old frames `[from, to)`.
    old: [i64; 2],
    /// The new frames the piece shows: its frames' new frames and those between them and after the last one that its
    /// media covers.
    claim: [i64; 2],
    kind: PieceKind,
    /// Exact: the new frame of an old frame is the old frame plus this.
    shift: i64,
    /// Snapped and retimed: how each old frame finds its new frame, on demand (None for an exact piece).
    near: Option<Box<Near>>,
    /// Whether the new frames go forward with the old ones (an exact piece's do).
    forward: bool,
    /// The old and new runs it matched (the hook's pieces of picture), as [`Timeline::run`] counts them.
    segs: Option<(usize, usize)>,
}

impl Piece {
    /// The exact piece that shows old frames `old` `shift` frames later.
    pub(crate) fn exact(old: [i64; 2], shift: i64) -> Self {
        let claim = [old[0].saturating_add(shift), old[1].saturating_add(shift)];
        Self { old, claim, kind: PieceKind::Exact, shift, near: None, forward: true, segs: None }
    }

    /// The piece of the old frames `old`, which `near` maps by nearest media time: exact when they lie on one frame grid
    /// (`grid`: the same rate and timing), its two ends show their media exactly and one whole-frame shift holds both;
    /// else snapped when the runs play their media `alike`, retimed when not. None for an empty run, or one whose ends
    /// have no new frame.
    fn near(old: [i64; 2], near: Near, grid: bool, alike: bool, segs: (usize, usize)) -> Option<Self> {
        let last = old[1].checked_sub(1).filter(|l| *l >= old[0])?;
        let ((g0, d0), (g1, d1)) = (near.hit(old[0])?, near.hit(last)?);
        let shift = g0.saturating_sub(old[0]);
        if grid && d0 <= SLACK && d1 <= SLACK && g1.saturating_sub(last) == shift {
            return Some(Self { segs: Some(segs), ..Self::exact(old, shift) });
        }
        let forward = g0 <= g1;
        let claim = if forward { [g0, near.end(last, g1)] } else { [g1, g0.saturating_add(1)] };
        let kind = if alike { PieceKind::Snapped } else { PieceKind::Retimed };
        Some(Self { old, claim, kind, shift: 0, near: Some(Box::new(near)), forward, segs: Some(segs) })
    }

    /// The new frame of old frame `f`, when the piece holds it.
    fn map(&self, f: i64) -> Option<i64> {
        if !(self.old[0]..self.old[1]).contains(&f) {
            return None;
        }
        match &self.near {
            None => f.checked_add(self.shift),
            Some(near) => near.hit(f).map(|(g, _)| g),
        }
    }

    /// The new frame boundary after old frame `f`: the first new frame after the ones `f` stands for.
    fn end_after(&self, f: i64) -> Option<i64> {
        let g = self.map(f)?;
        match &self.near {
            Some(near) if self.forward => Some(near.end(f, g)),
            _ => g.checked_add(1),
        }
    }

    /// Whether the new frame of old frame `f` shows its media exactly.
    fn exact_at(&self, f: i64) -> bool {
        match &self.near {
            None => self.kind == PieceKind::Exact,
            Some(near) => (self.old[0]..self.old[1]).contains(&f) && near.hit(f).is_some_and(|(_, d)| d <= SLACK),
        }
    }
}

/// How a snapped or retimed piece finds the new frame of each of its old frames, as the matcher chose them: both runs'
/// time maps and rates, and the free part of the new run it lands on.
#[derive(Clone, Debug)]
struct Near {
    old: TimeMap,
    new: TimeMap,
    r0: FrameRate,
    r1: FrameRate,
    /// The new frames `[u, v)` it may land on.
    part: [i64; 2],
}

impl Near {
    /// The key time of old frame `f`.
    fn key_old(&self, f: i64) -> i128 {
        i128::from(self.old.key(self.r0.tick_of(f)).0)
    }

    /// The key time of new frame `g`.
    fn key_new(&self, g: i64) -> i128 {
        i128::from(self.new.key(self.r1.tick_of(g)).0)
    }

    /// The frame of the part whose media time is nearest old frame `f`'s, ties to the earlier, with its distance in ticks.
    fn hit(&self, f: i64) -> Option<(i64, i128)> {
        nearest(|g| self.key_new(g), self.part, self.key_old(f), false)
    }

    /// The new frame boundary after old frame `f`, which lands on `g`: the frame of the part from `g` on (or the part's
    /// end) nearest the next old frame's media time, at least `g + 1`.
    fn end(&self, f: i64, g: i64) -> i64 {
        let next = nearest(|x| self.key_new(x), [g, self.part[1]], self.key_old(f.saturating_add(1)), true);
        let after = g.saturating_add(1);
        next.map_or(after, |(e, _)| e.max(after))
    }

    /// The old frames of `[from, to)` whose media times lie within half a frame step of the part's: the matcher's test
    /// `2 d <= step` at the part's nearest frame, which every media time between the part's first and last frame's passes.
    /// One run, since both time maps are monotone; found by bisection. None when no frame does.
    fn window(&self, [from, to]: [i64; 2]) -> Option<[i64; 2]> {
        let [u, v] = self.part;
        let last = v.checked_sub(1).filter(|l| *l >= u)?;
        let end = to.checked_sub(1).filter(|l| *l >= from)?;
        let key = |g: i64| self.key_new(g);
        let (a, b) = (key(u), key(last));
        let (half_a, half_b) = (step(&key, u) / 2, step(&key, last) / 2);
        let (lo, hi) = if a <= b { (a - half_a, b + half_b) } else { (b - half_b, a + half_a) };
        let rising = self.key_old(from) <= self.key_old(end);
        let start = first_frame(from, to, |f| if rising { self.key_old(f) >= lo } else { self.key_old(f) <= hi });
        let stop = first_frame(start, to, |f| if rising { self.key_old(f) > hi } else { self.key_old(f) < lo });
        (start < stop).then_some([start, stop])
    }
}

/// The map of an edit (spec section 4.8, step 2) or of a remap (section 4.12): pieces of old frames and the new frames
/// that show them, sorted by their old frames and apart on both sides; where the map breaks; and the new content.
pub(crate) struct Mapping {
    pieces: Vec<Piece>,
    /// Old frames `x` whose boundary with frame `x - 1` is a junction (a frame on one side or both survives), sorted.
    breaks: Vec<i64>,
    /// The old frames that start a piece after a junction (frame 0 too, when something now comes before it), sorted.
    starts: Vec<i64>,
    /// The old frames that end a piece before a junction, sorted.
    ends: Vec<i64>,
    /// The new frames no old frame shows: new content, sorted and apart.
    fresh: Vec<[i64; 2]>,
    /// The new frames inside `(0, new end)` that start a run after a junction, sorted.
    joins: Vec<i64>,
}

impl Mapping {
    /// The map of `pieces` (apart on both sides) between an old timeline of `old_end` frames and a new one of `new_end`.
    pub(crate) fn new(mut pieces: Vec<Piece>, old_end: i64, new_end: i64) -> Self {
        pieces.retain(|p| p.old[0] < p.old[1]);
        pieces.sort_by_key(|p| p.old[0]);
        let (mut breaks, mut starts, mut ends, mut joins) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut claimed = Vec::with_capacity(pieces.len());
        for (i, p) in pieces.iter().enumerate() {
            let before = i.checked_sub(1).and_then(|j| pieces.get(j));
            let after = pieces.get(i + 1);
            // the piece continues the one before it in both timelines: no junction between them
            let joined_before = before.is_some_and(|q| q.old[1] == p.old[0] && q.claim[1] == p.claim[0]);
            let joined_after = after.is_some_and(|q| p.old[1] == q.old[0] && p.claim[1] == q.claim[0]);
            if !joined_before && (p.old[0] > 0 || p.claim[0] > 0) {
                starts.push(p.old[0]);
                breaks.extend((p.old[0] > 0).then_some(p.old[0]));
                joins.push(p.claim[0]);
            }
            if !joined_after && p.old[1] < old_end {
                ends.push(p.old[1] - 1);
                breaks.push(p.old[1]);
            }
            claimed.push(p.claim);
        }
        let claimed = merge_ranges(claimed);
        let mut fresh = Vec::new();
        let mut from = 0;
        for &[x, y] in &claimed {
            if x > from {
                fresh.push([from, x.min(new_end)]);
            }
            from = from.max(y);
        }
        if from < new_end {
            fresh.push([from, new_end]);
        }
        fresh.retain(|r| r[0] < r[1]);
        joins.extend(fresh.iter().map(|r| r[0]));
        for list in [&mut breaks, &mut starts, &mut ends, &mut joins] {
            list.sort_unstable();
            list.dedup();
        }
        joins.retain(|g| *g > 0 && *g < new_end);
        Self { pieces, breaks, starts, ends, fresh, joins }
    }

    /// The piece holding old frame `f`.
    fn piece(&self, f: i64) -> Option<&Piece> {
        let i = self.pieces.partition_point(|p| p.old[1] <= f);
        self.pieces.get(i).filter(|p| p.old[0] <= f)
    }

    /// The new frame of old frame `f`; None when its content is gone.
    pub(crate) fn map(&self, f: i64) -> Option<i64> {
        self.piece(f)?.map(f)
    }

    /// Whether the new frame of old frame `f` shows its media exactly (always, in an exact piece).
    fn exact_at(&self, f: i64) -> bool {
        self.piece(f).is_some_and(|p| p.exact_at(f))
    }

    /// Whether old frames `s` and `o` survive with no junction between them.
    fn unbroken(&self, s: i64, o: i64) -> bool {
        if self.map(s).is_none() || self.map(o).is_none() {
            return false;
        }
        let i = self.breaks.partition_point(|&x| x <= s);
        self.breaks.get(i).is_none_or(|&x| x > o)
    }

    /// The shift of the exact pieces that hold every old frame from `s` to `o`, when one shift holds them all.
    fn shift_over(&self, s: i64, o: i64) -> Option<i64> {
        let first = self.pieces.partition_point(|p| p.old[1] <= s);
        let mut shift = None;
        for p in self.pieces.iter().skip(first) {
            if p.old[0] > o {
                break;
            }
            if p.kind != PieceKind::Exact || shift.is_some_and(|d| d != p.shift) {
                return None;
            }
            shift = Some(p.shift);
        }
        shift
    }

    /// The surviving old frames of `[a, b)`, cut at the junctions: runs whose frames map with no junction between them.
    fn runs(&self, [a, b]: [i64; 2]) -> Vec<[i64; 2]> {
        let mut out: Vec<[i64; 2]> = Vec::new();
        for p in self.pieces.iter().skip(self.pieces.partition_point(|p| p.old[1] <= a)) {
            if p.old[0] >= b {
                break;
            }
            let (x, y) = (p.old[0].max(a), p.old[1].min(b));
            match out.last_mut() {
                Some(last) if last[1] == x && self.breaks.binary_search(&x).is_err() => last[1] = y,
                _ => out.push([x, y]),
            }
        }
        out
    }

    /// The new frames a junction-free run of old frames shows: from its new frames' first to the boundary after its last.
    fn image(&self, [x, y]: [i64; 2]) -> Option<[i64; 2]> {
        let last = y.checked_sub(1)?;
        let (a, b) = (self.map(x)?, self.map(last)?);
        let end = self.piece(last)?.end_after(last)?;
        Some([a.min(b), end.max(a.saturating_add(1))])
    }

    /// The new frames that show the surviving old frames of `[a, b)`, one range per junction-free run, in old order.
    fn images(&self, range: [i64; 2]) -> Vec<[i64; 2]> {
        self.runs(range).into_iter().filter_map(|run| self.image(run)).collect()
    }

    /// The new frames from the first to the last that show a surviving old frame of `[a, b)` (a part's or a region's new
    /// range); None when none survives.
    fn span(&self, range: [i64; 2]) -> Option<[i64; 2]> {
        let images = self.images(range);
        let from = images.iter().map(|r| r[0]).min()?;
        let to = images.iter().map(|r| r[1]).max()?;
        Some([from, to])
    }
}

/// A clip whose head the edit extended: new content of the same clip right before its old first frame's new frame, and
/// continuous with it (spec section 4.8, step 5).
pub(crate) struct Head {
    /// The clip's old first frame.
    old_first: i64,
    /// The new frames of its new head.
    frames: [i64; 2],
    clip: ClipId,
}

/// What the hook knows about the 16:9 picture of an edit, besides the map (a remap knows none of it).
pub(crate) struct Picture {
    /// The new junction frames whose picture continues across the junction: they need no wall.
    continues: Vec<i64>,
    /// The new frames whose picture changed other than by new content or retiming: a changed framing of the clips shown,
    /// picture matched through hidden picture, and new, changed or removed video transitions. Sorted and apart.
    changed: Vec<[i64; 2]>,
    /// The old effective walls' frames.
    old_walls: Vec<i64>,
    heads: Vec<Head>,
    /// The leaf clip of every run of the old and the new timeline.
    old_leaves: Vec<([i64; 2], Option<ClipId>)>,
    new_leaves: Vec<([i64; 2], Option<ClipId>)>,
}

impl Picture {
    /// The leaf clip shown at a frame of `leaves`.
    fn leaf_in(leaves: &[([i64; 2], Option<ClipId>)], f: i64) -> Option<ClipId> {
        let i = leaves.partition_point(|(r, _)| r[1] <= f);
        leaves.get(i).filter(|(r, _)| r[0] <= f).and_then(|(_, leaf)| *leaf)
    }
}

/// The new frames of the hidden picture whose framing changed: each chain of clips the new timeline holds, against the
/// same chain in the old one (picture under an overlay still shows around it). Picture that is switched off, or a
/// multicam angle not shown, is compared too, which can only flag more.
fn hidden_framing_changed(old: &Timeline, new: &Timeline) -> Vec<[i64; 2]> {
    let mut was: BTreeMap<Vec<ClipId>, String> = BTreeMap::new();
    for s in &old.hidden {
        was.entry(s.ids()).or_insert_with(|| s.framing());
    }
    new.hidden.iter().filter(|s| was.get(&s.ids()).is_some_and(|f| *f != s.framing())).map(|s| [s.from, s.to]).collect()
}

/// Whether the picture of new timeline `t` continues from frame `g - 1` to frame `g`: the same media, the media time `g`
/// shows the one the run before it would show there (within the slack), and the same framing; or no picture on either
/// side. Two pieces of one held frame, and a clip's own extension, continue.
fn continuous(t: &Timeline, g: i64) -> bool {
    let (Some(a), Some(b)) = (t.at(g - 1), t.at(g)) else { return false };
    if a.chain.is_empty() || b.chain.is_empty() {
        return a.chain.is_empty() && b.chain.is_empty();
    }
    a.media == b.media && (a.shown(t.rate, g) - b.shown(t.rate, g)).abs() <= SLACK && a.framing() == b.framing()
}

/// The clips whose head the edit extended: a run of new content right before the new frame of the old first frame of a
/// clip, showing more of that clip, continuous with it.
fn extended_heads(old: &Timeline, new: &Timeline, map: &Mapping) -> Vec<Head> {
    let mut out = Vec::new();
    for &[c, d] in &map.fresh {
        let Some(piece) = map.pieces.iter().find(|p| p.claim[0] == d && p.map(p.old[0]) == Some(d)) else { continue };
        let o = piece.old[0];
        let (Some(at), Some(before), Some(was)) = (new.at(d), new.at(d - 1), old.at(o)) else { continue };
        let Some(clip) = at.leaf() else { continue };
        if before.leaf() != Some(clip) || was.from != o || was.leaf() != Some(clip) || !continuous(new, d) {
            continue;
        }
        out.push(Head { old_first: o, frames: [c.max(before.from), d], clip });
    }
    out
}

/// A video transition where a timeline shows it: the nested sequence it lies in (None at the top level), its track and
/// id; its content as the project file prints it (a point parameter, as an iris's centre, holds NaN for "auto", which
/// equals nothing); and the top-level frames it covers.
struct Placed {
    key: (Option<ItemId>, TrackId, TransitionId),
    content: String,
    frames: [i64; 2],
}

/// The video transitions timeline `t` of `seq` shows: those of the top level's enabled video tracks over their frames,
/// and, inside each nest a run descends into, those of the track that shows the run's clip there, over the run's frames
/// whose time at that level they cover.
fn placed_transitions(project: &Project, seq: &Sequence, t: &Timeline) -> Vec<Placed> {
    let rate = t.rate;
    let content = |tr: &Transition| digest_of(&(&tr.effect, tr.duration, tr.from, tr.to, tr.align, tr.reverse));
    let mut out = Vec::new();
    for track in seq.video_tracks.iter().filter(|track| track.enabled) {
        for tr in &track.transitions {
            let frames = [rate.frame_at(tr.start), shown_frame(rate, Tick(tr.start.0.saturating_add(tr.duration.0)))];
            out.push(Placed { key: (None, track.id, tr.id), content: content(tr), frames });
        }
    }
    for seg in &t.segs {
        for (k, item) in seg.chain.iter().enumerate() {
            let (Some(nested), Some(&track)) = (project.sequence(item.item), seg.path.get(k + 1)) else { continue };
            let Some(inner) = nested.video_tracks.iter().find(|tk| tk.id == track) else { continue };
            let level = seg.chain.get(..=k).unwrap_or_default();
            for tr in &inner.transitions {
                let end = Tick(tr.start.0.saturating_add(tr.duration.0));
                if let Some(frames) = shown_over(level, rate, [seg.from, seg.to], tr.start, end) {
                    out.push(Placed { key: (Some(item.item), track, tr.id), content: content(tr), frames });
                }
            }
        }
    }
    out
}

/// The frames of the run `[a, b)` whose time below the clips of `chain` lies in `[start, end)`, as one range, or None. That
/// time is monotone along the frames, so both ends are found by bisection.
fn shown_over(chain: &[&TrackItem], rate: FrameRate, [a, b]: [i64; 2], start: Tick, end: Tick) -> Option<[i64; 2]> {
    let last = b.checked_sub(1).filter(|l| *l >= a)?;
    let time = |f: i64| shown_time(chain, rate.tick_of(f));
    let (first, stop) = if time(a) <= time(last) {
        let first = first_frame(a, b, |f| time(f) >= start);
        (first, first_frame(first, b, |f| time(f) >= end))
    } else {
        let first = first_frame(a, b, |f| time(f) < end);
        (first, first_frame(first, b, |f| time(f) < start))
    };
    (first < stop).then_some([first, stop])
}

/// The new frames under a video transition the edit added or changed, and the new frames of the old content under one it
/// removed or changed, at every level the runs show. A transition is kept when its place, id and content are the same and
/// its frames moved by one exact shift.
fn transitions_changed(old: &[Placed], new: &[Placed], map: &Mapping) -> Vec<[i64; 2]> {
    let mut out = Vec::new();
    for o in old {
        let [a, b] = o.frames;
        let moved = |n: &Placed| b > a && map.unbroken(a, b - 1) && map.shift_over(a, b - 1).is_some_and(|d| a.saturating_add(d) == n.frames[0]);
        if !new.iter().any(|n| n.key == o.key && n.content == o.content && moved(n)) {
            out.extend(map.images(o.frames));
            out.extend(new.iter().filter(|n| n.key == o.key).map(|n| n.frames));
        }
    }
    out.extend(new.iter().filter(|n| !old.iter().any(|o| o.key == n.key)).map(|n| n.frames));
    out.retain(|r| r[0] < r[1]);
    out
}

/// How the frame changed (spec section 4.8, step 11): not at all; to a size of the same aspect (every window and rect
/// scales by the ratio); or to another aspect.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FrameChange {
    Same,
    Scale(f64),
    Other,
}

impl FrameChange {
    fn of(r0: &Raster, r1: &Raster) -> Self {
        if r0.width == r1.width && r0.height == r1.height {
            return Self::Same;
        }
        let same = u64::from(r0.width) * u64::from(r1.height) == u64::from(r1.width) * u64::from(r0.height);
        if same && r0.width > 0 { Self::Scale(f64::from(r1.width) / f64::from(r0.width)) } else { Self::Other }
    }
}

/// One vertical set carried through a map: the hook's (with what it knows of the picture) or a remap's.
pub(crate) struct Carry<'a> {
    /// The project and the sequence after the edit: their edit points are walls.
    pub(crate) project: &'a Project,
    pub(crate) seq: &'a Sequence,
    /// The set before the edit.
    pub(crate) old: &'a VerticalSet,
    /// The rasters before and after the edit.
    pub(crate) r0: Raster,
    pub(crate) r1: Raster,
    pub(crate) map: &'a Mapping,
    pub(crate) picture: Option<&'a Picture>,
    /// Who writes the positions the hook adds.
    pub(crate) writer: Writer,
}

/// A carried set and what the hook did to it.
pub(crate) struct Carried {
    pub(crate) set: VerticalSet,
    /// The plans the hook changed, in plan order.
    pub(crate) plans: Vec<PlanFollow>,
    pub(crate) changes: Vec<Change>,
    pub(crate) issues: Vec<Issue>,
}

impl Carry<'_> {
    /// The new frame of an old point (a wall, a key, a mark) that must lie inside `[lo, new end)`; None when its
    /// content is gone.
    fn point(&self, f: i64, lo: i64) -> Option<i64> {
        self.map.map(f).filter(|g| (lo..self.r1.end_frame).contains(g))
    }

    /// Whether new frame `g` is a junction where the picture does not continue: a wall must stand there. A remap's
    /// junctions all are (the editor cannot see the relinked master's cuts).
    fn walled(&self, g: i64) -> bool {
        let continues = self.picture.is_some_and(|p| p.continues.binary_search(&g).is_ok());
        g > 0 && g < self.r1.end_frame && self.map.joins.binary_search(&g).is_ok() && !continues
    }

    /// When old frame `f` was the first frame of a clip whose head the edit extended (spec section 4.8, step 5), the
    /// clip's new first frame: an opener goes there, and so does a stored wall, suppression or candidate on its edit point.
    fn head_at(&self, f: i64) -> Option<i64> {
        let head = self.picture?.heads.iter().find(|h| h.old_first == f)?;
        let [g, _] = head.frames;
        (g > 0 && g < self.r1.end_frame).then_some(g)
    }

    /// The new frames that show the surviving frames of the old ticks `[from, to)`, one range per junction-free run.
    fn images(&self, from: Tick, to: Tick) -> Vec<[i64; 2]> {
        let rate = self.r0.rate;
        self.map.images([rate.frame_at(from), rate.frame_at(to)])
    }

    /// The new range of the old ticks `[from, to)` (from the first to the last new frame that shows a surviving frame).
    fn span(&self, from: Tick, to: Tick) -> Option<[i64; 2]> {
        let rate = self.r0.rate;
        self.map.span([rate.frame_at(from), rate.frame_at(to)])
    }

    /// The start tick of new frame `g`.
    fn tick(&self, g: i64) -> Tick {
        self.r1.rate.tick_of(g)
    }
}

/// Carries a vertical set through a map (spec section 4.8, steps 3-12; section 4.12 for a remap): the set's frame-based
/// data, the walls at the junctions, each plan, the frame, and validation.
pub(crate) fn carry_set(c: &Carry) -> Carried {
    let frame = FrameChange::of(&c.r0, &c.r1);
    let mut changes = Vec::new();
    let mut set = c.old.clone();
    carry_walls(c, &mut set, &mut changes);
    carry_ranges(c, &mut set, &mut changes);
    carry_points(c, &mut set, &mut changes);
    let rate = c.r1.rate;
    let standing: Vec<i64> = walls_in(c.project, c.seq, Some(&set)).iter().map(|w| rate.frame_at(w.at)).collect();
    let suppressed: Vec<i64> = set.walls.suppressed.iter().map(|t| rate.frame_at(*t)).collect();
    for &g in &c.map.joins {
        // a person's suppression that followed its clip's extended head keeps that edit point free of a wall
        if c.walled(g) && standing.binary_search(&g).is_err() && suppressed.binary_search(&g).is_err() {
            set.walls.stored.push(StoredWall { at: c.tick(g), origin: WallOrigin::Reanchored, score: None });
            changes.push(Change { code: "wall_stored", detail: json!({"frame": g}) });
        }
    }
    set.walls.stored.sort_by_key(|w| rate.frame_at(w.at));
    let walls: Vec<i64> = walls_in(c.project, c.seq, Some(&set)).iter().map(|w| rate.frame_at(w.at)).collect();
    // the parts no frame of which is left: their approvals go too
    let gone: Vec<u32> = c.old.parts.iter().map(|p| p.id).filter(|id| !set.parts.iter().any(|p| p.id == *id)).collect();
    let mut plans = Vec::with_capacity(c.old.plans.len());
    let mut follows = Vec::new();
    for plan in &c.old.plans {
        let (carried, follow) = carry_plan(c, plan, &walls, frame, &gone, &mut changes);
        if !follow.is_empty() || carried != *plan {
            follows.push(follow);
        }
        plans.push(carried);
    }
    set.plans = plans;
    if let FrameChange::Scale(k) = frame {
        scale_data(&mut set, k, &c.r1);
    }
    let issues = set_issues(c, &set, frame);
    Carried { set, plans: follows, changes, issues }
}

/// The items of `(new frame, item)` pairs in frame order, the first of each frame kept (the earlier old frame).
fn settle<T>(mut items: Vec<(i64, T)>) -> Vec<T> {
    items.sort_by_key(|(g, _)| *g);
    items.dedup_by_key(|(g, _)| *g);
    items.into_iter().map(|(_, item)| item).collect()
}

/// The stored walls, suppressions and candidates move with their frame and go with removed content (reported); a
/// suppression that would land on a junction where the picture cuts goes too, or it would hide that junction's wall. One
/// on the first frame of a clip whose head the edit extended (a roll, slide or ripple trim of the incoming clip) follows
/// the clip's new first frame, as its opener does, so a suppressed edit point stays suppressed and a stored wall stays on
/// the cut. The analysed ranges keep exactly the analysed frames (inserted content was never analysed).
fn carry_walls(c: &Carry, set: &mut VerticalSet, changes: &mut Vec<Change>) {
    let rate = c.r0.rate;
    let old = &c.old.walls;
    let mut stored = Vec::new();
    for w in &old.stored {
        let f = rate.frame_at(w.at);
        match c.head_at(f).or_else(|| c.point(f, 1)) {
            Some(g) => stored.push((g, StoredWall { at: c.tick(g), ..*w })),
            None => changes.push(Change { code: "wall_dropped", detail: json!({"frame": f}) }),
        }
    }
    set.walls.stored = settle(stored);
    let mut suppressed = Vec::new();
    for t in &old.suppressed {
        let f = rate.frame_at(*t);
        match c.head_at(f).or_else(|| c.point(f, 1).filter(|g| !c.walled(*g))) {
            Some(g) => suppressed.push((g, c.tick(g))),
            None => changes.push(Change { code: "suppression_dropped", detail: json!({"frame": f}) }),
        }
    }
    set.walls.suppressed = settle(suppressed);
    let mut candidates = Vec::new();
    for x in &old.candidates {
        let f = rate.frame_at(x.at);
        match c.head_at(f).or_else(|| c.point(f, 1)) {
            Some(g) => candidates.push((g, Candidate { at: c.tick(g), ..*x })),
            None => changes.push(Change { code: "candidate_dropped", detail: json!({"frame": f}) }),
        }
    }
    set.walls.candidates = settle(candidates);
    let mut analysed = Vec::new();
    for a in &old.analysed {
        analysed.extend(c.images(a.from, a.to));
    }
    set.walls.analysed = merge_ranges(analysed).into_iter().map(|[x, y]| FrameRange { from: c.tick(x), to: c.tick(y) }).collect();
}

/// Parts and regions take the span of their surviving frames and go when none survives (reported); annotations and
/// active areas lose the frames that are gone and split at every junction: the piece holding an annotation's first
/// surviving frame keeps its id, the others take new ids in old-frame order (a piece no id is left for goes, reported).
/// An area with no frame left goes, and one that would now overlap the one before it starts at that one's end or goes
/// (I19); each is reported.
fn carry_ranges(c: &Carry, set: &mut VerticalSet, changes: &mut Vec<Change>) {
    let rate = c.r1.rate;
    let mut parts = Vec::with_capacity(c.old.parts.len());
    for p in &c.old.parts {
        match c.span(p.from, p.to) {
            Some([x, y]) => parts.push(Part { from: c.tick(x), to: c.tick(y), ..p.clone() }),
            None => changes.push(Change { code: "part_removed", detail: json!({"part": p.id}) }),
        }
    }
    // stable: parts that share a first frame keep their order (I14 asks only for sorted firsts)
    parts.sort_by_key(|p| rate.frame_at(p.from));
    set.parts = parts;
    let mut regions = Vec::with_capacity(c.old.regions.len());
    for g in &c.old.regions {
        match c.span(g.from, g.to) {
            Some([x, y]) => regions.push(ProtectedRegion { from: c.tick(x), to: c.tick(y), ..g.clone() }),
            None => changes.push(Change { code: "region_removed", detail: json!({"region": g.id}) }),
        }
    }
    set.regions = regions;
    let top = c.old.annotations.iter().map(|a| a.id).max().unwrap_or(0);
    let mut notes = Vec::with_capacity(c.old.annotations.len());
    for a in &c.old.annotations {
        let images = c.images(a.from, a.to);
        if images.is_empty() {
            changes.push(Change { code: "annotation_removed", detail: json!({"annotation": a.id}) });
        }
        for (k, [x, y]) in images.into_iter().enumerate() {
            let id = if k == 0 { Ok(a.id) } else { alloc(&mut set.next_annotation_id, top) };
            let Ok(id) = id else {
                changes.push(Change { code: "ids_exhausted", detail: json!({"kind": "annotation", "annotation": a.id, "frames": [x, y]}) });
                continue;
            };
            notes.push(Annotation { id, from: c.tick(x), to: c.tick(y), ..a.clone() });
        }
    }
    notes.sort_by_key(|a| (rate.frame_at(a.from), a.id));
    set.annotations = notes;
    let mut areas: Vec<(ActiveArea, [i64; 2])> = Vec::with_capacity(c.old.active_area.len());
    for a in &c.old.active_area {
        let old = [c.r0.rate.frame_at(a.from), c.r0.rate.frame_at(a.to)];
        let images = c.images(a.from, a.to);
        if images.is_empty() {
            changes.push(Change { code: "area_removed", detail: json!({"old": old}) });
        }
        for [x, y] in images {
            areas.push((ActiveArea { from: c.tick(x), to: c.tick(y), ..*a }, old));
        }
    }
    areas.sort_by_key(|(a, _)| rate.frame_at(a.from));
    // areas stay apart (I19): one that would start inside the one before it starts at that one's end, or goes
    let mut apart: Vec<ActiveArea> = Vec::with_capacity(areas.len());
    for (mut a, old) in areas {
        if let Some(end) = apart.last().map(|last| last.to).filter(|end| a.from < *end) {
            let frames = [rate.frame_at(a.from), rate.frame_at(a.to)];
            a.from = end;
            let (code, from) = if a.from < a.to { ("area_trimmed", Some(rate.frame_at(end))) } else { ("area_removed", None) };
            changes.push(Change { code, detail: json!({"old": old, "frames": frames, "from": from}) });
        }
        if a.from < a.to {
            apart.push(a);
        }
    }
    set.active_area = apart;
}

/// Box-track keys and the conform reference's frames are points: they move with their frame and go with removed
/// content (reported). A reference's wall pair stays while its two frames stay next to each other.
fn carry_points(c: &Carry, set: &mut VerticalSet, changes: &mut Vec<Change>) {
    let rate = c.r0.rate;
    for (track, old) in set.box_tracks.iter_mut().zip(&c.old.box_tracks) {
        let mut keys = Vec::with_capacity(old.keys.len());
        for k in &old.keys {
            let f = rate.frame_at(k.at);
            match c.point(f, 0) {
                Some(g) => keys.push((g, KeyBox { at: c.tick(g), ..*k })),
                None => changes.push(Change { code: "key_dropped", detail: json!({"track": old.id, "frame": f}) }),
            }
        }
        track.keys = settle(keys);
    }
    let Some(reference) = &c.old.reference else { return };
    let mut frames = Vec::with_capacity(reference.frames.len());
    for (i, &f) in reference.frames.iter().enumerate() {
        match c.point(f, 0) {
            Some(g) => frames.push((g, (g, reference.grids.get(i).cloned().unwrap_or_default()))),
            None => changes.push(Change { code: "reference_frame_dropped", detail: json!({"frame": f}) }),
        }
    }
    let (frames, grids): (Vec<i64>, Vec<String>) = settle(frames).into_iter().unzip();
    let mut wall_pairs = Vec::new();
    for &w in &reference.wall_pairs {
        let (Some(a), Some(b)) = (w.checked_sub(1).and_then(|p| c.map.map(p)), c.map.map(w)) else { continue };
        if b == a.saturating_add(1) {
            wall_pairs.push(b);
        }
    }
    set.reference = Some(ConformReference { frames, grids, wall_pairs, ..reference.clone() });
}

/// The rects of a set's regions, box-track keys and active areas scaled by `k` onto raster `r` (the frame's change to
/// a size of the same aspect), on the grid and inside the frame.
fn scale_data(set: &mut VerticalSet, k: f64, r: &Raster) {
    for g in &mut set.regions {
        g.rect = scale_rect(g.rect, k, r);
    }
    for key in set.box_tracks.iter_mut().flat_map(|t| &mut t.keys) {
        key.rect = scale_rect(key.rect, k, r);
    }
    for a in &mut set.active_area {
        a.rect = scale_rect(a.rect, k, r);
    }
}

/// The carried set's issues: `frame_changed` per plan when the frame changed and the plan's windows belong to a frame of
/// another aspect (`VerticalPlan::framed_on`, which keeps the error until a person repairs the plan), and `invalid_plan`
/// for every invariant the carried data breaks (spec section 4.8, step 12).
fn set_issues(c: &Carry, set: &VerticalSet, frame: FrameChange) -> Vec<Issue> {
    let mut issues = Vec::new();
    let now = [c.r1.width, c.r1.height];
    if frame != FrameChange::Same {
        for plan in &set.plans {
            let Some(from) = plan.framed_on.filter(|f| *f != now) else { continue };
            issues.push(Issue { code: "frame_changed", level: Level::Error, plan: Some(plan.id), frames: None, detail: json!({"from": from, "to": now}) });
        }
    }
    let scene = Scene::new(c.project, c.seq, Some(set));
    let mut violations = set_violations(&scene, set);
    for plan in &set.plans {
        violations.extend(plan_only_violations(&scene, plan));
    }
    issues.extend(invalid_issues(violations));
    issues
}

/// Where an entry of the plan being carried comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// A position of the plan before the edit.
    Original,
    /// An exact split of a move at a junction.
    Split,
    /// An exact anchor of the window that governed old content next to a junction.
    Anchor,
    /// The window a clip's new head inherits, held by a position of its own (spec section 4.8, step 6).
    Same,
}

/// One position of the plan being carried: the old frame it stands for, its new frame, and what happened to it.
struct Entry {
    source: Source,
    old: i64,
    new: i64,
    pos: Position,
    /// The old frame its arrival's move starts on.
    from: Option<i64>,
    /// The first position's move, which shows nothing (spec section 2.5).
    ignored: bool,
    /// Moved onto its clip's extended head (spec section 4.8, step 5).
    head: bool,
    /// Its move was shortened at a wall, at its predecessor or at a `fit` position (step 8).
    shortened: bool,
    /// Another entry landed on its frame too and was dropped, and showed another window or layout (step 4).
    collided: bool,
}

impl Entry {
    /// A position the hook adds at old frame `old`.
    fn added(source: Source, old: i64, window: Window, layout: Layout, arrival: Arrival, from: Option<i64>, writer: Writer) -> Self {
        let pos = Position { layout, ..Position::new(0, Tick::ZERO, window, arrival, Origin::Reanchored, writer) };
        Self { source, old, new: old, pos, from, ignored: false, head: false, shortened: false, collided: false }
    }
}

/// Carries one plan through the map (spec section 4.8, steps 3-11 for a plan): its positions with the splits and anchors
/// at the junctions, openers moved onto extended heads, the arrivals, new heads' anchors, the re-check of I5, I6 and I9,
/// the anchors that change nothing dropped, the flags, the ids, the approvals of the parts that are `gone`, its attention
/// marks and overwritten ranges, and the frame. A position no id is left for is left out (reported `ids_exhausted`), and
/// the positions on either side of it, which now govern its frames, are flagged.
fn carry_plan(c: &Carry, plan: &VerticalPlan, walls: &[i64], frame: FrameChange, gone: &[u32], changes: &mut Vec<Change>) -> (VerticalPlan, PlanFollow) {
    let mut follow = PlanFollow { plan: plan.id, ..PlanFollow::default() };
    let mut entries = place(c, plan, &mut follow, changes);
    for (i, e) in entries.iter_mut().enumerate() {
        e.pos.arrival = carried_arrival(c.map, e, i == 0);
    }
    if let Some(picture) = c.picture {
        inherit(picture, &mut entries, c.writer);
    }
    for i in 1..entries.len() {
        let Some(previous) = entries.get(i - 1).map(|p| (p.new, p.pos.layout)) else { continue };
        let Some(e) = entries.get_mut(i) else { continue };
        let Some(frames) = recheck(e, previous, walls) else { continue };
        if e.source == Source::Original {
            changes.push(Change { code: "move_shortened", detail: json!({"plan": plan.id, "position": e.pos.id, "frames": frames}) });
        }
    }
    prune(&mut entries, plan, walls, &c.r0);
    let mut out = plan.clone();
    let rate = c.r1.rate;
    out.positions = entries.iter().map(|e| Position { at: rate.tick_of(e.new), ..e.pos.clone() }).collect();
    let spans = plan_spans(&out, &c.r1);
    let changed = changed_frames(c);
    let top = plan.positions.iter().map(|p| p.id).max().unwrap_or(0);
    let mut positions: Vec<Position> = Vec::with_capacity(entries.len());
    let mut lost = Vec::new();
    for ((e, mut p), span) in entries.iter().zip(std::mem::take(&mut out.positions)).zip(spans) {
        if e.source != Source::Original {
            let Ok(id) = alloc(&mut out.next_position_id, top) else {
                lost.push(e.new);
                continue;
            };
            p.id = id;
            if e.collided {
                add_flag(&mut p, Flag::PictureChanged);
            }
            // a split or anchor holds the window of its old frame, which a snapped or retimed piece puts on the nearest new
            // frame: that frame shows other media (spec section 4.8, step 2), so the window it holds there is approximate
            if e.source != Source::Same && !c.map.exact_at(e.old) {
                add_flag(&mut p, Flag::TimeApproximate);
            }
            follow.added.push(id);
            positions.push(p);
            continue;
        }
        let approximate = !e.head && !c.map.exact_at(e.old);
        let mut gained = false;
        if frame == FrameChange::Other || e.head || e.collided || meets(&changed, span) {
            gained |= add_flag(&mut p, Flag::PictureChanged);
        }
        if e.shortened {
            gained |= add_flag(&mut p, Flag::MoveShortened);
        }
        if approximate {
            gained |= add_flag(&mut p, Flag::TimeApproximate);
        }
        if approximate || e.shortened || e.head {
            p.reviewed = false;
            if p.locked {
                follow.locked.push(p.id);
            }
        }
        if gained {
            follow.flagged.push(p.id);
        }
        if p.at != e.pos.at {
            follow.moved.push(p.id);
        }
        positions.push(p);
    }
    if !lost.is_empty() {
        for &g in &lost {
            let i = positions.partition_point(|p| rate.frame_at(p.at) < g);
            for k in [i.checked_sub(1), Some(i)].into_iter().flatten() {
                let Some(p) = positions.get_mut(k) else { continue };
                if add_flag(p, Flag::PictureChanged) && !follow.added.contains(&p.id) {
                    follow.flagged.push(p.id);
                }
            }
        }
        changes.push(Change { code: "ids_exhausted", detail: json!({"kind": "position", "plan": plan.id, "frames": lost}) });
    }
    out.positions = positions;
    for a in plan.part_approvals.iter().filter(|a| gone.contains(&a.part)) {
        changes.push(Change { code: "approval_removed", detail: json!({"plan": plan.id, "part": a.part}) });
    }
    out.part_approvals.retain(|a| !gone.contains(&a.part));
    out.attention = carry_marks(c, plan, changes);
    out.overwritten = carry_overwritten(c, plan, &mut out.next_record_id, changes);
    // step 11: the frame the windows were made on, while the sequence's frame has another aspect; it holds the error
    // `frame_changed` until a person repairs the plan, or the frame returns to it
    let now = [c.r1.width, c.r1.height];
    out.framed_on = match plan.framed_on {
        Some(f) => (f != now).then_some(f),
        None => (frame == FrameChange::Other).then_some([c.r0.width, c.r0.height]),
    };
    // the same aspect rescales the windows, unless they belong to another frame already
    if let (FrameChange::Scale(k), None) = (frame, plan.framed_on) {
        let (r, output) = (c.r1, out.output);
        for p in out.positions.iter_mut().chain(out.overwritten.iter_mut().flat_map(|o| &mut o.positions)) {
            p.window = scale_window(p.window, k, &r, output);
            p.focus = p.focus.map(|point| scale_point(point, k, &r));
        }
        out.starting_window = out.starting_window.map(|w| scale_window(w, k, &r, output));
    }
    for list in [&mut follow.moved, &mut follow.removed, &mut follow.flagged, &mut follow.added, &mut follow.locked] {
        list.sort_unstable();
        list.dedup();
    }
    (out, follow)
}

/// The plan's positions with exact splits of every move at the junction frames inside it and exact anchors at the old
/// frames that start a piece after a junction outside every move, and at the last frame of a piece that ends at a
/// junction before the first position (spec section 4.8, step 3), in old-frame order. Every split's window and offset is
/// computed on the plan before the edit: splits of splits would add up their rounding.
fn augment(c: &Carry, plan: &VerticalPlan) -> Vec<Entry> {
    let r = c.r0;
    let rate = r.rate;
    let mut cuts: Vec<i64> = c.map.starts.iter().chain(&c.map.ends).copied().collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut entries = Vec::with_capacity(plan.positions.len());
    let mut moves: Vec<[i64; 2]> = Vec::new();
    for (k, p) in plan.positions.iter().enumerate() {
        let old = rate.frame_at(p.at);
        let ignored = k == 0 && matches!(p.arrival, Arrival::Move { .. });
        let mut entry = Entry { source: Source::Original, old, new: old, pos: p.clone(), from: None, ignored, head: false, shortened: false, collided: false };
        if let Some(span) = move_span(plan, &r, k) {
            let (s, f) = (span.start_frame, span.end_frame);
            moves.push([s, f]);
            let (offset, total) = span.split.map_or((0, span.frames), |x| (x.offset, x.total));
            let piece = |a: i64, b: i64| piece_of(b - a, span.curve, i64::from(offset) + (a - s), total);
            let inside = cuts.get(cuts.partition_point(|&x| x <= s)..cuts.partition_point(|&x| x < f)).unwrap_or_default();
            let mut from = s;
            for &m in inside {
                let window = clamp_window(window_at(plan, &r, rate.tick_of(m)).window, &r, plan.output);
                entries.push(Entry::added(Source::Split, m, window, Layout::Fill, piece(from, m), Some(from), c.writer));
                from = m;
            }
            if from != s {
                entry.pos.arrival = piece(from, f);
            }
            entry.from = Some(from);
        }
        entries.push(entry);
    }
    if !plan.positions.is_empty() {
        let frames: Vec<i64> = plan.positions.iter().map(|p| rate.frame_at(p.at)).collect();
        for &a in &c.map.starts {
            let i = moves.partition_point(|m| m[0] < a);
            let inside = i.checked_sub(1).and_then(|j| moves.get(j)).is_some_and(|m| a < m[1]);
            if inside || frames.binary_search(&a).is_ok() {
                continue;
            }
            let shown = window_at(plan, &r, rate.tick_of(a));
            entries.push(Entry::added(Source::Anchor, a, shown.window, shown.layout, Arrival::Cut, None, c.writer));
        }
        // the frames before the first position show its window, and it may be gone or no longer first: a piece that ends
        // there keeps that window with an anchor on its last frame (one that starts there has one already)
        let first = frames.first().copied().unwrap_or(0);
        for &a in c.map.ends.iter().take_while(|&&a| a < first) {
            if c.map.starts.binary_search(&a).is_err() {
                let shown = window_at(plan, &r, rate.tick_of(a));
                entries.push(Entry::added(Source::Anchor, a, shown.window, shown.layout, Arrival::Cut, None, c.writer));
            }
        }
    }
    entries.sort_by_key(|e| e.old);
    entries
}

/// The arrival of a piece of `frames` frames at `offset` of a `total`-frame move along `curve`: a cut when it is one frame
/// long, as an exact split makes it (spec section 3.6).
fn piece_of(frames: i64, curve: Curve, offset: i64, total: u32) -> Arrival {
    match (u32::try_from(frames), u32::try_from(offset)) {
        (Ok(frames), Ok(offset)) if frames >= 2 => Arrival::Move { move_frames: frames, curve, split_of: Some(SplitOf { offset, total }) },
        _ => Arrival::Cut,
    }
}

/// The entries of a plan on their new frames (spec section 4.8, steps 4 and 5): positions on removed content removed and
/// reported, an opener that sat on a wall moved onto its clip's extended head, and of two landing on one frame the one of
/// the earlier old frame kept, a position of the plan before one the hook adds (it keeps the person's id, label and
/// review). The frames after a dropped entry followed it, so the kept one is flagged when it shows another window or
/// layout.
fn place(c: &Carry, plan: &VerticalPlan, follow: &mut PlanFollow, changes: &mut Vec<Change>) -> Vec<Entry> {
    let mut placed = Vec::new();
    for mut e in augment(c, plan) {
        match c.map.map(e.old) {
            Some(g) => {
                e.new = g;
                placed.push(e);
            }
            None if e.source == Source::Original => removed(follow, changes, plan.id, &e),
            None => {}
        }
    }
    if let Some(picture) = c.picture {
        for head in picture.heads.iter().filter(|h| picture.old_walls.binary_search(&h.old_first).is_ok()) {
            if let Some(e) = placed.iter_mut().find(|e| e.source == Source::Original && e.old == head.old_first) {
                e.new = head.frames[0];
                e.head = true;
            }
        }
    }
    placed.sort_by_key(|e| (e.new, e.source != Source::Original, e.old));
    let mut out: Vec<Entry> = Vec::with_capacity(placed.len());
    for e in placed {
        if let Some(kept) = out.last_mut().filter(|last| last.new == e.new) {
            if !same_bits(&kept.pos.window, &e.pos.window) || kept.pos.layout != e.pos.layout {
                kept.collided = true;
            }
            if e.source == Source::Original {
                removed(follow, changes, plan.id, &e);
            }
            continue;
        }
        out.push(e);
    }
    out
}

/// Reports a position of the plan before the edit that the hook removed.
fn removed(follow: &mut PlanFollow, changes: &mut Vec<Change>, plan: u32, e: &Entry) {
    follow.removed.push(e.pos.id);
    if e.pos.locked {
        follow.locked.push(e.pos.id);
    }
    let detail = json!({"plan": plan, "position": e.pos.id, "frame": e.old, "locked": e.pos.locked});
    changes.push(Change { code: "position_removed", detail });
}

/// The arrival of an entry on its new frame (spec section 4.8, step 4). A move whose frames map with no junction between
/// its start and its end keeps its length and split in exact pieces; in a snapped or retimed piece it runs from its
/// start's new frame to its end's (a split piece keeps its share of the original move when its frames scale by a whole
/// ratio, else it becomes a whole move; below 2 frames, a cut). A move across a junction becomes a cut. The first
/// position's move, which shows nothing, stays while the position is first.
fn carried_arrival(map: &Mapping, e: &Entry, first: bool) -> Arrival {
    let arrival = e.pos.arrival;
    let Arrival::Move { move_frames, curve, split_of } = arrival else { return Arrival::Cut };
    if e.ignored {
        return if first { arrival } else { Arrival::Cut };
    }
    let Some(s) = e.from.filter(|s| !e.head && map.unbroken(*s, e.old)) else { return Arrival::Cut };
    if map.shift_over(s, e.old).is_some() {
        return arrival;
    }
    let frames = map.map(s).and_then(|start| u32::try_from(e.new.saturating_sub(start)).ok()).filter(|n| *n >= 2);
    match frames {
        Some(n) => Arrival::Move { move_frames: n, curve, split_of: scaled_split(split_of, move_frames, n) },
        None => Arrival::Cut,
    }
}

/// A split piece's share of its original move once its `before` frames became `after` frames: kept when they scale by a
/// whole ratio (a rate change by a whole factor shows every frame's window again), else None (a whole move).
fn scaled_split(split: Option<SplitOf>, before: u32, after: u32) -> Option<SplitOf> {
    let s = split?;
    let d = u64::from(before);
    let (offset, total) = (u64::from(s.offset) * u64::from(after), u64::from(s.total) * u64::from(after));
    if d == 0 || !offset.is_multiple_of(d) || !total.is_multiple_of(d) {
        return None;
    }
    let (offset, total) = (u32::try_from(offset / d).ok()?, u32::try_from(total / d).ok()?);
    (total <= VE_MAX_FRAMES && offset.checked_add(after)? <= total).then_some(SplitOf { offset, total })
}

/// A clip with positions whose head the edit extended, with no opener moved there, gets an anchor on its new first frame
/// holding the window that governs that frame, when that window comes from another clip's position (spec section 4.8,
/// step 6): its new head is then framed by a position of its own, and no frame changes.
fn inherit(picture: &Picture, entries: &mut Vec<Entry>, writer: Writer) {
    for head in &picture.heads {
        let at = head.frames[0];
        let own = |e: &Entry| e.source == Source::Original && Picture::leaf_in(&picture.old_leaves, e.old) == Some(head.clip);
        if entries.iter().any(|e| e.new == at) || !entries.iter().any(own) {
            continue;
        }
        let i = entries.partition_point(|e| e.new < at);
        let Some(governing) = i.checked_sub(1).and_then(|j| entries.get(j)).or(entries.get(i)) else { continue };
        if Picture::leaf_in(&picture.new_leaves, governing.new) == Some(head.clip) {
            continue;
        }
        let (window, layout) = (governing.pos.window, governing.pos.layout);
        let entry = Entry { new: at, ..Entry::added(Source::Same, head.old_first, window, layout, Arrival::Cut, None, writer) };
        entries.insert(i, entry);
    }
}

/// The re-check of one entry's move against its predecessor `(frame, layout)` and the new walls (spec section 4.8, step
/// 8): a move that overlaps its predecessor (I5), holds a wall (I6) or touches a `fit` position (I9) starts at the latest
/// of the predecessor and the walls inside it, as a whole move, or becomes a cut below 2 frames. Returns the move's
/// frames before and after when it changed.
fn recheck(e: &mut Entry, (previous, layout): (i64, Layout), walls: &[i64]) -> Option<[u32; 2]> {
    let Arrival::Move { move_frames, curve, .. } = e.pos.arrival else { return None };
    let f = e.new;
    let s = f.saturating_sub(i64::from(move_frames));
    let fit = layout == Layout::Fit || e.pos.layout == Layout::Fit;
    let wall = walls.partition_point(|&w| w <= f).checked_sub(1).and_then(|i| walls.get(i)).copied().filter(|&w| w > s);
    let start = s.max(previous).max(wall.unwrap_or(s));
    if !fit && start == s {
        return None;
    }
    let frames = if fit { None } else { u32::try_from(f.saturating_sub(start)).ok().filter(|n| *n >= 2) };
    e.pos.arrival = frames.map_or(Arrival::Cut, |n| Arrival::Move { move_frames: n, curve, split_of: None });
    e.shortened = true;
    Some([move_frames, frames.unwrap_or(0)])
}

/// Drops the anchors that change no frame and stand on no wall (spec section 4.8, step 10).
fn prune(entries: &mut Vec<Entry>, plan: &VerticalPlan, walls: &[i64], r: &Raster) {
    let mut i = 0;
    while i < entries.len() {
        let anchor = entries.get(i).is_some_and(|e| matches!(e.source, Source::Anchor | Source::Same) && walls.binary_search(&e.new).is_err());
        if anchor && shows_without(entries, i, plan, r) {
            entries.remove(i);
        } else {
            i += 1;
        }
    }
}

/// Whether the plan without entry `i` shows that entry's window and layout on its frame, and so on every frame: the entry
/// before it holds there (no move starts before the entry it follows), or, for the first entry, the next one's window
/// shows before it.
fn shows_without(entries: &[Entry], i: usize, plan: &VerticalPlan, r: &Raster) -> bool {
    let Some(e) = entries.get(i) else { return false };
    let shown = match i.checked_sub(1).and_then(|j| entries.get(j)).or_else(|| entries.get(i + 1)) {
        Some(other) => (other.pos.window, other.pos.layout),
        None => (plan.starting_window.unwrap_or_else(|| centred_largest(r, plan.output)), Layout::Fill),
    };
    same_bits(&shown.0, &e.pos.window) && shown.1 == e.pos.layout
}

/// The new frames whose picture changed: new content, retimed content, and the changes the picture knows of.
fn changed_frames(c: &Carry) -> Vec<[i64; 2]> {
    let mut out = c.map.fresh.clone();
    out.extend(c.map.pieces.iter().filter(|p| p.kind == PieceKind::Retimed).map(|p| p.claim));
    if let Some(picture) = c.picture {
        out.extend(picture.changed.iter().copied());
    }
    merge_ranges(out)
}

/// A plan's attention marks: points that move with their frame and go with removed content (reported).
fn carry_marks(c: &Carry, plan: &VerticalPlan, changes: &mut Vec<Change>) -> Vec<AttentionMark> {
    let rate = c.r0.rate;
    let mut marks = Vec::with_capacity(plan.attention.len());
    for m in &plan.attention {
        let f = rate.frame_at(m.at);
        match c.point(f, 0) {
            Some(g) => marks.push((g, AttentionMark { at: c.tick(g), ..m.clone() })),
            None => changes.push(Change { code: "attention_dropped", detail: json!({"plan": plan.id, "frame": f}) }),
        }
    }
    settle(marks)
}

/// A plan's overwritten ranges: each loses the frames that are gone and splits at every junction (the piece holding its
/// first surviving frame keeps its id and its positions, which move as points; the others take new record ids, and a
/// piece no id is left for goes, reported); a range with no surviving frame goes (reported).
fn carry_overwritten(c: &Carry, plan: &VerticalPlan, counter: &mut u32, changes: &mut Vec<Change>) -> Vec<OverwrittenRange> {
    let rate = c.r0.rate;
    let top = plan.overwritten.iter().map(|o| o.id).max().unwrap_or(0);
    let mut out = Vec::with_capacity(plan.overwritten.len());
    for o in &plan.overwritten {
        let images = c.images(o.from, o.to);
        if images.is_empty() {
            changes.push(Change { code: "overwritten_removed", detail: json!({"plan": plan.id, "range": o.id}) });
            continue;
        }
        let mut kept = Vec::with_capacity(o.positions.len());
        for p in &o.positions {
            if let Some(g) = c.point(rate.frame_at(p.at), 0) {
                kept.push((g, Position { at: c.tick(g), ..p.clone() }));
            }
        }
        let mut positions = Some(settle(kept));
        for (k, [x, y]) in images.into_iter().enumerate() {
            let id = if k == 0 { Ok(o.id) } else { alloc(counter, top) };
            let Ok(id) = id else {
                changes.push(Change { code: "ids_exhausted", detail: json!({"kind": "record", "plan": plan.id, "range": o.id, "frames": [x, y]}) });
                continue;
            };
            let held = positions.take().unwrap_or_default();
            out.push(OverwrittenRange { id, from: c.tick(x), to: c.tick(y), positions: held, ..o.clone() });
        }
    }
    out.sort_by_key(|o| c.r1.rate.frame_at(o.from));
    out
}

/// A window scaled by `k` onto raster `r` with the directed rounding of spec section 2.6.
fn scale_window(w: Window, k: f64, r: &Raster, out: OutputSize) -> Window {
    clamp_window(Window { cx: w.cx * k, cy: w.cy * k, h: w.h * k }, r, out)
}

/// A focus point scaled by `k`, on the grid and inside the frame of `r`.
fn scale_point([x, y]: [f64; 2], k: f64, r: &Raster) -> [f64; 2] {
    [q(x * k).max(0.0).min(f64::from(r.width)), q(y * k).max(0.0).min(f64::from(r.height))]
}

/// A rect `[x, y, w, h]` scaled by `k`, on the grid and inside the frame of `r` (its corner first, then its size inward).
fn scale_rect([x, y, w, h]: [f64; 4], k: f64, r: &Raster) -> [f64; 4] {
    let (fw, fh) = (f64::from(r.width), f64::from(r.height));
    let (x, y) = (q(x * k).max(0.0).min(fw), q(y * k).max(0.0).min(fh));
    [x, y, q(w * k).min(floor_g(fw - x)), q(h * k).min(floor_g(fh - y))]
}

/// Whether two windows are equal bit for bit.
fn same_bits(a: &Window, b: &Window) -> bool {
    a.cx.to_bits() == b.cx.to_bits() && a.cy.to_bits() == b.cy.to_bits() && a.h.to_bits() == b.h.to_bits()
}
