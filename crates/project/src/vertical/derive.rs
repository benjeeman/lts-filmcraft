//! Derived values (spec section 2.7): computed from the stored data, never stored, returned by every read. Tags, moving
//! and static ranges, the governing position, spans, shots with their status, events, the framing library, part statuses,
//! the issues of a range, and the per-frame rows of `vertical.windows`.
//!
//! [`Scene`] is what every derivation reads: the project (which clips show picture), the sequence (its tracks, cues and
//! settings), a vertical set (the stored one, or the planner's working copy), the raster and the effective walls.

use std::collections::BTreeMap;

use filmcraft_time::Tick;
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::area::{active_issues_in, active_rect};
use super::eval::{MoveSpan, limits, move_span, window_at};
use super::measure::{corner_deltas, move_numbers_of, speed_of};
use super::parts::{PartStatus, approval_status, part_status_in};
use super::regions::region_issues_in;
use super::split::move_label;
use super::types::{
    Arrival, Curve, Flag, Issue, Layout, Level, Limits, OutputSize, Phase, Position, Raster, Rect, Refusal, Sample, VerticalPlan, VerticalSet, Window,
};
use super::validate::plan_violations;
use super::walls::{Wall, WallSource, shown_frame, shows_picture, walls_in};
use crate::{Project, Sequence};

/// SAME's tolerance: a window within this many sequence px of the one shown before is no visible change.
const SAME_EPS: f64 = 1e-6;

/// The tag of a position, first match wins (spec section 2.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Tag {
    /// The first position.
    Start,
    /// Its window and layout equal the ones shown on the frame before, within 1e-6 px: no visible change.
    Same,
    /// It arrives by a move.
    Move,
    /// A cut on a wall frame.
    Cut,
    /// A cut off a wall: an in-shot jump.
    Jump,
}

/// What a read says about one position besides its stored fields.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionInfo {
    pub id: u32,
    pub frame: i64,
    pub tag: Tag,
    /// The `FIT` badge.
    pub fit: bool,
    /// The index of its shot in [`Derived::shots`].
    pub shot: usize,
    /// `[s + 1, f_j)`: the interpolated frames of its arrival; empty (`[f_j, f_j)`) for a cut.
    pub moving: [i64; 2],
    /// `[f_j, next change)`, the next change being the next position's first changed frame or the sequence end.
    #[serde(rename = "static")]
    pub still: [i64; 2],
    /// `moving ∪ static`; the first position's span also covers the frames before it.
    pub span: [i64; 2],
    /// The length of `static`.
    pub hold_frames: i64,
    /// The measured peak speed of its arrival in output px per frame (spec section 3.9); 0 for a cut.
    pub peak_speed: f64,
    /// The move's label ("Ramp 20/20", "Ease in-out · part 1/2"); None for a cut.
    pub curve_label: Option<String>,
}

/// A shot's status in a plan (spec section 2.7). `Carried` and `Starting` are the warning `shot_not_own`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShotStatus {
    /// A position sits on the shot's first frame.
    Own,
    /// The first frame shows a window governed by a position of another shot.
    Carried,
    /// The plan has no position.
    Starting,
}

/// The frames from one effective wall to the next (frame 0 opens the first), half-open, with its status in a plan.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shot {
    /// 1-based, in time order.
    pub number: u32,
    pub from: i64,
    pub to: i64,
    /// The sources of the wall that opens it (empty for the shot at frame 0).
    pub sources: Vec<WallSource>,
    pub status: ShotStatus,
    /// The position on its first frame.
    pub opener: Option<u32>,
    /// For a carried shot, the position that governs its first frame.
    pub carried_from: Option<u32>,
    /// The positions with a frame in the shot, in time order.
    pub positions: Vec<u32>,
    /// How many of them are reviewed.
    pub reviewed: u32,
    /// How many of them carry flags.
    pub flagged: u32,
    /// The frames of the plan's attention marks in the shot.
    pub attention: Vec<i64>,
}

/// One row of a plan's history (spec section 2.7, the Events view).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Event {
    /// The window holds over `[from, to)`.
    Hold { from: i64, to: i64, position: u32 },
    /// A cut on a wall at `frame`.
    Cut { frame: i64, position: u32 },
    /// A cut off a wall at `frame`.
    Jump { frame: i64, position: u32 },
    /// A move from `s` (`from`) to `f_j` (`to`), with its distance (largest corner displacement, output px) and peak speed.
    Move { from: i64, to: i64, position: u32, curve: Curve, frames: u32, distance: f64, peak_speed: f64 },
    /// The layout changes at `frame`: to `fit` when `fit`, else back to `fill`.
    Fit { frame: i64, position: u32, fit: bool },
}

/// One label of the framing library: the positions carrying it, in time order (spec section 5.11).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LibraryEntry {
    pub label: String,
    pub positions: Vec<u32>,
}

/// A part's status in one plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PartState {
    pub part: u32,
    pub status: PartStatus,
}

/// Everything a read derives for one plan.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Derived {
    pub plan: u32,
    pub raster: Raster,
    pub limits: Limits,
    pub walls: Vec<Wall>,
    pub positions: Vec<PositionInfo>,
    pub shots: Vec<Shot>,
    pub events: Vec<Event>,
    pub library: Vec<LibraryEntry>,
    pub parts: Vec<PartState>,
    /// Every issue of the plan that needs no threshold, in frame order.
    pub issues: Vec<Issue>,
}

/// What every derivation reads: the project, the sequence, its vertical data, its raster and its effective walls.
pub(crate) struct Scene<'a> {
    pub project: &'a Project,
    pub seq: &'a Sequence,
    pub set: Option<&'a VerticalSet>,
    pub r: Raster,
    pub walls: Vec<Wall>,
    /// The walls' frames, sorted.
    pub wall_frames: Vec<i64>,
}

impl<'a> Scene<'a> {
    /// The scene of `seq` with `set` as its vertical data.
    pub fn new(project: &'a Project, seq: &'a Sequence, set: Option<&'a VerticalSet>) -> Self {
        let r = raster(seq);
        let walls = walls_in(project, seq, set);
        let wall_frames = walls.iter().map(|w| r.rate.frame_at(w.at)).collect();
        Self { project, seq, set, r, walls, wall_frames }
    }

    /// The scene of `seq` with its stored vertical data.
    pub fn of(project: &'a Project, seq: &'a Sequence) -> Self {
        Self::new(project, seq, seq.vertical.as_deref())
    }

    /// Plan `id` of the set.
    pub fn plan(&self, id: u32) -> Option<&'a VerticalPlan> {
        self.set?.plans.iter().find(|p| p.id == id)
    }

    /// Whether an effective wall stands on `frame`.
    pub fn is_wall(&self, frame: i64) -> bool {
        self.wall_frames.binary_search(&frame).is_ok()
    }

    /// The effective wall on `frame`.
    pub fn wall_at(&self, frame: i64) -> Option<&Wall> {
        self.wall_frames.binary_search(&frame).ok().and_then(|i| self.walls.get(i))
    }

    /// The shots' bounds `[from, to)`, in time order: frame 0 opens the first, each wall the next; none when the sequence
    /// has no frame.
    pub fn shot_bounds(&self) -> Vec<[i64; 2]> {
        if self.r.end_frame <= 0 {
            return Vec::new();
        }
        let starts = std::iter::once(0).chain(self.wall_frames.iter().copied());
        let ends = self.wall_frames.iter().copied().chain(std::iter::once(self.r.end_frame));
        starts.zip(ends).map(|(a, b)| [a, b]).collect()
    }

    /// The index of the shot holding `frame` (frames past the end belong to the last shot).
    pub fn shot_index(&self, frame: i64) -> usize {
        self.wall_frames.partition_point(|&w| w <= frame)
    }

    /// The bounds of the shot holding `frame`.
    pub fn shot_of(&self, frame: i64) -> [i64; 2] {
        let i = self.shot_index(frame);
        let from = i.checked_sub(1).and_then(|j| self.wall_frames.get(j)).copied().unwrap_or(0);
        let to = self.wall_frames.get(i).copied().unwrap_or(self.r.end_frame.max(from));
        [from, to]
    }
}

/// The frames, move spans and tags of one plan's positions: what most derivations need.
pub(crate) struct Facts<'a> {
    pub plan: &'a VerticalPlan,
    /// Each position's frame.
    pub frames: Vec<i64>,
    /// The move into each position, when it is a valid one (`move_span`).
    pub spans: Vec<Option<MoveSpan>>,
    pub tags: Vec<Tag>,
}

impl<'a> Facts<'a> {
    pub fn new(scene: &Scene, plan: &'a VerticalPlan) -> Self {
        let r = scene.r;
        let frames: Vec<i64> = plan.positions.iter().map(|p| r.rate.frame_at(p.at)).collect();
        let spans: Vec<Option<MoveSpan>> = (0..plan.positions.len()).map(|k| move_span(plan, &r, k)).collect();
        let mut facts = Self { plan, frames, spans, tags: Vec::new() };
        facts.tags = (0..plan.positions.len()).map(|k| facts.tag_of(scene, k)).collect();
        facts
    }

    /// The tag of position `k`, first match wins.
    fn tag_of(&self, scene: &Scene, k: usize) -> Tag {
        if k == 0 {
            return Tag::Start;
        }
        let (Some(p), Some(&f)) = (self.plan.positions.get(k), self.frames.get(k)) else { return Tag::Jump };
        let before = window_at(self.plan, &scene.r, scene.r.rate.tick_of(f.saturating_sub(1)));
        if before.layout == p.layout && same_window(&before.window, &p.window) {
            return Tag::Same;
        }
        if self.spans.get(k).is_some_and(Option::is_some) {
            return Tag::Move;
        }
        if scene.is_wall(f) { Tag::Cut } else { Tag::Jump }
    }

    /// The tag of position `k`.
    pub fn tag(&self, k: usize) -> Tag {
        self.tags.get(k).copied().unwrap_or(Tag::Jump)
    }

    /// The frame of position `k` (0 past the end).
    pub fn frame(&self, k: usize) -> i64 {
        self.frames.get(k).copied().unwrap_or(0)
    }

    /// The index of the position on `frame`.
    pub fn index_at(&self, frame: i64) -> Option<usize> {
        self.frames.binary_search(&frame).ok()
    }

    /// `[s + 1, f_j)` for a move, `[f_j, f_j]` for a cut.
    pub fn moving(&self, k: usize) -> [i64; 2] {
        let f = self.frame(k);
        match self.spans.get(k).copied().flatten() {
            Some(span) => [span.start_frame.saturating_add(1).min(f), f],
            None => [f, f],
        }
    }

    /// The first frame position `k + 1` changes: `s + 1` for a move, its frame for a cut, the end after the last.
    pub fn next_change(&self, k: usize, end: i64) -> i64 {
        let next = k + 1;
        if next >= self.frames.len() {
            return end.max(self.frame(k));
        }
        match self.spans.get(next).copied().flatten() {
            Some(span) => span.start_frame.saturating_add(1),
            None => self.frame(next),
        }
    }

    /// `[f_j, next change)`.
    pub fn still(&self, k: usize, end: i64) -> [i64; 2] {
        let f = self.frame(k);
        [f, self.next_change(k, end).max(f)]
    }

    /// `moving ∪ static`; the first position's covers the frames before it.
    pub fn span(&self, k: usize, end: i64) -> [i64; 2] {
        let from = if k == 0 { 0 } else { self.moving(k)[0] };
        [from, self.still(k, end)[1]]
    }

    /// Whether `frame` is the arrival frame of a valid move.
    pub fn arrives_moving(&self, frame: i64) -> bool {
        self.index_at(frame).is_some_and(|k| self.spans.get(k).is_some_and(Option::is_some))
    }
}

/// Two windows within [`SAME_EPS`] in every component.
pub(crate) fn same_window(a: &Window, b: &Window) -> bool {
    (a.cx - b.cx).abs() <= SAME_EPS && (a.cy - b.cy).abs() <= SAME_EPS && (a.h - b.h).abs() <= SAME_EPS
}

/// The 16:9 frame a sequence's plans are evaluated on: its frame size and rate, and its end frame, the number of frames
/// its content covers (`frame_at(duration - 1) + 1`; 0 for an empty sequence). Ends saturate, so hostile ticks never
/// overflow.
pub fn raster(seq: &Sequence) -> Raster {
    let rate = seq.settings.frame_rate;
    let items = seq.video_tracks.iter().chain(&seq.audio_tracks).flat_map(|t| &t.items).map(|i| i.start.0.saturating_add(i.duration.0));
    let cues = seq.caption_tracks.iter().flat_map(|t| &t.captions).map(|c| c.start.0.saturating_add(c.duration.0));
    let end = items.chain(cues).fold(0i64, i64::max);
    let end_frame = if end > 0 { rate.frame_at(Tick(end.saturating_sub(1))).saturating_add(1) } else { 0 };
    Raster { width: seq.settings.width, height: seq.settings.height, rate, end_frame }
}

/// Everything derived for plan `plan` of `seq`; None when the plan does not exist.
pub fn derive_plan(project: &Project, seq: &Sequence, plan: u32) -> Option<Derived> {
    let scene = Scene::of(project, seq);
    let p = scene.plan(plan)?;
    let facts = Facts::new(&scene, p);
    let parts = scene.set.map(|s| s.parts.as_slice()).unwrap_or_default();
    let states = parts.iter().filter_map(|part| part_status_in(&scene, p, part).map(|status| PartState { part: part.id, status })).collect();
    Some(Derived {
        plan,
        raster: scene.r,
        limits: limits(&scene.r, p.output),
        walls: scene.walls.clone(),
        positions: positions_of(&scene, &facts),
        shots: shots_of(&scene, &facts),
        events: events_of(&scene, &facts),
        library: library(p),
        parts: states,
        issues: plan_issues(&scene, &facts),
    })
}

/// The governing position of `frame` (spec section 2.7), from `window_at` at the frame's start: the first position before
/// it (phase `Before`), the sample's position in a hold (phase `Hold`, the position on the frame included), none inside a
/// move or while the plan has no position. Captions take their anchor from it at a cue's first frame (lane L3b agrees).
pub fn governing(plan: &VerticalPlan, r: &Raster, frame: i64) -> Option<u32> {
    let s = window_at(plan, r, r.rate.tick_of(frame));
    match s.phase {
        Phase::Before | Phase::Hold => s.position,
        Phase::Move | Phase::Starting => None,
    }
}

/// The shots of plan `plan`, with their status.
pub fn shots(project: &Project, seq: &Sequence, plan: u32) -> Vec<Shot> {
    let scene = Scene::of(project, seq);
    scene.plan(plan).map(|p| shots_of(&scene, &Facts::new(&scene, p))).unwrap_or_default()
}

/// The shots of a plan in a scene.
pub(crate) fn shots_of(scene: &Scene, facts: &Facts) -> Vec<Shot> {
    let plan = facts.plan;
    let mut out = Vec::new();
    for (i, [from, to]) in scene.shot_bounds().into_iter().enumerate() {
        let lo = facts.frames.partition_point(|&f| f < from);
        let hi = facts.frames.partition_point(|&f| f < to);
        let inside = plan.positions.get(lo..hi).unwrap_or_default();
        let opener = facts.index_at(from).and_then(|k| plan.positions.get(k)).map(|p| p.id);
        let status = if plan.positions.is_empty() {
            ShotStatus::Starting
        } else if opener.is_some() {
            ShotStatus::Own
        } else {
            ShotStatus::Carried
        };
        let carried_from = (status == ShotStatus::Carried).then(|| governing(plan, &scene.r, from)).flatten();
        let attention = plan.attention.iter().map(|m| scene.r.rate.frame_at(m.at)).filter(|&f| f >= from && f < to).collect();
        let sources = i.checked_sub(1).and_then(|j| scene.walls.get(j)).map(|w| w.sources.clone()).unwrap_or_default();
        out.push(Shot {
            number: u32::try_from(i + 1).unwrap_or(u32::MAX),
            from,
            to,
            sources,
            status,
            opener,
            carried_from,
            positions: inside.iter().map(|p| p.id).collect(),
            reviewed: count(inside.iter().filter(|p| p.reviewed)),
            flagged: count(inside.iter().filter(|p| !p.flags.is_empty())),
            attention,
        });
    }
    out
}

/// How many items an iterator yields, as u32.
fn count<T>(items: impl Iterator<Item = T>) -> u32 {
    u32::try_from(items.count()).unwrap_or(u32::MAX)
}

/// The derived facts of every position.
pub(crate) fn positions_of(scene: &Scene, facts: &Facts) -> Vec<PositionInfo> {
    let end = scene.r.end_frame;
    let plan = facts.plan;
    let mut out = Vec::with_capacity(plan.positions.len());
    for (k, p) in plan.positions.iter().enumerate() {
        let still = facts.still(k, end);
        let span = facts.spans.get(k).copied().flatten();
        let peak_speed = span.map_or(0.0, |s| move_numbers_of(plan, &scene.r, &s).peak_speed);
        out.push(PositionInfo {
            id: p.id,
            frame: facts.frame(k),
            tag: facts.tag(k),
            fit: p.layout == Layout::Fit,
            shot: scene.shot_index(facts.frame(k)),
            moving: facts.moving(k),
            still,
            span: facts.span(k, end),
            hold_frames: still[1].saturating_sub(still[0]),
            peak_speed,
            curve_label: span.and_then(|_| move_label(plan, k)),
        });
    }
    out
}

/// The events of plan `plan`, in time order.
pub fn events(project: &Project, seq: &Sequence, plan: u32) -> Vec<Event> {
    let scene = Scene::of(project, seq);
    scene.plan(plan).map(|p| events_of(&scene, &Facts::new(&scene, p))).unwrap_or_default()
}

/// The events of a plan in a scene: per position its arrival (a cut, a jump or a move), its layout change, its hold.
pub(crate) fn events_of(scene: &Scene, facts: &Facts) -> Vec<Event> {
    let end = scene.r.end_frame;
    let plan = facts.plan;
    let mut out = Vec::new();
    let mut layout = plan.positions.first().map_or(Layout::Fill, |p| p.layout);
    for (k, p) in plan.positions.iter().enumerate() {
        let f = facts.frame(k);
        match facts.tag(k) {
            Tag::Cut => out.push(Event::Cut { frame: f, position: p.id }),
            Tag::Jump => out.push(Event::Jump { frame: f, position: p.id }),
            Tag::Move => {
                if let (Some(span), Arrival::Move { move_frames, curve, .. }) = (facts.spans.get(k).copied().flatten(), p.arrival) {
                    let numbers = move_numbers_of(plan, &scene.r, &span);
                    let distance = move_distance(plan, &span);
                    let (from, to, frames) = (span.start_frame, span.end_frame, move_frames);
                    out.push(Event::Move { from, to, position: p.id, curve, frames, distance, peak_speed: numbers.peak_speed });
                }
            }
            Tag::Start | Tag::Same => {}
        }
        if k > 0 && p.layout != layout {
            out.push(Event::Fit { frame: f, position: p.id, fit: p.layout == Layout::Fit });
        }
        layout = p.layout;
        let still = facts.still(k, end);
        let from = if k == 0 { 0 } else { still[0] };
        if still[1] > from {
            out.push(Event::Hold { from, to: still[1], position: p.id });
        }
    }
    out
}

/// The largest corner displacement of a whole move, output px (`D_out` of spec section 3.8).
pub(crate) fn move_distance(plan: &VerticalPlan, span: &MoveSpan) -> f64 {
    let (Some(a), Some(b)) = (plan.positions.get(span.from), plan.positions.get(span.to)) else { return 0.0 };
    corner_deltas(&a.window, &b.window, plan.output).map_or(0.0, |d| speed_of(&d))
}

/// The framing library of `plan`: for each non-empty label, in order of first appearance, its positions in time order.
pub fn library(plan: &VerticalPlan) -> Vec<LibraryEntry> {
    let mut out: Vec<LibraryEntry> = Vec::new();
    for p in plan.positions.iter().filter(|p| !p.label.is_empty()) {
        match out.iter_mut().find(|e| e.label == p.label) {
            Some(entry) => entry.positions.push(p.id),
            None => out.push(LibraryEntry { label: p.label.clone(), positions: vec![p.id] }),
        }
    }
    out
}

/// The framing `label` gives the shot holding `frame` (spec section 5.11): the window and layout of the last position
/// labelled `label` before the shot's first frame, else of the first one at or after the shot's end; `label_not_found
/// {label}` otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LabelSource {
    pub position: u32,
    pub window: Window,
    pub layout: Layout,
}

/// See [`LabelSource`]. `not_found {plan}` for a plan that does not exist.
pub fn from_label(project: &Project, seq: &Sequence, plan: u32, frame: i64, label: &str) -> Result<LabelSource, Refusal> {
    let scene = Scene::of(project, seq);
    let p = scene.plan(plan).ok_or_else(|| Refusal::new("not_found", format!("the sequence has no plan {plan}"), json!({"plan": plan})))?;
    from_label_in(&scene, p, frame, label)
}

/// The framing library's rule in a scene.
pub(crate) fn from_label_in(scene: &Scene, plan: &VerticalPlan, frame: i64, label: &str) -> Result<LabelSource, Refusal> {
    let [from, to] = scene.shot_of(frame);
    let rate = scene.r.rate;
    let labelled = |p: &&Position| p.label == label;
    let before = plan.positions.iter().filter(labelled).rfind(|p| rate.frame_at(p.at) < from);
    let source = before.or_else(|| plan.positions.iter().filter(labelled).find(|p| rate.frame_at(p.at) >= to));
    match source {
        Some(p) => Ok(LabelSource { position: p.id, window: p.window, layout: p.layout }),
        None => Err(Refusal::new("label_not_found", format!("no position is labelled {label:?} outside this shot"), json!({"label": label}))),
    }
}

/// The issues of plan `plan` that touch the frames `[from, to)`, plus every error-level issue of the plan: the facts of
/// spec section 7.6 that need no threshold (`shot_not_own`, the flags, `move_spans_candidate`, the region warnings,
/// `window_outside_active`, approvals changed, `arrival_ignored`, dissolves, `redundant_position` (a `SAME` position that
/// opens no shot), `subject_unknown`, `attention`, `nonsquare_source`).
pub fn issues(project: &Project, seq: &Sequence, plan: u32, from: i64, to: i64) -> Vec<Issue> {
    let scene = Scene::of(project, seq);
    let Some(p) = scene.plan(plan) else { return Vec::new() };
    let ranges = [[from, to]];
    let all = plan_issues_near(&scene, &Facts::new(&scene, p), Some(ranges.as_slice()));
    all.into_iter().filter(|i| touches(i, &ranges)).collect()
}

/// Whether an issue is an error, concerns the whole plan, or touches one of `ranges`.
pub(crate) fn touches(issue: &Issue, ranges: &[[i64; 2]]) -> bool {
    let Some([a, b]) = issue.frames else { return true };
    issue.level == Level::Error || ranges.iter().any(|&[from, to]| a < to && from < b.max(a.saturating_add(1)))
}

/// Whether a warning over frames `[a, b)` (or one inside them) can touch `near`: [`touches`]'s test, true without `near`.
/// What cannot touch is not computed, which is exact because a region's, an area's or an approval's warning lies
/// inside its own frames.
pub(crate) fn may_touch(near: Option<&[[i64; 2]]>, [a, b]: [i64; 2]) -> bool {
    near.is_none_or(|ranges| ranges.iter().any(|&[from, to]| a < to && from < b.max(a.saturating_add(1))))
}

/// An issue of a plan over frames `[from, to)`.
pub(crate) fn issue(code: &'static str, level: Level, plan: Option<u32>, frames: Option<[i64; 2]>, detail: Value) -> Issue {
    Issue { code, level, plan, frames, detail }
}

/// Every issue of a plan that needs no threshold, in frame order.
pub(crate) fn plan_issues(scene: &Scene, facts: &Facts) -> Vec<Issue> {
    plan_issues_near(scene, facts, None)
}

/// The issues of a plan that needs no threshold, in frame order, with every one that [`touches`] `near` (all of them
/// without `near`): the region, active-area and approval warnings that cannot touch it are not computed, since each one
/// costs a window per frame or a digest.
pub(crate) fn plan_issues_near(scene: &Scene, facts: &Facts, near: Option<&[[i64; 2]]>) -> Vec<Issue> {
    let plan = facts.plan;
    let id = Some(plan.id);
    let r = scene.r;
    let end = r.end_frame;
    let mut out = error_issues(scene, facts);
    let tracks = scene.set.map(|s| s.box_tracks.as_slice()).unwrap_or_default();
    // the candidates by frame, so a move finds the ones inside it by bisection (I10 keeps them sorted; this keeps bad data
    // cheap too)
    let mut candidates: Vec<(i64, f32)> = scene.set.iter().flat_map(|s| &s.walls.candidates).map(|c| (r.rate.frame_at(c.at), c.score)).collect();
    candidates.sort_by_key(|c| c.0);
    for (k, p) in plan.positions.iter().enumerate() {
        let f = facts.frame(k);
        let at = Some([f, f.saturating_add(1)]);
        for flag in &p.flags {
            let code = match flag {
                Flag::PictureChanged => "picture_changed",
                Flag::MoveShortened => "move_shortened",
                Flag::TimeApproximate => "time_approximate",
            };
            out.push(issue(code, Level::Warning, id, Some(facts.span(k, end)), json!({"position": p.id})));
        }
        if k == 0 && matches!(p.arrival, Arrival::Move { .. }) {
            out.push(issue("arrival_ignored", Level::Info, id, at, json!({"position": p.id})));
        }
        if facts.tag(k) == Tag::Same && !scene.is_wall(f) {
            out.push(issue("redundant_position", Level::Info, id, at, json!({"position": p.id})));
        }
        for subject in p.subjects.iter().filter(|s| !tracks.iter().any(|t| t.id == **s)) {
            out.push(issue("subject_unknown", Level::Info, id, at, json!({"position": p.id, "subject": subject})));
        }
        if let Some(span) = facts.spans.get(k).copied().flatten() {
            let lo = candidates.partition_point(|c| c.0 <= span.start_frame);
            let hi = candidates.partition_point(|c| c.0 <= span.end_frame);
            for &(cf, score) in candidates.get(lo..hi).unwrap_or_default() {
                let detail = json!({"position": p.id, "frame": cf, "score": f64::from(score)});
                out.push(issue("move_spans_candidate", Level::Warning, id, Some([span.start_frame, span.end_frame.saturating_add(1)]), detail));
            }
        }
    }
    out.extend(shot_issues(scene, facts));
    out.extend(dissolve_issues(scene, facts));
    out.extend(region_issues_in(scene, plan, near));
    out.extend(active_issues_in(scene, plan, near));
    out.extend(approval_issues(scene, plan, near));
    for range in &plan.overwritten {
        let frames = [r.rate.frame_at(range.from), r.rate.frame_at(range.to)];
        out.push(issue("agent_overwrote_reviewed", Level::Warning, id, Some(frames), json!({"range": range.id, "from": frames[0], "to": frames[1]})));
    }
    out.sort_by_key(|i| i.frames.map_or(i64::MIN, |[a, _]| a));
    out
}

/// The error-level issues of a plan, which block its vertical export and keep its parts from being reviewed:
/// `invalid_plan`, `frame_changed` (the windows were made on a frame of another aspect, spec section 4.8 step 11, until a
/// person repairs the plan), `outside_picture` (a stored window outside the current frame) and `nonsquare_source`.
pub(crate) fn error_issues(scene: &Scene, facts: &Facts) -> Vec<Issue> {
    let plan = facts.plan;
    let id = Some(plan.id);
    let r = scene.r;
    let mut out = Vec::new();
    let violations = plan_violations(scene, plan);
    if !violations.is_empty() {
        out.push(issue("invalid_plan", Level::Error, id, None, json!({"violations": violations})));
    }
    let now = [r.width, r.height];
    if let Some(from) = plan.framed_on.filter(|f| *f != now) {
        out.push(issue("frame_changed", Level::Error, id, None, json!({"from": from, "to": now})));
    }
    let (fw, fh) = (f64::from(r.width), f64::from(r.height));
    for (k, p) in plan.positions.iter().enumerate() {
        let f = facts.frame(k);
        let rect = p.window.rect(plan.output);
        let inside = rect.x >= -1e-6 && rect.y >= -1e-6 && rect.x + rect.w <= fw + 1e-6 && rect.y + rect.h <= fh + 1e-6;
        if !inside {
            out.push(issue("outside_picture", Level::Error, id, Some([f, f.saturating_add(1)]), json!({"position": p.id, "frame": f})));
        }
    }
    out.extend(nonsquare_issues(scene, plan.id));
    out
}

/// `shot_not_own` (warning) per shot that is carried or starting, and `attention` (info) per shot with marks.
fn shot_issues(scene: &Scene, facts: &Facts) -> Vec<Issue> {
    let id = Some(facts.plan.id);
    let mut out = Vec::new();
    for shot in shots_of(scene, facts) {
        let frames = Some([shot.from, shot.to]);
        if shot.status != ShotStatus::Own {
            let mut detail = Map::new();
            detail.insert("shot".into(), json!(shot.number));
            detail.insert("from".into(), json!(shot.from));
            detail.insert("to".into(), json!(shot.to));
            detail.insert("status".into(), json!(shot.status));
            if let Some(from) = shot.carried_from {
                detail.insert("carriedFrom".into(), json!(from));
            }
            out.push(issue("shot_not_own", Level::Warning, id, frames, Value::Object(detail)));
        }
        if !shot.attention.is_empty() {
            out.push(issue("attention", Level::Info, id, frames, json!({"shot": shot.number, "frames": shot.attention})));
        }
    }
    out
}

/// `wall_in_dissolve` (info) per effective wall under a video transition, and `move_in_dissolve` (info) per move that
/// overlaps one.
fn dissolve_issues(scene: &Scene, facts: &Facts) -> Vec<Issue> {
    let id = Some(facts.plan.id);
    let rate = scene.r.rate;
    let mut out = Vec::new();
    for track in scene.seq.video_tracks.iter().filter(|t| t.enabled) {
        for t in &track.transitions {
            let (a, b) = (rate.frame_at(t.start), shown_frame(rate, Tick(t.start.0.saturating_add(t.duration.0))));
            for &w in scene.wall_frames.iter().filter(|&&w| w >= a && w < b) {
                out.push(issue("wall_in_dissolve", Level::Info, id, Some([w, w + 1]), json!({"frame": w, "track": track.id.0, "transition": t.id.0})));
            }
            for (k, span) in facts.spans.iter().enumerate() {
                let Some(span) = span else { continue };
                let Some(p) = facts.plan.positions.get(k) else { continue };
                if span.start_frame < b && a <= span.end_frame {
                    let detail = json!({"position": p.id, "track": track.id.0, "transition": t.id.0});
                    out.push(issue("move_in_dissolve", Level::Info, id, Some([span.start_frame, span.end_frame.saturating_add(1)]), detail));
                }
            }
        }
    }
    out
}

/// `part_changed_since_approval` and `part_output_changed_since_approval` (warnings) per approved part (that may touch
/// `near`).
fn approval_issues(scene: &Scene, plan: &VerticalPlan, near: Option<&[[i64; 2]]>) -> Vec<Issue> {
    let rate = scene.r.rate;
    let mut out = Vec::new();
    for part in scene.set.iter().flat_map(|s| &s.parts) {
        let frames = [rate.frame_at(part.from), rate.frame_at(part.to)];
        let Some(approval) = plan.part_approvals.iter().find(|a| a.part == part.id).filter(|_| may_touch(near, frames)) else { continue };
        let code = match approval_status(scene, plan, part, approval) {
            PartStatus::ChangedSinceApproval => "part_changed_since_approval",
            PartStatus::OutputChangedSinceApproval => "part_output_changed_since_approval",
            _ => continue,
        };
        out.push(issue(code, Level::Warning, Some(plan.id), Some(frames), json!({"part": part.id})));
    }
    out
}

/// `nonsquare_source` (error) per media item with non-square pixels (from the file or `Interpretation.par`) that a picture
/// clip on an enabled video track shows, over the frames of its clips.
fn nonsquare_issues(scene: &Scene, plan: u32) -> Vec<Issue> {
    let rate = scene.r.rate;
    let mut by_item: BTreeMap<u64, ([u32; 2], i64, i64)> = BTreeMap::new();
    for track in scene.seq.video_tracks.iter().filter(|t| t.enabled) {
        for item in track.items.iter().filter(|i| i.enabled && shows_picture(scene.project, i)) {
            let Some(clip) = scene.project.item(item.item).and_then(|i| i.as_media()) else { continue };
            let par = clip.interpret.par.or(clip.info.video.as_ref().map(|v| v.par)).unwrap_or((1, 1));
            if par.0 == par.1 || par.0 == 0 || par.1 == 0 {
                continue;
            }
            let (a, b) = (rate.frame_at(item.start), rate.frame_at(Tick(item.start.0.saturating_add(item.duration.0))));
            let entry = by_item.entry(item.item.0).or_insert(([par.0, par.1], a, b));
            entry.1 = entry.1.min(a);
            entry.2 = entry.2.max(b);
        }
    }
    let detail = |item: u64, par: [u32; 2]| json!({"item": item, "par": par});
    by_item.into_iter().map(|(item, (par, a, b))| issue("nonsquare_source", Level::Error, Some(plan), Some([a, b]), detail(item, par))).collect()
}

/// One frame of `vertical.windows` (spec section 7.4).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRow {
    pub frame: i64,
    pub tick: Tick,
    pub window: Window,
    pub rect: Rect,
    pub layout: Layout,
    /// `d` the starting window, `b` before the first position, `s` the first position's frame, `h` static, `m` moving,
    /// `c` arrival by a cut on a wall, `j` arrival by an in-shot jump, `f` a fit frame.
    pub state: char,
    /// The governing position; inside a move, the one it departs from.
    pub position: Option<u32>,
    pub u: f64,
    pub e: f64,
    /// The arriving speed in output px per frame (spec section 3.9).
    pub speed: f64,
    pub upscale: f64,
    /// The governing position's focus point.
    pub focus: Option<[f64; 2]>,
    /// The active rect.
    pub active: [f64; 4],
}

/// The rows of frames `from, from + step, ...` below `to` of plan `plan` (`step` at least 1), inside the sequence. Empty
/// for a plan that does not exist.
pub fn frame_rows(project: &Project, seq: &Sequence, plan: u32, from: i64, to: i64, step: i64) -> Vec<FrameRow> {
    let scene = Scene::of(project, seq);
    let Some(p) = scene.plan(plan) else { return Vec::new() };
    let facts = Facts::new(&scene, p);
    let r = scene.r;
    let step = step.max(1);
    let (from, to) = (from.max(0), to.min(r.end_frame));
    let mut rows = Vec::new();
    let mut f = from;
    while f < to {
        let s = window_at(p, &r, r.rate.tick_of(f));
        let speed = arriving_speed(&scene, &facts, f, &s);
        rows.push(FrameRow {
            frame: f,
            tick: r.rate.tick_of(f),
            window: s.window,
            rect: s.window.rect(p.output),
            layout: s.layout,
            state: state_of(&facts, f, &s),
            position: s.position,
            u: s.u,
            e: s.e,
            speed,
            upscale: upscale_of(&s, &r, p.output),
            focus: governing(p, &r, f).and_then(|g| p.positions.iter().find(|x| x.id == g)).and_then(|x| x.focus),
            active: scene.set.map_or([0.0, 0.0, f64::from(r.width), f64::from(r.height)], |set| active_rect(set, &r, r.rate.tick_of(f))),
        });
        let Some(next) = f.checked_add(step) else { break };
        f = next;
    }
    rows
}

/// The state character of `frame` in a plan.
fn state_of(facts: &Facts, frame: i64, s: &Sample) -> char {
    if facts.plan.positions.is_empty() {
        return 'd';
    }
    if s.phase == Phase::Before {
        return 'b';
    }
    if s.layout == Layout::Fit {
        return 'f';
    }
    if let Some(k) = facts.index_at(frame) {
        return match facts.tag(k) {
            Tag::Start => 's',
            Tag::Cut => 'c',
            Tag::Jump => 'j',
            Tag::Same | Tag::Move => 'h',
        };
    }
    if s.phase == Phase::Move && s.u > 0.0 && s.u < 1.0 { 'm' } else { 'h' }
}

/// The upscale shown on a frame: `OH / h` in fill, `OW / W` (a downscale) in fit.
pub(crate) fn upscale_of(s: &Sample, r: &Raster, out: OutputSize) -> f64 {
    match s.layout {
        Layout::Fill => s.window.upscale(out),
        Layout::Fit => f64::from(out.width) / f64::from(r.width.max(1)),
    }
}

/// The speed arriving at `frame` (spec section 3.9): the largest corner displacement from the frame before, on a frame a
/// valid move arrives at or moves through; 0 on a cut, a fit frame and a hold.
pub(crate) fn arriving_speed(scene: &Scene, facts: &Facts, frame: i64, s: &Sample) -> f64 {
    if frame <= 0 || !(s.phase == Phase::Move || facts.arrives_moving(frame)) {
        return 0.0;
    }
    let before = window_at(facts.plan, &scene.r, scene.r.rate.tick_of(frame - 1));
    if before.layout != Layout::Fill || s.layout != Layout::Fill {
        return 0.0;
    }
    corner_deltas(&before.window, &s.window, facts.plan.output).map_or(0.0, |d| speed_of(&d))
}

/// The targets of `vertical.goTo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GoTo {
    PrevPosition,
    NextPosition,
    PrevWall,
    NextWall,
    /// The previous window break (a `CUT` or a `JUMP`).
    PrevBreak,
    NextBreak,
    /// The next attention mark.
    NextAttention,
}

/// The frame `vertical.goTo` moves the playhead to from `frame`, strictly before or after it; None when there is none.
pub fn go_to(project: &Project, seq: &Sequence, plan: u32, frame: i64, to: GoTo) -> Option<i64> {
    let scene = Scene::of(project, seq);
    let p = scene.plan(plan)?;
    let facts = Facts::new(&scene, p);
    let breaks: Vec<i64> = (0..p.positions.len()).filter(|&k| matches!(facts.tag(k), Tag::Cut | Tag::Jump)).map(|k| facts.frame(k)).collect();
    let marks: Vec<i64> = p.attention.iter().map(|m| scene.r.rate.frame_at(m.at)).collect();
    let prev = |list: &[i64]| list.iter().copied().filter(|&f| f < frame).max();
    let next = |list: &[i64]| list.iter().copied().filter(|&f| f > frame).min();
    match to {
        GoTo::PrevPosition => prev(&facts.frames),
        GoTo::NextPosition => next(&facts.frames),
        GoTo::PrevWall => prev(&scene.wall_frames),
        GoTo::NextWall => next(&scene.wall_frames),
        GoTo::PrevBreak => prev(&breaks),
        GoTo::NextBreak => next(&breaks),
        GoTo::NextAttention => next(&marks),
    }
}
