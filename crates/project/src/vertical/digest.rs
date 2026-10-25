//! Content digests (spec section 2.6): FNV-1a 64 over canonical bytes (`serde_json::to_vec` of the value, in struct field
//! order, id counters omitted), printed `fnv1a64:` and 16 lowercase hex digits, the policy's own format. Equal content gives
//! an equal digest, so a digest taken before an undo holds again after it.

use filmcraft_time::Tick;
use serde::Serialize;

use super::derive::Scene;
use super::eval::window_at;
use super::types::{
    ActiveArea, Annotation, Arrival, AttentionMark, BoxTrack, CaptionAnchor, Filter, FitBackground, Layout, OutputSize, OverwrittenRange, Part, PartApproval,
    Phase, Position, ProtectedRegion, VerticalCaptions, VerticalPlan, VerticalSet, WallSet, Window,
};
use super::walls::WallSource;
use crate::{Label, Project, Sequence};

/// FNV-1a 64: its offset basis and its prime, as the policy's `targeting_digest` uses them.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 of `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    fold(FNV_OFFSET, bytes)
}

/// `fnv1a64:` and the 16 lowercase hex digits of `hash`.
pub fn digest_text(hash: u64) -> String {
    format!("fnv1a64:{hash:016x}")
}

/// The plan's `digest` (the `expect` of every write naming the plan): the plan (positions, settings, approvals, attention
/// marks, overwritten ranges, the frame of another aspect its windows were made on) and the set's walls, parts and
/// regions. Not the id counters, annotations, box tracks or active areas.
pub fn plan_digest(seq: &Sequence, plan: &VerticalPlan) -> String {
    let set = seq.vertical.as_deref();
    let content = PlanContent {
        id: plan.id,
        name: &plan.name,
        output: plan.output,
        filter: plan.filter,
        starting_window: plan.starting_window,
        captions: &plan.captions,
        fit_background: plan.fit_background,
        positions: &plan.positions,
        part_approvals: &plan.part_approvals,
        attention: &plan.attention,
        overwritten: &plan.overwritten,
        framed_on: plan.framed_on,
    };
    let parts = set.map(|s| s.parts.as_slice()).unwrap_or_default();
    let regions = set.map(|s| s.regions.as_slice()).unwrap_or_default();
    digest_of(&PlanScope { plan: content, walls: set.map(|s| &s.walls), parts, regions })
}

/// The set's `dataDigest`: its annotations, box tracks and active areas.
pub fn data_digest(set: &VerticalSet) -> String {
    digest_of(&DataScope { annotations: &set.annotations, box_tracks: &set.box_tracks, active_area: &set.active_area })
}

/// The set's `wallsDigest`: the stored wall set.
pub fn walls_digest(set: &VerticalSet) -> String {
    digest_of(&set.walls)
}

/// The sequence's `effectiveWallsDigest` (the `expectWalls` guard): the effective walls of [`walls`](super::walls::walls)
/// with every source (spec section 2.6), not their scores: a measurement that only re-scores a cut moves no wall. It
/// changes with a 16:9 edit that moves an edit point.
pub fn effective_walls_digest(project: &Project, seq: &Sequence) -> String {
    effective_walls_digest_in(&Scene::of(project, seq))
}

/// The effective-walls digest of a scene (the planner's working copy, or the stored set).
pub(crate) fn effective_walls_digest_in(scene: &Scene) -> String {
    let walls: Vec<WallView> = scene.walls.iter().map(|w| WallView { at: w.at, sources: &w.sources }).collect();
    digest_of(&walls)
}

/// The `rangeDigest` of the frames `[from, to)` of `plan`, its framing digest (the `expectRange` guard and an approval's
/// `framingDigest`): the positions with a frame in the range, each with the fields whose change is a change (spec section
/// 7.5: id, frame, window, layout, arrival, caption anchor, label, colour, tags, note, subjects, focus, confidence and
/// risk; not `reviewed`, `locked`, flags, origin, `modifiedBy` or `proposed`, so locking or reviewing a part keeps its
/// approval), the position governing frame `from` (its id, window, layout and arrival), the effective walls in `[from,
/// to]` with their sources, and the parts and regions overlapping the range. Not the counters, attention marks,
/// approvals, annotations, box tracks or active areas.
///
/// Two additions keep it covering every input of the range's frames: when the range ends inside a move, the position the
/// move arrives at (its id, window, layout and arrival), and, while the plan has no position, the window it shows. The
/// range itself is part of the content.
pub fn range_digest(project: &Project, seq: &Sequence, plan: &VerticalPlan, from: i64, to: i64) -> String {
    range_digest_in(&Scene::of(project, seq), plan, from, to)
}

/// The range digest of a plan in a scene.
pub(crate) fn range_digest_in(scene: &Scene, plan: &VerticalPlan, from: i64, to: i64) -> String {
    let r = scene.r;
    let rate = r.rate;
    let in_range = |f: i64| f >= from && f < to;
    let positions: Vec<FramingView> = plan.positions.iter().filter(|p| in_range(rate.frame_at(p.at))).map(FramingView::of).collect();
    let view = |id: Option<u32>| id.and_then(|id| plan.positions.iter().find(|p| p.id == id)).map(PositionView::of);
    let first = window_at(plan, &r, rate.tick_of(from));
    let last = window_at(plan, &r, rate.tick_of(to.saturating_sub(1)));
    let arriving = view(last.next).filter(|p| !in_range(rate.frame_at(p.at)));
    let starting = (first.phase == Phase::Starting).then_some(first.window);
    let walls = scene.walls.iter().filter(|w| (from..=to).contains(&rate.frame_at(w.at))).map(|w| WallView { at: w.at, sources: &w.sources }).collect();
    let overlapping = |a: Tick, b: Tick| rate.frame_at(a) < to && rate.frame_at(b) > from;
    let parts = scene.set.iter().flat_map(|s| &s.parts).filter(|p| overlapping(p.from, p.to)).map(PartView::of).collect();
    let regions = scene.set.iter().flat_map(|s| &s.regions).filter(|g| overlapping(g.from, g.to)).collect();
    digest_of(&RangeScope { from, to, positions, governing: view(first.position), arriving, starting, walls, parts, regions })
}

/// The set's `partsDigest`, the `expect` of `vertical.setParts` (the spec names no guard for it): the parts as they print.
pub fn parts_digest(set: &VerticalSet) -> String {
    digest_of(&set.parts)
}

/// The set's `regionsDigest`, the `expect` of `vertical.setRegions`: the protected regions as they print.
pub fn regions_digest(set: &VerticalSet) -> String {
    digest_of(&set.regions)
}

/// What a range digest covers.
#[derive(Serialize)]
struct RangeScope<'a> {
    from: i64,
    to: i64,
    positions: Vec<FramingView<'a>>,
    governing: Option<PositionView>,
    arriving: Option<PositionView>,
    starting: Option<Window>,
    walls: Vec<WallView<'a>>,
    parts: Vec<PartView<'a>>,
    regions: Vec<&'a ProtectedRegion>,
}

/// A position as far as its framing goes: the fields whose change is a change (spec section 7.5), in the order a position
/// prints them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FramingView<'a> {
    id: u32,
    at: Tick,
    window: Window,
    layout: Layout,
    arrival: Arrival,
    caption_anchor: Option<CaptionAnchor>,
    label: &'a str,
    color: Option<Label>,
    tags: &'a [String],
    note: &'a str,
    subjects: &'a [String],
    focus: Option<[f64; 2]>,
    confidence: Option<f64>,
    risk: &'a str,
}

impl<'a> FramingView<'a> {
    fn of(p: &'a Position) -> Self {
        Self {
            id: p.id,
            at: p.at,
            window: p.window,
            layout: p.layout,
            arrival: p.arrival,
            caption_anchor: p.caption_anchor,
            label: &p.label,
            color: p.color,
            tags: &p.tags,
            note: &p.note,
            subjects: &p.subjects,
            focus: p.focus,
            confidence: p.confidence,
            risk: &p.risk,
        }
    }
}

/// A position as far as the frames after it depend on it: id, frame, window, layout and arrival.
#[derive(Serialize)]
struct PositionView {
    id: u32,
    at: Tick,
    window: Window,
    layout: Layout,
    arrival: Arrival,
}

impl PositionView {
    fn of(p: &Position) -> Self {
        Self { id: p.id, at: p.at, window: p.window, layout: p.layout, arrival: p.arrival }
    }
}

/// An effective wall without its score: re-measuring a cut reframes nothing.
#[derive(Serialize)]
struct WallView<'a> {
    at: Tick,
    sources: &'a [WallSource],
}

/// A part as far as framing depends on it: its id, name and range (its fades and metadata are the output digest's).
#[derive(Serialize)]
struct PartView<'a> {
    id: u32,
    name: &'a str,
    from: Tick,
    to: Tick,
}

impl<'a> PartView<'a> {
    fn of(p: &'a Part) -> Self {
        Self { id: p.id, name: &p.name, from: p.from, to: p.to }
    }
}

/// What a plan's digest covers: the plan without its id counters, and the walls, parts and regions of its sequence's set
/// (null and empty for a sequence without one).
#[derive(Serialize)]
struct PlanScope<'a> {
    plan: PlanContent<'a>,
    walls: Option<&'a WallSet>,
    #[serde(skip_serializing_if = "is_empty")]
    parts: &'a [Part],
    #[serde(skip_serializing_if = "is_empty")]
    regions: &'a [ProtectedRegion],
}

/// A plan as it prints in the project file, without `nextPositionId` and `nextRecordId`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanContent<'a> {
    id: u32,
    name: &'a str,
    output: OutputSize,
    filter: Filter,
    #[serde(skip_serializing_if = "Option::is_none")]
    starting_window: Option<Window>,
    captions: &'a VerticalCaptions,
    fit_background: FitBackground,
    positions: &'a [Position],
    #[serde(skip_serializing_if = "is_empty")]
    part_approvals: &'a [PartApproval],
    #[serde(skip_serializing_if = "is_empty")]
    attention: &'a [AttentionMark],
    #[serde(skip_serializing_if = "is_empty")]
    overwritten: &'a [OverwrittenRange],
    #[serde(skip_serializing_if = "Option::is_none")]
    framed_on: Option<[u32; 2]>,
}

/// What the data digest covers: the set's annotations, box tracks and active areas, as they print in the project file.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DataScope<'a> {
    #[serde(skip_serializing_if = "is_empty")]
    annotations: &'a [Annotation],
    #[serde(skip_serializing_if = "is_empty")]
    box_tracks: &'a [BoxTrack],
    #[serde(skip_serializing_if = "is_empty")]
    active_area: &'a [ActiveArea],
}

/// The digest of `value`'s canonical bytes. They stream into the hash as they print, so even a set with 50 000 box-track
/// keys allocates no buffer.
pub(crate) fn digest_of<T: Serialize>(value: &T) -> String {
    let mut hasher = Fnv1a(FNV_OFFSET);
    // Printing plain data (string keys, numbers or null) into a writer that never fails cannot fail; were it to, the hash
    // would cover the bytes printed before the failure, still a pure function of the value.
    let _ = serde_json::to_writer(&mut hasher, value);
    digest_text(hasher.0)
}

/// Whether a borrowed list is empty: the `skip_serializing_if` of the views' lists, as `Vec::is_empty` is the stored
/// types'.
fn is_empty<T>(items: &&[T]) -> bool {
    items.is_empty()
}

/// `hash` folded over `bytes`, one byte at a time: xor, then multiply by the prime.
fn fold(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME))
}

/// The running FNV-1a 64 state, written into by `serde_json`.
struct Fnv1a(u64);

impl std::io::Write for Fnv1a {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = fold(self.0, bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
